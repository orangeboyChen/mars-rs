//! `mars/stn/src/anti_avalanche.cc` — the two gates a task has to pass.
//!
//! The frequency gate always applies, the flow gate on a mobile network only,
//! which is what the C++ does with `comm::kMobile == comm::getNetInfo()`.
//!
//! A task a gate refused is reported to the app (`ReportTaskLimited`, `:47`
//! and `:52`), which is what the samples at the end of this file are about.

use std::sync::{Arc, Mutex};

use marsrs_stn::config::MAX_VOL;
use marsrs_stn::{AntiAvalanche, LimitKind, Task};

const T0: u64 = 1_000_000;

/// What the app was asked about a refused task: which gate, and the measure it
/// refused it by.
type Asked = Vec<(LimitKind, u32)>;

fn task() -> Task {
    Task::new(1, 1)
}

/// A gate whose app writes down what it was asked, and answers `answer`.
fn reported(answer: u32) -> (AntiAvalanche, Arc<Mutex<Asked>>) {
    let mut avalanche = AntiAvalanche::new_at(true, T0);
    let asked: Arc<Mutex<Asked>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&asked);
    avalanche.set_on_limited(move |kind, _task, param| {
        recorder.lock().unwrap().push((kind, param));
        answer
    });
    (avalanche, asked)
}

#[test]
fn a_task_passes_both_gates_on_wifi() {
    let mut avalanche = AntiAvalanche::new_at(true, T0);
    assert_eq!(avalanche.check_at(&task(), b"body", false, T0), Ok(()));
    assert_eq!(avalanche.check_at(&task(), b"body", false, T0 + 1), Ok(()));
}

#[test]
fn the_flow_gate_only_applies_to_a_mobile_network() {
    let mut avalanche = AntiAvalanche::new_at(true, T0);
    let body = vec![0u8; MAX_VOL as usize + 1];

    // Wi-Fi: the flow gate is not consulted at all
    assert_eq!(avalanche.check_at(&task(), &body, false, T0), Ok(()));

    // mobile: the body does not fit in the funnel
    assert_eq!(
        avalanche.check_at(&task(), &body, true, T0),
        Err(LimitKind::Flow)
    );
}

#[test]
fn the_frequency_gate_closes_after_105_sends_of_one_body() {
    let mut avalanche = AntiAvalanche::new_at(true, T0);
    // 105 sends of one body are allowed, the 106th is the avalanche
    for send in 0..105 {
        assert_eq!(
            avalanche.check_at(&task(), b"avalanche", false, T0 + send),
            Ok(())
        );
    }
    assert_eq!(
        avalanche.check_at(&task(), b"avalanche", false, T0 + 200),
        Err(LimitKind::Frequency)
    );
    // a different body is still let through
    assert_eq!(
        avalanche.check_at(&task(), b"other", false, T0 + 200),
        Ok(())
    );
}

#[test]
fn the_frequency_gate_is_checked_before_the_flow_gate() {
    let mut avalanche = AntiAvalanche::new_at(true, T0);
    for send in 0..105 {
        assert_eq!(
            avalanche.check_at(&task(), b"avalanche", true, T0 + send),
            Ok(())
        );
    }
    // the body is far over the funnel as well, but the frequency gate is the
    // one that refuses it
    assert_eq!(
        avalanche.check_at(&task(), b"avalanche", true, T0 + 200),
        Err(LimitKind::Frequency)
    );
}

#[test]
fn on_signal_active_switches_the_funnel_speed() {
    let mut avalanche = AntiAvalanche::new_at(true, T0);
    let task = task();
    assert!(avalanche.check_at(&task, b"body", true, T0).is_ok());
    assert_eq!(
        avalanche.flow_limit().funnel_speed(),
        80 * 1024 * 1024 / 3600
    );

    avalanche.set_active_at(false, T0);
    assert_eq!(
        avalanche.flow_limit().funnel_speed(),
        20 * 1024 * 1024 / 3600
    );

    // ... and the real-clock entry point does the same
    avalanche.on_signal_active(true);
    assert_eq!(
        avalanche.flow_limit().funnel_speed(),
        80 * 1024 * 1024 / 3600
    );
}

#[test]
fn the_gates_run_against_the_real_clock_too() {
    let mut avalanche = AntiAvalanche::new(true);
    assert_eq!(avalanche.check(&task(), b"body", false), Ok(()));
    assert_eq!(avalanche.frequency_limit().records().len(), 1);
}

/// Upstream's `test1` and `test2`: a megabyte a time, until the funnel says no.
///
/// Its `checkTimes == 8` is the `kMaxVol` of the day — 8 MiB. `MAX_VOL` is
/// 80 MiB now, so the funnel holds eighty of them and not eight, and the
/// hundred the wi-fi case sends are all let through (the frequency gate's 105
/// is not reached either).
#[test]
fn a_megabyte_a_time_fills_the_funnel_on_mobile_and_not_on_wifi() {
    let megabyte = vec![0u8; 1024 * 1024];

    let mut avalanche = AntiAvalanche::new_at(true, T0);
    for send in 0..80 {
        assert_eq!(
            avalanche.check_at(&task(), &megabyte, true, T0),
            Ok(()),
            "send {send} of eighty"
        );
    }
    assert_eq!(
        avalanche.check_at(&task(), &megabyte, true, T0),
        Err(LimitKind::Flow),
        "the eighty-first megabyte does not fit"
    );

    // wi-fi: the flow gate is not consulted at all, so the loop the C++ runs
    // — a hundred of them — never leaves one behind
    let mut avalanche = AntiAvalanche::new_at(true, T0);
    for send in 0..100 {
        assert_eq!(
            avalanche.check_at(&task(), &megabyte, false, T0),
            Ok(()),
            "send {send} of a hundred"
        );
    }
}

#[test]
fn the_frequency_gate_reports_the_task_it_refused() {
    let (mut avalanche, asked) = reported(0);

    // 105 sends of one body go out, and the app hears nothing of them
    for send in 0..105 {
        assert_eq!(
            avalanche.check_at(&task(), b"avalanche", false, T0 + send),
            Ok(())
        );
    }
    assert!(
        asked.lock().unwrap().is_empty(),
        "a task that went out is not one the app is asked about"
    );

    // the 106th is the avalanche: the app is told which gate refused it, and
    // how long ago the same body went out last — the 96 ticks since the send
    // before it, which is what the C++'s `_span` is
    assert_eq!(
        avalanche.check_at(&task(), b"avalanche", false, T0 + 200),
        Err(LimitKind::Frequency)
    );
    assert_eq!(*asked.lock().unwrap(), vec![(LimitKind::Frequency, 96)]);
}

#[test]
fn the_flow_gate_reports_the_task_it_refused() {
    // the app shrinks the limit it is held to, which is what the C++'s
    // `_param` goes in and comes out of
    let (mut avalanche, asked) = reported(1024);
    let body = vec![0u8; MAX_VOL as usize + 1];

    // ... and the C++'s own callers hand that answer a temporary, so the task
    // stays refused whatever the app said
    assert_eq!(
        avalanche.check_at(&task(), &body, true, T0),
        Err(LimitKind::Flow)
    );
    assert_eq!(
        *asked.lock().unwrap(),
        vec![(LimitKind::Flow, MAX_VOL as u32 + 1)]
    );

    // wi-fi: the gate is not consulted, so the app is not asked either
    let (mut avalanche, asked) = reported(0);
    assert_eq!(avalanche.check_at(&task(), &body, false, T0), Ok(()));
    assert!(asked.lock().unwrap().is_empty());
}

#[test]
fn a_gate_with_no_app_refuses_a_task_all_the_same() {
    let mut avalanche = AntiAvalanche::new_at(true, T0);
    let body = vec![0u8; MAX_VOL as usize + 1];

    assert_eq!(
        avalanche.check_at(&task(), &body, true, T0),
        Err(LimitKind::Flow)
    );
}

#[test]
fn the_gates_report_what_they_refuse_against_the_real_clock_too() {
    let (mut avalanche, asked) = reported(0);
    assert_eq!(avalanche.check(&task(), b"body", false), Ok(()));
    assert!(asked.lock().unwrap().is_empty());

    let body = vec![0u8; MAX_VOL as usize + 1];
    assert_eq!(avalanche.check(&task(), &body, true), Err(LimitKind::Flow));
    assert_eq!(asked.lock().unwrap().len(), 1);
}
