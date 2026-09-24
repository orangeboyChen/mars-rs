//! `mars/sdt/sdt.h` — the vocabulary of the diagnosis: what to check, what
//! came of it, and who is told.

use std::collections::BTreeMap;

use crate::netchecker_profile::CheckResultProfile;

/// `CheckIPPort` — one host and port of a link, as handed to
/// [`crate::SdtLogic::start_active_check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckIPPort {
    /// The host, as an IP or a name.
    pub ip: String,
    /// The port.
    pub port: u16,
}

impl CheckIPPort {
    /// `CheckIPPort(ip, port)`.
    pub fn new(ip: impl Into<String>, port: u16) -> Self {
        Self {
            ip: ip.into(),
            port,
        }
    }

    /// `CheckIPPort::Reset()`.
    pub fn reset(&mut self) {
        self.ip.clear();
        self.port = 0;
    }
}

impl Default for CheckIPPort {
    /// `CheckIPPort()` — which calls `Reset()`.
    fn default() -> Self {
        Self {
            ip: String::new(),
            port: 0,
        }
    }
}

impl Ord for CheckIPPort {
    /// The C++ `operator<` compares the ip only. The port is the tiebreak here
    /// so that the order is total, and agrees with `==`, which compares both.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.ip.cmp(&other.ip).then(self.port.cmp(&other.port))
    }
}

impl PartialOrd for CheckIPPort {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// `CheckIPPorts` — the hosts of one link, keyed by the name they are known
/// under. The C++ is a `std::map`, so this is a `BTreeMap`.
pub type CheckIPPorts = BTreeMap<String, Vec<CheckIPPort>>;

/// `NetCheckStatus` — where the diagnosis as a whole is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetCheckStatus {
    /// `kNone` — nothing has been started.
    #[default]
    None = 0,
    /// `kChecking`.
    Checking = 1,
    /// `kCheckEnd`.
    CheckEnd,
}

/// `NetCheckType` — the kind of one check, and the value
/// `CheckResultProfile::netcheck_type` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetCheckType {
    /// `kPingCheck`.
    PingCheck = 0,
    /// `kDnsCheck`.
    DnsCheck = 1,
    /// `kNewDnsCheck`.
    NewDnsCheck,
    /// `kTcpCheck`.
    TcpCheck,
    /// `kHttpCheck`.
    HttpCheck,
    /// `kTracerouteCheck`.
    TracerouteCheck,
    /// `kReqBufCheck`.
    ReqBufCheck,
}

impl NetCheckType {
    /// The integer of `netcheck_type`, for a profile that is going on the air.
    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    /// The type of a `netcheck_type`. `-1`, the value `Reset()` leaves behind,
    /// is not one.
    pub const fn of(netcheck_type: i32) -> Option<Self> {
        match netcheck_type {
            0 => Some(Self::PingCheck),
            1 => Some(Self::DnsCheck),
            2 => Some(Self::NewDnsCheck),
            3 => Some(Self::TcpCheck),
            4 => Some(Self::HttpCheck),
            5 => Some(Self::TracerouteCheck),
            6 => Some(Self::ReqBufCheck),
            _ => None,
        }
    }
}

/// `CheckErrCode` — why a check did not happen at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckErrCode {
    /// `kCheckOK`.
    #[default]
    CheckOK = 0,
    /// `kDns` — the host could not be resolved.
    Dns = 1,
    /// `kSocket` — the socket could not be created.
    Socket = 2,
    /// `kIsRunning` — a check is already in flight.
    IsRunning,
}

/// `TcpErrCode` — the result of a TCP check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TcpErrCode {
    /// `kTcpSucc`.
    #[default]
    TcpSucc = 0,
    /// `kTcpNonErr`.
    TcpNonErr = 1,
    /// `kSelectErr`.
    SelectErr = -1,
    /// `kPipeIntr`.
    PipeIntr = -2,
    /// `kSndRcvErr`.
    SndRcvErr = -3,
    /// `kAssertErr`.
    AssertErr = -4,
    /// `kTimeoutErr`.
    TimeoutErr = -5,
    /// `kSelectExpErr`.
    SelectExpErr = -6,
    /// `kPipeExp`.
    PipeExp = -7,
    /// `kConnectErr`.
    ConnectErr = -8,
    /// `kTcpRespErr`.
    TcpRespErr = -9,
}

/// `CheckStatus` — whether the run loop keeps going.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckStatus {
    /// `kCheckContinue` — the next check still runs.
    #[default]
    CheckContinue = 0,
    /// `kCheckFinish` — the run stops here.
    CheckFinish = 1,
}

/// `Callback` — who is told what the checks found.
///
/// The C++ class has an empty default for `ReportNetCheckResult`; so does this
/// trait, which makes a listener that only cares about some results possible.
pub trait Callback {
    /// `Callback::ReportNetCheckResult(_check_results)`.
    fn report_net_check_result(&self, _check_results: &[CheckResultProfile]) {}
}

/// A callback that keeps what it was told, so a host (or a test) can look at it
/// afterwards.
#[derive(Debug, Default)]
pub struct CollectingCallback {
    results: std::cell::RefCell<Vec<CheckResultProfile>>,
}

impl CollectingCallback {
    /// Nothing reported yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything that was reported so far.
    pub fn results(&self) -> Vec<CheckResultProfile> {
        self.results.borrow().clone()
    }

    /// How many results were reported.
    pub fn len(&self) -> usize {
        self.results.borrow().len()
    }

    /// Whether nothing was reported.
    pub fn is_empty(&self) -> bool {
        self.results.borrow().is_empty()
    }
}

impl Callback for CollectingCallback {
    fn report_net_check_result(&self, check_results: &[CheckResultProfile]) {
        self.results.borrow_mut().extend_from_slice(check_results);
    }
}
