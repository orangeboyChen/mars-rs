//! `mars/stn/src/timing_sync.cc` — the periodic sync the app is asked for.
//!
//! One alarm, restarted every time something changes: how long it waits is
//! [`ACTIVE_SYNC_INTERVAL`] while the app is active and logged in,
//! [`UNLOGIN_SYNC_INTERVAL`] while it is active but not logged in, and
//! [`INACTIVE_SYNC_INTERVAL`] while it is not active at all, times
//! [`NONET_SALT_RATE`] when there is no network. When it goes off the app is
//! asked for a sync — but only when there is a network to do it on — and the
//! alarm is armed again.
//!
//! The C++ reads `ActiveLogic`, `AppManager::GetAccountInfo()`, `getNetInfo()`
//! and `StnManager::RequestSync()`; all four are callbacks here, and
//! `comm::Alarm` is a due time the host compares its own clock against
//! ([`TimingSync::due_time`]), the way the rest of the crate models one.

use crate::longlink_connect_monitor::LongLinkStatus;
use crate::smart_heartbeat::NO_NET;
use mars_comm::tickcount::gettickcount;

/// `ACTIVE_SYNC_INTERVAL` — how long an active, logged-in app waits.
pub const ACTIVE_SYNC_INTERVAL: u64 = 90 * 1000;
/// `UNLOGIN_SYNC_INTERVAL` — an active app with nobody logged in waits longer.
pub const UNLOGIN_SYNC_INTERVAL: u64 = 4 * 60 * 1000;
/// `INACTIVE_SYNC_INTERVAL` — and an inactive one longer still.
pub const INACTIVE_SYNC_INTERVAL: u64 = 10 * 60 * 1000;
/// `NONET_SALT_RATE` — what the wait is multiplied by with no network.
pub const NONET_SALT_RATE: u64 = 3;

/// `GetAlarmTime(_is_actived, _is_logoned)` — the C++'s `int`, and the
/// `getNetInfo()` it reads itself is an argument here.
pub fn alarm_time(is_actived: bool, is_logoned: bool, net_info: i32) -> u64 {
    let time = if is_actived && !is_logoned {
        UNLOGIN_SYNC_INTERVAL
    } else if is_actived {
        ACTIVE_SYNC_INTERVAL
    } else {
        INACTIVE_SYNC_INTERVAL
    };
    if net_info == NO_NET {
        time * NONET_SALT_RATE
    } else {
        time
    }
}

/// `ActiveLogic::IsActive()` — unset answers `false`.
pub type IsActive = dyn FnMut() -> bool + Send;
/// `GetAccountInfo().is_logoned` — unset answers `false`.
pub type IsLogoned = dyn FnMut() -> bool + Send;
/// `getNetInfo()` — unset answers [`NO_NET`].
pub type NetInfo = dyn FnMut() -> i32 + Send;
/// `StnManager::RequestSync()`.
pub type RequestSync = dyn FnMut() + Send;

/// `TimingSync`.
pub struct TimingSync {
    /// `alarm_` — the reading it is due at, [`None`] for one that was
    /// cancelled.
    due: Option<u64>,
    is_active: Option<Box<IsActive>>,
    is_logoned: Option<Box<IsLogoned>>,
    net_info: Option<Box<NetInfo>>,
    request_sync: Option<Box<RequestSync>>,
}

impl Default for TimingSync {
    /// `TimingSync(…)`, at the tick count the clock gives.
    fn default() -> Self {
        Self::new()
    }
}

impl TimingSync {
    /// `TimingSync(…)` — which arms the alarm in the constructor.
    pub fn new() -> Self {
        Self::new_at(gettickcount())
    }

    /// The same, with the reading the C++'s `gettickcount()` would hand out.
    pub fn new_at(now: u64) -> Self {
        let mut sync = Self {
            due: None,
            is_active: None,
            is_logoned: None,
            net_info: None,
            request_sync: None,
        };
        sync.start(now);
        sync
    }

    /// `ActiveLogic::IsActive()`.
    pub fn set_is_active(&mut self, is_active: impl FnMut() -> bool + Send + 'static) {
        self.is_active = Some(Box::new(is_active));
    }

    /// `GetAccountInfo().is_logoned`.
    pub fn set_is_logoned(&mut self, is_logoned: impl FnMut() -> bool + Send + 'static) {
        self.is_logoned = Some(Box::new(is_logoned));
    }

    /// `getNetInfo()`.
    pub fn set_net_info(&mut self, net_info: impl FnMut() -> i32 + Send + 'static) {
        self.net_info = Some(Box::new(net_info));
    }

    /// `StnManager::RequestSync()` — what the alarm asks for.
    pub fn set_request_sync(&mut self, request_sync: impl FnMut() + Send + 'static) {
        self.request_sync = Some(Box::new(request_sync));
    }

    /// When the alarm is due, [`None`] for one that is not armed.
    pub fn due_time(&self) -> Option<u64> {
        self.due
    }

    /// `OnActiveChanged(_is_actived)` — the C++ only rearms a waiting alarm.
    pub fn on_active_changed(&mut self, is_actived: bool) {
        self.on_active_changed_at(gettickcount(), is_actived)
    }

    /// The same, with the reading handed in.
    pub fn on_active_changed_at(&mut self, now: u64, is_actived: bool) {
        if self.due.is_some() {
            let is_logoned = self.is_logoned();
            self.start_with(now, is_actived, is_logoned);
        }
    }

    /// `OnNetworkChange()` — the same alarm, restarted for the same reason:
    /// `getNetInfo()` may have changed what the wait is.
    pub fn on_network_change(&mut self) {
        self.on_network_change_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn on_network_change_at(&mut self, now: u64) {
        if self.due.is_some() {
            self.start(now);
        }
    }

    /// `OnLongLinkStatuChanged(_status, _channel_id)` — a link that came up
    /// cancels the alarm and one that went down arms it. The other two states
    /// are not looked at.
    pub fn on_longlink_status_changed(&mut self, status: LongLinkStatus) {
        self.on_longlink_status_changed_at(gettickcount(), status)
    }

    /// The same, with the reading handed in.
    pub fn on_longlink_status_changed_at(&mut self, now: u64, status: LongLinkStatus) {
        match status {
            LongLinkStatus::Connected => {
                self.due = None;
            }
            LongLinkStatus::DisConnected => {
                self.start(now);
            }
            _ => {}
        }
    }

    /// `__OnAlarm()` — ask for the sync when there is a network to do it on,
    /// then arm the alarm again whatever the network is: the C++ starts it
    /// either way. The reading it is due at next is what it answers.
    pub fn on_alarm(&mut self) -> u64 {
        self.on_alarm_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn on_alarm_at(&mut self, now: u64) -> u64 {
        if self.net_info() != NO_NET {
            if let Some(request_sync) = self.request_sync.as_mut() {
                request_sync();
            }
        }
        self.start(now)
    }

    /// `alarm_.Cancel()` — what the C++'s destructor does, and what a host
    /// calls when it stops asking for syncs.
    pub fn cancel(&mut self) {
        self.due = None;
    }

    /// `alarm_.Start(GetAlarmTime(…))` — armed at the reading it answers with.
    fn start(&mut self, now: u64) -> u64 {
        let is_actived = self.is_active();
        let is_logoned = self.is_logoned();
        self.start_with(now, is_actived, is_logoned)
    }

    fn start_with(&mut self, now: u64, is_actived: bool, is_logoned: bool) -> u64 {
        let wait = alarm_time(is_actived, is_logoned, self.net_info());
        let due = now.saturating_add(wait);
        self.due = Some(due);
        due
    }

    fn is_active(&mut self) -> bool {
        match self.is_active.as_mut() {
            Some(is_active) => is_active(),
            None => false,
        }
    }

    fn is_logoned(&mut self) -> bool {
        match self.is_logoned.as_mut() {
            Some(is_logoned) => is_logoned(),
            None => false,
        }
    }

    fn net_info(&mut self) -> i32 {
        match self.net_info.as_mut() {
            Some(net_info) => net_info(),
            None => NO_NET,
        }
    }
}

impl std::fmt::Debug for TimingSync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimingSync")
            .field("due", &self.due)
            .field("has_request_sync", &self.request_sync.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sync on a wifi network, with nobody logged in and the app inactive:
    /// the longest wait there is.
    fn a_sync(now: u64) -> TimingSync {
        TimingSync::new_at(now)
    }

    #[test]
    fn the_wait_is_what_the_app_and_the_network_make_it() {
        // active and logged in
        assert_eq!(alarm_time(true, true, 1), ACTIVE_SYNC_INTERVAL);
        // active but nobody logged in
        assert_eq!(alarm_time(true, false, 1), UNLOGIN_SYNC_INTERVAL);
        // not active, logged in or not
        assert_eq!(alarm_time(false, true, 1), INACTIVE_SYNC_INTERVAL);
        assert_eq!(alarm_time(false, false, 1), INACTIVE_SYNC_INTERVAL);
        // ... and with no network every one of them is three times as long
        assert_eq!(
            alarm_time(true, true, NO_NET),
            ACTIVE_SYNC_INTERVAL * NONET_SALT_RATE
        );
        assert_eq!(
            alarm_time(false, false, NO_NET),
            INACTIVE_SYNC_INTERVAL * NONET_SALT_RATE
        );
    }

    #[test]
    fn the_constructor_arms_the_alarm() {
        let sync = a_sync(1_000);
        // no host, so no network and nobody logged in: the longest wait
        assert_eq!(
            sync.due_time(),
            Some(1_000 + INACTIVE_SYNC_INTERVAL * NONET_SALT_RATE)
        );
    }

    #[test]
    fn a_change_restarts_a_waiting_alarm_and_only_a_waiting_one() {
        let mut sync = a_sync(0);
        sync.set_is_active(|| true);
        sync.set_is_logoned(|| true);
        sync.set_net_info(|| 1);
        sync.on_active_changed_at(0, true);
        assert_eq!(sync.due_time(), Some(ACTIVE_SYNC_INTERVAL));

        sync.on_network_change_at(0);
        assert_eq!(sync.due_time(), Some(ACTIVE_SYNC_INTERVAL));

        // a cancelled alarm is not rearmed by either
        sync.cancel();
        sync.on_active_changed_at(0, true);
        sync.on_network_change_at(0);
        assert_eq!(sync.due_time(), None);
    }

    #[test]
    fn a_link_that_came_up_cancels_the_alarm_and_one_that_went_down_arms_it() {
        let mut sync = a_sync(0);
        sync.set_net_info(|| 1);
        sync.set_is_active(|| true);

        sync.on_longlink_status_changed_at(0, LongLinkStatus::Connected);
        assert_eq!(sync.due_time(), None);
        // the states in between are not looked at
        sync.on_longlink_status_changed_at(0, LongLinkStatus::Connecting);
        assert_eq!(sync.due_time(), None);
        sync.on_longlink_status_changed_at(1_000, LongLinkStatus::DisConnected);
        assert_eq!(sync.due_time(), Some(1_000 + UNLOGIN_SYNC_INTERVAL));
        sync.on_longlink_status_changed_at(2_000, LongLinkStatus::ConnectFailed);
        assert_eq!(sync.due_time(), Some(1_000 + UNLOGIN_SYNC_INTERVAL));
    }

    #[test]
    fn the_alarm_asks_for_a_sync_and_arms_itself_again() {
        let mut sync = a_sync(0);
        sync.set_net_info(|| 1);
        sync.set_is_active(|| true);
        sync.set_is_logoned(|| true);
        let syncs = std::sync::Arc::new(std::sync::Mutex::new(0usize));
        let record = std::sync::Arc::clone(&syncs);
        sync.set_request_sync(move || {
            *record.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        });

        assert_eq!(
            sync.on_alarm_at(ACTIVE_SYNC_INTERVAL),
            2 * ACTIVE_SYNC_INTERVAL
        );
        assert_eq!(*syncs.lock().unwrap_or_else(|e| e.into_inner()), 1);

        // with no network the app is not asked, but the alarm is armed again
        sync.set_net_info(|| NO_NET);
        assert_eq!(
            sync.on_alarm_at(2 * ACTIVE_SYNC_INTERVAL),
            2 * ACTIVE_SYNC_INTERVAL + ACTIVE_SYNC_INTERVAL * NONET_SALT_RATE
        );
        assert_eq!(*syncs.lock().unwrap_or_else(|e| e.into_inner()), 1);
    }

    #[test]
    fn without_a_host_the_alarm_still_answers() {
        let mut sync = TimingSync::default();
        assert!(sync.due_time().is_some());
        sync.on_alarm();
        sync.on_active_changed(true);
        sync.on_network_change();
        sync.on_longlink_status_changed(LongLinkStatus::Connected);
        assert_eq!(sync.due_time(), None);
        assert!(format!("{sync:?}").contains("TimingSync"));
    }
}
