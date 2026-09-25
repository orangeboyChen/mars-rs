//! The timing sync, through the public api.
//!
//! Ninety seconds between two syncs while the app is active and logged in, four
//! minutes while nobody is logged in, ten while it is not active at all — and
//! three times as long with no network to sync on. A link that came up cancels
//! the alarm, and a change only restarts an alarm that is still waiting.

use std::sync::{Arc, Mutex};

use mars_stn::timing_sync::{
    alarm_time, TimingSync, ACTIVE_SYNC_INTERVAL, INACTIVE_SYNC_INTERVAL, NONET_SALT_RATE,
    UNLOGIN_SYNC_INTERVAL,
};
use mars_stn::LongLinkStatus;

/// A sync on a wifi network, with the app active and logged in.
fn a_sync(now: u64) -> (TimingSync, Arc<Mutex<usize>>) {
    let mut sync = TimingSync::new_at(now);
    sync.set_is_active(|| true);
    sync.set_is_logoned(|| true);
    sync.set_net_info(|| 1);
    let syncs = Arc::new(Mutex::new(0usize));
    let record = Arc::clone(&syncs);
    sync.set_request_sync(move || {
        *record.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    });
    (sync, syncs)
}

#[test]
fn the_wait_is_what_the_app_and_the_network_make_it() {
    // active and logged in: `ACTIVE_SYNC_INTERVAL`
    assert_eq!(alarm_time(true, true, 1), ACTIVE_SYNC_INTERVAL);
    // active, but nobody logged in
    assert_eq!(alarm_time(true, false, 1), UNLOGIN_SYNC_INTERVAL);
    // not active, logged in or not
    assert_eq!(alarm_time(false, true, 1), INACTIVE_SYNC_INTERVAL);
    assert_eq!(alarm_time(false, false, 1), INACTIVE_SYNC_INTERVAL);

    // ... and with no network every one of them is three times as long
    assert_eq!(
        alarm_time(true, true, -1),
        ACTIVE_SYNC_INTERVAL * NONET_SALT_RATE
    );
    assert_eq!(
        alarm_time(false, false, -1),
        INACTIVE_SYNC_INTERVAL * NONET_SALT_RATE
    );
}

#[test]
fn the_constructor_arms_the_alarm_and_every_callback_rearms_it() {
    let mut sync = TimingSync::new_at(0);
    // before any of the host: not active, nobody logged in, and no network,
    // which is the longest wait there is
    assert_eq!(
        sync.due_time(),
        Some(INACTIVE_SYNC_INTERVAL * NONET_SALT_RATE)
    );

    // the C++ has all of this before its constructor runs, and the port gets
    // it afterwards — so arming it again is what installing it does
    sync.set_net_info(|| 1);
    assert_eq!(sync.due_time(), Some(INACTIVE_SYNC_INTERVAL));
    sync.set_is_active(|| true);
    assert_eq!(sync.due_time(), Some(UNLOGIN_SYNC_INTERVAL));
    sync.set_is_logoned(|| true);
    assert_eq!(sync.due_time(), Some(ACTIVE_SYNC_INTERVAL));

    // ... and a host that installs them all at once gets there in one step
    let (sync, _syncs) = a_sync(0);
    assert_eq!(sync.due_time(), Some(ACTIVE_SYNC_INTERVAL));
}

#[test]
fn the_alarm_asks_for_a_sync_and_arms_itself_again() {
    let (mut sync, syncs) = a_sync(0);
    assert_eq!(
        sync.on_alarm_at(ACTIVE_SYNC_INTERVAL),
        2 * ACTIVE_SYNC_INTERVAL
    );
    assert_eq!(sync.due_time(), Some(2 * ACTIVE_SYNC_INTERVAL));
    assert_eq!(*syncs.lock().unwrap_or_else(|e| e.into_inner()), 1);

    // with no network the app is not asked, but the alarm is armed all the
    // same — and for three times as long
    sync.set_net_info(|| -1);
    assert_eq!(
        sync.on_alarm_at(2 * ACTIVE_SYNC_INTERVAL),
        2 * ACTIVE_SYNC_INTERVAL + ACTIVE_SYNC_INTERVAL * NONET_SALT_RATE
    );
    assert_eq!(*syncs.lock().unwrap_or_else(|e| e.into_inner()), 1);
}

#[test]
fn a_change_restarts_a_waiting_alarm_and_only_a_waiting_one() {
    let (mut sync, _syncs) = a_sync(0);
    sync.on_active_changed_at(0, true);
    assert_eq!(sync.due_time(), Some(ACTIVE_SYNC_INTERVAL));

    // active but nobody logged in is the four minute wait
    sync.set_is_logoned(|| false);
    sync.on_network_change_at(0);
    assert_eq!(sync.due_time(), Some(UNLOGIN_SYNC_INTERVAL));

    // a cancelled alarm is not rearmed by either
    sync.cancel();
    sync.on_active_changed_at(0, true);
    sync.on_network_change_at(0);
    assert_eq!(sync.due_time(), None);
}

#[test]
fn a_link_that_came_up_cancels_the_alarm_and_one_that_went_down_arms_it() {
    let (mut sync, _syncs) = a_sync(0);
    sync.on_longlink_status_changed_at(0, LongLinkStatus::Connected);
    assert_eq!(sync.due_time(), None);

    // the states in between are not looked at
    sync.on_longlink_status_changed_at(0, LongLinkStatus::Connecting);
    assert_eq!(sync.due_time(), None);
    sync.on_longlink_status_changed_at(0, LongLinkStatus::ConnectIdle);
    assert_eq!(sync.due_time(), None);

    sync.on_longlink_status_changed_at(1_000, LongLinkStatus::DisConnected);
    assert_eq!(sync.due_time(), Some(1_000 + ACTIVE_SYNC_INTERVAL));
    sync.on_longlink_status_changed_at(2_000, LongLinkStatus::ConnectFailed);
    assert_eq!(
        sync.due_time(),
        Some(1_000 + ACTIVE_SYNC_INTERVAL),
        "only the two states the sync looks at move it"
    );
}

#[test]
fn the_alarm_of_an_app_with_no_host_at_all_is_armed_for_the_longest_wait() {
    // no host: not active, nobody logged in, and no network
    let mut sync = TimingSync::new_at(0);
    assert_eq!(
        sync.due_time(),
        Some(INACTIVE_SYNC_INTERVAL * NONET_SALT_RATE)
    );
    sync.on_alarm();
    sync.on_active_changed(true);
    sync.on_network_change();
    sync.on_longlink_status_changed(LongLinkStatus::Connected);
    assert_eq!(sync.due_time(), None);
}

#[test]
fn the_methods_that_take_no_reading_ask_the_clock_themselves() {
    let mut sync = TimingSync::default();
    assert!(sync.due_time().is_some());
    sync.on_alarm();
    sync.on_active_changed(true);
    sync.on_network_change();
    sync.on_longlink_status_changed(LongLinkStatus::Connected);
    assert_eq!(sync.due_time(), None);
    assert!(format!("{sync:?}").contains("TimingSync"));
}
