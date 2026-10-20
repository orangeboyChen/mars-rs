//! `mars/stn/src/net_check_logic.cc`, through the public api.
//!
//! The samples are what the C++ answers for the same calls: a link with sixteen
//! successes behind it and seven failures in front of it is worth diagnosing,
//! the diagnosis is refused for five minutes after the last one and then for
//! ten minutes more every time, and a link that is fine again starts the wait
//! over.

use std::sync::{Arc, Mutex};

use marsrs_sdt::sdt::CheckIPPorts;
use marsrs_stn::net_check_logic::{
    NetCheckLogic, CHECK_IF_ABOVE_COUNT, CHECK_IF_BELOW_COUNT, CHECK_TIME_SPAN_INCREMENT_STEP,
    LIMIT_COUNT, LIMIT_TIME_SPAN, MIN_CHECK_TIME_SPAN, NET_CHECK_MODE, SECOND_RECENT_TASK_START_N,
    VALID_BITS_FILTER,
};

/// Every host and port of a `CheckIPPorts`, in the order they went in.
fn pairs(items: &CheckIPPorts) -> Vec<(String, u16)> {
    items
        .values()
        .flat_map(|list| list.iter().map(|item| (item.ip.clone(), item.port)))
        .collect()
}

/// What `StartActiveCheck` was handed, in order.
type Started = Arc<Mutex<Vec<(Vec<(String, u16)>, Vec<(String, u16)>, i32)>>>;

/// A logic whose long link is reachable at one host and two ports, whose short
/// link the app names one host for, and whose new dns only knows the long
/// link's host.
fn a_logic() -> (NetCheckLogic, Started) {
    let mut logic = NetCheckLogic::new_at(0);
    logic.set_long_link_hosts(|| vec!["long.example".to_string()]);
    logic.set_long_link_ports(|| vec![80, 443]);
    logic.set_short_link_port(|| 8080);
    logic.set_request_short_link_hosts(|| vec!["short.example".to_string()]);
    logic.set_new_dns(|host| {
        if host.starts_with("long") {
            vec!["1.2.3.4".to_string()]
        } else {
            Vec::new()
        }
    });
    logic.set_dns(|_host| vec!["5.6.7.8".to_string()]);

    let started: Started = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&started);
    logic.set_start_active_check(move |longlink, shortlink, mode| {
        record.lock().unwrap_or_else(|e| e.into_inner()).push((
            pairs(longlink),
            pairs(shortlink),
            mode,
        ));
    });
    (logic, started)
}

/// Sixteen successes and then `failures` failures on the long link: a link
/// that was fine and is broken now.
fn broken_longlink(logic: &mut NetCheckLogic, now: u64, failures: usize) {
    for _ in 0..16 {
        logic.update_long_link_info_at(now, 0, true);
    }
    for _ in 0..failures {
        logic.update_long_link_info_at(now, 0, false);
    }
}

#[test]
fn a_link_that_was_fine_and_then_broke_starts_a_check() {
    let (mut logic, started) = a_logic();
    // the window starts full of successes, which is a link that never failed
    assert_eq!(logic.longlink_records(), VALID_BITS_FILTER);

    // ... and it takes seven failures for fewer than `kCheckifBelowCount` of
    // the eight most recent tasks to be successes
    broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN, 7);
    let first = started.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(first.len(), 1);
    let (longlink, shortlink, mode) = &first[0];
    assert_eq!(*mode, NET_CHECK_MODE);
    assert_eq!(
        *longlink,
        vec![("1.2.3.4".to_string(), 80), ("1.2.3.4".to_string(), 443)],
        "each port of each ip of each host"
    );
    // the short link's host is one only the old dns knows
    assert_eq!(*shortlink, vec![("5.6.7.8".to_string(), 8080)]);
    assert_eq!(logic.longlink_last_failed_time(), MIN_CHECK_TIME_SPAN);

    // the eighth failure is inside the wait the last check started, and what
    // says so is the record and not a copy of it taken before it happened
    logic.update_long_link_info_at(MIN_CHECK_TIME_SPAN, 0, false);
    assert_eq!(started.lock().unwrap_or_else(|e| e.into_inner()).len(), 1);
    assert_eq!(logic.increment_steps(), 1);
}

#[test]
fn a_check_that_went_out_lengthens_the_wait_of_the_next() {
    let (mut logic, started) = a_logic();
    broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN, 7);
    assert_eq!(started.lock().unwrap_or_else(|e| e.into_inner()).len(), 1);

    // `last_netcheck_time_` is a field the C++ never writes, so what the wait
    // is measured from is the age of the process: a quarter of an hour is what
    // the check that went out asks for, and this reading is a millisecond short
    let quarter = MIN_CHECK_TIME_SPAN + CHECK_TIME_SPAN_INCREMENT_STEP;
    logic.update_long_link_info_at(quarter - 1, 0, false);
    assert_eq!(
        started.lock().unwrap_or_else(|e| e.into_inner()).len(),
        1,
        "a millisecond short of the wait"
    );
    // ... and the next millisecond is not
    logic.update_long_link_info_at(quarter, 0, false);
    assert_eq!(started.lock().unwrap_or_else(|e| e.into_inner()).len(), 2);
    assert_eq!(logic.increment_steps(), 2);
}

#[test]
fn the_frequency_limit_is_what_refuses_the_third_check_of_the_hour() {
    let (mut logic, started) = a_logic();
    // `LIMIT_COUNT` is one, and `CommFrequencyLimit::Check` passes while the
    // touches it holds are no more than the count, so an hour holds two
    // checks and not one
    assert_eq!(LIMIT_COUNT, 1);

    // five minutes in, and a quarter of an hour in
    broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN, 7);
    let quarter = MIN_CHECK_TIME_SPAN + CHECK_TIME_SPAN_INCREMENT_STEP;
    logic.update_long_link_info_at(quarter, 0, false);
    assert_eq!(started.lock().unwrap_or_else(|e| e.into_inner()).len(), 2);

    // the third, twenty-five minutes in. The wait is met — it is the age of
    // the process it is measured against and not the last check, since
    // `last_netcheck_time_` is a field the C++ never writes — and the hour of
    // the first has not run out, so what says no is the limit
    let third = MIN_CHECK_TIME_SPAN + 2 * CHECK_TIME_SPAN_INCREMENT_STEP;
    logic.update_long_link_info_at(third, 0, false);
    assert_eq!(
        started.lock().unwrap_or_else(|e| e.into_inner()).len(),
        2,
        "the third of the hour"
    );
    assert!(
        third - MIN_CHECK_TIME_SPAN < LIMIT_TIME_SPAN,
        "the hour of the first has not run out"
    );

    // the same reading once it has goes through, which is what makes the
    // refusal above the limit's and not the wait's
    logic.update_long_link_info_at(third + LIMIT_TIME_SPAN, 0, false);
    assert_eq!(started.lock().unwrap_or_else(|e| e.into_inner()).len(), 3);
}

#[test]
fn a_link_that_is_fine_again_resets_the_wait() {
    let (mut logic, _started) = a_logic();
    broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN, 7);
    assert_eq!(logic.increment_steps(), 1);

    // more than `kCheckifAboveCount` of the eight most recent tasks succeed
    // again, and the wait starts over
    for _ in 0..7 {
        logic.update_long_link_info_at(MIN_CHECK_TIME_SPAN, 0, true);
    }
    assert_eq!(logic.increment_steps(), 0);
    assert_eq!(CHECK_IF_BELOW_COUNT, 3);
    assert_eq!(CHECK_IF_ABOVE_COUNT, 5);
    assert_eq!(SECOND_RECENT_TASK_START_N, [17, 8]);
}

#[test]
fn the_short_link_decides_on_its_own() {
    let (mut logic, started) = a_logic();
    for _ in 0..16 {
        logic.update_short_link_info_at(MIN_CHECK_TIME_SPAN, 0, true);
    }
    for _ in 0..7 {
        logic.update_short_link_info_at(MIN_CHECK_TIME_SPAN, 0, false);
    }
    // the short link's window is the broken one, and the long link's is not
    // even touched
    assert_eq!(logic.longlink_records(), VALID_BITS_FILTER);
    assert_eq!(logic.increment_steps(), 1);
    assert_eq!(started.lock().unwrap_or_else(|e| e.into_inner()).len(), 1);
}

#[test]
fn a_link_without_hosts_or_ports_or_ips_is_not_checked() {
    let started = Arc::new(Mutex::new(0usize));

    // nothing at all set: the hosts are the first thing the C++ gives up on
    let mut logic = NetCheckLogic::new_at(0);
    let record = Arc::clone(&started);
    logic.set_start_active_check(move |_, _, _| {
        *record.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    });
    broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN, 7);
    assert_eq!(*started.lock().unwrap_or_else(|e| e.into_inner()), 0);

    // hosts but no ports
    let (mut logic, _) = a_logic();
    logic.set_long_link_ports(Vec::new);
    let record = Arc::clone(&started);
    logic.set_start_active_check(move |_, _, _| {
        *record.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    });
    broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN, 7);
    assert_eq!(
        *started.lock().unwrap_or_else(|e| e.into_inner()),
        0,
        "no ports"
    );

    // hosts and ports, but a dns that knows neither host
    let (mut logic, _) = a_logic();
    logic.set_new_dns(|_| Vec::new());
    logic.set_dns(|_| Vec::new());
    let record = Arc::clone(&started);
    logic.set_start_active_check(move |_, _, _| {
        *record.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    });
    broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN, 7);
    assert_eq!(
        *started.lock().unwrap_or_else(|e| e.into_inner()),
        0,
        "nothing to check"
    );
}

#[test]
fn without_a_host_nothing_is_asked_for() {
    let mut logic = NetCheckLogic::default();
    logic.update_long_link_info(0, false);
    logic.update_short_link_info(0, false);
    assert_eq!(
        logic.longlink_records(),
        0b1111_1111_1111_1111_1111_1111_1111_1110
    );
    assert_eq!(
        logic.shortlink_records(),
        0b1111_1111_1111_1111_1111_1111_1111_1110
    );
    // ... and it is the clock, not a `0` handed in, that was written down
    assert_ne!(logic.shortlink_last_failed_time(), 0);

    let cancelled = Arc::new(Mutex::new(0usize));
    let record = Arc::clone(&cancelled);
    logic.set_cancel_active_check(move || {
        *record.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    });
    // what the C++'s destructor does
    logic.cancel_active_check();
    assert_eq!(*cancelled.lock().unwrap_or_else(|e| e.into_inner()), 1);
}
