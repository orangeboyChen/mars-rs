//! `mars/sdt/src/sdt_core.cc` — the diagnosis itself.
//!
//! What is ported is everything but the sockets: [`SdtCore::start_check`]
//! turns the mode into the list of checks to run, [`SdtCore::run_on`] runs
//! them in order until one is cancelled or finishes the request, and the
//! results are handed back for [`crate::SdtLogic`] to report. The C++ creates
//! `PingChecker` / `DnsChecker` / `HttpChecker` / `TcpChecker` here; those need
//! a network, so the caller passes the check to run as a closure.

use crate::constants::{mode_basic, mode_long, mode_short};
use crate::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use crate::sdt::{CheckIPPorts, CheckStatus, NetCheckType};

/// `SdtCore`.
#[derive(Debug, Clone)]
pub struct SdtCore {
    /// `check_list_` — the checks of the current request, in the order they run.
    check_list: Vec<NetCheckType>,
    /// `check_request_`.
    check_request: CheckRequestProfile,
    /// `cancel_`.
    cancel: bool,
    /// `checking_`.
    checking: bool,
    /// `netcheck_cgi_` — the URL the HTTP check goes to.
    netcheck_cgi: String,
}

impl Default for SdtCore {
    fn default() -> Self {
        Self::new()
    }
}

impl SdtCore {
    /// `SdtCore(context)`.
    pub fn new() -> Self {
        Self {
            check_list: Vec::new(),
            check_request: CheckRequestProfile::new(),
            cancel: false,
            checking: false,
            netcheck_cgi: String::new(),
        }
    }

    /// `SdtCore::StartCheck(longlink_items, shortlink_items, mode, timeout)`.
    ///
    /// `false` when a check is already in flight: the C++ takes the lock, sees
    /// `checking_` and returns without touching the request.
    pub fn start_check(
        &mut self,
        longlink_items: &CheckIPPorts,
        shortlink_items: &CheckIPPorts,
        mode: i32,
        timeout: u32,
    ) -> bool {
        if self.checking {
            return false;
        }
        self.init_check_request(longlink_items, shortlink_items, mode, timeout);
        true
    }

    /// `SdtCore::__InitCheckReq(...)` — the request, and the checks the mode
    /// asks for.
    pub fn init_check_request(
        &mut self,
        longlink_items: &CheckIPPorts,
        shortlink_items: &CheckIPPorts,
        mode: i32,
        timeout: u32,
    ) {
        self.checking = true;

        self.check_request.reset();
        self.check_request.longlink_items = longlink_items.clone();
        self.check_request.mode = mode;
        self.check_request.total_timeout = timeout;

        self.check_list.clear();

        if mode_basic(mode) {
            self.check_list.push(NetCheckType::PingCheck);
            self.check_list.push(NetCheckType::DnsCheck);
        }

        if mode_short(mode) {
            self.check_request.shortlink_items = shortlink_items.clone();
            self.check_list.push(NetCheckType::HttpCheck);
        }

        if mode_long(mode) {
            self.check_list.push(NetCheckType::TcpCheck);
        }
    }

    /// `SdtCore::CancelCheck()`.
    pub fn cancel_check(&mut self) {
        self.cancel = true;
    }

    /// `SdtCore::__Reset()` — the checks are dropped and the request is over.
    ///
    /// `cancel_` is *not* cleared, exactly as in the C++: a cancelled core stays
    /// cancelled and has to be replaced.
    pub fn reset(&mut self) {
        self.check_list.clear();
        self.checking = false;
    }

    /// `SdtCore::__RunOn()` — one check after another, stopping at a cancel or
    /// at [`CheckStatus::CheckFinish`], and handing back what they recorded.
    ///
    /// `do_check` is the checker the C++ would have created in
    /// `__InitCheckReq`; it records into the request it is given.
    pub fn run_on(
        &mut self,
        mut do_check: impl FnMut(NetCheckType, &mut CheckRequestProfile),
    ) -> Vec<CheckResultProfile> {
        let plan = self.check_list.clone();
        for kind in plan {
            if self.cancel || self.check_request.check_status == CheckStatus::CheckFinish {
                break;
            }
            do_check(kind, &mut self.check_request);
        }

        let results = std::mem::take(&mut self.check_request.checkresult_profiles);
        self.reset();
        results
    }

    /// `SdtCore::SetHttpNetcheckCGI(cgi)`.
    pub fn set_http_netcheck_cgi(&mut self, cgi: impl Into<String>) {
        self.netcheck_cgi = cgi.into();
    }

    /// The URL the HTTP check goes to.
    pub fn http_netcheck_cgi(&self) -> &str {
        &self.netcheck_cgi
    }

    /// Whether a check is in flight.
    pub fn is_checking(&self) -> bool {
        self.checking
    }

    /// Whether the request was cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel
    }

    /// The checks of the current request, in the order they run.
    pub fn plan(&self) -> &[NetCheckType] {
        &self.check_list
    }

    /// The request the checks are running against.
    pub fn request(&self) -> &CheckRequestProfile {
        &self.check_request
    }
}
