//! `mars/sdt/src/tools/netchecker_trafficmonitor.cc` — the budget a run of
//! probes is given.

use mars_sdt::{NetCheckTrafficMonitor, DEFAULT_WIFI_DATA_THRESHOLD};

/// A monitor with a mobile budget of `mobile` bytes, wifi unlimited — what
/// `tcpquery.cc` builds when only the mobile data is rationed.
fn monitor(mobile: u64, is_ignore_recv_data: bool) -> NetCheckTrafficMonitor {
    NetCheckTrafficMonitor::new(mobile, is_ignore_recv_data)
}

#[test]
fn a_fresh_monitor_has_counted_nothing() {
    let mut monitor = monitor(100, true);
    assert_eq!(monitor.wifi_send(), 0);
    assert_eq!(monitor.wifi_recv(), 0);
    assert_eq!(monitor.mobile_send(), 0);
    assert_eq!(monitor.mobile_recv(), 0);

    // nothing to send is nothing to refuse, and it is not counted either
    assert!(!monitor.send_limit_check(0, false));
    assert!(!monitor.send_limit_check(0, true));
    assert!(!monitor.recv_limit_check(0, false));
    assert_eq!(monitor.wifi_send(), 0);
    assert_eq!(monitor.mobile_send(), 0);
}

#[test]
fn what_a_probe_sent_is_counted_on_the_network_it_went_over() {
    let mut monitor = monitor(100, true);

    assert!(!monitor.send_limit_check(40, false));
    assert_eq!(monitor.wifi_send(), 40);
    assert_eq!(monitor.mobile_send(), 0);

    assert!(!monitor.send_limit_check(30, true));
    assert_eq!(monitor.wifi_send(), 40);
    assert_eq!(monitor.mobile_send(), 30);

    // and what came back is counted the same way
    assert!(!monitor.recv_limit_check(5, true));
    assert_eq!(monitor.mobile_recv(), 5);
    assert_eq!(monitor.wifi_recv(), 0);
}

#[test]
fn a_send_that_would_pass_the_threshold_is_refused_and_not_counted() {
    let mut monitor = monitor(100, true);

    assert!(!monitor.send_limit_check(99, true));
    // one byte more than the budget: the C++ warns and sends nothing
    assert!(monitor.send_limit_check(2, true));
    assert_eq!(monitor.mobile_send(), 99, "the refused size is not counted");

    // and the next one that fits is still taken
    assert!(!monitor.send_limit_check(1, true));
    assert_eq!(monitor.mobile_send(), 100);
    assert!(monitor.send_limit_check(1, true));
}

#[test]
fn the_send_check_looks_at_both_budgets_whatever_the_data_went_over() {
    // a wifi budget of 50 bytes and a mobile one of 500: the C++ compares the
    // size against both, so 60 bytes of mobile data is refused for the wifi
    // budget it never touches
    let mut monitor = NetCheckTrafficMonitor::with_wifi_threshold(500, true, 50);

    assert!(monitor.send_limit_check(60, true));
    assert_eq!(monitor.mobile_send(), 0);
    assert_eq!(monitor.wifi_send(), 0);

    // 40 bytes fit in both
    assert!(!monitor.send_limit_check(40, true));
    assert_eq!(monitor.mobile_send(), 40);

    // and 60 again, now that the mobile budget has 460 bytes left: refused for
    // the wifi one, which the mobile data never touches
    assert!(monitor.send_limit_check(60, true));
    assert_eq!(monitor.mobile_send(), 40);
}

#[test]
fn a_monitor_without_a_wifi_budget_only_rationed_the_mobile_data() {
    let mut monitor = monitor(100, true);
    assert_eq!(DEFAULT_WIFI_DATA_THRESHOLD, u64::MAX);

    // the wifi budget is one wifi traffic cannot use up: a hundred bytes, and a
    // hundred more, although the mobile budget is 100
    assert!(!monitor.send_limit_check(100, false));
    assert!(!monitor.send_limit_check(100, false));
    assert_eq!(monitor.wifi_send(), 200);

    // the mobile budget is checked against the same size, so a hundred bytes of
    // mobile data are the most there is, and one more is refused
    assert!(!monitor.send_limit_check(100, true));
    assert_eq!(monitor.mobile_send(), 100);
    assert!(monitor.send_limit_check(1, true));
    assert!(monitor.send_limit_check(101, true));
}

#[test]
fn received_data_is_counted_even_by_a_monitor_that_ignores_it() {
    // `isIgnoreRecvData` — what came back does not count towards the budget,
    // but `__data` is called before the check, so it is remembered
    let mut monitor = monitor(100, true);

    assert!(!monitor.recv_limit_check(200, false));
    assert_eq!(monitor.wifi_recv(), 200);

    assert!(!monitor.recv_limit_check(u64::MAX, true));
    assert_eq!(monitor.mobile_recv(), u64::MAX);
}

#[test]
fn received_data_over_the_threshold_is_refused_and_still_counted() {
    let mut monitor = NetCheckTrafficMonitor::with_wifi_threshold(100, false, 100);

    assert!(!monitor.send_limit_check(50, false));
    assert!(!monitor.recv_limit_check(40, false));
    assert_eq!(monitor.wifi_recv(), 40);

    // 50 sent + 60 received is more than the 100 the budget allows
    assert!(monitor.recv_limit_check(60, false));
    assert_eq!(monitor.wifi_recv(), 100, "the refused bytes are counted");
}

#[test]
fn the_received_bytes_of_the_other_network_are_the_ones_that_are_not_counted() {
    // the threshold is checked on both networks, but on the counters of the
    // network the bytes went over — so what came back over mobile data does not
    // use up the wifi budget
    let mut monitor = NetCheckTrafficMonitor::with_wifi_threshold(500, false, 100);

    assert!(!monitor.recv_limit_check(400, true));
    assert_eq!(monitor.mobile_recv(), 400);

    // 60 bytes over wifi, and 60 more: the wifi budget is 100
    assert!(!monitor.recv_limit_check(60, false));
    assert!(monitor.recv_limit_check(60, false));
    assert_eq!(monitor.wifi_recv(), 120);
    assert_eq!(
        monitor.mobile_recv(),
        400,
        "mobile data is not counted twice"
    );
}

#[test]
fn a_reset_monitor_has_forgotten_its_budgets_and_refuses_everything() {
    let mut monitor = NetCheckTrafficMonitor::with_wifi_threshold(100, false, 100);
    assert!(!monitor.send_limit_check(40, true));
    assert!(!monitor.recv_limit_check(40, true));

    monitor.reset();
    assert_eq!(monitor.mobile_send(), 0);
    assert_eq!(monitor.mobile_recv(), 0);

    // `reset()` zeroes the thresholds as well, so every size is over them
    assert!(monitor.send_limit_check(1, true));
    assert!(monitor.recv_limit_check(1, true));
    assert!(!monitor.send_limit_check(0, true), "0 is not over 0");
}

#[test]
fn the_counters_of_a_monitor_are_what_it_reports() {
    let mut monitor = monitor(100, true);
    monitor.send_limit_check(10, false);
    monitor.recv_limit_check(20, true);

    let same = monitor.clone();
    assert_eq!(same, monitor);
    assert_eq!(same.wifi_send(), 10);
    assert_eq!(same.mobile_recv(), 20);
    assert!(
        format!("{monitor:?}").contains("NetCheckTrafficMonitor"),
        "{monitor:?}"
    );
}
