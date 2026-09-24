//! `mars/stn/src/weak_network_logic.cc` — when the network is called weak.
//!
//! STN marks the network weak when a connect is slow or falls back to another
//! ip, when a package takes too long, or when a task fails; it stops marking it
//! weak when a task is quick again, when a connect fails, after
//! [`MARK_TIMEOUT`], or when the app goes to the background. Everything the
//! network does while weak is reported as a `(key, value, is_important)` pair,
//! which is what [`WeakKey`] names.
//!
//! The C++ asks `ActiveLogic::Instance()->IsForeground()` for the one thing it
//! cannot see for itself, and subscribes to `SignalForeground` for the change.
//! Here the host says so: [`WeakNetworkLogic::on_foreground`] is that signal
//! and the flag starts out foreground, like the C++'s `ActiveLogic` does.
//!
//! The clock is [`mars_comm::tickcount::gettickcount`]; the `*_at` methods take
//! a reading instead, which is what makes the spans testable.

use mars_comm::tickcount::gettickcount;

use crate::task_profile::{ErrCmdType, TaskFailStep, TaskOutcome};

/// `MARK_TIMEOUT` — how long a weak network stays marked without a new reason.
pub const MARK_TIMEOUT: u64 = 60 * 1000;
/// `WEAK_CONNECT_RTT`.
pub const WEAK_CONNECT_RTT: i32 = 2 * 1000;
/// `WEAK_PKG_SPAN`.
pub const WEAK_PKG_SPAN: i32 = 2 * 1000;
/// `GOOD_TASK_SPAN`.
pub const GOOD_TASK_SPAN: u64 = 600;
/// `SURE_WEAK_SPAN`.
pub const SURE_WEAK_SPAN: u64 = 5 * 1000;
/// `WEAK_TASK_SPAN`.
pub const WEAK_TASK_SPAN: u64 = 5 * 1000;
/// `WEAK_LEAST_SPAN` — how long a weak network is marked before it may be
/// un-marked again.
pub const WEAK_LEAST_SPAN: u64 = 8 * 1000;

/// `TKey` — "do not delete or insert": the values are what the host's report
/// sees, and the `kFailStep*` ones are an offset `GetFailStep()` is added to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeakKey {
    /// `kEnterWeak`
    EnterWeak = 0,
    /// `kExitWeak`
    ExitWeak,
    /// `kWeakTime`
    WeakTime,
    /// `kCGICount`
    CgiCount,
    /// `kCGICost`
    CgiCost,
    /// `kCGISucc`
    CgiSucc,
    /// `kSceneRtt`
    SceneRtt,
    /// `kSceneIndex`
    SceneIndex,
    /// `kSceneFirstPkg`
    SceneFirstPkg,
    /// `kScenePkgPkg`
    ScenePkgPkg,
    /// `kSceneTask`
    SceneTask,
    /// `kExitSceneTask`
    ExitSceneTask,
    /// `kExitSceneTimeout`
    ExitSceneTimeout,
    /// `kExitSceneConnect`
    ExitSceneConnect,
    /// `kExitSceneBackground`
    ExitSceneBackground,
    /// `kExitQuickConnectNoNet`
    ExitQuickConnectNoNet,
    /// `kSceneTaskBad`
    SceneTaskBad,
    /// `kFailStepDns`
    FailStepDns = 31,
    /// `kFailStepConnect`
    FailStepConnect,
    /// `kFailStepFirstPkg`
    FailStepFirstPkg,
    /// `kFailStepPkgPkg`
    FailStepPkgPkg,
    /// `kFailStepDecode`
    FailStepDecode,
    /// `kFailStepOther`
    FailStepOther,
    /// `kFailStepTimeout`
    FailStepTimeout,
    /// `kFailStepServer`
    FailStepServer,
    /// `kFailCurrent`
    FailCurrent,
    /// `kFailSecond`
    FailSecond,
    /// `kFailThird`
    FailThird,
    /// `kFailMore`
    FailMore,
}

impl WeakKey {
    /// `kFailStepDns + _task_profile.GetFailStep() - 1` — `None` for
    /// [`TaskFailStep::Succ`], which is the one step the C++ never reports (it
    /// only asks for the step of a task that failed).
    pub fn for_fail_step(step: TaskFailStep) -> Option<Self> {
        Some(match step {
            TaskFailStep::Succ => return None,
            TaskFailStep::Dns => Self::FailStepDns,
            TaskFailStep::Connect => Self::FailStepConnect,
            TaskFailStep::FirstPkg => Self::FailStepFirstPkg,
            TaskFailStep::PkgPkg => Self::FailStepPkgPkg,
            TaskFailStep::Decode => Self::FailStepDecode,
            TaskFailStep::Other => Self::FailStepOther,
            TaskFailStep::Timeout => Self::FailStepTimeout,
            TaskFailStep::Server => Self::FailStepServer,
        })
    }

    /// `kFailCurrent + (cgi_fail_num >= 4 ? 4 : cgi_fail_num) - 1` — how many
    /// tasks have failed since the network was marked weak, counted up to
    /// "more".
    pub fn for_fail_count(count: u32) -> Self {
        match count {
            1 => Self::FailCurrent,
            2 => Self::FailSecond,
            3 => Self::FailThird,
            _ => Self::FailMore,
        }
    }
}

/// `report_weak_logic_` — who is told what the network is doing.
pub type ReportWeak = dyn FnMut(WeakKey, i32, bool) + Send;

/// `WeakNetworkLogic`.
pub struct WeakNetworkLogic {
    report: Option<Box<ReportWeak>>,
    /// What the C++ asks `ActiveLogic::Instance()` for.
    foreground: bool,
    /// `first_mark_tick_` — `None` is an invalid `tickcount_t`.
    first_mark_tick: Option<u64>,
    /// `last_mark_tick_`.
    last_mark_tick: Option<u64>,
    /// `is_curr_weak_`.
    is_curr_weak: bool,
    /// `connect_after_weak_`.
    connect_after_weak: u32,
    /// `last_connect_fail_tick_`.
    last_connect_fail_tick: Option<u64>,
    /// `last_connect_suc_tick_`.
    last_connect_suc_tick: Option<u64>,
    /// `cgi_fail_num_`.
    cgi_fail_num: u32,
}

impl Default for WeakNetworkLogic {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for WeakNetworkLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WeakNetworkLogic")
            .field("has_report", &self.report.is_some())
            .field("foreground", &self.foreground)
            .field("is_curr_weak", &self.is_curr_weak)
            .field("connect_after_weak", &self.connect_after_weak)
            .field("cgi_fail_num", &self.cgi_fail_num)
            .finish()
    }
}

impl WeakNetworkLogic {
    /// `WeakNetworkLogic()` — not weak, foreground, and no report set.
    pub fn new() -> Self {
        Self {
            report: None,
            foreground: true,
            first_mark_tick: None,
            last_mark_tick: None,
            is_curr_weak: false,
            connect_after_weak: 0,
            last_connect_fail_tick: None,
            last_connect_suc_tick: None,
            cgi_fail_num: 0,
        }
    }

    /// `report_weak_logic_` — who is told what the network is doing.
    pub fn set_report(&mut self, report: impl FnMut(WeakKey, i32, bool) + Send + 'static) {
        self.report = Some(Box::new(report));
    }

    /// `report_weak_logic_ = NULL`.
    pub fn clear_report(&mut self) {
        self.report = None;
    }

    /// `__SignalForeground` — what the C++ connects to
    /// `ActiveLogic::Instance()->SignalForeground`. Going to the background
    /// ends a weak network.
    pub fn on_foreground(&mut self, is_foreground: bool) {
        self.on_foreground_at(gettickcount(), is_foreground);
    }

    /// [`WeakNetworkLogic::on_foreground`] at a reading the caller took.
    pub fn on_foreground_at(&mut self, now: u64, is_foreground: bool) {
        if !is_foreground && self.is_curr_weak {
            self.mark_weak(false, now);
            self.report(WeakKey::ExitSceneBackground, 1, false);
        }
        self.foreground = is_foreground;
    }

    /// `ActiveLogic::Instance()->IsForeground()`.
    pub fn is_foreground(&self) -> bool {
        self.foreground
    }

    /// `IsCurrentNetworkWeak()` — and the mark is dropped once nothing has
    /// renewed it for [`MARK_TIMEOUT`].
    pub fn is_current_network_weak(&mut self) -> bool {
        self.is_current_network_weak_at(gettickcount())
    }

    /// [`WeakNetworkLogic::is_current_network_weak`] at a reading the caller
    /// took.
    pub fn is_current_network_weak_at(&mut self, now: u64) -> bool {
        if !self.is_curr_weak {
            return false;
        }
        if self.span_since(self.last_mark_tick, now) < MARK_TIMEOUT {
            return true;
        }
        self.mark_weak(false, now);
        self.report(WeakKey::ExitSceneTimeout, 1, false);
        false
    }

    /// `IsLastValidConnectFail(_span)` — `None` when there is no reading at
    /// all, `Some((true, span))` when the last connect failed and
    /// `Some((false, span))` when it succeeded.
    pub fn is_last_valid_connect_fail(&self) -> Option<(bool, u64)> {
        self.is_last_valid_connect_fail_at(gettickcount())
    }

    /// [`WeakNetworkLogic::is_last_valid_connect_fail`] at a reading the caller
    /// took.
    pub fn is_last_valid_connect_fail_at(&self, now: u64) -> Option<(bool, u64)> {
        if let Some(tick) = self.last_connect_fail_tick {
            Some((true, now.saturating_sub(tick)))
        } else {
            self.last_connect_suc_tick
                .map(|tick| (false, now.saturating_sub(tick)))
        }
    }

    /// `OnConnectEvent(_is_suc, _rtt, _index)`.
    pub fn on_connect_event(&mut self, is_suc: bool, rtt: i32, index: i32) {
        self.on_connect_event_at(gettickcount(), is_suc, rtt, index);
    }

    /// [`WeakNetworkLogic::on_connect_event`] at a reading the caller took.
    pub fn on_connect_event_at(&mut self, now: u64, is_suc: bool, rtt: i32, index: i32) {
        if is_suc {
            self.last_connect_fail_tick = None;
            self.last_connect_suc_tick = Some(now);
        } else {
            self.last_connect_fail_tick = Some(now);
            self.last_connect_suc_tick = None;
        }

        // the C++ ignores everything but the two ticks in the background
        if !self.foreground {
            return;
        }

        if self.is_curr_weak {
            self.connect_after_weak += 1;
        }

        if !is_suc {
            if self.is_curr_weak && self.span_since(self.last_mark_tick, now) >= WEAK_LEAST_SPAN {
                self.mark_weak(false, now);
                self.report(WeakKey::ExitSceneConnect, 1, false);
                // a network that was weak for less than SURE_WEAK_SPAN and
                // then lost the very first connect is reported as "no network"
                // rather than as a weak one — which cannot happen with these
                // constants, because a mark has to be WEAK_LEAST_SPAN old
                // before a connect ends it and that is the longer span of the
                // two, so `kExitQuickConnectNoNet` is never reported
                if self.connect_after_weak <= 1
                    && self.span_since(self.first_mark_tick, now) < SURE_WEAK_SPAN
                {
                    self.report(WeakKey::ExitQuickConnectNoNet, 1, false);
                }
            }
            return;
        }

        let mut is_weak = false;
        if index > 0 {
            is_weak = true;
            if !self.is_curr_weak {
                self.report(WeakKey::SceneIndex, 1, false);
            }
        } else if rtt > WEAK_CONNECT_RTT {
            is_weak = true;
            if !self.is_curr_weak {
                self.report(WeakKey::SceneRtt, 1, false);
            }
        }

        if is_weak {
            if !self.is_curr_weak {
                self.mark_weak(true, now);
            } else {
                self.last_mark_tick = Some(now);
            }
        }
    }

    /// `OnPkgEvent(_is_firstpkg, _span)`.
    pub fn on_pkg_event(&mut self, is_first_pkg: bool, span: i32) {
        self.on_pkg_event_at(gettickcount(), is_first_pkg, span);
    }

    /// [`WeakNetworkLogic::on_pkg_event`] at a reading the caller took.
    pub fn on_pkg_event_at(&mut self, now: u64, is_first_pkg: bool, span: i32) {
        if !self.foreground {
            return;
        }
        if span <= WEAK_PKG_SPAN {
            return;
        }
        if !self.is_curr_weak {
            self.mark_weak(true, now);
            self.report(
                if is_first_pkg {
                    WeakKey::SceneFirstPkg
                } else {
                    WeakKey::ScenePkgPkg
                },
                1,
                false,
            );
        } else {
            self.last_mark_tick = Some(now);
        }
    }

    /// `OnTaskEvent(_task_profile)`.
    pub fn on_task_event(&mut self, outcome: &TaskOutcome) {
        self.on_task_event_at(gettickcount(), outcome);
    }

    /// [`WeakNetworkLogic::on_task_event`] at a reading the caller took.
    pub fn on_task_event_at(&mut self, now: u64, outcome: &TaskOutcome) {
        if !self.foreground {
            return;
        }

        let old_weak = self.is_curr_weak;
        let mut is_weak = false;
        // a task that had to fall back to another ip and then failed for
        // anything other than the codec
        if outcome.ip_index > 0
            && outcome.err_type != ErrCmdType::Ok
            && outcome.err_type != ErrCmdType::EnDecode
        {
            is_weak = true;
            if !self.is_curr_weak {
                self.report(WeakKey::SceneTask, 1, false);
            }
        } else if outcome.err_type == ErrCmdType::Ok && outcome.cost() >= WEAK_TASK_SPAN {
            is_weak = true;
            if !self.is_curr_weak {
                self.report(WeakKey::SceneTaskBad, 1, false);
            }
        }

        if is_weak {
            if !self.is_curr_weak {
                self.mark_weak(true, now);
            } else {
                self.last_mark_tick = Some(now);
            }
        } else if outcome.err_type == ErrCmdType::Ok
            && outcome.cost() < GOOD_TASK_SPAN
            && self.is_curr_weak
            && self.span_since(self.last_mark_tick, now) >= WEAK_LEAST_SPAN
        {
            self.mark_weak(false, now);
            self.report(WeakKey::ExitSceneTask, 1, false);
        }

        // what happened while the network was weak, weak or not any more
        if self.is_curr_weak || old_weak {
            self.report(WeakKey::CgiCount, 1, false);
            if outcome.err_type == ErrCmdType::Ok {
                self.report(WeakKey::CgiSucc, 1, false);
                self.report(WeakKey::CgiCost, outcome.cost() as i32, false);
            } else {
                self.cgi_fail_num += 1;
                if let Some(key) = WeakKey::for_fail_step(outcome.fail_step()) {
                    self.report(key, 1, false);
                }
                self.report(WeakKey::for_fail_count(self.cgi_fail_num), 1, false);
            }
        }
    }

    /// `is_curr_weak_`.
    pub fn is_weak(&self) -> bool {
        self.is_curr_weak
    }

    /// `connect_after_weak_` — the connects since the network was marked weak.
    pub fn connect_after_weak(&self) -> u32 {
        self.connect_after_weak
    }

    /// `cgi_fail_num_` — the failed tasks since the network was marked weak.
    pub fn cgi_fail_num(&self) -> u32 {
        self.cgi_fail_num
    }

    /// `__MarkWeak(_isWeak)` — `now` is only read for the span a weak network
    /// is reported to have lasted.
    fn mark_weak(&mut self, is_weak: bool, now: u64) {
        if is_weak {
            self.is_curr_weak = true;
            self.connect_after_weak = 0;
            self.cgi_fail_num = 0;
            self.first_mark_tick = Some(now);
            self.last_mark_tick = Some(now);
            self.report(WeakKey::EnterWeak, 1, false);
        } else {
            self.is_curr_weak = false;
            self.report(WeakKey::ExitWeak, 1, false);
            self.report(
                WeakKey::WeakTime,
                self.span_since(self.first_mark_tick, now) as i32,
                false,
            );
        }
    }

    /// `__ReportWeakLogic(_key, _value, _is_important)`.
    fn report(&mut self, key: WeakKey, value: i32, is_important: bool) {
        if let Some(report) = self.report.as_mut() {
            report(key, value, is_important);
        }
    }

    /// `gettickspan` of a tick that may be invalid — an invalid one is "as long
    /// ago as it gets", which is what the C++ gets from a zeroed `tickcount_t`.
    fn span_since(&self, tick: Option<u64>, now: u64) -> u64 {
        tick.map_or(u64::MAX, |tick| now.saturating_sub(tick))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// The (key, value) pairs the logic reported, in order.
    #[derive(Debug, Default, Clone)]
    struct Reports(Arc<Mutex<Vec<(WeakKey, i32)>>>);

    impl Reports {
        fn keys(&self) -> Vec<WeakKey> {
            self.0.lock().unwrap().iter().map(|(k, _)| *k).collect()
        }

        fn value_of(&self, key: WeakKey) -> Option<i32> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| *v)
        }

        fn count_of(&self, key: WeakKey) -> usize {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|(k, _)| *k == key)
                .count()
        }
    }

    fn reporting(reports: &Reports) -> WeakNetworkLogic {
        let mut logic = WeakNetworkLogic::new();
        let shared = Arc::clone(&reports.0);
        logic.set_report(move |key, value, _| {
            shared.lock().unwrap().push((key, value));
        });
        logic
    }

    #[test]
    fn a_slow_connect_marks_the_network_weak() {
        let reports = Reports::default();
        let mut logic = reporting(&reports);
        assert!(!logic.is_weak());

        logic.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT + 1, 0);
        assert!(logic.is_weak(), "a connect slower than WEAK_CONNECT_RTT");
        assert_eq!(reports.keys(), vec![WeakKey::SceneRtt, WeakKey::EnterWeak]);
        assert_eq!(
            logic.is_last_valid_connect_fail_at(1_500),
            Some((false, 500))
        );

        // and the mark is renewed by another reason, and holds
        logic.on_pkg_event_at(2_000, false, WEAK_PKG_SPAN + 1);
        assert!(logic.is_current_network_weak_at(2_000));
        // ... until nothing renews it for MARK_TIMEOUT
        assert!(!logic.is_current_network_weak_at(2_000 + MARK_TIMEOUT));
        assert_eq!(
            reports.keys().last(),
            Some(&WeakKey::ExitSceneTimeout),
            "the timeout is why it stopped being weak"
        );
    }

    #[test]
    fn a_connect_that_fell_back_to_another_ip_marks_the_network_weak() {
        let reports = Reports::default();
        let mut logic = reporting(&reports);
        // the rtt is fine, but the connect used the second ip
        logic.on_connect_event_at(1_000, true, 10, 1);
        assert!(logic.is_weak());
        assert_eq!(
            reports.keys(),
            vec![WeakKey::SceneIndex, WeakKey::EnterWeak]
        );
    }

    #[test]
    fn a_failed_connect_ends_a_weak_network() {
        let reports = Reports::default();
        let mut logic = reporting(&reports);
        logic.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT + 1, 0);
        assert!(logic.is_weak());

        // the connect after the mark has to wait WEAK_LEAST_SPAN: at 5 s
        // nothing happens yet
        logic.on_connect_event_at(1_000 + 5_000, false, 0, 0);
        assert!(logic.is_weak(), "WEAK_LEAST_SPAN has not passed");
        assert_eq!(logic.connect_after_weak(), 1);

        // at 9 s the mark is dropped, and the connect is what ended it
        logic.on_connect_event_at(1_000 + 9_000, false, 0, 0);
        assert!(!logic.is_weak());
        assert_eq!(logic.connect_after_weak(), 2);
        assert_eq!(
            reports.keys(),
            vec![
                WeakKey::SceneRtt,
                WeakKey::EnterWeak,
                WeakKey::ExitWeak,
                WeakKey::WeakTime,
                WeakKey::ExitSceneConnect,
            ]
        );
        // `kExitQuickConnectNoNet` is never reported: it needs the mark to be
        // older than WEAK_LEAST_SPAN (8 s) and younger than SURE_WEAK_SPAN
        // (5 s) at the same time, and the second span is the shorter one
        assert_eq!(reports.count_of(WeakKey::ExitQuickConnectNoNet), 0);
        assert_eq!(
            logic.is_last_valid_connect_fail_at(10_100),
            Some((true, 100))
        );
    }

    #[test]
    fn a_quick_task_ends_a_weak_network() {
        let reports = Reports::default();
        let mut logic = reporting(&reports);
        logic.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT + 1, 0);

        // a slow task keeps it weak and is reported
        let slow = TaskOutcome::new(1_000, 1_000 + WEAK_TASK_SPAN);
        logic.on_task_event_at(2_000, &slow);
        assert!(logic.is_weak());
        assert_eq!(reports.count_of(WeakKey::CgiCount), 1);
        assert_eq!(
            reports.value_of(WeakKey::CgiCost),
            Some(WEAK_TASK_SPAN as i32)
        );

        // a quick one ends it, but only after WEAK_LEAST_SPAN
        let quick = TaskOutcome::new(2_000, 2_000 + GOOD_TASK_SPAN - 1);
        logic.on_task_event_at(2_000 + 5_000, &quick);
        assert!(logic.is_weak(), "WEAK_LEAST_SPAN has not passed");
        logic.on_task_event_at(2_000 + 9_000, &TaskOutcome::new(9_000, 9_100));
        assert!(!logic.is_weak());
        assert!(reports.keys().contains(&WeakKey::ExitSceneTask));
    }

    #[test]
    fn a_failed_task_marks_the_network_weak_and_counts_the_failures() {
        let reports = Reports::default();
        let mut logic = reporting(&reports);

        // a task that had to fall back to another ip and then failed
        let failed = TaskOutcome {
            err_type: ErrCmdType::Socket,
            ip_index: 1,
            last_receive_pkg_time: 500,
            ..TaskOutcome::new(1_000, 2_000)
        };
        logic.on_task_event_at(2_000, &failed);
        assert!(logic.is_weak());
        assert_eq!(
            reports.keys(),
            vec![
                WeakKey::SceneTask,
                WeakKey::EnterWeak,
                WeakKey::CgiCount,
                WeakKey::FailStepPkgPkg,
                WeakKey::FailCurrent,
            ]
        );

        // ... and the next failures are counted
        let reports = Reports::default();
        let mut second = reporting(&reports);
        second.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT + 1, 0);
        for at in [2_000, 3_000, 4_000, 5_000, 6_000] {
            second.on_task_event_at(at, &failed);
        }
        assert_eq!(second.cgi_fail_num(), 5);
        assert_eq!(reports.count_of(WeakKey::FailCurrent), 1);
        assert_eq!(reports.count_of(WeakKey::FailSecond), 1);
        assert_eq!(reports.count_of(WeakKey::FailThird), 1);
        // everything after the third is "more"
        assert_eq!(reports.count_of(WeakKey::FailMore), 2);
    }

    #[test]
    fn the_background_ends_a_weak_network_and_ignores_what_follows() {
        let reports = Reports::default();
        let mut logic = reporting(&reports);
        logic.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT + 1, 0);
        assert!(logic.is_weak());

        logic.on_foreground_at(1_500, false);
        assert!(!logic.is_weak());
        assert_eq!(reports.keys().last(), Some(&WeakKey::ExitSceneBackground));

        // in the background nothing is judged at all, but the connect ticks
        // still move
        logic.on_connect_event_at(2_000, true, WEAK_CONNECT_RTT + 1, 0);
        logic.on_pkg_event_at(2_000, true, WEAK_PKG_SPAN + 1);
        logic.on_task_event_at(
            2_000,
            &TaskOutcome {
                err_type: ErrCmdType::Socket,
                ip_index: 1,
                last_receive_pkg_time: 500,
                ..TaskOutcome::new(1_000, 2_000)
            },
        );
        assert!(!logic.is_weak(), "the background is not judged");
        assert_eq!(
            logic.is_last_valid_connect_fail_at(2_100),
            Some((false, 100))
        );

        logic.on_foreground_at(3_000, true);
        assert!(logic.is_foreground());
    }

    #[test]
    fn nothing_is_weak_without_a_reason() {
        let reports = Reports::default();
        let mut logic = reporting(&reports);
        // and there has been no connect yet
        assert_eq!(logic.is_last_valid_connect_fail_at(1_000), None);

        logic.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT, 0);
        logic.on_pkg_event_at(1_000, true, WEAK_PKG_SPAN);
        logic.on_task_event_at(1_000, &TaskOutcome::new(1_000, 1_000 + GOOD_TASK_SPAN));
        assert!(!logic.is_weak());
        assert!(!logic.is_current_network_weak_at(1_000));
        assert!(reports.keys().is_empty(), "nothing was reported");
    }

    #[test]
    fn the_methods_that_take_no_reading_ask_the_clock_themselves() {
        let reports = Reports::default();
        let mut logic = reporting(&reports);
        // nothing here is weak, so whatever the clock says the answers are the
        // ones the `*_at` methods give
        logic.on_connect_event(true, 10, 0);
        logic.on_pkg_event(true, 10);
        logic.on_task_event(&TaskOutcome::new(0, 100));
        logic.on_foreground(true);
        assert!(!logic.is_current_network_weak());
        assert_eq!(
            logic.is_last_valid_connect_fail().map(|(failed, _)| failed),
            Some(false),
            "the only connect succeeded"
        );
        assert!(logic.is_foreground());
        assert!(reports.keys().is_empty());
    }

    #[test]
    fn the_first_package_and_the_next_one_are_reported_apart() {
        for (is_first_pkg, expected) in [
            (true, WeakKey::SceneFirstPkg),
            (false, WeakKey::ScenePkgPkg),
        ] {
            let reports = Reports::default();
            let mut logic = reporting(&reports);
            logic.on_pkg_event_at(1_000, is_first_pkg, WEAK_PKG_SPAN + 1);
            assert!(logic.is_weak());
            // the C++ marks the network weak first and names the reason after
            assert_eq!(reports.keys(), vec![WeakKey::EnterWeak, expected]);
        }
    }
}
