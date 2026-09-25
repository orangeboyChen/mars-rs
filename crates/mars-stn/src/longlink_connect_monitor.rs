//! `mars/stn/src/longlink_connect_monitor.cc` — how soon the long link is
//! tried again.
//!
//! The long link is not reconnected the moment it goes down: the monitor waits
//! an interval that depends on what the app is doing ([`INTERVALS`] — foreground
//! and active get the short ones, background and inactive the long ones), and
//! while the app is in the background it walks a ladder of its own
//! ([`RECONNECT_INTERVAL`]), up when the link is being rebuilt quickly and down
//! when it has been up for a while. How long there is left to wait is what the
//! methods that ask it answer, and `0` means "now".
//!
//! Everything the C++ reads from `LongLink`, `ActiveLogic`, `getNetInfo()` and
//! `AppManager::GetAccountInfo()` is a callback here, and so is
//! `rand()`:
//!
//! * an unset query answers the way the C++ would before the app has said
//!   anything — `false` for `IsActive` / `IsForeground` / an account name, and
//!   [`NO_NET`] for `getNetInfo()`, which is what makes an unhosted monitor
//!   take the *inactive* column of the table;
//! * `Alarm::Start` on the two alarms the C++ owns is a due time the host
//!   compares its own clock against ([`LongLinkConnectMonitor::rebuild_due_time`]
//!   and [`LongLinkConnectMonitor::wake_due_time`]), the way the rest of the
//!   crate models a message queue;
//! * the two `OnHeartbeatAlarm*` methods write to the Android system log in the
//!   C++ and do nothing else, so they are here for the host to call and stay
//!   empty;
//! * what is not ported is the `#ifdef __APPLE__` half — the periodic
//!   `socket_gethostbyname` probe and the "the socket has not received anything
//!   for 4.5 minutes" reconnect — because both are Apple-only paths over a
//!   thread and a dns call the port does not have. What they call is
//!   [`LongLinkConnectMonitor::reconnect`], which stays.

use crate::smart_heartbeat::NO_NET;
use crate::Random;

/// `kTimeCheckPeriod` / `kStartCheckPeriod` — the period of the Apple-only
/// probe, in milliseconds.
pub const TIME_CHECK_PERIOD: u64 = 10 * 1000;
pub const START_CHECK_PERIOD: u64 = 3 * 1000;
/// `kInactiveBuffer` — how much longer the wait is while the app is not active,
/// so a dozing platform does not wake up to find nothing to do.
pub const INACTIVE_BUFFER: u64 = 30 * 1000;

/// `kNoNetSaltRate` / `kNoNetSaltRise` — the interval with no network at all.
pub const NO_NET_SALT_RATE: u64 = 3;
pub const NO_NET_SALT_RISE: u64 = 600;
/// `kNoAccountInfoSaltRate` / `kNoAccountInfoSaltRise` — the interval with no
/// account to connect for.
pub const NO_ACCOUNT_INFO_SALT_RATE: u64 = 2;
pub const NO_ACCOUNT_INFO_SALT_RISE: u64 = 300;
/// `kNoAccountInfoInactiveInterval` — a week, in seconds: with no account and
/// no active app there is nothing to reconnect for.
pub const NO_ACCOUNT_INFO_INACTIVE_INTERVAL: u64 = 7 * 24 * 60 * 60;

/// `kUpOrDownThreshold` — ten minutes: a link that came back sooner than this
/// moves the ladder up, one that held longer moves it down.
pub const UP_OR_DOWN_THRESHOLD: u64 = 10 * 60 * 1000;

/// `wake_alarm_.Start(500)` after a disconnect or a failed connect.
pub const WAKE_ALARM_INTERVAL: u64 = 500;

/// `sg_interval[][5]` — seconds to wait, by [`ConnectType`] and
/// [`ActiveState`].
pub const INTERVALS: [[u64; 5]; 3] = [
    [5, 10, 20, 30, 300],
    [15, 30, 120, 300, 600],
    [0, 0, 0, 0, 0],
];

/// `reconnect_interval[]` — the ladder the background wait walks, in seconds.
pub const RECONNECT_INTERVAL: [u64; 7] = [0, 60, 120, 240, 360, 480, 600];
/// `kInternalMaxIndex` — the top of the ladder.
pub const INTERNAL_MAX_INDEX: usize = 6;

/// `LongLink::TLongLinkStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LongLinkStatus {
    /// `kConnectIdle`
    #[default]
    ConnectIdle = 0,
    /// `kConnecting`
    Connecting = 1,
    /// `kConnected`
    Connected = 2,
    /// `kDisConnected`
    DisConnected = 3,
    /// `kConnectFailed`
    ConnectFailed = 4,
}

/// Why a connect is being asked for: `kTaskConnect`, `kLongLinkConnect`,
/// `kNetworkChangeConnect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectType {
    /// `kTaskConnect` — a task needs the link.
    #[default]
    Task = 0,
    /// `kLongLinkConnect` — the alarm went off.
    LongLink = 1,
    /// `kNetworkChangeConnect`
    NetworkChange = 2,
}

/// `__CurActiveState`: `kForgroundOneMinute`, `kForgroundTenMinute`,
/// `kForgroundActive`, `kBackgroundActive`, `kInactive`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveState {
    /// Foreground for less than a minute.
    #[default]
    ForegroundOneMinute = 0,
    /// Foreground for more than a minute.
    ForegroundTenMinute = 1,
    /// Foreground for more than ten minutes.
    ForegroundActive = 2,
    /// Active, but in the background.
    BackgroundActive = 3,
    /// Not active at all.
    Inactive = 4,
}

/// `LongLinkConnectMonitor`.
pub struct LongLinkConnectMonitor {
    /// `status_`
    status: LongLinkStatus,
    /// `last_connect_time_`
    last_connect_time: u64,
    /// `last_connect_net_type_` — `last_connect_net_type_(kNoNet)`.
    last_connect_net_type: i32,
    /// `current_interval_index_`
    current_interval_index: usize,
    /// `rebuild_longlink_`
    rebuild_longlink: bool,
    /// `is_keep_alive_`
    is_keep_alive: bool,
    /// `rebuild_alarm_` — the reading it is due at.
    rebuild_due: Option<u64>,
    /// `wake_alarm_` — the reading it is due at.
    wake_due: Option<u64>,

    is_svr_trig_off: Option<Box<dyn FnMut() -> bool + Send>>,
    connect_status: Option<Box<dyn FnMut() -> LongLinkStatus + Send>>,
    dns_time: Option<Box<dyn FnMut() -> u64 + Send>>,
    make_sure_connected: Option<Box<dyn FnMut() -> bool + Send>>,
    disconnect: Option<Box<dyn FnMut() + Send>>,
    is_active: Option<Box<dyn FnMut() -> bool + Send>>,
    is_foreground: Option<Box<dyn FnMut() -> bool + Send>>,
    last_foreground_change_time: Option<Box<dyn FnMut() -> u64 + Send>>,
    net_info: Option<Box<dyn FnMut() -> i32 + Send>>,
    has_account: Option<Box<dyn FnMut() -> bool + Send>>,
    /// `fun_longlink_reset_`
    longlink_reset: Option<Box<dyn FnMut() + Send>>,
    random: Box<Random>,
}

impl LongLinkConnectMonitor {
    /// `LongLinkConnectMonitor(…, _is_keep_alive)`.
    pub fn new(is_keep_alive: bool) -> Self {
        Self::new_at(mars_comm::tickcount::gettickcount(), is_keep_alive)
    }

    /// The same, with the reading the C++'s `gettickcount()` would hand out.
    pub fn new_at(now: u64, is_keep_alive: bool) -> Self {
        Self {
            // `status_(LongLink::kDisConnected)`
            status: LongLinkStatus::DisConnected,
            last_connect_time: now,
            last_connect_net_type: NO_NET,
            current_interval_index: 0,
            rebuild_longlink: false,
            is_keep_alive,
            rebuild_due: None,
            wake_due: None,
            is_svr_trig_off: None,
            connect_status: None,
            dns_time: None,
            make_sure_connected: None,
            disconnect: None,
            is_active: None,
            is_foreground: None,
            last_foreground_change_time: None,
            net_info: None,
            has_account: None,
            longlink_reset: None,
            random: Box::new(crate::xorshift(mars_comm::tickcount::gettickcount())),
        }
    }

    /// `longlink_.IsSvrTrigOff()` — unset answers `false`.
    pub fn set_is_svr_trig_off(&mut self, is_svr_trig_off: impl FnMut() -> bool + Send + 'static) {
        self.is_svr_trig_off = Some(Box::new(is_svr_trig_off));
    }

    /// `longlink_.ConnectStatus()` — unset answers [`LongLinkStatus::ConnectIdle`].
    pub fn set_connect_status(
        &mut self,
        connect_status: impl FnMut() -> LongLinkStatus + Send + 'static,
    ) {
        self.connect_status = Some(Box::new(connect_status));
    }

    /// `longlink_.Profile().dns_time` — unset answers `0`.
    pub fn set_dns_time(&mut self, dns_time: impl FnMut() -> u64 + Send + 'static) {
        self.dns_time = Some(Box::new(dns_time));
    }

    /// `longlink_.MakeSureConnected(&newone)` — unset does nothing and answers
    /// `false`.
    pub fn set_make_sure_connected(
        &mut self,
        make_sure_connected: impl FnMut() -> bool + Send + 'static,
    ) {
        self.make_sure_connected = Some(Box::new(make_sure_connected));
    }

    /// `longlink_.Disconnect(kNetworkChange)` — unset does nothing.
    pub fn set_disconnect(&mut self, disconnect: impl FnMut() + Send + 'static) {
        self.disconnect = Some(Box::new(disconnect));
    }

    /// `activelogic_.IsActive()` — unset answers `false`, which is `kInactive`.
    pub fn set_is_active(&mut self, is_active: impl FnMut() -> bool + Send + 'static) {
        self.is_active = Some(Box::new(is_active));
    }

    /// `activelogic_.IsForeground()` — unset answers `false`.
    pub fn set_is_foreground(&mut self, is_foreground: impl FnMut() -> bool + Send + 'static) {
        self.is_foreground = Some(Box::new(is_foreground));
    }

    /// `activelogic_.LastForegroundChangeTime()` — unset answers `0`.
    pub fn set_last_foreground_change_time(
        &mut self,
        last_foreground_change_time: impl FnMut() -> u64 + Send + 'static,
    ) {
        self.last_foreground_change_time = Some(Box::new(last_foreground_change_time));
    }

    /// `getNetInfo()` — unset answers [`NO_NET`].
    pub fn set_net_info(&mut self, net_info: impl FnMut() -> i32 + Send + 'static) {
        self.net_info = Some(Box::new(net_info));
    }

    /// `!GetAccountInfo().username.empty()` — unset answers `false`, which is
    /// the C++ with no account to connect for.
    pub fn set_has_account(&mut self, has_account: impl FnMut() -> bool + Send + 'static) {
        self.has_account = Some(Box::new(has_account));
    }

    /// `fun_longlink_reset_ = …` — what [`LongLinkConnectMonitor::reconnect`]
    /// calls.
    pub fn set_longlink_reset(&mut self, longlink_reset: impl FnMut() + Send + 'static) {
        self.longlink_reset = Some(Box::new(longlink_reset));
    }

    /// `fun_longlink_reset_ = NULL`.
    pub fn clear_longlink_reset(&mut self) {
        self.longlink_reset = None;
    }

    /// `rand()` — the C++'s `rand() % 20` of jitter on the reconnect interval.
    pub fn set_random(&mut self, random: impl FnMut(usize) -> usize + Send + 'static) {
        self.random = Box::new(random);
    }

    /// `status_`.
    pub fn status(&self) -> LongLinkStatus {
        self.status
    }

    /// `last_connect_time_`.
    pub fn last_connect_time(&self) -> u64 {
        self.last_connect_time
    }

    /// `last_connect_net_type_`.
    pub fn last_connect_net_type(&self) -> i32 {
        self.last_connect_net_type
    }

    /// `current_interval_index_`.
    pub fn current_interval_index(&self) -> usize {
        self.current_interval_index
    }

    /// `is_keep_alive_`.
    pub fn is_keep_alive(&self) -> bool {
        self.is_keep_alive
    }

    /// When `rebuild_alarm_` is due, [`None`] when it is not running.
    pub fn rebuild_due_time(&self) -> Option<u64> {
        self.rebuild_due
    }

    /// When `wake_alarm_` is due, [`None`] when it is not running.
    pub fn wake_due_time(&self) -> Option<u64> {
        self.wake_due
    }

    /// `MakeSureConnected()` — `true` when the link is up when it is done.
    pub fn make_sure_connected(&mut self) -> bool {
        self.make_sure_connected_at(mars_comm::tickcount::gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn make_sure_connected_at(&mut self, now: u64) -> bool {
        if self.is_svr_trig_off() {
            return false;
        }
        self.interval_connect(now, ConnectType::Task);
        self.connect_status() == LongLinkStatus::Connected
    }

    /// `NetworkChange()` — the C++ drops the link first and then asks for a new
    /// one, and answers `0 == __IntervalConnect(kNetworkChangeConnect)`: what it
    /// answers is **that the connect was asked for now**, not that the link is
    /// up. The same `0` comes back when the server turned the trigger off and
    /// nothing was asked for at all, and when the link is merely connecting —
    /// the C++ ignores the `bool` its `MakeSureConnected` hands back. The
    /// question "is the link up" is [`Self::make_sure_connected_at`]'s, which
    /// is the one the C++ asks `longlink_.ConnectStatus()` to answer.
    pub fn network_change(&mut self) -> bool {
        self.network_change_at(mars_comm::tickcount::gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn network_change_at(&mut self, now: u64) -> bool {
        if let Some(disconnect) = self.disconnect.as_mut() {
            disconnect();
        }
        self.interval_connect(now, ConnectType::NetworkChange) == 0
    }

    /// `__OnSignalForeground(_isForeground)` — the C++ only subscribes to the
    /// signal when it keeps the link alive, and the port's host calls this
    /// either way.
    pub fn on_foreground_changed(&mut self, _is_foreground: bool) {
        self.on_foreground_changed_at(mars_comm::tickcount::gettickcount(), _is_foreground)
    }

    /// The same, with the reading handed in.
    pub fn on_foreground_changed_at(&mut self, now: u64, _is_foreground: bool) {
        self.auto_interval_connect(now);
    }

    /// `__OnSignalActive(_isactive)`.
    pub fn on_active_changed(&mut self, _is_active: bool) {
        self.on_active_changed_at(mars_comm::tickcount::gettickcount(), _is_active)
    }

    /// The same, with the reading handed in.
    pub fn on_active_changed_at(&mut self, now: u64, _is_active: bool) {
        self.auto_interval_connect(now);
    }

    /// `__OnLongLinkStatuChanged(_status, _channel_id)` — the C++ hands the
    /// channel id in and never looks at it, so the port does not take it.
    pub fn on_longlink_status_changed(&mut self, status: LongLinkStatus) {
        self.on_longlink_status_changed_at(mars_comm::tickcount::gettickcount(), status)
    }

    /// The same, with the reading handed in: both alarms are cancelled, and a
    /// link that went down or failed to come up arms the wake one.
    pub fn on_longlink_status_changed_at(&mut self, now: u64, status: LongLinkStatus) {
        self.rebuild_due = None;
        self.wake_due = None;

        if status == LongLinkStatus::ConnectFailed || status == LongLinkStatus::DisConnected {
            self.wake_due = Some(now.saturating_add(WAKE_ALARM_INTERVAL));
        }

        self.status = status;
        self.last_connect_time = now;
        self.last_connect_net_type = self.net_info();
    }

    /// `__OnAlarm(_rebuild_longlink)`.
    pub fn on_alarm(&mut self, rebuild_longlink: bool) -> u64 {
        self.on_alarm_at(mars_comm::tickcount::gettickcount(), rebuild_longlink)
    }

    /// The same, with the reading handed in.
    pub fn on_alarm_at(&mut self, now: u64, rebuild_longlink: bool) -> u64 {
        self.rebuild_longlink = rebuild_longlink;
        self.auto_interval_connect(now)
    }

    /// `OnHeartbeatAlarmSet(_interval)` — the C++ writes it to the Android
    /// system log and does nothing else, and the port has no system log.
    pub fn on_heartbeat_alarm_set(&mut self, _interval: u64) {}

    /// `OnHeartbeatAlarmReceived(_is_noop_timeout)` — the same.
    pub fn on_heartbeat_alarm_received(&mut self, _is_noop_timeout: bool) {}

    /// `__ReConnect()` — `fun_longlink_reset_()`, which is what the C++'s
    /// Apple-only paths call.
    pub fn reconnect(&mut self) {
        if let Some(reset) = self.longlink_reset.as_mut() {
            reset();
        }
    }

    /// `__IntervalConnect(_type)` — how long there is left to wait, in
    /// milliseconds, and `0` means the link was asked for now.
    fn interval_connect(&mut self, now: u64, kind: ConnectType) -> u64 {
        if self.is_svr_trig_off() {
            return 0;
        }
        let status = self.connect_status();
        if status == LongLinkStatus::Connecting || status == LongLinkStatus::Connected {
            return 0;
        }

        let interval = self.interval(now, kind).saturating_mul(1000);
        // `gettickcount() - longlink_.Profile().dns_time`
        let posttime = now.saturating_sub(self.dns_time());
        let buffer = if self.is_active() { 0 } else { INACTIVE_BUFFER };

        if self.is_active() || kind == ConnectType::NetworkChange {
            self.current_interval_index = 0;
            if posttime.saturating_add(buffer) >= interval {
                self.connect();
                return 0;
            }
            return interval.saturating_sub(posttime);
        }

        if self.rebuild_longlink {
            self.rebuild_longlink = false;
            self.connect();
            return 0;
        }
        self.rebuild_longlink = false;

        if posttime < UP_OR_DOWN_THRESHOLD {
            self.current_interval_index = (self.current_interval_index + 1).min(INTERNAL_MAX_INDEX);
        } else {
            self.current_interval_index = self.current_interval_index.saturating_sub(1);
        }
        let interval_final = RECONNECT_INTERVAL[self.current_interval_index]
            .saturating_mul(1000)
            .saturating_sub(posttime)
            .min(UP_OR_DOWN_THRESHOLD);
        if interval_final == 0 {
            self.connect();
            return 0;
        }
        interval_final
    }

    /// `__AutoIntervalConnect()` — cancel both alarms, ask how long there is
    /// left, and arm the rebuild one with it.
    fn auto_interval_connect(&mut self, now: u64) -> u64 {
        self.rebuild_due = None;
        self.wake_due = None;

        let remain = self.interval_connect(now, ConnectType::LongLink);
        if remain == 0 {
            return 0;
        }
        self.rebuild_due = Some(now.saturating_add(remain));
        remain
    }

    /// `__Interval(_type)` — in **seconds**, which is what [`INTERVALS`] is in.
    fn interval(&mut self, now: u64, kind: ConnectType) -> u64 {
        let state = self.cur_active_state(now);
        let mut interval = INTERVALS[kind as usize][state as usize];

        if kind != ConnectType::LongLink {
            return interval;
        }

        if state == ActiveState::Inactive || state == ActiveState::ForegroundActive {
            if !self.is_active() && !self.has_account() {
                interval = NO_ACCOUNT_INFO_INACTIVE_INTERVAL;
            } else if self.net_info() == NO_NET {
                interval = interval * NO_NET_SALT_RATE + NO_NET_SALT_RISE;
            } else if !self.has_account() {
                interval = interval * NO_ACCOUNT_INFO_SALT_RATE + NO_ACCOUNT_INFO_SALT_RISE;
            } else {
                interval += self.random(20) as u64;
            }
        }

        interval
    }

    /// `__CurActiveState(_activeLogic)`.
    fn cur_active_state(&mut self, now: u64) -> ActiveState {
        if !self.is_active() {
            return ActiveState::Inactive;
        }
        if !self.is_foreground() {
            return ActiveState::BackgroundActive;
        }
        let since = now.saturating_sub(self.last_foreground_change_time());
        if since >= 10 * 60 * 1000 {
            return ActiveState::ForegroundActive;
        }
        if since >= 60 * 1000 {
            return ActiveState::ForegroundTenMinute;
        }
        ActiveState::ForegroundOneMinute
    }

    /// `longlink_.MakeSureConnected(&newone)`.
    fn connect(&mut self) {
        if let Some(make_sure_connected) = self.make_sure_connected.as_mut() {
            let _ = make_sure_connected();
        }
    }

    fn is_svr_trig_off(&mut self) -> bool {
        self.is_svr_trig_off
            .as_mut()
            .is_some_and(|is_svr_trig_off| is_svr_trig_off())
    }

    fn connect_status(&mut self) -> LongLinkStatus {
        self.connect_status
            .as_mut()
            .map_or(LongLinkStatus::ConnectIdle, |connect_status| {
                connect_status()
            })
    }

    fn dns_time(&mut self) -> u64 {
        self.dns_time.as_mut().map_or(0, |dns_time| dns_time())
    }

    fn is_active(&mut self) -> bool {
        self.is_active.as_mut().is_some_and(|is_active| is_active())
    }

    fn is_foreground(&mut self) -> bool {
        self.is_foreground
            .as_mut()
            .is_some_and(|is_foreground| is_foreground())
    }

    fn last_foreground_change_time(&mut self) -> u64 {
        self.last_foreground_change_time
            .as_mut()
            .map_or(0, |last| last())
    }

    fn net_info(&mut self) -> i32 {
        self.net_info.as_mut().map_or(NO_NET, |net_info| net_info())
    }

    fn has_account(&mut self) -> bool {
        self.has_account
            .as_mut()
            .is_some_and(|has_account| has_account())
    }

    fn random(&mut self, bound: usize) -> usize {
        (self.random)(bound)
    }
}

impl Default for LongLinkConnectMonitor {
    fn default() -> Self {
        Self::new(false)
    }
}

impl std::fmt::Debug for LongLinkConnectMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LongLinkConnectMonitor")
            .field("status", &self.status)
            .field("last_connect_time", &self.last_connect_time)
            .field("last_connect_net_type", &self.last_connect_net_type)
            .field("current_interval_index", &self.current_interval_index)
            .field("rebuild_longlink", &self.rebuild_longlink)
            .field("is_keep_alive", &self.is_keep_alive)
            .field("rebuild_due", &self.rebuild_due)
            .field("wake_due", &self.wake_due)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A monitor whose long link is down, on a network, with an account, and
    /// whose app has been in the foreground since `0`.
    fn a_monitor() -> (LongLinkConnectMonitor, Arc<Mutex<Vec<&'static str>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut monitor = LongLinkConnectMonitor::new_at(0, true);
        monitor.set_connect_status(|| LongLinkStatus::DisConnected);
        monitor.set_net_info(|| 1);
        monitor.set_has_account(|| true);
        monitor.set_is_active(|| true);
        monitor.set_is_foreground(|| true);
        monitor.set_last_foreground_change_time(|| 0);
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
        (monitor, calls)
    }

    #[test]
    fn the_state_the_app_is_in_picks_the_column_of_the_table() {
        let mut monitor = LongLinkConnectMonitor::new_at(0, false);
        monitor.set_is_active(|| true);
        monitor.set_is_foreground(|| true);
        monitor.set_last_foreground_change_time(|| 0);
        monitor.set_net_info(|| 1);
        monitor.set_has_account(|| true);
        // `rand() % 20`, pinned
        monitor.set_random(|_bound| 19);

        // foreground for less than a minute: the first column
        assert_eq!(
            monitor.cur_active_state(0),
            ActiveState::ForegroundOneMinute
        );
        assert_eq!(
            monitor.interval(0, ConnectType::LongLink),
            INTERVALS[1][0],
            "the long-link row"
        );
        assert_eq!(monitor.interval(0, ConnectType::Task), INTERVALS[0][0]);

        // ... for more than one, and more than ten
        assert_eq!(
            monitor.cur_active_state(60_000),
            ActiveState::ForegroundTenMinute
        );
        assert_eq!(
            monitor.interval(60_000, ConnectType::LongLink),
            INTERVALS[1][1]
        );
        assert_eq!(
            monitor.cur_active_state(10 * 60 * 1000),
            ActiveState::ForegroundActive
        );
        // the two states the long-link row is salted in are this one and the
        // inactive one, and with a network and an account the salt is jitter
        assert_eq!(
            monitor.interval(10 * 60 * 1000, ConnectType::LongLink),
            INTERVALS[1][2] + 19
        );

        // active but in the background, and not active at all
        monitor.set_is_foreground(|| false);
        assert_eq!(monitor.cur_active_state(0), ActiveState::BackgroundActive);
        assert_eq!(monitor.interval(0, ConnectType::LongLink), INTERVALS[1][3]);
        monitor.set_is_active(|| false);
        assert_eq!(monitor.cur_active_state(0), ActiveState::Inactive);
        assert_eq!(
            monitor.interval(0, ConnectType::LongLink),
            INTERVALS[1][4] + 19
        );
    }

    #[test]
    fn what_the_interval_is_salted_with_depends_on_the_network_and_the_account() {
        let mut monitor = LongLinkConnectMonitor::new_at(0, false);
        monitor.set_is_active(|| false);
        monitor.set_is_foreground(|| false);
        monitor.set_random(|_bound| 19);

        // neither active nor logged in: a week
        assert_eq!(
            monitor.interval(0, ConnectType::LongLink),
            NO_ACCOUNT_INFO_INACTIVE_INTERVAL
        );

        // ... with an account, but no network at all
        monitor.set_has_account(|| true);
        assert_eq!(
            monitor.interval(0, ConnectType::LongLink),
            INTERVALS[1][4] * NO_NET_SALT_RATE + NO_NET_SALT_RISE
        );

        // ... and on a network, but with no account to connect for, which the
        // foreground-active state is the only one that reaches
        monitor.set_net_info(|| 1);
        monitor.set_has_account(|| false);
        monitor.set_is_active(|| true);
        monitor.set_is_foreground(|| true);
        assert_eq!(
            monitor.cur_active_state(10 * 60 * 1000),
            ActiveState::ForegroundActive
        );
        assert_eq!(
            monitor.interval(10 * 60 * 1000, ConnectType::LongLink),
            INTERVALS[1][2] * NO_ACCOUNT_INFO_SALT_RATE + NO_ACCOUNT_INFO_SALT_RISE
        );

        // a task connect and a network change are never salted
        assert_eq!(monitor.interval(0, ConnectType::Task), INTERVALS[0][0]);
        assert_eq!(monitor.interval(0, ConnectType::NetworkChange), 0);
    }

    #[test]
    fn an_active_app_that_waited_long_enough_connects_at_once() {
        let (mut monitor, calls) = a_monitor();
        // `sg_interval[kTaskConnect][kForgroundOneMinute]` is 5s, and the dns
        // was 6s ago
        monitor.set_dns_time(|| 0);
        assert_eq!(monitor.interval_connect(6_000, ConnectType::Task), 0);
        assert_eq!(
            *calls.lock().unwrap_or_else(|e| e.into_inner()),
            vec!["connect"]
        );

        // ... and one that has not waited long enough is told how long is left
        let (mut monitor, calls) = a_monitor();
        monitor.set_dns_time(|| 0);
        assert_eq!(monitor.interval_connect(1_000, ConnectType::Task), 4_000);
        assert!(calls.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
    }

    #[test]
    fn a_link_that_is_already_up_or_coming_up_is_not_asked_for() {
        let (mut monitor, calls) = a_monitor();
        monitor.set_connect_status(|| LongLinkStatus::Connecting);
        assert_eq!(monitor.interval_connect(0, ConnectType::Task), 0);
        monitor.set_connect_status(|| LongLinkStatus::Connected);
        assert_eq!(monitor.interval_connect(0, ConnectType::Task), 0);
        assert!(calls.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
    }

    #[test]
    fn in_the_background_the_wait_walks_the_ladder() {
        let (mut monitor, _calls) = a_monitor();
        monitor.set_is_active(|| false);
        monitor.set_is_foreground(|| false);
        monitor.set_dns_time(|| 0);

        // one rung up per call while the link keeps coming back inside ten
        // minutes: `reconnect_interval[1]` is a minute
        assert_eq!(monitor.interval_connect(0, ConnectType::LongLink), 60_000);
        assert_eq!(monitor.current_interval_index(), 1);
        // ... but the wait is capped at `kUpOrDownThreshold`
        monitor.current_interval_index = INTERNAL_MAX_INDEX;
        assert_eq!(
            monitor.interval_connect(0, ConnectType::LongLink),
            UP_OR_DOWN_THRESHOLD
        );
        assert_eq!(monitor.current_interval_index(), INTERNAL_MAX_INDEX);

        // a link that held for longer than the threshold walks back down — and
        // as the longest rung is the threshold itself, `reconnect_interval` of
        // the rung below minus what has already been spent is never positive,
        // so walking down is always a connect now
        monitor.set_dns_time(|| 0);
        monitor.current_interval_index = 3;
        assert_eq!(
            monitor.interval_connect(UP_OR_DOWN_THRESHOLD, ConnectType::LongLink),
            0,
            "`reconnect_interval[2]` is shorter than the wait already spent"
        );
        assert_eq!(monitor.current_interval_index(), 2);

        // ... and the ladder does not go below the first rung
        monitor.current_interval_index = 0;
        assert_eq!(
            monitor.interval_connect(UP_OR_DOWN_THRESHOLD, ConnectType::LongLink),
            0
        );
        assert_eq!(monitor.current_interval_index(), 0);
    }

    #[test]
    fn an_alarm_that_rebuilds_the_link_connects_at_once() {
        let (mut monitor, calls) = a_monitor();
        monitor.set_is_active(|| false);
        monitor.set_is_foreground(|| false);

        // the wake alarm: `rebuild_longlink` is false, so the ladder decides
        assert_eq!(monitor.on_alarm_at(0, false), 60_000);
        assert_eq!(monitor.rebuild_due_time(), Some(60_000));
        assert!(calls.lock().unwrap_or_else(|e| e.into_inner()).is_empty());

        // the rebuild alarm: it connects, and no alarm is armed
        assert_eq!(monitor.on_alarm_at(60_000, true), 0);
        assert_eq!(monitor.rebuild_due_time(), None);
        assert_eq!(
            *calls.lock().unwrap_or_else(|e| e.into_inner()),
            vec!["connect"]
        );
    }

    #[test]
    fn a_link_that_went_down_arms_the_wake_alarm() {
        let (mut monitor, _calls) = a_monitor();
        monitor.on_longlink_status_changed_at(1_000, LongLinkStatus::DisConnected);
        assert_eq!(monitor.status(), LongLinkStatus::DisConnected);
        assert_eq!(monitor.last_connect_time(), 1_000);
        assert_eq!(monitor.last_connect_net_type(), 1, "the net it is on");
        assert_eq!(monitor.wake_due_time(), Some(1_000 + WAKE_ALARM_INTERVAL));

        // a link that came up arms neither
        monitor.on_longlink_status_changed_at(2_000, LongLinkStatus::Connected);
        assert_eq!(monitor.wake_due_time(), None);
        assert_eq!(monitor.rebuild_due_time(), None);

        // ... and a failed connect does arm it
        monitor.on_longlink_status_changed_at(3_000, LongLinkStatus::ConnectFailed);
        assert_eq!(monitor.wake_due_time(), Some(3_000 + WAKE_ALARM_INTERVAL));
    }

    #[test]
    fn a_network_change_drops_the_link_and_asks_for_a_new_one() {
        let (mut monitor, calls) = a_monitor();
        monitor.set_dns_time(|| 0);
        // `sg_interval[kNetworkChangeConnect][…]` is `0`, so it connects at once
        assert!(monitor.network_change_at(0));
        assert_eq!(
            *calls.lock().unwrap_or_else(|e| e.into_inner()),
            vec!["disconnect", "connect"]
        );
    }

    #[test]
    fn network_change_answers_that_a_connect_was_asked_for_and_not_that_the_link_is_up() {
        let (mut monitor, _calls) = a_monitor();
        monitor.set_dns_time(|| 0);
        // the host says the link is down, and saying so is all the connect the
        // monitor asks for does here
        assert!(monitor.network_change_at(0), "the connect was asked for");
        assert!(!monitor.make_sure_connected_at(0), "and the link is not up");

        // a server that turned the trigger off asks for nothing at all, and the
        // answer is the same `true`
        let (mut monitor, calls) = a_monitor();
        monitor.set_is_svr_trig_off(|| true);
        assert!(monitor.network_change_at(0));
        assert_eq!(
            *calls.lock().unwrap_or_else(|e| e.into_inner()),
            vec!["disconnect"],
            "the link was dropped and nothing was asked for"
        );
    }

    #[test]
    fn make_sure_connected_answers_whether_the_link_is_up() {
        let (mut monitor, _calls) = a_monitor();
        monitor.set_dns_time(|| 0);
        assert!(!monitor.make_sure_connected_at(0), "still disconnected");

        // the callback the monitor asks now says the link is up
        monitor.set_connect_status(|| LongLinkStatus::Connected);
        assert!(monitor.make_sure_connected_at(0));

        // ... and a server that turned the trigger off is never asked
        monitor.set_is_svr_trig_off(|| true);
        monitor.set_connect_status(|| LongLinkStatus::DisConnected);
        assert!(!monitor.make_sure_connected_at(0));
        assert_eq!(monitor.interval_connect(0, ConnectType::Task), 0);
    }

    #[test]
    fn without_a_host_the_monitor_still_answers() {
        let mut monitor = LongLinkConnectMonitor::default();
        assert!(!monitor.is_keep_alive());
        assert_eq!(monitor.status(), LongLinkStatus::DisConnected);
        assert_eq!(monitor.last_connect_net_type(), NO_NET);
        assert!(format!("{monitor:?}").contains("LongLinkConnectMonitor"));

        // inactive and with no account, so the ladder decides — and the first
        // rung of it is a minute from now
        assert_eq!(monitor.on_alarm_at(0, false), 60_000);
        assert_eq!(monitor.rebuild_due_time(), Some(60_000));
        monitor.reconnect();
        monitor.on_heartbeat_alarm_set(1);
        monitor.on_heartbeat_alarm_received(true);
        monitor.on_foreground_changed_at(0, true);
        monitor.on_active_changed_at(0, true);
        monitor.on_longlink_status_changed(LongLinkStatus::Connected);
        // `sg_interval[kNetworkChangeConnect]` is all zeros, so it is now
        assert!(monitor.network_change_at(0));
    }

    #[test]
    fn the_reset_the_apple_paths_call_is_the_one_the_host_set() {
        let mut monitor = LongLinkConnectMonitor::new(false);
        let resets = Arc::new(Mutex::new(0));
        let record = Arc::clone(&resets);
        monitor.set_longlink_reset(move || {
            *record.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        });
        monitor.reconnect();
        assert_eq!(*resets.lock().unwrap_or_else(|e| e.into_inner()), 1);

        monitor.clear_longlink_reset();
        monitor.reconnect();
        assert_eq!(*resets.lock().unwrap_or_else(|e| e.into_inner()), 1);
    }
}
