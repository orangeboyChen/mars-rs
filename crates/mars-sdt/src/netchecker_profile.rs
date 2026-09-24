//! `mars/sdt/netchecker_profile.h` — what a check was asked for, and what it
//! found.
//!
//! [`CheckResultProfile`] is the record one check fills in; the fields the C++
//! declares are all here, with the ones each kind of check uses marked.

use std::fmt;

use crate::constants::{NET_CHECK_BASIC, UNUSE_TIMEOUT};
use crate::sdt::{CheckIPPorts, CheckStatus, NetCheckType};

/// `CheckResultProfile` — the result of one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResultProfile {
    /// `netcheck_type` — which check produced this; `-1` right after `Reset()`.
    pub netcheck_type: i32,

    /// `error_code`.
    pub error_code: i32,
    /// `network_type`.
    pub network_type: i32,

    /// `ip` — ping, tcp, http.
    pub ip: String,
    /// `port` — tcp, http.
    pub port: u32,
    /// `conntime` — how long the connect took.
    pub conntime: u64,
    /// `rtt` — tcp and http round trip, dns resolve time.
    pub rtt: u64,
    /// `rtt_str` — ping, which reports a string.
    pub rtt_str: String,

    /// `url` — http.
    pub url: String,
    /// `status_code` — http.
    pub status_code: i32,

    /// `checkcount` — ping: how many pings went out.
    pub checkcount: u32,
    /// `loss_rate` — ping.
    pub loss_rate: String,

    /// `domain_name` — the dns host.
    pub domain_name: String,
    /// `local_dns` — dns.
    pub local_dns: String,
    /// `ip1` — dns.
    pub ip1: String,
    /// `ip2` — dns.
    pub ip2: String,
}

impl Default for CheckResultProfile {
    /// `CheckResultProfile()` — which calls `Reset()`.
    fn default() -> Self {
        Self {
            netcheck_type: -1,
            error_code: 0,
            network_type: 0,
            ip: String::new(),
            port: 0,
            conntime: 0,
            rtt: 0,
            rtt_str: String::new(),
            url: String::new(),
            status_code: 0,
            checkcount: 0,
            loss_rate: String::new(),
            domain_name: String::new(),
            local_dns: String::new(),
            ip1: String::new(),
            ip2: String::new(),
        }
    }
}

impl CheckResultProfile {
    /// `CheckResultProfile()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `CheckResultProfile::Reset()`.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Which check produced this, if the type is one of [`NetCheckType`].
    pub fn kind(&self) -> Option<NetCheckType> {
        NetCheckType::of(self.netcheck_type)
    }

    /// `CheckResultProfile` of one kind, for callers that fill the rest in.
    pub fn of(kind: NetCheckType) -> Self {
        Self {
            netcheck_type: kind.as_i32(),
            ..Self::default()
        }
    }
}

impl fmt::Display for CheckResultProfile {
    /// `SdtCore::__DumpCheckResult()`, line for line: the C++ logs one line per
    /// result, and the fields it picks are the ones that kind of check fills in.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind() {
            Some(NetCheckType::TcpCheck) => write!(
                f,
                "tcp check result, error_code:{}, ip:{}, port:{}, network_type:{}, rtt:{}",
                self.error_code, self.ip, self.port, self.network_type, self.rtt
            ),
            Some(NetCheckType::HttpCheck) => write!(
                f,
                "http check result, status_code:{}, url:{}, ip:{}, port:{}, network_type:{}, rtt:{}",
                self.status_code, self.url, self.ip, self.port, self.network_type, self.rtt
            ),
            Some(NetCheckType::PingCheck) => write!(
                f,
                "ping check result, error_code:{}, ip:{}, network_type:{}, loss_rate:{}, rtt:{}",
                self.error_code, self.ip, self.network_type, self.loss_rate, self.rtt_str
            ),
            Some(NetCheckType::DnsCheck) => write!(
                f,
                "dns check result, error_code:{}, domain_name:{}, network_type:{}, ip1:{}, rtt:{}",
                self.error_code, self.domain_name, self.network_type, self.ip1, self.rtt
            ),
            // the C++ switch has no case for these, so nothing is logged
            _ => Ok(()),
        }
    }
}

/// `CheckRequestProfile` — what one diagnosis was asked for, and what its
/// checks have recorded so far.
#[derive(Debug, Clone)]
pub struct CheckRequestProfile {
    /// `longlink_items`.
    pub longlink_items: CheckIPPorts,
    /// `shortlink_items`.
    pub shortlink_items: CheckIPPorts,

    /// `mode` — a combination of the `NET_CHECK_*` bits.
    pub mode: i32,
    /// `check_status`.
    pub check_status: CheckStatus,

    /// `total_timeout`, in milliseconds.
    pub total_timeout: u32,

    /// `checkresult_profiles` — what the checks that have run recorded.
    pub checkresult_profiles: Vec<CheckResultProfile>,
}

impl Default for CheckRequestProfile {
    /// `CheckRequestProfile()`, which is `Reset()` — so a fresh request already
    /// asks for the basic check, like the C++ one.
    fn default() -> Self {
        Self {
            longlink_items: CheckIPPorts::new(),
            shortlink_items: CheckIPPorts::new(),
            mode: NET_CHECK_BASIC,
            check_status: CheckStatus::CheckContinue,
            total_timeout: 0,
            checkresult_profiles: Vec::new(),
        }
    }
}

impl CheckRequestProfile {
    /// `CheckRequestProfile()` — which calls `Reset()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `CheckRequestProfile::Reset()`.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Whether the mode asks for the short-link hosts, which is when
    /// `__InitCheckReq` copies them in.
    pub fn shortlink_is_checked(&self) -> bool {
        crate::constants::mode_short(self.mode)
    }

    /// The overall timeout, defaulted the way `StartCheck` does it.
    pub fn timeout(&self) -> u32 {
        if self.total_timeout == 0 {
            UNUSE_TIMEOUT
        } else {
            self.total_timeout
        }
    }
}
