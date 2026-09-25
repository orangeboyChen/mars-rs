//! `mars/sdt/src/checkimpl/` — the four probes the checks reach for.
//!
//! `dnsquery.cc`, `tcpquery.cc`, `httpquery.cc` and `pingquery.cc` are the four
//! places the C++ opens a socket: `socket_gethostbyname`, a TCP connect that
//! sends a long-link noop and reads the answer, an HTTP request against the
//! net-check CGI, and an ICMP ping. Not one of those is this crate's to open —
//! `mars/comm/socket` and `mars/comm/dns` are the platform's here, and so is
//! the long-link package the TCP check puts on the wire — so all four are one
//! seam: a [`Query`], an [`Answer`], and an [`Ask`] whose answerer is the
//! host's.
//!
//! What *is* ported of them is what the socket is asked, what it answers back,
//! and — in [`crate::activecheck`] — what the four checks make of the answer.
//! `dnsquery.cc`'s resolver and `pingquery.cc`'s ICMP stay behind.
//!
//! Every answer carries how long the probe took: that is the `cost_time` the
//! C++ measures with `gettickcount()` around the call, and it is what spends
//! the timeout of the run. Here the host measures the same interval around the
//! call that is its own.

use std::fmt;

/// `socket_gethostbyname(host, &ipinfo, timeout, NULL)` — what
/// `DNSQuery::GetDNSQuery` reaches, for one host of the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Query {
    /// The resolve of one host name.
    Dns {
        /// `profile.domain_name` — the host, as the request files it.
        domain: String,
        /// In milliseconds: [`crate::constants::DEFAULT_DNS_TIMEOUT`] when the
        /// run was started without a timeout.
        timeout_ms: u32,
    },
    /// `TcpQuery(ip, port, 0)` — `tcp_send` of a long-link noop and
    /// `tcp_receive` of whatever comes back.
    ///
    /// What goes on the wire is the long link's own package
    /// (`longlink_noop_req_body` + `longlink_pack`), which is `mars/stn`'s and
    /// not this crate's, so the noop and the unpack are the host's too: what is
    /// asked for is the round trip, and what comes back is whether it happened
    /// and whether what came back was the answer to it.
    Tcp {
        /// The host, as an IP.
        ip: String,
        /// The port, as `TcpQuery` takes it.
        port: u16,
        /// In milliseconds: [`crate::constants::DEFAULT_TCP_CONN_TIMEOUT`] for
        /// a run that was started without one.
        timeout_ms: u32,
    },
    /// `SendHttpQuery(url, status_code, errmsg, timeout)` — the request that
    /// tells the net-check CGI somebody is looking for it.
    Http {
        /// `profile.url` — the host of the request with the CGI behind it, and
        /// `http://` in front when the host has no scheme of its own.
        url: String,
        /// In milliseconds, as the C++ hands it over: a run without a timeout
        /// hands [`UNUSE_TIMEOUT`](crate::constants::UNUSE_TIMEOUT), which is
        /// what `__DoCheck` has, default and all.
        timeout_ms: u32,
    },
    /// `PingQuery::RunPingQuery(0, 0, timeout, host)`.
    Ping {
        /// The host to ping: [`DEFAULT_PING_HOST`](crate::constants::DEFAULT_PING_HOST)
        /// for an item whose ip is empty.
        host: String,
        /// In **seconds**, which is what the C++ hands it: `total_timeout /
        /// 1000`, and `0` for a run that was started without one.
        timeout_s: u32,
    },
}

/// `struct PingStatus` — what `PingQuery::GetPingStatus` hands back.
///
/// The C++'s carries the ip as well, which no checker reads.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PingStatus {
    /// `loss_rate` — `1.0` is every ping of the run lost, which the C++ calls
    /// "ping check failed" even though the query itself came back.
    pub loss_rate: f32,
    /// `avgrtt` — what `profile.rtt_str` is made of.
    pub avgrtt: f32,
}

impl PingStatus {
    /// `PingStatus(loss_rate, avgrtt)`.
    pub fn new(loss_rate: f32, avgrtt: f32) -> Self {
        Self { loss_rate, avgrtt }
    }
}

/// What one probe answers back.
///
/// Every answer but [`Answer::Nothing`] carries how long the probe took, which
/// is the `cost_time` the C++ measures around the call: the host measures the
/// same interval, around the call that is its own.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Answer {
    /// `socket_gethostbyname` — its return value, how long the resolve took,
    /// and the addresses it found, in the order they came. `0` is a resolve
    /// that worked, and the C++ reads `ipinfo.ip[0]` and `ipinfo.ip[1]` out of
    /// it; anything below `0` is a host that could not be resolved.
    Dns {
        /// `profile.error_code`.
        error_code: i32,
        /// `cost_time`.
        rtt: u64,
        /// The addresses, up to the two the profile has room for.
        ips: Vec<String>,
    },
    /// The noop round trip: `tcp_send` and then `tcp_receive`, which is why it
    /// answers for both. The C++ keeps the return value of the last call it
    /// made in one `ret`; here each call keeps its own.
    Tcp {
        /// `tcp_send` — `0` and above is a noop that went out, below `0` a
        /// send that failed.
        sent: i32,
        /// `tcp_receive` — `0` and above is an answer that came back, below
        /// `0` a receive that failed after a noop that went out. Read for a
        /// noop that went out only: a send that failed is not received on, and
        /// the checker records
        /// [`TcpErrCode::SndRcvErr`](crate::TcpErrCode::SndRcvErr) for either
        /// one, whatever the socket's own error was.
        received: i32,
        /// `__NoopResp` — whether what came back was the answer to the noop
        /// that went out.
        is_noop_resp: bool,
        /// `cost_time`, which the C++ takes only for a round trip that worked.
        rtt: u64,
    },
    /// `SendHttpQuery` — its return value, the status code it read out of the
    /// answer, and how long the request took.
    Http {
        /// `ret`: `0` and above is a request that came back.
        error_code: i32,
        /// `profile.status_code`.
        status_code: i32,
        /// `cost_time`.
        rtt: u64,
    },
    /// `RunPingQuery` — its return value, how long it took, and the status of
    /// a run that came back, which is the C++'s `if (0 == ret)`.
    Ping {
        /// `profile.error_code`.
        error_code: i32,
        /// `cost_time`, which is what spends the timeout: the round trip the
        /// profile reports is [`PingStatus::avgrtt`], not this.
        rtt: u64,
        /// `GetPingStatus`, when there is a status to get.
        status: Option<PingStatus>,
    },
    /// Nobody answered: a host with no network to probe with, which every
    /// check reads as a failure.
    #[default]
    Nothing,
}

impl Answer {
    /// The resolve: `(-1, 0, [])` when this is not what was answered, which is
    /// a host that could not be resolved.
    pub fn dns(&self) -> (i32, u64, &[String]) {
        match self {
            Self::Dns {
                error_code,
                rtt,
                ips,
            } => (*error_code, *rtt, ips.as_slice()),
            _ => (-1, 0, &[]),
        }
    }

    /// The noop round trip: `(-1, 0, false, 0)` when this is not what was
    /// answered, which is a send that failed.
    pub fn tcp(&self) -> (i32, i32, bool, u64) {
        match self {
            Self::Tcp {
                sent,
                received,
                is_noop_resp,
                rtt,
            } => (*sent, *received, *is_noop_resp, *rtt),
            _ => (-1, 0, false, 0),
        }
    }

    /// The HTTP request: `(-1, 0, 0)` when this is not what was answered.
    pub fn http(&self) -> (i32, i32, u64) {
        match self {
            Self::Http {
                error_code,
                status_code,
                rtt,
            } => (*error_code, *status_code, *rtt),
            _ => (-1, 0, 0),
        }
    }

    /// The ping: `(-1, 0, None)` when this is not what was answered.
    pub fn ping(&self) -> (i32, u64, Option<&PingStatus>) {
        match self {
            Self::Ping {
                error_code,
                rtt,
                status,
            } => (*error_code, *rtt, status.as_ref()),
            _ => (-1, 0, None),
        }
    }
}

/// What the four probes are asked with.
///
/// The C++ has none of this: `dnsquery.cc` and the rest are linked straight
/// into the checkers, so a diagnosis there can only be run against the
/// platform's own sockets. The port hands the answerer in, which is what makes
/// a diagnosis testable — and what makes a host that is not a phone able to
/// run one at all.
pub struct Ask {
    ask: Box<dyn FnMut(Query) -> Answer + Send>,
}

impl Ask {
    /// The probes the host answers: a real resolver, a real socket, and so on.
    pub fn new(ask: impl FnMut(Query) -> Answer + Send + 'static) -> Self {
        Self { ask: Box::new(ask) }
    }

    /// One probe.
    pub(crate) fn ask(&mut self, query: Query) -> Answer {
        (self.ask)(query)
    }
}

impl Default for Ask {
    /// A diagnosis nobody can probe with: every question goes unanswered, and
    /// every check fails — which is what a host with no network of its own
    /// gets, and what [`crate::activecheck`]'s samples start from.
    fn default() -> Self {
        Self::new(|_| Answer::Nothing)
    }
}

impl fmt::Debug for Ask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ask").finish_non_exhaustive()
    }
}
