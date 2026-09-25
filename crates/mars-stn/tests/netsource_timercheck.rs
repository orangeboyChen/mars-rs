//! `mars/stn/src/netsource_timercheck.cc`, through the public api.
//!
//! The samples are what the C++ answers for the same calls: a check posted
//! every two and a half minutes while the app is in the foreground, a long link
//! on a backup ip as the only thing it looks at, an ip and a port picked at
//! random out of what dns and `NetSource` hand out, and a pair that answers is
//! one whose ban is lifted and whose long link is dropped.

use std::sync::{Arc, Mutex};

use mars_stn::netsource_timercheck::{
    NetSourceTimerCheck, INTERVAL_TIME, MAX_SPEED_TEST_COUNT, TIMEOUT, TIME_CHECK_PERIOD,
};
use mars_stn::IpSourceType;

/// What the check asked for: the pairs it tested, the bans it lifted, and how
/// many times the app was told to reset the link.
#[derive(Default)]
struct Seen {
    tested: Vec<(String, u16)>,
    unbanned: Vec<String>,
    sucs: usize,
}

type Recorder = Arc<Mutex<Seen>>;

/// A check whose long link is on a backup ip at `1.2.3.4`, whose new dns knows
/// two other ips, and whose `NetSource` offers two ports.
fn a_check() -> (NetSourceTimerCheck, Recorder) {
    let mut check = NetSourceTimerCheck::new_at(0);
    check.set_ip_type(|| IpSourceType::Backup);
    check.set_host(|| "long.example".to_string());
    check.set_ip(|| "1.2.3.4".to_string());
    check.set_new_dns(|_host| vec!["5.6.7.8".to_string(), "9.10.11.12".to_string()]);
    check.set_longlink_ports(|| vec![80, 443]);
    check.set_random(|bound| bound - 1);

    let seen: Recorder = Arc::new(Mutex::new(Seen::default()));
    let record = Arc::clone(&seen);
    check.set_speed_test(move |ip, port| {
        record
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tested
            .push((ip.to_string(), port));
        true
    });
    let record = Arc::clone(&seen);
    check.set_remove_long_ban_ip(move |ip| {
        record
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .unbanned
            .push(ip.to_string());
    });
    let record = Arc::clone(&seen);
    check.set_on_time_check_suc(move || {
        record.lock().unwrap_or_else(|e| e.into_inner()).sucs += 1;
    });
    (check, seen)
}

#[test]
fn the_numbers_are_the_ones_the_c_plus_plus_writes_down() {
    assert_eq!(TIME_CHECK_PERIOD, 150 * 1000, "two and a half minutes");
    assert_eq!(TIMEOUT, 10 * 1000, "what `Select` is given");
    assert_eq!(MAX_SPEED_TEST_COUNT, 30);
    assert_eq!(INTERVAL_TIME, 60 * 60 * 1000);
}

#[test]
fn a_check_is_posted_and_keeps_being_posted() {
    let (mut check, seen) = a_check();
    // nothing is posted until the app is in the foreground
    assert_eq!(check.period_due(), None);
    assert_eq!(check.check_at(TIME_CHECK_PERIOD), None);
    assert!(seen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .tested
        .is_empty());

    check.on_active_changed_at(0, true);
    assert_eq!(check.period_due(), Some(TIME_CHECK_PERIOD));
    // ... and a second start does not move the post
    check.start_check_at(1_000);
    assert_eq!(check.period_due(), Some(TIME_CHECK_PERIOD));

    // the post ran: the test is made, and the post is armed again
    assert_eq!(check.check_at(TIME_CHECK_PERIOD), Some(true));
    assert_eq!(check.period_due(), Some(2 * TIME_CHECK_PERIOD));
    assert!(!check.is_testing());
}

#[test]
fn a_backup_ip_is_the_only_thing_the_check_looks_at() {
    // a link that got its ip from the new dns is left alone
    let (mut check, seen) = a_check();
    check.set_ip_type(|| IpSourceType::NewDns);
    check.start_check_at(0);
    assert_eq!(check.check_at(TIME_CHECK_PERIOD), None);
    assert!(seen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .tested
        .is_empty());

    // ... and so is one that never connected
    let (mut check, seen) = a_check();
    check.set_ip_type(|| IpSourceType::Null);
    check.start_check_at(0);
    assert_eq!(check.check_at(TIME_CHECK_PERIOD), None);
    assert!(seen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .tested
        .is_empty());
}

#[test]
fn the_pair_is_picked_at_random_and_a_test_that_succeeds_lifts_its_ban() {
    let (mut check, seen) = a_check();
    check.start_check_at(0);
    assert_eq!(check.check_at(TIME_CHECK_PERIOD), Some(true));
    let seen = seen.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        seen.tested,
        vec![("9.10.11.12".to_string(), 443)],
        "the last of each, the random pick pinned to `bound - 1`"
    );
    assert_eq!(seen.unbanned, vec!["9.10.11.12".to_string()]);
    assert_eq!(seen.sucs, 1, "and the app is told to reset the link");
}

#[test]
fn a_test_that_does_not_succeed_tells_nobody() {
    let (mut check, seen) = a_check();
    // a test that is made and does not succeed
    let tested: Arc<Mutex<Vec<(String, u16)>>> = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&tested);
    check.set_speed_test(move |ip, port| {
        record
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((ip.to_string(), port));
        false
    });
    check.start_check_at(0);
    assert_eq!(check.check_at(TIME_CHECK_PERIOD), Some(false));
    assert_eq!(
        tested.lock().unwrap_or_else(|e| e.into_inner()).len(),
        1,
        "the test was made"
    );
    let seen = seen.lock().unwrap_or_else(|e| e.into_inner());
    assert!(seen.unbanned.is_empty(), "no ban was lifted");
    assert_eq!(seen.sucs, 0);
}

#[test]
fn a_host_no_dns_knows_and_an_ip_in_use_are_not_tested() {
    // a dns that answers nothing at all
    let (mut check, seen) = a_check();
    check.set_new_dns(|_| Vec::new());
    check.start_check_at(0);
    assert_eq!(check.check_at(TIME_CHECK_PERIOD), Some(false));
    assert!(seen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .tested
        .is_empty());

    // the fallback is what is asked when the new dns answers nothing
    let (mut check, seen) = a_check();
    check.set_new_dns(|_| Vec::new());
    check.set_dns(|_| vec!["13.14.15.16".to_string()]);
    check.start_check_at(0);
    assert_eq!(check.check_at(TIME_CHECK_PERIOD), Some(true));
    assert_eq!(
        seen.lock().unwrap_or_else(|e| e.into_inner()).tested,
        vec![("13.14.15.16".to_string(), 443)]
    );

    // and the ip the link is on already rules the whole host out
    let (mut check, seen) = a_check();
    check.set_ip(|| "5.6.7.8".to_string());
    check.start_check_at(0);
    assert_eq!(check.check_at(TIME_CHECK_PERIOD), Some(false));
    assert!(seen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .tested
        .is_empty());
}

#[test]
fn a_burst_of_checks_is_refused_after_thirty_one() {
    let (mut check, seen) = a_check();
    check.start_check_at(0);
    // `size() <= count`, so one more than `MAX_SPEED_TEST_COUNT` go through
    for index in 0..MAX_SPEED_TEST_COUNT + 1 {
        assert_eq!(check.check_at(1_000 * (index as u64 + 1)), Some(true));
    }
    assert_eq!(
        seen.lock().unwrap_or_else(|e| e.into_inner()).tested.len(),
        MAX_SPEED_TEST_COUNT + 1
    );

    assert_eq!(
        check.check_at(1_000 * (MAX_SPEED_TEST_COUNT as u64 + 2)),
        None
    );
    // ... and the oldest of them has to be more than an hour old
    assert_eq!(check.check_at(INTERVAL_TIME + 2_000), Some(true));
}

#[test]
fn leaving_the_foreground_leaves_a_check_that_is_not_running_alone() {
    let (mut check, _seen) = a_check();
    check.on_active_changed_at(0, true);
    check.on_active_changed_at(TIME_CHECK_PERIOD, false);
    assert_eq!(
        check.period_due(),
        Some(TIME_CHECK_PERIOD),
        "the C++ returns before it clears `asyncpost_`, so the post keeps coming"
    );
    assert_eq!(
        check.check_at(2 * TIME_CHECK_PERIOD),
        Some(true),
        "and it is still being made"
    );
}

#[test]
fn without_a_host_nothing_is_asked_for() {
    let mut check = NetSourceTimerCheck::default();
    check.start_check();
    assert_eq!(check.check(), None, "no host: not on a backup ip");
    assert!(!check.try_connect_at(0));
    check.cancel_connect();
    check.stop_check();
    assert!(format!("{check:?}").contains("NetSourceTimerCheck"));
}
