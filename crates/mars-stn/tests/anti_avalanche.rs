//! `mars/stn/src/anti_avalanche.cc` — the two gates a task has to pass.
//!
//! The frequency gate always applies, the flow gate on a mobile network only,
//! which is what the C++ does with `comm::kMobile == comm::getNetInfo()`.

use mars_stn::config::MAX_VOL;
use mars_stn::{AntiAvalanche, LimitKind, Task};

const T0: u64 = 1_000_000;

fn task() -> Task {
    Task::new(1, 1)
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
