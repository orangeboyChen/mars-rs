//! `mars/sdt/sdt_logic.cc` — the surface the app calls.
//!
//! The C++ forwards every one of these to the `SdtManager` of the boot
//! context; there is no context here, so [`SdtLogic`] owns the core and the
//! callback directly.

use crate::checkimpl::Ask;
use crate::constants::{NET_CHECK_BASIC, NET_CHECK_LONG, NET_CHECK_SHORT};
use crate::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use crate::sdt::{Callback, CheckIPPorts, NetCheckStatus, NetCheckType};
use crate::sdt_core::{CancelHandle, SdtCore};

/// Which checks a diagnosis runs — the `mode` of the C++, named instead of
/// counted.
///
/// The C++ hands this around as an `int` of `NET_CHECK_*` bits, and every
/// platform spells the bits differently: `NET_CHECK_BASIC` here,
/// `CheckMode.K_BASIC` in the shared Kotlin, `1 | 2` in an app that kept the
/// numbers. This is the same bits with names, and `|` to put them together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode(i32);

impl Mode {
    /// Nothing to check.
    pub const NONE: Self = Self(0);
    /// `NET_CHECK_BASIC` — ping and DNS.
    pub const BASIC: Self = Self(NET_CHECK_BASIC);
    /// `NET_CHECK_LONG` — TCP against the long-link hosts.
    pub const LONG: Self = Self(NET_CHECK_LONG);
    /// `NET_CHECK_SHORT` — HTTP against the net-check CGI.
    pub const SHORT: Self = Self(NET_CHECK_SHORT);
    /// Every one of them, which is what an app that wants the whole picture
    /// asks for.
    pub const ALL: Self = Self(NET_CHECK_BASIC | NET_CHECK_LONG | NET_CHECK_SHORT);

    /// The `mode` the C++ wrote, which is what [`SdtLogic::start_active_check`]
    /// takes and what a bridge gets from an app that still counts.
    pub const fn bits(self) -> i32 {
        self.0
    }

    /// The other way round: a `mode` an app was handed, which is how the
    /// upstream call is still answered with a mode it understands.
    pub const fn of(bits: i32) -> Self {
        Self(bits)
    }
}

impl std::ops::BitOr for Mode {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Mode {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl From<Mode> for i32 {
    fn from(mode: Mode) -> Self {
        mode.bits()
    }
}

impl From<i32> for Mode {
    fn from(bits: i32) -> Self {
        Self::of(bits)
    }
}

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

    /// The whole diagnosis in one call: [`SdtLogic::start_active_check`],
    /// [`SdtLogic::run_checks`] and the report, which is what the C++ does in
    /// `StartActiveCheck` and on the `__RunOn` thread it starts behind it.
    ///
    /// [`None`] is a request that was not taken: a check that is already in
    /// flight, or a mode of no checks at all, which is the `false` of the
    /// upstream call. The results are also handed to the app's
    /// [`Callback`], the way they are when an app makes the three calls itself.
    ///
    /// What it is not is a future: every check is a probe that runs to its own
    /// timeout on the thread that asks for it, and an app that wants it off
    /// the thread it is on puts it there — `std::thread::spawn` around this
    /// call, or an executor's own `spawn_blocking`.
    pub fn diagnose(
        &mut self,
        longlink_items: &CheckIPPorts,
        shortlink_items: &CheckIPPorts,
        mode: Mode,
        timeout: u32,
        ask: &mut Ask,
        network_type: i32,
    ) -> Option<Vec<CheckResultProfile>> {
        if !self
            .core
            .start_check(longlink_items, shortlink_items, mode.bits(), timeout)
        {
            return None;
        }
        Some(self.run_checks(ask, network_type))
    }

    /// `StartActiveCheck(longlink_check_item, shortlink_check_item, mode, timeout)`.
    ///
    /// `false` when no check was started: one that is already in flight, or a
    /// `mode` with none of the three `NET_CHECK_*` bits in it, which is a plan
    /// of nothing. [`SdtLogic::is_checking`] tells the two apart.
    ///
    /// A `timeout` of `0` is a run with no timeout of its own: every probe is
    /// asked with the default of its kind, and nothing breaks the plan off for
    /// having spent too long. A bridge that hands a signed timeout across reads
    /// a negative one as this — the C++ hands the negative to its probes
    /// instead, which is a run that ends behind the first of them.
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

    /// `netcheck_status_` — where the diagnosis as a whole is:
    /// [`NetCheckStatus::Checking`] while a request is in flight,
    /// [`NetCheckStatus::CheckEnd`] once a run is over, and
    /// [`NetCheckStatus::None`] before the first one.
    ///
    /// A listener that was handed results asks for this and for
    /// [`SdtLogic::is_cancelled`] together: `CheckEnd` of a cancelled run is a
    /// diagnosis that was cut short, whose results are the ones the checks
    /// before the cancel recorded — not one that ended on its own, and not
    /// something the results themselves say.
    pub fn status(&self) -> NetCheckStatus {
        self.core.status()
    }

    /// Whether the request was cancelled: what makes
    /// [`NetCheckStatus::CheckEnd`] a truncated diagnosis rather than a
    /// finished one.
    pub fn is_cancelled(&self) -> bool {
        self.core.is_cancelled()
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
    ///
    /// The C++'s own is an empty body (`sdt_manager.cc:71`): the comment in it
    /// says the results were meant for `MMReportNetCheckResult` in `stn`, which
    /// is a piece this port does not carry, so reporting here is what the app
    /// asked [`SdtLogic::set_callback`] for and nothing else.
    pub fn report(&self, check_results: &[CheckResultProfile]) {
        if let Some(callback) = &self.callback {
            callback.report_net_check_result(check_results);
        }
    }
}
