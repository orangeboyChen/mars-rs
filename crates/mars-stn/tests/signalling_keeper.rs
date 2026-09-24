//! `mars/stn/src/signalling_keeper.cc`, through the public api.
//!
//! The samples are what the C++ answers for the same calls: the first `Keep`
//! sends a buffer at once, network data posts the next one `g_period` later, a
//! `g_keepTime` that ran out ends the signalling, and `Stop` only ends a keeper
//! that has a post outstanding.
//!
//! `g_period` and `g_keepTime` are one value for the process, so the test that
//! moves them takes a lock of its own — `cargo test` runs this binary apart
//! from the crate's unit tests, but every test in here shares one process.

use std::sync::{Arc, Mutex, OnceLock};

use mars_stn::longlink::SIGNALKEEP_CMDID;
use mars_stn::signalling_keeper::{set_strategy, DEFAULT_KEEP_TIME, DEFAULT_PERIOD};
use mars_stn::SignallingKeeper;

/// `g_period` / `g_keepTime` are process-wide.
fn strategy() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A keeper whose send records the cmdids it was called with.
fn keeper_that_records() -> (SignallingKeeper, Arc<Mutex<Vec<u32>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recording = Arc::clone(&seen);
    let mut keeper = SignallingKeeper::new();
    keeper.set_send(move |cmdid| {
        recording
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(cmdid);
        0
    });
    (keeper, seen)
}

#[test]
fn the_first_touch_sends_a_buffer_at_once() {
    let (mut keeper, seen) = keeper_that_records();
    assert!(!keeper.is_keeping());

    keeper.keep_at(1_000);
    assert!(keeper.is_keeping());
    assert_eq!(keeper.last_touch_time(), Some(1_000));
    assert_eq!(
        *seen.lock().unwrap_or_else(|p| p.into_inner()),
        vec![SIGNALKEEP_CMDID]
    );
    assert_eq!(keeper.sent(), 1);
    assert_eq!(keeper.due_time(), None, "nothing is posted yet");

    // and keeping again only moves the time
    keeper.keep_at(2_000);
    assert_eq!(keeper.sent(), 1);
    assert_eq!(keeper.last_touch_time(), Some(2_000));
}

#[test]
fn network_data_posts_the_next_buffer_a_period_later() {
    let guard = strategy();
    set_strategy(1_000, 3_000);
    let (mut keeper, _seen) = keeper_that_records();
    keeper.keep_at(1_000);

    keeper.on_network_data_changed_at(1_500);
    assert_eq!(keeper.due_time(), Some(2_500));
    assert!(keeper.is_keeping());

    // what the post does: send again, and leave the post alone
    keeper.on_timeout();
    assert_eq!(keeper.sent(), 2);
    assert_eq!(keeper.due_time(), Some(2_500));

    keeper.on_network_data_changed_at(2_000);
    assert_eq!(keeper.due_time(), Some(3_000));

    set_strategy(DEFAULT_PERIOD, DEFAULT_KEEP_TIME);
    drop(guard);
}

#[test]
fn a_keep_time_that_ran_out_stops_the_signalling() {
    let guard = strategy();
    set_strategy(1_000, 3_000);
    let (mut keeper, _seen) = keeper_that_records();
    keeper.keep_at(1_000);

    keeper.on_network_data_changed_at(4_000);
    assert!(keeper.is_keeping(), "exactly keepTime is still inside it");
    assert_eq!(keeper.due_time(), Some(5_000));

    keeper.on_network_data_changed_at(4_001);
    assert!(!keeper.is_keeping());
    assert_eq!(keeper.due_time(), Some(5_000), "the post is not cancelled");

    // a clock that went backwards is treated the same way
    let (mut backwards, _seen) = keeper_that_records();
    backwards.keep_at(5_000);
    backwards.on_network_data_changed_at(4_000);
    assert!(!backwards.is_keeping());

    set_strategy(DEFAULT_PERIOD, DEFAULT_KEEP_TIME);
    drop(guard);
}

#[test]
fn stop_only_ends_a_keeper_that_has_a_post_outstanding() {
    let (mut keeper, _seen) = keeper_that_records();
    keeper.keep_at(1_000);
    keeper.stop();
    assert!(keeper.is_keeping(), "nothing was posted yet");

    keeper.on_network_data_changed_at(1_000);
    keeper.stop();
    assert!(!keeper.is_keeping());
    assert_eq!(keeper.due_time(), None);

    // and a keeper that stopped keeps time but posts nothing more
    keeper.on_network_data_changed_at(1_100);
    assert_eq!(keeper.due_time(), None);
    assert_eq!(keeper.sent(), 1);
}

#[test]
fn a_cleared_send_keeps_the_time_but_sends_nothing() {
    let mut keeper = SignallingKeeper::new();
    keeper.set_send(|_| 7);
    keeper.clear_send();
    keeper.keep_at(1_000);
    assert!(keeper.is_keeping());
    assert_eq!(keeper.sent(), 1, "the buffer still went out");
}

#[test]
fn a_zero_period_or_keep_time_is_refused() {
    let guard = strategy();
    set_strategy(0, 30_000);
    assert_eq!(
        (
            mars_stn::signalling_keeper::period(),
            mars_stn::signalling_keeper::keep_time()
        ),
        (DEFAULT_PERIOD, DEFAULT_KEEP_TIME)
    );
    set_strategy(1_000, 0);
    assert_eq!(
        (
            mars_stn::signalling_keeper::period(),
            mars_stn::signalling_keeper::keep_time()
        ),
        (DEFAULT_PERIOD, DEFAULT_KEEP_TIME)
    );

    set_strategy(1_000, 3_000);
    assert_eq!(
        (
            mars_stn::signalling_keeper::period(),
            mars_stn::signalling_keeper::keep_time()
        ),
        (1_000, 3_000)
    );
    set_strategy(DEFAULT_PERIOD, DEFAULT_KEEP_TIME);
    drop(guard);
}

#[test]
fn the_methods_that_take_no_reading_ask_the_clock_themselves() {
    let (mut keeper, _seen) = keeper_that_records();
    keeper.keep();
    assert!(keeper.is_keeping());
    assert!(keeper.last_touch_time().is_some());

    keeper.on_network_data_changed();
    // `g_keepTime` from the touch the keeper just made, so it is still keeping
    assert!(keeper.is_keeping());
    assert!(keeper.due_time().is_some());
    assert!(format!("{keeper:?}").contains("SignallingKeeper"));
}
