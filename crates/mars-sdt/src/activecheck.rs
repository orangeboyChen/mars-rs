//! `mars/sdt/src/activecheck/` — the four checks of one diagnosis.
//!
//! `basechecker.cc` is [`Check`], and the four classes the C++ derives from it
//! — `PingChecker`, `DnsChecker`, `HttpChecker`, `TcpChecker` — are the four
//! checks of one [`Check`]: each walks the hosts of the request, asks [`Ask`]
//! for the probe the C++ would have opened a socket for, and records one
//! [`CheckResultProfile`] per host. The socket is the host's, so what is here
//! is the loop around it, the error code the answer turns into, and the
//! timeout that spends — all of it the C++'s.
//!
//! Not ported: the `#if defined(ANDROID) || defined(__APPLE__)` around
//! `PingChecker::StartDoCheck` (whether a host can ping is its own business
//! here, and a ping it cannot send is answered [`crate::checkimpl::Answer::Nothing`], like any
//! other probe nobody made), and the three check types the C++ has no checker
//! for — `kNewDnsCheck`, `kTracerouteCheck`, `kReqBufCheck` — which
//! [`Check::start_do_check`] leaves alone.

use crate::checkimpl::{Ask, Query};
use crate::constants::{
    DEFAULT_DNS_TIMEOUT, DEFAULT_HTTP_HOST, DEFAULT_PING_COUNT, DEFAULT_PING_HOST,
    DEFAULT_TCP_CONN_TIMEOUT, UNUSE_TIMEOUT,
};
use crate::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use crate::sdt::{CheckIPPort, CheckIPPorts, CheckStatus, NetCheckType, TcpErrCode};
use crate::sdt_core::CancelHandle;

/// Why a check stopped walking the hosts of one link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// It walked all of them.
    Done,
    /// `is_canceled_` — the C++ `return`s from `__DoCheck`, so the link that
    /// would have followed is not walked either.
    Cancelled,
    /// The timeout is used up: the C++ `break`s out of the loop it is in, and
    /// the second link of the request is walked anyway.
    Timeout,
}

/// `BaseChecker` — what the four checks of one run have in common: whether the
/// run was cancelled, and the timeout it still has to spend.
///
/// The C++ keeps `is_canceled_` in the checker and what is left of the timeout
/// in the request. This port keeps both here: the request's `total_timeout` is
/// `0` for "no timeout was given" — [`CheckRequestProfile::timeout`] answers
/// [`UNUSE_TIMEOUT`] for it — so "it is used up" has nowhere to live in it,
/// and one [`CancelHandle`] is shared by the run and by whoever has to stop it.
#[derive(Debug, Clone)]
pub struct Check {
    /// `is_canceled_`.
    cancel: CancelHandle,
    /// What is left of `total_timeout`.
    remaining: u32,
}

impl Check {
    /// `BaseChecker::BaseChecker()` — a run nothing has cancelled, with
    /// `timeout` still to spend. `0` — the port's "no timeout was given" — is
    /// [`UNUSE_TIMEOUT`], the way [`CheckRequestProfile::timeout`] reads it.
    pub fn new(cancel: CancelHandle, timeout: u32) -> Self {
        Self {
            cancel,
            remaining: if timeout == 0 { UNUSE_TIMEOUT } else { timeout },
        }
    }

    /// `BaseChecker::CancelDoCheck()` — what the C++'s destructor does as well.
    ///
    /// The flag is the run's own, shared with every [`CancelHandle`] handed out
    /// before it: the check that is blocked on a socket is the one that sets
    /// it, which is the C++'s own reason for having a handle at all.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Whether the run was cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// What is left of the timeout: [`UNUSE_TIMEOUT`] for a run that was
    /// started without one, `0` for one that has spent it.
    ///
    /// The request's own `total_timeout` is left as it was handed over, so this
    /// is where a caller sees the timeout go.
    pub fn remaining(&self) -> u32 {
        self.remaining
    }

    /// `BaseChecker::StartDoCheck(_check_request)` — `false`, the C++'s `0`,
    /// when the run stops **here**: `total_timeout <= 0` sets
    /// `check_status = kCheckFinish` and the check does not run at all.
    pub fn start_do_check(
        &mut self,
        kind: NetCheckType,
        request: &mut CheckRequestProfile,
        ask: &mut Ask,
        network_type: i32,
        cgi: &str,
    ) -> bool {
        if self.remaining == 0 {
            request.check_status = CheckStatus::CheckFinish;
            return false;
        }
        match kind {
            NetCheckType::PingCheck => self.ping_check(request, ask, network_type),
            NetCheckType::DnsCheck => self.dns_check(request, ask, network_type),
            NetCheckType::TcpCheck => self.tcp_check(request, ask, network_type),
            NetCheckType::HttpCheck => self.http_check(request, ask, network_type, cgi),
            // the C++ has no checker for these, so nothing is checked
            NetCheckType::NewDnsCheck
            | NetCheckType::TracerouteCheck
            | NetCheckType::ReqBufCheck => {}
        }
        true
    }

    /// `DnsChecker::__DoCheck(_check_request)` — the host names of the request,
    /// long link first: the C++ writes the same loop twice, and so does this,
    /// once per link.
    fn dns_check(&mut self, request: &mut CheckRequestProfile, ask: &mut Ask, network_type: i32) {
        let longlink = hosts_of(&request.longlink_items);
        if self.dns_check_hosts(&longlink, request, ask, network_type) == Stop::Cancelled {
            return;
        }
        let shortlink = hosts_of(&request.shortlink_items);
        self.dns_check_hosts(&shortlink, request, ask, network_type);
    }

    /// One walk of the DNS check: a resolve per host name.
    fn dns_check_hosts(
        &mut self,
        hosts: &[String],
        request: &mut CheckRequestProfile,
        ask: &mut Ask,
        network_type: i32,
    ) -> Stop {
        for domain in hosts {
            if self.cancel.is_cancelled() {
                return Stop::Cancelled;
            }

            let mut profile = CheckResultProfile::of(NetCheckType::DnsCheck);
            profile.domain_name = domain.clone();
            profile.network_type = network_type;

            // the C++'s `UNUSE_TIMEOUT == total_timeout ? DEFAULT_DNS_TIMEOUT : total_timeout`
            let timeout_ms = self.probe_timeout(DEFAULT_DNS_TIMEOUT);
            let answer = ask.ask(Query::Dns {
                domain: domain.clone(),
                timeout_ms,
            });
            let (error_code, rtt, ips) = answer.dns();
            profile.error_code = error_code;
            profile.rtt = rtt;

            // the C++'s `if (0 == ret)`, and its `ipinfo.size` inside it: a
            // resolve that failed takes no address from the answer, not even
            // one the resolver filled in on the way out, and a resolve that
            // worked with no address at all is what the C++ logs an error
            // about — there is nothing to log here, so the profile keeps its
            // empty `ip1` / `ip2`.
            if error_code == 0 {
                if let Some(ip1) = ips.first() {
                    profile.ip1 = ip1.clone();
                }
                if let Some(ip2) = ips.get(1) {
                    profile.ip2 = ip2.clone();
                }
            }

            request.checkresult_profiles.push(profile);
            // the C++'s `ret >= 0 ? kCheckContinue : kCheckFinish`
            request.check_status = if error_code >= 0 {
                CheckStatus::CheckContinue
            } else {
                CheckStatus::CheckFinish
            };

            if !self.spend(rtt) {
                return Stop::Timeout;
            }
        }
        Stop::Done
    }

    /// `TcpChecker::__DoCheck(_check_request)` — a noop round trip per long-link
    /// host. The short-link hosts are not checked over TCP.
    fn tcp_check(&mut self, request: &mut CheckRequestProfile, ask: &mut Ask, network_type: i32) {
        let longlink = ports_of(&request.longlink_items);
        self.tcp_check_ports(&longlink, request, ask, network_type);
    }

    /// One walk of the TCP check: a noop and whatever comes back.
    fn tcp_check_ports(
        &mut self,
        ports: &[CheckIPPort],
        request: &mut CheckRequestProfile,
        ask: &mut Ask,
        network_type: i32,
    ) -> Stop {
        for port in ports {
            if self.cancel.is_cancelled() {
                return Stop::Cancelled;
            }

            let mut profile = CheckResultProfile::of(NetCheckType::TcpCheck);
            profile.ip = port.ip.clone();
            profile.port = port.port as u32;
            profile.network_type = network_type;

            let timeout_ms = self.probe_timeout(DEFAULT_TCP_CONN_TIMEOUT);
            let answer = ask.ask(Query::Tcp {
                ip: port.ip.clone(),
                port: port.port,
                timeout_ms,
            });
            let (sent, received, is_noop_resp, rtt) = answer.tcp();

            // `kSndRcvErr` for a noop that did not go out or that nothing came
            // back from, `kTcpRespErr` for an answer that was not the noop's:
            // what the C++ records is never the socket's own error code, and
            // `cost_time` is the C++'s, which it takes only for a round trip
            // that worked.
            let (error_code, rtt) = if sent < 0 || received < 0 {
                (TcpErrCode::SndRcvErr.as_i32(), 0)
            } else if !is_noop_resp {
                (TcpErrCode::TcpRespErr.as_i32(), rtt)
            } else {
                (0, rtt)
            };
            profile.error_code = error_code;
            profile.rtt = rtt;

            request.checkresult_profiles.push(profile);

            // The C++ `continue`s on a receive that failed — and only on that:
            // the profile is recorded, but neither the status the run reports
            // nor the timeout it has left is the failed receive's to decide,
            // so the next host is probed with the budget as it was.
            if sent >= 0 && received < 0 {
                continue;
            }

            request.check_status = if error_code == 0 {
                CheckStatus::CheckContinue
            } else {
                CheckStatus::CheckFinish
            };

            if !self.spend(rtt) {
                return Stop::Timeout;
            }
        }
        Stop::Done
    }

    /// `HttpChecker::__DoCheck(_check_request)` — one request per short-link
    /// host, to the CGI the core was given.
    fn http_check(
        &mut self,
        request: &mut CheckRequestProfile,
        ask: &mut Ask,
        network_type: i32,
        cgi: &str,
    ) {
        let shortlink = named_ports_of(&request.shortlink_items);
        self.http_check_ports(&shortlink, request, ask, network_type, cgi);
    }

    /// One walk of the HTTP check: the CGI, on the host of one item.
    fn http_check_ports(
        &mut self,
        ports: &[(String, CheckIPPort)],
        request: &mut CheckRequestProfile,
        ask: &mut Ask,
        network_type: i32,
        cgi: &str,
    ) -> Stop {
        for (host, port) in ports {
            if self.cancel.is_cancelled() {
                return Stop::Cancelled;
            }

            let mut profile = CheckResultProfile::of(NetCheckType::HttpCheck);
            profile.network_type = network_type;
            profile.ip = port.ip.clone();
            profile.port = port.port as u32;

            // the C++'s `(iter->first.empty() ? DEFAULT_HTTP_HOST : iter->first) + sg_netcheck_cgi`
            let mut url = if host.is_empty() {
                DEFAULT_HTTP_HOST.to_owned()
            } else {
                host.clone()
            };
            url.push_str(cgi);
            if !url.starts_with("http://") {
                url = format!("http://{url}");
            }
            profile.url = url.clone();

            // `SendHttpQuery` gets the timeout as the request has it, default
            // and all: the C++ hands `_check_request.total_timeout` over
            // without a fallback of its own.
            let answer = ask.ask(Query::Http {
                url,
                timeout_ms: self.remaining,
            });
            let (error_code, status_code, rtt) = answer.http();
            profile.status_code = status_code;
            profile.rtt = rtt;

            request.checkresult_profiles.push(profile);
            request.check_status = if error_code >= 0 {
                CheckStatus::CheckContinue
            } else {
                CheckStatus::CheckFinish
            };

            if !self.spend(rtt) {
                return Stop::Timeout;
            }
        }
        Stop::Done
    }

    /// `PingChecker::__DoCheck(_check_request)` — the ips of the request, long
    /// link first, the same two loops the DNS check walks.
    fn ping_check(&mut self, request: &mut CheckRequestProfile, ask: &mut Ask, network_type: i32) {
        let longlink = ports_of(&request.longlink_items);
        if self.ping_check_ports(&longlink, request, ask, Some(network_type)) == Stop::Cancelled {
            return;
        }
        let shortlink = ports_of(&request.shortlink_items);
        // the C++'s short-link loop does not fill `network_type` in, and
        // neither does this one
        self.ping_check_ports(&shortlink, request, ask, None);
    }

    /// One walk of the ping check: `DEFAULT_PING_COUNT` pings per ip.
    fn ping_check_ports(
        &mut self,
        ports: &[CheckIPPort],
        request: &mut CheckRequestProfile,
        ask: &mut Ask,
        network_type: Option<i32>,
    ) -> Stop {
        for port in ports {
            if self.cancel.is_cancelled() {
                return Stop::Cancelled;
            }

            // `(*ipport).ip.empty() ? DEFAULT_PING_HOST : (*ipport).ip`
            let host = if port.ip.is_empty() {
                DEFAULT_PING_HOST.to_owned()
            } else {
                port.ip.clone()
            };
            let mut profile = CheckResultProfile::of(NetCheckType::PingCheck);
            profile.ip = host.clone();
            if let Some(network_type) = network_type {
                profile.network_type = network_type;
            }

            // the C++'s `UNUSE_TIMEOUT == total_timeout ? 0 : total_timeout / 1000`
            let timeout_s = if self.remaining == UNUSE_TIMEOUT {
                0
            } else {
                self.remaining / 1000
            };
            let answer = ask.ask(Query::Ping { host, timeout_s });
            let (error_code, rtt, status) = answer.ping();
            profile.error_code = error_code;
            // `DEFAULT_PING_COUNT`, however the run went
            profile.checkcount = DEFAULT_PING_COUNT;

            // the C++'s `if (0 == ret) { GetPingStatus(); snprintf(...) }`
            if let Some(status) = status.filter(|_| error_code == 0) {
                // `snprintf(loss_rate, 16, "%f", ...)` — six decimals, like `%f`
                profile.loss_rate = format!("{:.6}", status.loss_rate);
                profile.rtt_str = format!("{:.6}", status.avgrtt);
            }

            request.checkresult_profiles.push(profile);
            request.check_status = if error_code == 0 {
                CheckStatus::CheckContinue
            } else {
                CheckStatus::CheckFinish
            };

            if !self.spend(rtt) {
                return Stop::Timeout;
            }
        }
        Stop::Done
    }

    /// The timeout one probe gets: the C++'s
    /// `UNUSE_TIMEOUT == total_timeout ? <the default> : total_timeout`.
    fn probe_timeout(&self, default_ms: u32) -> u32 {
        if self.remaining == UNUSE_TIMEOUT {
            default_ms
        } else {
            self.remaining
        }
    }

    /// `if (total_timeout != UNUSE_TIMEOUT) { total_timeout -= cost_time; if
    /// (total_timeout <= 0) break; }`
    ///
    /// `false` when the timeout is used up. A run that was started without one
    /// never is.
    fn spend(&mut self, cost: u64) -> bool {
        if self.remaining == UNUSE_TIMEOUT {
            return true;
        }
        let cost = u32::try_from(cost).unwrap_or(u32::MAX);
        self.remaining = self.remaining.saturating_sub(cost);
        self.remaining != 0
    }
}

/// The host names of one link, in the order the C++ walks them: a `std::map`,
/// so this is the order of the keys.
fn hosts_of(items: &CheckIPPorts) -> Vec<String> {
    items.keys().cloned().collect()
}

/// The ip/port of one link, in the order the C++ walks them.
fn ports_of(items: &CheckIPPorts) -> Vec<CheckIPPort> {
    items.values().flatten().cloned().collect()
}

/// The ip/port of one link, each with the host name it is filed under — which
/// is what the HTTP check builds its URL from.
fn named_ports_of(items: &CheckIPPorts) -> Vec<(String, CheckIPPort)> {
    items
        .iter()
        .flat_map(|(host, ports)| ports.iter().map(move |port| (host.clone(), port.clone())))
        .collect()
}
