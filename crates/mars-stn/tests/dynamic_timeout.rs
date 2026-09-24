//! `mars/stn/src/dynamic_timeout.cc` — "is the network good right now?"
//!
//! Every finished task is classified against the budgets of `stn/config.h` and
//! the last ten are kept in a sliding window. The tick count is passed
//! explicitly ([`DynamicTimeout::record_at`]) so the five-minute expiry of that
//! window is testable without waiting.

use mars_stn::config::*;
use mars_stn::dynamic_timeout::{DynamicTimeout, DynamicTimeoutStatus, NetworkKind};

const T0: u64 = 100_000;

/// A Wi-Fi package that answered within its budget.
fn good(network: NetworkKind) -> (u32, u64) {
    match network {
        NetworkKind::Wifi => (1024, 400),
        NetworkKind::Mobile => (1024, 900),
    }
}

/// A package that answered, but slower than its budget.
fn slow(network: NetworkKind) -> (u32, u64) {
    match network {
        NetworkKind::Wifi => (1024, 900),
        NetworkKind::Mobile => (1024, 1500),
    }
}

/// A bigger package that answered within its budget: this is the one that
/// refreshes the "latest big package" clock the excellent check needs.
fn good_bigpkg(network: NetworkKind) -> (u32, u64) {
    match network {
        NetworkKind::Wifi => (20 * 1024, 3000),
        NetworkKind::Mobile => (20 * 1024, 4500),
    }
}

fn failed() -> (u32, u64) {
    (DYN_TIME_TASK_FAILED_PKG_LEN, 0)
}

#[test]
fn a_fresh_network_is_evaluating() {
    let timeout = DynamicTimeout::default();
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Evaluating);
    assert_eq!(timeout.continuous_good_count(), 0);
    assert_eq!(timeout.normal_count(), 10);
}

#[test]
fn a_small_package_within_its_budget_counts_as_good() {
    let mut timeout = DynamicTimeout::new();
    let (size, cost) = good(NetworkKind::Wifi);

    timeout.record_at(NetworkKind::Wifi, size, cost, T0);
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Evaluating);
    assert_eq!(timeout.continuous_good_count(), 1);
    assert_eq!(timeout.normal_count(), 10);
}

#[test]
fn the_mobile_budgets_are_looser_than_the_wifi_ones() {
    let mut wifi = DynamicTimeout::new();
    let mut mobile = DynamicTimeout::new();

    // 900 ms is too slow for a small package on Wi-Fi (500 ms) but inside the
    // mobile budget (1000 ms)
    wifi.record_at(NetworkKind::Wifi, 1024, 900, T0);
    mobile.record_at(NetworkKind::Mobile, 1024, 900, T0);

    assert_eq!(wifi.continuous_good_count(), 0);
    assert_eq!(mobile.continuous_good_count(), 1);
}

#[test]
fn ten_good_packages_in_a_row_make_the_network_excellent() {
    let mut timeout = DynamicTimeout::new();
    for i in 0..9 {
        let (size, cost) = good(NetworkKind::Wifi);
        timeout.record_at(NetworkKind::Wifi, size, cost, T0 + i);
        assert_eq!(timeout.status(), DynamicTimeoutStatus::Evaluating);
    }
    let (size, cost) = good(NetworkKind::Wifi);
    timeout.record_at(NetworkKind::Wifi, size, cost, T0 + 9);
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Excellent);
}

#[test]
fn the_good_run_only_counts_while_it_is_recent() {
    let mut timeout = DynamicTimeout::new();
    // the clock has run past the expiry, and no big package refreshed it
    for i in 0..DYN_TIME_MAX_CONTINUOUS_EXCELLENT_COUNT as u64 {
        let (size, cost) = good(NetworkKind::Wifi);
        timeout.record_at(NetworkKind::Wifi, size, cost, 10 * 60 * 1000 + i);
    }
    assert_eq!(
        timeout.continuous_good_count(),
        DYN_TIME_MAX_CONTINUOUS_EXCELLENT_COUNT
    );
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Evaluating);
}

#[test]
fn every_package_class_is_measured_against_its_own_budget() {
    // (size, cost time within the Wi-Fi budget, cost time one ms over it)
    let classes = [
        (1024u32, 500u64, 501u64),           // small: 500 ms
        (5 * 1024, 2 * 1000, 2 * 1000 + 1),  // middle: 2 s
        (20 * 1024, 4 * 1000, 4 * 1000 + 1), // big: 4 s
        (64 * 1024, 6 * 1000, 6 * 1000 + 1), // bigger: 6 s
    ];

    for (size, within, over) in classes {
        let mut fast = DynamicTimeout::new();
        fast.record_at(NetworkKind::Wifi, size, within, T0);
        assert_eq!(
            fast.continuous_good_count(),
            1,
            "{size} bytes in {within} ms should meet its budget"
        );

        let mut slow = DynamicTimeout::new();
        slow.record_at(NetworkKind::Wifi, size, over, T0);
        assert_eq!(
            slow.continuous_good_count(),
            0,
            "{size} bytes in {over} ms is over its budget"
        );
        assert_eq!(slow.status(), DynamicTimeoutStatus::Evaluating);
    }
}

#[test]
fn a_slow_package_breaks_the_good_run() {
    let mut timeout = DynamicTimeout::new();
    for i in 0..5 {
        let (size, cost) = good(NetworkKind::Wifi);
        timeout.record_at(NetworkKind::Wifi, size, cost, T0 + i);
    }
    assert_eq!(timeout.continuous_good_count(), 5);

    let (size, cost) = slow(NetworkKind::Wifi);
    timeout.record_at(NetworkKind::Wifi, size, cost, T0 + 5);
    assert_eq!(timeout.continuous_good_count(), 0);
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Evaluating);
    assert_eq!(timeout.normal_count(), 10);
}

#[test]
fn a_failed_package_is_a_failure() {
    let mut timeout = DynamicTimeout::new();
    let (size, cost) = failed();
    timeout.record_at(NetworkKind::Wifi, size, cost, T0);
    assert_eq!(timeout.normal_count(), 9);
    assert_eq!(timeout.continuous_good_count(), 0);

    // a cost time of zero fails whatever the size
    let mut zero_cost = DynamicTimeout::new();
    zero_cost.record_at(NetworkKind::Wifi, 1024, 0, T0);
    assert_eq!(zero_cost.normal_count(), 9);
}

#[test]
fn four_failures_out_of_ten_make_the_network_bad() {
    let mut timeout = DynamicTimeout::new();
    for i in 0..3 {
        let (size, cost) = failed();
        timeout.record_at(NetworkKind::Wifi, size, cost, T0 + i);
        assert_eq!(timeout.status(), DynamicTimeoutStatus::Evaluating);
        assert_eq!(timeout.normal_count(), 10 - i as usize - 1);
    }
    let (size, cost) = failed();
    timeout.record_at(NetworkKind::Wifi, size, cost, T0 + 3);
    // 6 normals out of 10 is the floor
    assert_eq!(timeout.normal_count(), DYN_TIME_MIN_NORMAL_PKG_COUNT);
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Bad);
}

#[test]
fn a_bad_network_climbs_back_through_evaluating_to_excellent() {
    let mut timeout = DynamicTimeout::new();
    for i in 0..4 {
        let (size, cost) = failed();
        timeout.record_at(NetworkKind::Wifi, size, cost, T0 + i);
    }
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Bad);

    // the window is restarted (and emptied) when the status flips to bad, so
    // seven good packages are needed before it leaves "bad"
    for i in 0..6 {
        let (size, cost) = good_bigpkg(NetworkKind::Wifi);
        timeout.record_at(NetworkKind::Wifi, size, cost, T0 + 100 + i);
        assert_eq!(timeout.status(), DynamicTimeoutStatus::Bad);
    }
    let (size, cost) = good_bigpkg(NetworkKind::Wifi);
    timeout.record_at(NetworkKind::Wifi, size, cost, T0 + 107);
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Evaluating);

    // ... and ten more in a row before it is called excellent again
    for i in 0..9 {
        let (size, cost) = good_bigpkg(NetworkKind::Wifi);
        timeout.record_at(NetworkKind::Wifi, size, cost, T0 + 200 + i);
        assert_eq!(timeout.status(), DynamicTimeoutStatus::Evaluating);
    }
    let (size, cost) = good_bigpkg(NetworkKind::Wifi);
    timeout.record_at(NetworkKind::Wifi, size, cost, T0 + 300);
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Excellent);
}

#[test]
fn a_failed_package_drops_excellent_back_to_evaluating() {
    let mut timeout = DynamicTimeout::new();
    for i in 0..10 {
        let (size, cost) = good(NetworkKind::Wifi);
        timeout.record_at(NetworkKind::Wifi, size, cost, T0 + i);
    }
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Excellent);

    let (size, cost) = failed();
    timeout.record_at(NetworkKind::Wifi, size, cost, T0 + 10);
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Evaluating);
}

#[test]
fn the_window_is_forgotten_after_five_minutes() {
    let mut timeout = DynamicTimeout::new();
    for i in 0..3 {
        let (size, cost) = failed();
        timeout.record_at(NetworkKind::Wifi, size, cost, T0 + i);
    }
    assert_eq!(timeout.normal_count(), 7);

    // the window expired: it is reset, and a bad status would start it empty
    let (size, cost) = failed();
    timeout.record_at(
        NetworkKind::Wifi,
        size,
        cost,
        T0 + DYN_TIME_COUNT_EXPIRE_TIME + 1,
    );
    assert_eq!(timeout.normal_count(), 9);
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Evaluating);
}

#[test]
fn reset_goes_back_to_the_fresh_state() {
    let mut timeout = DynamicTimeout::new();
    for i in 0..10 {
        let (size, cost) = good(NetworkKind::Wifi);
        timeout.record_at(NetworkKind::Wifi, size, cost, T0 + i);
    }
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Excellent);

    timeout.reset();
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Evaluating);
    assert_eq!(timeout.continuous_good_count(), 0);
    assert_eq!(timeout.normal_count(), 10);
}

#[test]
fn the_status_runs_against_the_real_clock_too() {
    let mut timeout = DynamicTimeout::new();
    let (size, cost) = good(NetworkKind::Mobile);
    timeout.record(NetworkKind::Mobile, size, cost);
    assert_eq!(timeout.continuous_good_count(), 1);
    assert_eq!(timeout.status(), DynamicTimeoutStatus::Evaluating);
}
