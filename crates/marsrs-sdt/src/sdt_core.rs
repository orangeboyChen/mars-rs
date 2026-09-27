//! `mars/sdt/src/sdt_core.cc` — the diagnosis itself.
//!
//! What is ported is everything but the sockets: [`SdtCore::start_check`]
//! turns the mode into the list of checks to run, [`SdtCore::run_on`] runs
//! them in order until one is cancelled or finishes the request, and the
//! results are handed back for [`crate::SdtLogic`] to report. The C++ creates
//! `PingChecker` / `DnsChecker` / `HttpChecker` / `TcpChecker` here; those need
//! a network, so the caller passes the check to run as a closure.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::activecheck::Check;
use crate::checkimpl::Ask;
use crate::constants::{mode_basic, mode_long, mode_short};
use crate::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use crate::sdt::{CheckIPPorts, CheckStatus, NetCheckType};

/// The `cancel_` of one [`SdtCore`], reachable from outside the core.
///
/// The C++ keeps `cancel_` inside `SdtCore` and guards it with the core's
/// mutex, but `__RunOn` holds that mutex for as long as the checks take — so
/// `CancelCheck()` can only ever cancel a request that has not started yet,
/// never one that is blocked on a socket. The port has the same shape:
/// [`SdtCore::run_on`] borrows the core for the whole run. So the flag lives
/// in its own shared cell and a caller that wants to stop a running diagnosis
/// takes a [`CancelHandle`] ([`SdtCore::cancel_handle`]) before the run and
/// sets it from wherever it is — the app thread, or the checker itself, which
/// is where the socket that has to be interrupted is.
#[derive(Debug, Clone)]
pub struct CancelHandle(Arc<AtomicBool>);

impl CancelHandle {
    /// A flag that is not set.
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// `SdtCore::CancelCheck()` — stops the run at its next check.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether somebody has cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Takes the cancellation back, so the core can be used again.
    fn clear(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Default for CancelHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// `SdtCore`.
#[derive(Debug, Clone)]
pub struct SdtCore {
    /// `check_list_` — the checks of the current request, in the order they run.
    check_list: Vec<NetCheckType>,
    /// `check_request_`.
    check_request: CheckRequestProfile,
    /// `cancel_` — shared, so it can be set while a run borrows the core.
    cancel: CancelHandle,
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
            cancel: CancelHandle::new(),
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
        // A core that was cancelled once has to be able to run again: the
        // request that is accepted here is a new one, and it is not the one
        // anybody cancelled.
        self.cancel.clear();
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

    /// `SdtCore::CancelCheck()` — the run stops at its next check.
    ///
    /// `&self`, and shared with every [`CancelHandle`] handed out for this
    /// core: that is what lets a caller cancel a check that is already
    /// running, which [`SdtCore::run_on`] borrowing the core for the whole
    /// run would otherwise make impossible.
    pub fn cancel_check(&self) {
        self.cancel.cancel();
    }

    /// The cancellation flag of this core, for a caller that has to cancel
    /// while the checks are running.
    pub fn cancel_handle(&self) -> CancelHandle {
        self.cancel.clone()
    }

    /// `SdtCore::__Reset()` — the checks are dropped and the request is over.
    ///
    /// `cancel_` survives the reset so that whoever asked for the run can
    /// still see that it was cancelled; it is cleared when the core accepts
    /// the next request instead ([`SdtCore::init_check_request`]), which is
    /// what makes one core usable for more than one cancellation.
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
            if self.cancel.is_cancelled()
                || self.check_request.check_status == CheckStatus::CheckFinish
            {
                break;
            }
            do_check(kind, &mut self.check_request);
        }

        let results = std::mem::take(&mut self.check_request.checkresult_profiles);
        self.reset();
        results
    }

    /// `SdtCore::__RunOn()` with the port's own checkers — the four the C++
    /// creates in `__InitCheckReq` — asked through `ask`.
    ///
    /// `network_type` is the `comm::getNetInfo()` every checker writes into its
    /// profiles: the platform is the host's here, so it comes with the run
    /// instead of being asked for once per result.
    pub fn run_checks(&mut self, ask: &mut Ask, network_type: i32) -> Vec<CheckResultProfile> {
        let mut check = Check::new(self.cancel.clone(), self.check_request.timeout());
        let cgi = self.netcheck_cgi.clone();
        self.run_on(|kind, request| {
            check.start_do_check(kind, request, ask, network_type, &cgi);
        })
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
        self.cancel.is_cancelled()
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
