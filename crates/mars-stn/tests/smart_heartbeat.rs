//! `mars/stn/src/smart_heartbeat.cc`, driven the way the long link drives it:
//! a noop goes out on the interval it is given, and the answer — or the lack
//! of one — comes back.

use std::sync::{Mutex, MutexGuard, OnceLock};

use mars_stn::config::{
    HEART_STEP, MAX_HEART_INTERVAL, MIN_HEART_INTERVAL, NET_STABLE_TEST_COUNT, SUCCESS_STEP,
};
use mars_stn::smart_heartbeat::{
    outer_setted_heart, set_heartbeat, NetHeartbeatInfo, SmartHeartBeatAction, SmartHeartBeatType,
    SmartHeartbeat, NO_NET,
};

/// `outer_setted_heart_` is process-wide.
fn hearts() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One heartbeat: the long link asks how long to wait, sends the noop, and
/// reports the answer. `now` is a tick count in milliseconds and a wall clock
/// in seconds; only the difference between two of them matters.
fn beat(hb: &mut SmartHeartbeat, answered: bool, tick: u64, seconds: i64) -> u32 {
    let interval = hb.get_next_heartbeat_interval(false);
    hb.on_heartbeat_start(tick);
    hb.on_heart_result(answered, false, seconds);
    interval
}

/// A network that answers everything, until it does not.
fn session(hb: &mut SmartHeartbeat, beats: usize, answered: impl Fn(usize) -> bool) -> Vec<u32> {
    let mut intervals = Vec::new();
    for index in 0..beats {
        intervals.push(beat(hb, answered(index), index as u64 * 1_000, 0));
    }
    intervals
}

#[test]
fn the_interval_grows_while_the_network_answers() {
    let _guard = hearts();
    let mut hb = SmartHeartbeat::new();
    hb.on_longlink_established("wifi-home", 1);

    // the first `NetStableTestCount` heartbeats only test the minimum
    let intervals = session(&mut hb, 60, |_| true);
    assert_eq!(intervals[0], MIN_HEART_INTERVAL);
    assert!(intervals.iter().all(|it| *it >= MIN_HEART_INTERVAL));

    // and it stops growing at the ceiling, which is where it settles
    assert_eq!(
        *intervals.last().unwrap(),
        MAX_HEART_INTERVAL - SUCCESS_STEP
    );
    assert!(hb.info().is_stable);
    assert_eq!(hb.info().heart_type, SmartHeartBeatType::SmartHeartBeat);
}

#[test]
fn a_network_that_stops_answering_is_pulled_back_down() {
    let _guard = hearts();
    let mut hb = SmartHeartbeat::new();
    hb.on_longlink_established("4g", 2);

    // long enough to settle on the ceiling
    session(&mut hb, 60, |_| true);
    assert!(hb.info().is_stable);

    // and then a stretch where nothing comes back
    let intervals = session(&mut hb, 20, |_| false);
    assert_eq!(*intervals.last().unwrap(), MIN_HEART_INTERVAL);
    assert!(!hb.info().is_stable, "the ceiling was given up on");
}

#[test]
fn the_interval_a_host_kept_is_the_one_the_next_session_starts_on() {
    let _guard = hearts();
    let mut first = SmartHeartbeat::new();
    first.on_longlink_established("wifi-home", 1);
    session(&mut first, 20, |_| true);
    let kept = first.info().clone();
    assert!(kept.cur_heart > MIN_HEART_INTERVAL);

    // `Heartbeat.ini` is a file in the C++; here it is whatever the host kept
    let mut second = SmartHeartbeat::new();
    second.on_longlink_established(&kept.net_detail, kept.net_type);
    *second.info_mut() = kept.clone();
    assert_eq!(second.info().cur_heart, kept.cur_heart);

    // the new session tests the minimum again, and from the heartbeat after
    // that it carries on where the last one stopped, not from the short end
    let intervals = session(&mut second, 10, |_| true);
    assert_eq!(intervals[0], MIN_HEART_INTERVAL);
    assert_eq!(
        intervals[NET_STABLE_TEST_COUNT as usize], kept.cur_heart,
        "{intervals:?}"
    );
    assert!(intervals.last() >= intervals.first());
}

#[test]
fn an_interval_the_app_sets_wins_over_the_computed_one() {
    let guard = hearts();
    let mut hb = SmartHeartbeat::new();
    hb.on_longlink_established("wifi-home", 1);
    session(&mut hb, 20, |_| true);

    set_heartbeat(45 * 1000);
    assert_eq!(hb.get_next_heartbeat_interval(false), 45 * 1000);
    assert_eq!(outer_setted_heart(), 45 * 1000);

    // `SetHeartBeat(0)` is what `TrigNooping` asks for
    set_heartbeat(-1);
    assert_eq!(hb.get_next_heartbeat_interval(false), hb.info().cur_heart);
    drop(guard);
}

#[test]
fn what_a_session_reports() {
    let _guard = hearts();
    let mut hb = SmartHeartbeat::new();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&seen);
    hb.set_report(move |action, info, timeout| {
        sink.lock()
            .unwrap()
            .push((action, info.cur_heart, info.is_stable, timeout));
    });
    hb.on_longlink_established("wifi-home", 1);

    session(&mut hb, 60, |_| true);
    let reports = seen.lock().unwrap().clone();
    // one `CalcEnd` for every interval it settled on, and the last one is the
    // ceiling
    assert!(reports.iter().any(|(action, cur_heart, stable, _)| *action
        == SmartHeartBeatAction::CalcEnd
        && *cur_heart == MAX_HEART_INTERVAL - SUCCESS_STEP
        && *stable));

    // and a network that dies on a settled interval is reported as one
    seen.lock().unwrap().clear();
    hb.on_heartbeat_start(0);
    hb.on_longlink_disconnect(0);
    assert_eq!(
        seen.lock().unwrap().first().map(|(action, ..)| *action),
        Some(SmartHeartBeatAction::Disconnect)
    );
}

#[test]
fn a_network_that_cannot_hold_the_minimum_is_reported_as_bad() {
    let _guard = hearts();
    let mut hb = SmartHeartbeat::new();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&seen);
    hb.set_report(move |action, _, _| sink.lock().unwrap().push(action));
    hb.on_longlink_established("", NO_NET);

    // no network at all: nothing is computed, nothing is reported
    session(&mut hb, 10, |_| false);
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(hb.get_next_heartbeat_interval(false), MIN_HEART_INTERVAL);

    // and one that is named but never answers is reported before it is given
    // up on
    hb.on_longlink_established("4g", 2);
    session(&mut hb, 10, |_| false);
    assert_eq!(
        seen.lock().unwrap().first().copied(),
        Some(SmartHeartBeatAction::BadNetwork)
    );
}

#[test]
fn the_record_of_a_network_is_replaced_with_the_defaults() {
    let mut info = NetHeartbeatInfo::new();
    assert_eq!(info.net_type, NO_NET);
    assert_eq!(info.cur_heart, MIN_HEART_INTERVAL);
    assert!(!info.is_stable);
    assert_eq!(info.heart_type, SmartHeartBeatType::NoSmartHeartBeat);
    assert!(!info.has_network());

    info.net_detail = "wifi-home".to_owned();
    info.cur_heart = MIN_HEART_INTERVAL + HEART_STEP;
    assert!(info.has_network());

    info.clear();
    assert_eq!(info, NetHeartbeatInfo::new());
    assert_eq!(info, NetHeartbeatInfo::default());
}

#[test]
fn the_heartbeat_count_that_decides_a_network_is_stable() {
    let _guard = hearts();
    let mut hb = SmartHeartbeat::new();
    hb.on_longlink_established("wifi-home", 1);

    // `NetStableTestCount` answers, and not one fewer: the interval does not
    // move before the network has been tested
    session(&mut hb, NET_STABLE_TEST_COUNT as usize - 1, |_| true);
    assert_eq!(hb.info().cur_heart, MIN_HEART_INTERVAL);
    assert!(!hb.info().is_stable);
}
