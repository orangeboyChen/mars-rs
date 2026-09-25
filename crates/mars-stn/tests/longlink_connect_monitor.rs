//! `mars/stn/src/longlink_connect_monitor.cc`, through the public api.
//!
//! The samples are what the C++ answers for the same calls: a link that went
//! down while the app is in the background is retried a minute later and then
//! two, the wait is cut short by thirty seconds of buffer while the app is not
//! active, a network change drops the link before asking for a new one, and a
//! server that has turned the trigger off is never asked at all.

use std::sync::{Arc, Mutex};

use mars_stn::longlink_connect_monitor::{
    ConnectType, LongLinkConnectMonitor, LongLinkStatus, INACTIVE_BUFFER, INTERVALS,
    NO_ACCOUNT_INFO_INACTIVE_INTERVAL, UP_OR_DOWN_THRESHOLD, WAKE_ALARM_INTERVAL,
};

/// What the app was asked to do, in order.
type Calls = Arc<Mutex<Vec<&'static str>>>;

/// The monitor as an app that is active and in the foreground would see it,
/// with an account and a wifi network — which is what makes the long-link
/// interval the plain [`INTERVALS`] one, unsalted.
fn a_monitor(now: u64) -> (LongLinkConnectMonitor, Calls) {
    let mut monitor = LongLinkConnectMonitor::new_at(now, true);
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&calls);
    monitor.set_make_sure_connected(move || {
        record
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push("connect");
        true
    });
    let record = Arc::clone(&calls);
    monitor.set_disconnect(move || {
        record
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push("disconnect");
    });
    monitor.set_is_active(|| true);
    monitor.set_is_foreground(|| true);
    monitor.set_last_foreground_change_time(|| 0);
    monitor.set_net_info(|| 1);
    monitor.set_has_account(|| true);
    monitor.set_random(|_bound| 0);
    (monitor, calls)
}

#[test]
fn a_foreground_app_waits_the_first_column_of_the_table() {
    let (mut monitor, calls) = a_monitor(0);
    // `sg_interval[kLongLinkConnect][kForgroundOneMinute]` is 15s
    assert_eq!(monitor.on_alarm_at(0, false), 15_000);
    assert_eq!(monitor.rebuild_due_time(), Some(15_000));
    assert!(calls.lock().unwrap_or_else(|e| e.into_inner()).is_empty());

    // ... and the alarm it armed is the one that asks for the link
    assert_eq!(monitor.on_alarm_at(15_000, false), 0);
    assert_eq!(monitor.rebuild_due_time(), None);
    assert_eq!(
        *calls.lock().unwrap_or_else(|e| e.into_inner()),
        vec!["connect"]
    );
    // the active path resets the ladder
    assert_eq!(monitor.current_interval_index(), 0);
}

#[test]
fn in_the_background_the_wait_climbs_the_ladder() {
    let (mut monitor, calls) = a_monitor(0);
    monitor.set_is_active(|| false);
    monitor.set_is_foreground(|| false);

    // `reconnect_interval[1]` is a minute
    assert_eq!(monitor.on_alarm_at(0, false), 60_000);
    assert_eq!(monitor.current_interval_index(), 1);
    // the next rung is two minutes, but a minute of it has been spent already
    assert_eq!(monitor.on_alarm_at(60_000, false), 60_000);
    assert_eq!(monitor.current_interval_index(), 2);
    // ... and the one after that is four, of which two are spent
    assert_eq!(monitor.on_alarm_at(120_000, false), 120_000);
    assert_eq!(monitor.current_interval_index(), 3);
    assert!(calls.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
}

#[test]
fn a_link_that_was_up_for_ten_minutes_is_retried_at_once() {
    let (mut monitor, calls) = a_monitor(0);
    monitor.set_is_active(|| false);
    monitor.set_is_foreground(|| false);
    // the dns was at `0`, so ten minutes later the link "posted" ten minutes
    // ago — more than `kUpOrDownThreshold`, which walks the ladder down and
    // makes the wait that is left negative
    monitor.set_dns_time(|| 0);
    assert_eq!(monitor.on_alarm_at(UP_OR_DOWN_THRESHOLD, false), 0);
    assert_eq!(
        *calls.lock().unwrap_or_else(|e| e.into_inner()),
        vec!["connect"]
    );
    assert_eq!(
        monitor.current_interval_index(),
        0,
        "the first rung is as low as it goes"
    );
    assert_eq!(monitor.rebuild_due_time(), None);
}

#[test]
fn the_rebuild_alarm_asks_for_the_link_and_arms_nothing() {
    let (mut monitor, calls) = a_monitor(0);
    monitor.set_is_active(|| false);
    monitor.set_is_foreground(|| false);

    assert_eq!(monitor.on_alarm_at(0, true), 0);
    assert_eq!(monitor.rebuild_due_time(), None);
    assert_eq!(
        *calls.lock().unwrap_or_else(|e| e.into_inner()),
        vec!["connect"]
    );
}

#[test]
fn a_link_that_is_up_or_coming_up_is_left_alone() {
    let (mut monitor, calls) = a_monitor(0);
    monitor.set_connect_status(|| LongLinkStatus::Connecting);
    assert_eq!(monitor.on_alarm_at(0, false), 0);
    monitor.set_connect_status(|| LongLinkStatus::Connected);
    assert_eq!(monitor.on_alarm_at(0, false), 0);
    assert!(calls.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
    assert_eq!(monitor.rebuild_due_time(), None, "nothing was armed");
}

#[test]
fn a_task_connect_asks_the_table_and_the_link_is_what_answers() {
    let (mut monitor, calls) = a_monitor(0);
    // `sg_interval[kTaskConnect][kForgroundOneMinute]` is 5s, and the dns was
    // six seconds ago
    monitor.set_dns_time(|| 0);
    assert!(!monitor.make_sure_connected_at(6_000));
    assert_eq!(
        *calls.lock().unwrap_or_else(|e| e.into_inner()),
        vec!["connect"]
    );

    // a second in, four seconds are left, so the link is not asked for — and
    // `MakeSureConnected` answers whether it is up, which it is not
    let (mut monitor, calls) = a_monitor(1_000);
    monitor.set_dns_time(|| 0);
    assert!(!monitor.make_sure_connected_at(1_000));
    assert!(calls.lock().unwrap_or_else(|e| e.into_inner()).is_empty());

    // `kInactiveBuffer`, the thirty seconds the C++ adds to the wait of an app
    // that is not active, is only ever compared against the all-zero network
    // change interval, so it cannot change an answer
    assert_eq!(INACTIVE_BUFFER, 30_000);
    assert_eq!(INTERVALS[ConnectType::NetworkChange as usize][0], 0);
}

#[test]
fn a_network_change_drops_the_link_before_asking_for_a_new_one() {
    let (mut monitor, calls) = a_monitor(0);
    // `sg_interval[kNetworkChangeConnect]` is all zeros, so it is always now
    assert!(monitor.network_change_at(0));
    assert_eq!(
        *calls.lock().unwrap_or_else(|e| e.into_inner()),
        vec!["disconnect", "connect"]
    );
}

#[test]
fn a_server_that_turned_the_trigger_off_is_never_asked() {
    let (mut monitor, calls) = a_monitor(0);
    monitor.set_is_svr_trig_off(|| true);
    monitor.set_connect_status(|| LongLinkStatus::Connected);

    assert_eq!(monitor.on_alarm_at(0, false), 0);
    assert!(!monitor.make_sure_connected_at(0));
    assert!(calls.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
    assert_eq!(monitor.rebuild_due_time(), None);
}

#[test]
fn the_wake_alarm_follows_a_link_that_went_down() {
    let (mut monitor, _calls) = a_monitor(1_000);
    monitor.on_longlink_status_changed_at(1_000, LongLinkStatus::DisConnected);
    assert_eq!(monitor.status(), LongLinkStatus::DisConnected);
    assert_eq!(monitor.last_connect_time(), 1_000);
    assert_eq!(monitor.last_connect_net_type(), 1);
    assert_eq!(monitor.wake_due_time(), Some(1_000 + WAKE_ALARM_INTERVAL));

    // ... and the wake alarm is what the host fires half a second later: a
    // half second of the fifteen the table asks for has gone by already
    assert_eq!(monitor.on_alarm_at(1_500, false), 13_500);
    assert_eq!(monitor.rebuild_due_time(), Some(15_000));
    assert_eq!(
        monitor.wake_due_time(),
        None,
        "the C++ cancels both alarms before it answers"
    );

    // a link that came up cancels both
    monitor.on_longlink_status_changed_at(2_000, LongLinkStatus::Connected);
    assert_eq!(monitor.wake_due_time(), None);
    assert_eq!(monitor.rebuild_due_time(), None);
}

#[test]
fn the_signals_the_app_sends_ask_the_same_question() {
    let (mut monitor, _calls) = a_monitor(0);
    monitor.set_is_foreground(|| false);
    // active but in the background: `sg_interval[1][3]` is five minutes
    monitor.on_foreground_changed_at(0, false);
    assert_eq!(monitor.rebuild_due_time(), Some(300_000));
    monitor.on_active_changed_at(0, true);
    assert_eq!(monitor.rebuild_due_time(), Some(300_000));

    // ... and the heartbeat alarms the C++ only logs are here to be called
    monitor.on_heartbeat_alarm_set(180_000);
    monitor.on_heartbeat_alarm_received(true);
}

#[test]
fn without_an_account_and_without_a_network_the_wait_is_a_week() {
    let (mut monitor, _calls) = a_monitor(0);
    monitor.set_is_active(|| false);
    monitor.set_is_foreground(|| false);
    monitor.set_has_account(|| false);
    // inactive and no account to connect for — the interval the ladder is
    // walked from is enormous, but the ladder does not look at it
    assert_eq!(
        monitor.on_alarm_at(0, false),
        60_000,
        "`reconnect_interval[1]`, whatever the table said"
    );
    assert_eq!(NO_ACCOUNT_INFO_INACTIVE_INTERVAL, 7 * 24 * 60 * 60);
}

#[test]
fn without_a_host_the_monitor_still_answers() {
    let mut monitor = LongLinkConnectMonitor::default();
    // no host at all: not active, so the ladder decides
    assert_eq!(monitor.on_alarm_at(0, false), 60_000);
    assert_eq!(monitor.on_alarm_at(60_000, true), 0);
    assert_eq!(monitor.status(), LongLinkStatus::DisConnected);
    assert!(!monitor.is_keep_alive());

    // ... and what the C++'s Apple paths call is the reset the host set
    let resets = Arc::new(Mutex::new(0));
    let record = Arc::clone(&resets);
    monitor.set_longlink_reset(move || *record.lock().unwrap_or_else(|e| e.into_inner()) += 1);
    monitor.reconnect();
    assert_eq!(*resets.lock().unwrap_or_else(|e| e.into_inner()), 1);
    monitor.clear_longlink_reset();
    monitor.reconnect();
    assert_eq!(*resets.lock().unwrap_or_else(|e| e.into_inner()), 1);
    assert!(format!("{monitor:?}").contains("LongLinkConnectMonitor"));
}

#[test]
fn the_methods_that_take_no_reading_ask_the_clock_themselves() {
    let mut monitor = LongLinkConnectMonitor::new(true);
    assert!(monitor.is_keep_alive());
    assert!(monitor.rebuild_due_time().is_none());

    monitor.on_alarm(false);
    monitor.make_sure_connected();
    monitor.network_change();
    monitor.on_foreground_changed(true);
    monitor.on_active_changed(true);
    monitor.on_longlink_status_changed(LongLinkStatus::Connected);
    monitor.on_heartbeat_alarm_set(0);
    monitor.on_heartbeat_alarm_received(false);
    monitor.reconnect();
    assert_eq!(monitor.status(), LongLinkStatus::Connected);
}
