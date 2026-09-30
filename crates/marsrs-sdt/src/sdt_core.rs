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
use crate::constants::{has_check, mode_basic, mode_long, mode_short};
use crate::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use crate::sdt::{CheckIPPorts, CheckStatus, NetCheckStatus, NetCheckType};

/// The `cancel_` of one [`SdtCore`], reachable from outside the core.
///
/// The C++ keeps `cancel_` in the core as a `volatile bool` and sets it from
/// `CancelCheck()` without taking `checking_mutex_`, which `StartCheck` has
/// already released by the time `__RunOn` runs the checks — so the C++ does
/// reach a check that is in flight, which is the point of it, and what it
/// cannot do is run another one afterwards, because nothing ever clears
/// `cancel_` again.
///
/// The port shares the flag for a different reason: [`SdtCore::run_on`] borrows
/// the core for the whole run, so the flag lives in its own cell — and a caller
/// that wants to stop a running diagnosis takes a [`CancelHandle`]
/// ([`SdtCore::cancel_handle`]) before the run and sets it from wherever it is:
/// the app thread, or the checker itself, which is where the socket that has to
/// be interrupted is.
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

/// One run of [`SdtCore::run_on`], and the whole of what its end leaves
/// behind.
///
/// Given back by [`Drop`] and not by the end of the run, which a probe that
/// panics never reaches: the panic comes out of the closure the host handed
/// in. A core left `checking` would answer `false` to every `start_check`
/// after it — a host's panic retiring the core for good — and a request left
/// as it was would answer [`CheckStatus::CheckFinish`] to the next run, which
/// stops before it starts and hands back the results of the run that panicked.
struct RunOn<'a> {
    checking: &'a mut NetCheckStatus,
    check_list: &'a mut Vec<NetCheckType>,
    request: &'a mut CheckRequestProfile,
}

impl Drop for RunOn<'_> {
    /// `__RunOn` is over, panic or not.
    fn drop(&mut self) {
        *self.checking = NetCheckStatus::CheckEnd;
        self.check_list.clear();
        self.request.reset();
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
    /// `checking_`, and `netcheck_status_` behind it: [`NetCheckStatus::None`]
    /// for a core nothing has been started on, [`NetCheckStatus::Checking`]
    /// while a request is in flight, [`NetCheckStatus::CheckEnd`] once a run
    /// is over — which is what a listener asks for to tell a diagnosis that
    /// ran to its end from one that was cut short.
    checking: NetCheckStatus,
    /// `netcheck_cgi_` — the URL the HTTP check goes to.
    ///
    /// The C++'s is a file-static `sg_netcheck_cgi` (`httpchecker.cc:31`) that
    /// every checker of the process reads, whatever core it belongs to; a core
    /// here keeps its own, which is what no process of its own means.
    netcheck_cgi: String,
}

impl Default for SdtCore {
    fn default() -> Self {
        Self::new()
    }
}

impl SdtCore {
    /// `SdtCore()` — a core that is not checking, with a default request,
    /// nothing cancelled and no CGI.
    pub fn new() -> Self {
        Self {
            check_list: Vec::new(),
            check_request: CheckRequestProfile::new(),
            cancel: CancelHandle::new(),
            checking: NetCheckStatus::None,
            netcheck_cgi: String::new(),
        }
    }

    /// `SdtCore::StartCheck(longlink_items, shortlink_items, mode, timeout)`.
    ///
    /// `false` when a check is already in flight: the C++ takes the lock, sees
    /// `checking_` and returns without touching the request. And `false` for a
    /// mode with none of the three `NET_CHECK_*` bits in it, which is a plan
    /// of nothing: the run behind it would check nothing and report nothing,
    /// and a caller that asked for a diagnosis would be told it has one. What
    /// the C ABI answers for the same request is `MARS_SDT_ERR_BAD_ARG`, which
    /// is the one seam that says *why*; the seams that answer a bool say no,
    /// and say it alike.
    pub fn start_check(
        &mut self,
        longlink_items: &CheckIPPorts,
        shortlink_items: &CheckIPPorts,
        mode: i32,
        timeout: u32,
    ) -> bool {
        if self.is_checking() || !has_check(mode) {
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
        self.checking = NetCheckStatus::Checking;

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
        self.checking = NetCheckStatus::CheckEnd;
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
        // The whole end of a run is the guard's and not three lines below the
        // loop: a probe that panics unwinds past them, and the core, the plan
        // and the request are given back by [`RunOn::drop`] instead.
        let run = RunOn {
            checking: &mut self.checking,
            check_list: &mut self.check_list,
            request: &mut self.check_request,
        };
        for kind in plan {
            if self.cancel.is_cancelled() || run.request.check_status == CheckStatus::CheckFinish {
                break;
            }
            do_check(kind, run.request);
        }
        std::mem::take(&mut run.request.checkresult_profiles)
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

    /// Whether a check is in flight: the request was taken and its run has
    /// not ended yet.
    pub fn is_checking(&self) -> bool {
        self.checking == NetCheckStatus::Checking
    }

    /// `netcheck_status_` — where the diagnosis as a whole is.
    ///
    /// A caller that was handed results asks for this and for
    /// [`SdtCore::is_cancelled`] together: [`NetCheckStatus::CheckEnd`] of a
    /// cancelled run is a diagnosis that was cut short, and the results are
    /// the ones the checks before the cancel recorded — which is not the same
    /// thing as a run that finished on its own, and which nothing in the
    /// results themselves says.
    pub fn status(&self) -> NetCheckStatus {
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
