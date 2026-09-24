//! `mars/sdt/sdt_logic.cc` — the surface the app calls.
//!
//! The C++ forwards every one of these to the `SdtManager` of the boot
//! context; there is no context here, so [`SdtLogic`] owns the core and the
//! callback directly.

use crate::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use crate::sdt::{Callback, CheckIPPorts, NetCheckType};
use crate::sdt_core::SdtCore;

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
    pub fn cancel_active_check(&mut self) {
        self.core.cancel_check();
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
    pub fn run(
        &mut self,
        do_check: impl FnMut(NetCheckType, &mut CheckRequestProfile),
    ) -> Vec<CheckResultProfile> {
        let results = self.core.run_on(do_check);
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
