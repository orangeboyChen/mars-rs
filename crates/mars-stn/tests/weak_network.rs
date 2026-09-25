//! `mars/stn/src/weak_network_logic.cc`, through the public api.
//!
//! The samples are the ones the C++ judges: a connect slower than
//! [`WEAK_CONNECT_RTT`] or on a fallback ip, a package slower than
//! [`WEAK_PKG_SPAN`], a task slower than [`WEAK_TASK_SPAN`], and the ways a
//! weak network is un-marked again. Every reading is passed in — the `*_at`
//! methods — so a minute of waiting is a number here.

use std::sync::{Arc, Mutex};

use mars_stn::task_profile::{ErrCmdType, TaskOutcome};
use mars_stn::weak_network::{
    WeakKey, WeakNetworkLogic, GOOD_TASK_SPAN, WEAK_CONNECT_RTT, WEAK_LEAST_SPAN, WEAK_PKG_SPAN,
    WEAK_TASK_SPAN,
};

/// The keys the logic reported, in order.
#[derive(Debug, Default, Clone)]
struct Reports(Arc<Mutex<Vec<WeakKey>>>);

impl Reports {
    fn keys(&self) -> Vec<WeakKey> {
        self.0.lock().unwrap().clone()
    }

    fn count_of(&self, key: WeakKey) -> usize {
        self.0.lock().unwrap().iter().filter(|k| **k == key).count()
    }
}

/// A logic whose report goes into `reports`.
fn reporting(reports: &Reports) -> WeakNetworkLogic {
    let mut logic = WeakNetworkLogic::new();
    let shared = Arc::clone(&reports.0);
    logic.set_report(move |key, _value, _is_important| {
        shared.lock().unwrap().push(key);
    });
    logic
}

#[test]
fn a_slow_connect_marks_the_network_weak() {
    let reports = Reports::default();
    let mut logic = reporting(&reports);

    logic.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT, 0);
    assert!(!logic.is_weak(), "exactly WEAK_CONNECT_RTT is not slow");

    logic.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT + 1, 0);
    assert!(logic.is_weak());
    assert_eq!(reports.keys(), vec![WeakKey::SceneRtt, WeakKey::EnterWeak]);
    // the connect succeeded, and that is what is on record
    assert_eq!(
        logic.is_last_valid_connect_fail_at(1_500),
        Some((false, 500))
    );
}

#[test]
fn a_connect_on_a_fallback_ip_marks_the_network_weak() {
    let reports = Reports::default();
    let mut logic = reporting(&reports);
    // a fast connect, but not to the first ip
    logic.on_connect_event_at(1_000, true, 10, 1);
    assert!(logic.is_weak());
    assert_eq!(
        reports.keys(),
        vec![WeakKey::SceneIndex, WeakKey::EnterWeak]
    );
}

#[test]
fn a_package_that_took_too_long_marks_the_network_weak() {
    let reports = Reports::default();
    let mut logic = reporting(&reports);
    logic.on_pkg_event_at(1_000, true, WEAK_PKG_SPAN);
    assert!(!logic.is_weak(), "exactly WEAK_PKG_SPAN is not slow");

    logic.on_pkg_event_at(1_000, false, WEAK_PKG_SPAN + 1);
    assert!(logic.is_weak());
    // the reason is named after the mark
    assert_eq!(
        reports.keys(),
        vec![WeakKey::EnterWeak, WeakKey::ScenePkgPkg]
    );
}

#[test]
fn a_weak_network_expires_when_nothing_renews_it() {
    let reports = Reports::default();
    let mut logic = reporting(&reports);
    logic.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT + 1, 0);

    // a slow package renews the mark
    logic.on_pkg_event_at(30_000, true, WEAK_PKG_SPAN + 1);
    assert!(logic.is_current_network_weak_at(60_000));
    // ... and 60 s after *that* it is over
    assert!(!logic.is_current_network_weak_at(60_000 + 60_000));
    assert_eq!(reports.keys().last(), Some(&WeakKey::ExitSceneTimeout));
    assert!(!logic.is_weak());
}

#[test]
fn a_failed_connect_ends_a_weak_network_after_eight_seconds() {
    let reports = Reports::default();
    let mut logic = reporting(&reports);
    logic.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT + 1, 0);

    logic.on_connect_event_at(1_000 + 5_000, false, 0, 0);
    assert!(logic.is_weak(), "WEAK_LEAST_SPAN has not passed");
    assert_eq!(logic.connect_after_weak(), 1);

    logic.on_connect_event_at(1_000 + WEAK_LEAST_SPAN, false, 0, 0);
    assert!(!logic.is_weak());
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
    assert_eq!(
        logic.is_last_valid_connect_fail_at(9_100),
        Some((true, 100))
    );
}

#[test]
fn a_quick_task_ends_a_weak_network() {
    let reports = Reports::default();
    let mut logic = reporting(&reports);
    logic.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT + 1, 0);

    // a task that took WEAK_TASK_SPAN keeps the network weak
    logic.on_task_event_at(2_000, &TaskOutcome::new(1_000, 1_000 + WEAK_TASK_SPAN));
    assert!(logic.is_weak());

    // a quick one ends it — but not before WEAK_LEAST_SPAN has passed
    logic.on_task_event_at(5_000, &TaskOutcome::new(5_000, 5_100));
    assert!(logic.is_weak(), "WEAK_LEAST_SPAN has not passed");

    // the last reason was 2 s in, so the eight seconds start there
    logic.on_task_event_at(2_000 + WEAK_LEAST_SPAN, &TaskOutcome::new(10_000, 10_100));
    assert!(!logic.is_weak());
    assert_eq!(reports.count_of(WeakKey::ExitSceneTask), 1);
}

#[test]
fn a_failed_task_marks_the_network_weak_and_is_counted() {
    let reports = Reports::default();
    let mut logic = reporting(&reports);

    // a task that went to a fallback ip and then failed on the socket
    let failed = TaskOutcome {
        err_type: ErrCmdType::Socket,
        ip_index: 1,
        last_receive_pkg_time: 1_500,
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

    // every failure after it is counted, up to "more"
    let reports = Reports::default();
    let mut logic = reporting(&reports);
    for at in [2_000, 3_000, 4_000, 5_000] {
        logic.on_task_event_at(at, &failed);
    }
    assert_eq!(logic.cgi_fail_num(), 4);
    assert_eq!(reports.count_of(WeakKey::FailCurrent), 1);
    assert_eq!(reports.count_of(WeakKey::FailSecond), 1);
    assert_eq!(reports.count_of(WeakKey::FailThird), 1);
    assert_eq!(reports.count_of(WeakKey::FailMore), 1);
    // and the mark is what reset the counter
    assert!(logic.is_weak());
}

#[test]
fn the_background_ends_a_weak_network_and_is_not_judged() {
    let reports = Reports::default();
    let mut logic = reporting(&reports);
    logic.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT + 1, 0);
    assert!(logic.is_weak());

    logic.on_foreground_at(1_500, false);
    assert!(!logic.is_weak());
    assert_eq!(reports.keys().last(), Some(&WeakKey::ExitSceneBackground));

    // nothing is judged while the app is in the background
    logic.on_connect_event_at(2_000, true, WEAK_CONNECT_RTT + 1, 0);
    logic.on_pkg_event_at(2_000, true, WEAK_PKG_SPAN + 1);
    logic.on_task_event_at(
        2_000,
        &TaskOutcome {
            err_type: ErrCmdType::Socket,
            ip_index: 1,
            last_receive_pkg_time: 1_500,
            ..TaskOutcome::new(1_000, 2_000)
        },
    );
    assert!(!logic.is_weak());
    assert_eq!(reports.count_of(WeakKey::CgiCount), 0);

    logic.on_foreground_at(3_000, true);
    assert!(logic.is_foreground());
    // ... and the foreground is judged again
    logic.on_connect_event_at(3_000, true, WEAK_CONNECT_RTT + 1, 0);
    assert!(logic.is_weak());
}

#[test]
fn a_second_reason_renews_the_mark_instead_of_reporting_it_again() {
    let reports = Reports::default();
    let mut logic = reporting(&reports);
    logic.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT + 1, 0);

    // a weak network that is marked again gets no second `kEnterWeak`: the
    // reason only moves the mark forward
    logic.on_connect_event_at(30_000, true, WEAK_CONNECT_RTT + 1, 0);
    assert_eq!(reports.count_of(WeakKey::EnterWeak), 1);
    assert_eq!(reports.count_of(WeakKey::SceneRtt), 1);
    assert!(logic.is_current_network_weak_at(30_000));
    // and the 60 s start from the second one, not from the first
    assert!(logic.is_current_network_weak_at(30_000 + 59_000));
    assert!(!logic.is_current_network_weak_at(30_000 + 60_000));
}

#[test]
fn a_task_that_succeeded_but_was_slow_marks_the_network_weak() {
    let reports = Reports::default();
    let mut logic = reporting(&reports);
    // a task that answered but took WEAK_TASK_SPAN
    logic.on_task_event_at(6_000, &TaskOutcome::new(1_000, 1_000 + WEAK_TASK_SPAN));
    assert!(logic.is_weak());
    assert_eq!(
        reports.keys(),
        vec![
            WeakKey::SceneTaskBad,
            WeakKey::EnterWeak,
            WeakKey::CgiCount,
            WeakKey::CgiSucc,
            WeakKey::CgiCost
        ]
    );
}

#[test]
fn the_fail_step_is_the_offset_the_report_key_is_built_from() {
    use mars_stn::task_profile::TaskFailStep;

    for (step, expected) in [
        (TaskFailStep::Succ, None),
        (TaskFailStep::Dns, Some(WeakKey::FailStepDns)),
        (TaskFailStep::Connect, Some(WeakKey::FailStepConnect)),
        (TaskFailStep::FirstPkg, Some(WeakKey::FailStepFirstPkg)),
        (TaskFailStep::PkgPkg, Some(WeakKey::FailStepPkgPkg)),
        (TaskFailStep::Decode, Some(WeakKey::FailStepDecode)),
        (TaskFailStep::Other, Some(WeakKey::FailStepOther)),
        (TaskFailStep::Timeout, Some(WeakKey::FailStepTimeout)),
        (TaskFailStep::Server, Some(WeakKey::FailStepServer)),
    ] {
        assert_eq!(WeakKey::for_fail_step(step), expected, "{step:?}");
    }
    assert_eq!(WeakKey::FailStepDns as i32, 31);
    assert_eq!(WeakKey::FailMore as i32, 42);
}

#[test]
fn the_report_can_be_cleared_again() {
    let reports = Reports::default();
    // `Default` is the same start as `new`
    let mut logic = WeakNetworkLogic::default();
    assert!(!logic.is_weak());
    assert!(format!("{logic:?}").contains("is_curr_weak: false"));

    let shared = Arc::clone(&reports.0);
    logic.set_report(move |key, _, _| shared.lock().unwrap().push(key));
    logic.on_connect_event_at(1_000, true, WEAK_CONNECT_RTT + 1, 0);
    assert_eq!(reports.count_of(WeakKey::EnterWeak), 1);

    // `report_weak_logic_ = NULL`: what happens is not reported any more
    logic.clear_report();
    logic.on_pkg_event_at(2_000, true, WEAK_PKG_SPAN + 1);
    assert!(logic.is_weak());
    assert_eq!(reports.count_of(WeakKey::ScenePkgPkg), 0);
}

#[test]
fn a_fresh_logic_has_nothing_to_say() {
    let reports = Reports::default();
    let mut logic = reporting(&reports);
    assert!(!logic.is_weak());
    assert!(logic.is_foreground(), "like the C++'s ActiveLogic");
    assert_eq!(logic.is_last_valid_connect_fail_at(1_000), None);
    assert!(!logic.is_current_network_weak_at(1_000));
    assert!(reports.keys().is_empty());

    // a fast connect, a fast package and a quick task: still nothing
    logic.on_connect_event_at(1_000, true, 10, 0);
    logic.on_pkg_event_at(1_000, true, 10);
    logic.on_task_event_at(1_000, &TaskOutcome::new(1_000, 1_000 + GOOD_TASK_SPAN));
    assert!(!logic.is_weak());
    assert!(reports.keys().is_empty());
}
