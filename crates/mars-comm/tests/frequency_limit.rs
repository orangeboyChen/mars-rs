//! `CommFrequencyLimit`, checked against mars/comm/comm_frequency_limit.cc.
//!
//! The C++ passes while `size() <= count` — so `count + 1` calls go through —
//! and then blocks until the oldest recorded call falls out of the window.

use mars_comm::FrequencyLimit;

#[test]
fn lets_the_budget_through_and_then_blocks() {
    let mut limit = FrequencyLimit::new(2, 1000);
    assert!(limit.check_at(1));
    assert!(limit.check_at(2));
    assert!(limit.check_at(3));
    assert!(!limit.check_at(4));
    assert!(!limit.check_at(10));
    assert!(!limit.check_at(1000));
}

#[test]
fn opens_again_as_the_oldest_calls_fall_out_of_the_window() {
    let mut limit = FrequencyLimit::new(2, 1000);
    assert!(limit.check_at(1));
    assert!(limit.check_at(2));
    assert!(limit.check_at(3));
    assert!(!limit.check_at(4));

    // 1002 - 1 > 1000, so the oldest touch is deleted and this call passes
    assert!(limit.check_at(1002));
    assert!(limit.check_at(1003));
    assert!(limit.check_at(1004));
    // three touches inside the new window again
    assert!(!limit.check_at(1005));
}

#[test]
fn repairs_the_history_when_the_clock_goes_backwards() {
    let mut limit = FrequencyLimit::new(2, 10_000);
    assert!(limit.check_at(5000));
    assert!(limit.check_at(6000));
    assert!(limit.check_at(7000));
    assert!(!limit.check_at(8000));

    // The user changed the clock. The C++ does not drop the history (that
    // would hand out a fresh budget) — it rewrites every recorded touch to
    // `now - 1` so the window keeps expiring against the new clock.
    assert!(!limit.check_at(10));
    assert_eq!(limit.len(), 3);
    assert!(limit.touch_times_are_amended());
    // 10_011 - 9 > 10_000: the window has expired and the budget is back
    assert!(limit.check_at(10_011));
}

#[test]
fn counts_against_the_real_clock() {
    let mut limit = FrequencyLimit::new(1, 60_000);
    assert!(limit.check());
    assert!(limit.check());
    assert!(!limit.check());
    assert!(!limit.is_empty());
}

#[test]
#[should_panic]
fn rejects_a_zero_budget() {
    FrequencyLimit::new(0, 1000);
}

#[test]
#[should_panic]
fn rejects_a_zero_span() {
    FrequencyLimit::new(1, 0);
}
