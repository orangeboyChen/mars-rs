//! `mars/sdt/sdt_logic.cc` — the surface the app calls.
//!
//! The C++ forwards every one of these to the `SdtManager` of the boot
//! context; there is no context here, so [`SdtLogic`] owns the core and the
//! callback directly.

use crate::checkimpl::Ask;
use crate::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use crate::sdt::{Callback, CheckIPPorts, NetCheckType};
use crate::sdt_core::{CancelHandle, SdtCore};

/// The `sdt_logic` of the C++, as one value.
pub struct SdtLogic {
    core: SdtCore,
    callback: Option<Box<dyn Callback + Send>>,
}

impl Default for SdtLogic {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SdtLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SdtLogic")
            .field("core", &self.core)
            .field("has_callback", &self.callback.is_some())
            .finish()
    }
}

impl SdtLogic {
    /// Nothing is being checked and nobody is listening yet.
    pub fn new() -> Self {
        Self {
            core: SdtCore::new(),
            callback: None,
        }
    }

    /// `SetCallBack(callback)`.
    ///
    /// Replaces whoever listened before: the C++ keeps a single pointer.
    pub fn set_callback(&mut self, callback: impl Callback + Send + 'static) {
        self.callback = Some(Box::new(callback));
    }

    /// `SetCallBack(NULL)`.
    pub fn clear_callback(&mut self) {
        self.callback = None;
    }

    /// Whether somebody is listening.
    pub fn has_callback(&self) -> bool {
        self.callback.is_some()
    }

    /// `SetHttpNetcheckCGI(cgi)`.
    pub fn set_http_netcheck_cgi(&mut self, cgi: impl Into<String>) {
        self.core.set_http_netcheck_cgi(cgi);
    }

    /// The URL the HTTP check goes to.
    pub fn http_netcheck_cgi(&self) -> &str {
        self.core.http_netcheck_cgi()
    }

    /// `StartActiveCheck(longlink_check_item, shortlink_check_item, mode, timeout)`.
    ///
    /// `false` when a check is already in flight.
    pub fn start_active_check(
        &mut self,
        longlink_items: &CheckIPPorts,
        shortlink_items: &CheckIPPorts,
        mode: i32,
        timeout: u32,
    ) -> bool {
        self.core
            .start_check(longlink_items, shortlink_items, mode, timeout)
    }

    /// `CancelActiveCheck()`.
    ///
    /// `&self`, and it still stops a check that is already running: the flag
    /// it sets is shared with the run, which borrows this logic exclusively
    /// for as long as the checks take. A caller that does not own the logic
    /// while the checks run takes a [`CancelHandle`] instead.
    pub fn cancel_active_check(&self) {
        self.core.cancel_check();
    }

    /// The cancellation flag of the current request, so that a caller can
    /// cancel while [`SdtLogic::run`] is still in flight.
    pub fn cancel_handle(&self) -> CancelHandle {
        self.core.cancel_handle()
    }

    /// The checks the running request is going to make, in order.
    pub fn plan(&self) -> &[NetCheckType] {
        self.core.plan()
    }

    /// The request the checks are running against.
    pub fn request(&self) -> &CheckRequestProfile {
        self.core.request()
    }

    /// Whether a check is in flight.
    pub fn is_checking(&self) -> bool {
        self.core.is_checking()
    }

    /// Runs the checks of the request — one `do_check` per planned check, in
    /// order — and reports what they recorded. This is the `__RunOn` thread of
    /// the C++, called by the host instead of started by it.
    ///
    /// The run borrows the logic exclusively, so nobody can call
    /// [`SdtLogic::cancel_active_check`] while it is in flight: take a
    /// [`CancelHandle`] with [`SdtLogic::cancel_handle`] first and hand it to
    /// whoever has to stop the run (the checker is the usual candidate, since
    /// it owns the socket that has to give up).
    pub fn run(
        &mut self,
        do_check: impl FnMut(NetCheckType, &mut CheckRequestProfile),
    ) -> Vec<CheckResultProfile> {
        let results = self.core.run_on(do_check);
        self.report(&results);
        results
    }

    /// [`SdtLogic::run`] with the port's own checkers: the four classes the C++
    /// creates in `__InitCheckReq`, asked through `ask`.
    ///
    /// `network_type` is the `comm::getNetInfo()` every checker writes into its
    /// profiles — the platform is the host's here, so it comes with the run.
    pub fn run_checks(&mut self, ask: &mut Ask, network_type: i32) -> Vec<CheckResultProfile> {
        let results = self.core.run_checks(ask, network_type);
        self.report(&results);
        results
    }

    /// `ReportNetCheckResult(_check_results)` — hands the results to whoever
    /// was set with [`SdtLogic::set_callback`], if anybody was.
    pub fn report(&self, check_results: &[CheckResultProfile]) {
        if let Some(callback) = &self.callback {
            callback.report_net_check_result(check_results);
        }
    }
}
