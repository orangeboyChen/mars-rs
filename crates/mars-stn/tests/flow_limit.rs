//! `mars/stn/src/flow_limit.cc` — the byte funnel.
//!
//! The drain is computed from whole seconds since the last call, so the tests
//! pass the tick count explicitly ([`FlowLimit::check_at`]) instead of sleeping.

use mars_stn::config::{ACTIVE_SPEED, INACTIVE_MIN_VOL, INACTIVE_SPEED, MAX_VOL};
use mars_stn::flow_limit::FlowLimit;
use mars_stn::Task;

const T0: u64 = 1_000_000;

fn task() -> Task {
    Task::new(1, 1)
}

#[test]
fn the_speeds_match_the_cpp() {
    assert_eq!(ACTIVE_SPEED, 80 * 1024 * 1024 / 3600);
    assert_eq!(INACTIVE_SPEED, 20 * 1024 * 1024 / 3600);
    assert_eq!(INACTIVE_MIN_VOL, 60 * 1024 * 1024);
    assert_eq!(MAX_VOL, 80 * 1024 * 1024);

    assert_eq!(FlowLimit::new_at(true, T0).funnel_speed(), ACTIVE_SPEED);
    assert_eq!(FlowLimit::new_at(false, T0).funnel_speed(), INACTIVE_SPEED);
}

#[test]
fn a_task_that_does_not_limit_flow_is_never_charged() {
    let mut limit = FlowLimit::new_at(true, T0);
    let mut task = task();
    task.limit_flow = false;

    for i in 0..10 {
        assert!(limit.check_at(&task, MAX_VOL, T0 + i * 1000));
    }
    assert_eq!(limit.current_volume(), 0);
}

#[test]
fn the_funnel_caps_a_task_at_max_vol() {
    let mut limit = FlowLimit::new_at(true, T0);
    let task = task();

    assert!(limit.check_at(&task, MAX_VOL, T0));
    assert_eq!(limit.current_volume(), MAX_VOL);
    // nothing left: even one byte more is refused, and the volume is untouched
    assert!(!limit.check_at(&task, 1, T0));
    assert_eq!(limit.current_volume(), MAX_VOL);

    // a task that fits exactly still goes out
    let mut fresh = FlowLimit::new_at(true, T0);
    assert!(fresh.check_at(&task, MAX_VOL - 1, T0));
    assert!(fresh.check_at(&task, 1, T0));
    assert!(!fresh.check_at(&task, 1, T0));
}

#[test]
fn nothing_drains_inside_the_second_it_was_charged() {
    let mut limit = FlowLimit::new_at(true, T0);
    let task = task();

    assert!(limit.check_at(&task, 1000, T0));
    assert!(limit.check_at(&task, 1000, T0 + 999));
    assert_eq!(limit.current_volume(), 2000);
}

#[test]
fn the_funnel_drains_by_the_speed_times_the_whole_seconds() {
    let mut limit = FlowLimit::new_at(true, T0);
    let task = task();

    assert!(limit.check_at(&task, 100_000, T0));
    // one second of the foreground speed
    assert!(limit.check_at(&task, 1, T0 + 1000));
    assert_eq!(limit.current_volume(), 100_000 - ACTIVE_SPEED + 1);
    // three more seconds
    assert!(limit.check_at(&task, 1, T0 + 4000));
    assert_eq!(limit.current_volume(), 100_000 - 4 * ACTIVE_SPEED + 2);
}

#[test]
fn the_background_drains_four_times_slower() {
    let mut limit = FlowLimit::new_at(false, T0);
    let task = task();

    assert!(limit.check_at(&task, 100_000, T0));
    assert!(limit.check_at(&task, 0, T0 + 10_000));
    assert_eq!(limit.current_volume(), 100_000 - 10 * INACTIVE_SPEED);
}

#[test]
fn the_volume_never_drains_below_zero() {
    let mut limit = FlowLimit::new_at(true, T0);
    let task = task();

    assert!(limit.check_at(&task, 100, T0));
    assert!(limit.check_at(&task, 0, T0 + 600_000));
    assert_eq!(limit.current_volume(), 0);
}

#[test]
fn going_to_the_background_caps_the_volume_and_the_speed() {
    let mut limit = FlowLimit::new_at(true, T0);
    let task = task();

    assert!(limit.check_at(&task, MAX_VOL, T0));
    limit.set_active_at(false, T0);
    assert_eq!(limit.current_volume(), INACTIVE_MIN_VOL);
    assert_eq!(limit.funnel_speed(), INACTIVE_SPEED);

    // a volume below the cap is left alone, and the foreground only raises the
    // speed
    let mut small = FlowLimit::new_at(true, T0);
    assert!(small.check_at(&task, 1024, T0));
    small.set_active_at(false, T0);
    assert_eq!(small.current_volume(), 1024);
    assert_eq!(small.funnel_speed(), INACTIVE_SPEED);
    small.set_active_at(true, T0);
    assert_eq!(small.current_volume(), 1024);
    assert_eq!(small.funnel_speed(), ACTIVE_SPEED);
}

#[test]
fn the_funnel_runs_against_the_real_clock_too() {
    let mut limit = FlowLimit::new(true);
    let task = task();
    assert!(limit.check(&task, 4096));
    assert_eq!(limit.current_volume(), 4096);
    limit.set_active(false);
    assert_eq!(limit.funnel_speed(), INACTIVE_SPEED);
    limit.set_active(true);
    assert_eq!(limit.funnel_speed(), ACTIVE_SPEED);
}
