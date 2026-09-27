//! `mars/stn/src/smart_heartbeat.cc` — the heartbeat interval the long link
//! settles on.
//!
//! A long link has to send a noop often enough to keep the NAT entry alive,
//! and not more often than that. `SmartHeartbeat` looks for the largest
//! interval that still answers, inside [`MIN_HEART_INTERVAL`]…
//! [`MAX_HEART_INTERVAL`]: it starts at the short end, and every
//! [`BASE_SUCC_COUNT`] answers on one interval it tries a
//! [`HEART_STEP`] bigger one; [`MAX_HEART_FAIL_COUNT`] failures on one
//! interval make it back off and settle there.
//!
//! Two things are different from the C++:
//!
//! * the record of one network (`NetHeartbeatInfo`) lives in a file in the
//!   C++ — `Heartbeat.ini`, read and written through `SpecialINI` — and there
//!   is no file here. [`SmartHeartbeat::on_longlink_established`] takes the
//!   network instead of looking it up, and the host keeps whatever it wants
//!   to keep: [`SmartHeartbeat::info`] is what the C++ would have loaded and
//!   [`SmartHeartbeat::info_mut`] is what it would have saved.
//! * `ActiveLogic` and `getNetInfo()` are app and platform state, so they are
//!   arguments ([`SmartHeartbeat::get_next_heartbeat_interval`],
//!   [`SmartHeartbeat::judge_doze_style`]). `time(NULL)` is one too
//!   ([`SmartHeartbeat::on_heart_result`]), which is what makes the
//!   "a week has passed, probe a bigger interval" case testable.
//!   `::isNetworkConnected()` is a query of its own
//!   ([`SmartHeartbeat::set_is_network_connected`]), because it is asked on
//!   the way *in* and not by the method the host calls to get an interval.
//!
//! Everything else — the order of the tests, the counters, the window MIUI's
//! alarm alignment is judged in — is the C++'.

use std::sync::atomic::{AtomicI32, Ordering};

use crate::config::{
    BASE_SUCC_COUNT, DOZE_JUDGE_WINDOW, HEART_STEP, MAX_HEART_FAIL_COUNT, MAX_HEART_INTERVAL,
    MIN_HEART_INTERVAL, NET_STABLE_TEST_COUNT, PROBE_BIGGER_HEART_AGE, SUCCESS_STEP,
};

/// `kNoNet` — `NetHeartbeatInfo::Clear()` leaves `net_type_` at this.
pub const NO_NET: i32 = -1;

/// How many failures on the short interval before the C++ reports a network
/// that cannot even keep the minimum heartbeat — its literal `6`.
pub const BAD_NETWORK_FAIL_COUNT: u32 = 6;

/// `TSmartHeartBeatType` — how the interval that is in force was arrived at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SmartHeartBeatType {
    /// `kNoSmartHeartBeat` — nothing has been computed yet.
    #[default]
    NoSmartHeartBeat = 0,
    /// `kSmartHeartBeat` — computed.
    SmartHeartBeat,
    /// `kDozeModeHeart` — computed, on a network that looks like it dozes.
    DozeModeHeart,
}

/// `TSmartHeartBeatAction` — what the report callback is told happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartHeartBeatAction {
    /// `kActionCalcEnd` — an interval was found.
    CalcEnd = 0,
    /// `kActionReCalc` — the one in force was given up on.
    ReCalc = 1,
    /// `kActionDisconnect` — the long link died on a stable interval.
    Disconnect = 2,
    /// `kActionBadNetwork` — `BAD_NETWORK_FAIL_COUNT` failures on the minimum.
    BadNetwork = 3,
}

/// `NetHeartbeatInfo` — what is remembered about one network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetHeartbeatInfo {
    /// `net_detail_` — the network the record is for; empty is "no network".
    pub net_detail: String,
    /// `net_type_`.
    pub net_type: i32,
    /// `cur_heart_` — the interval in force, in milliseconds.
    pub cur_heart: u32,
    /// `heart_type_`.
    pub heart_type: SmartHeartBeatType,
    /// `is_stable_` — whether `cur_heart` is one that was settled on.
    pub is_stable: bool,
    /// `last_modify_time_` — when the record last changed, in seconds.
    pub last_modify_time: i64,
    /// `fail_heart_count_` — failures on `cur_heart`.
    pub fail_heart_count: u32,
    /// `succ_heart_count_` — successes on `cur_heart`.
    pub succ_heart_count: u32,
    /// `min_heart_fail_count_` — failures on [`MIN_HEART_INTERVAL`].
    pub min_heart_fail_count: u32,
}

impl Default for NetHeartbeatInfo {
    fn default() -> Self {
        Self::new()
    }
}

impl NetHeartbeatInfo {
    /// `NetHeartbeatInfo::NetHeartbeatInfo()` — `Clear()`.
    pub fn new() -> Self {
        Self {
            net_detail: String::new(),
            net_type: NO_NET,
            cur_heart: MIN_HEART_INTERVAL,
            heart_type: SmartHeartBeatType::NoSmartHeartBeat,
            is_stable: false,
            last_modify_time: 0,
            fail_heart_count: 0,
            succ_heart_count: 0,
            min_heart_fail_count: 0,
        }
    }

    /// `NetHeartbeatInfo::Clear()`.
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// Whether this record is for a network at all.
    pub fn has_network(&self) -> bool {
        !self.net_detail.is_empty()
    }
}

/// `outer_setted_heart_` — what `SetHeartBeat` set; `-1` is "not set".
static OUTER_SETTED_HEART: AtomicI32 = AtomicI32::new(-1);

/// `SmartHeartbeat::SetHeartBeat(heart)` — an interval from outside, which
/// wins over anything computed.
///
/// `0` is an interval like any other, not "no override": the C++ keeps
/// `outer_setted_heart_` as an `int` that starts at `-1` and honours it while
/// `outer_setted_heart_ != -1 && outer_setted_heart_ >= 0`, and
/// `Java_com_tencent_mars_stn_StnLogic_trigNooping` is `SetHeartBeat(0)` — so
/// a noop triggered from Java really does leave the interval at `0` until
/// somebody sets it again. `-1` is what clears it.
pub fn set_heartbeat(heart: i32) {
    OUTER_SETTED_HEART.store(heart, Ordering::SeqCst);
}

/// The interval [`set_heartbeat`] set, `-1` when there is none.
pub fn outer_setted_heart() -> i32 {
    OUTER_SETTED_HEART.load(Ordering::SeqCst)
}

/// `report_smart_heart_` — who is told what the computation did: the action,
/// the record it acted on, and whether the heartbeat timed out.
pub type ReportSmartHeart = dyn FnMut(SmartHeartBeatAction, &NetHeartbeatInfo, bool) + Send;

/// `::isNetworkConnected()` — whether the device has a network at all, which
/// is what the C++ asks before it calls one *bad*: heartbeats that went
/// unanswered while the device is offline say nothing about the network.
pub type IsNetworkConnected = dyn FnMut() -> bool + Send;

/// `SmartHeartbeat`.
pub struct SmartHeartbeat {
    report: Option<Box<ReportSmartHeart>>,
    is_network_connected: Option<Box<IsNetworkConnected>>,
    is_wait_heart_response: bool,
    /// `success_heart_count_` — heartbeats that answered on this TCP, whatever
    /// the interval was.
    success_heart_count: u32,
    last_heart: u32,
    pre_heart: u32,
    cur_heart: u32,
    info: NetHeartbeatInfo,
    doze_mode_count: i32,
    normal_mode_count: i32,
    /// `noop_start_tick_` — `None` is an invalid `tickcount_t`.
    noop_start_tick: Option<u64>,
}

impl Default for SmartHeartbeat {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SmartHeartbeat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmartHeartbeat")
            .field("has_report", &self.report.is_some())
            .field("success_heart_count", &self.success_heart_count)
            .field("last_heart", &self.last_heart)
            .field("cur_heart", &self.cur_heart)
            .field("info", &self.info)
            .finish()
    }
}

impl SmartHeartbeat {
    /// `SmartHeartbeat(context)` — the C++ parses `Heartbeat.ini` here; the
    /// port starts from the defaults.
    pub fn new() -> Self {
        Self {
            report: None,
            is_network_connected: None,
            is_wait_heart_response: false,
            success_heart_count: 0,
            last_heart: MIN_HEART_INTERVAL,
            pre_heart: MIN_HEART_INTERVAL,
            cur_heart: MIN_HEART_INTERVAL,
            info: NetHeartbeatInfo::new(),
            doze_mode_count: 0,
            normal_mode_count: 0,
            noop_start_tick: None,
        }
    }

    /// `report_smart_heart_` — who is told what the computation did.
    pub fn set_report(
        &mut self,
        report: impl FnMut(SmartHeartBeatAction, &NetHeartbeatInfo, bool) + Send + 'static,
    ) {
        self.report = Some(Box::new(report));
    }

    /// `report_smart_heart_ = NULL`.
    pub fn clear_report(&mut self) {
        self.report = None;
    }

    /// `::isNetworkConnected()` — unset answers `true`: the port has no
    /// platform to ask, and a host that hands nothing in keeps the report the
    /// C++ gates on this question.
    pub fn set_is_network_connected(
        &mut self,
        is_network_connected: impl FnMut() -> bool + Send + 'static,
    ) {
        self.is_network_connected = Some(Box::new(is_network_connected));
    }

    /// `::isNetworkConnected()` answered by nobody again.
    pub fn clear_is_network_connected(&mut self) {
        self.is_network_connected = None;
    }

    /// The record of the network the long link is on: what the C++ would have
    /// read out of `Heartbeat.ini`.
    pub fn info(&self) -> &NetHeartbeatInfo {
        &self.info
    }

    /// The same record, mutable — the way a host restores one it kept, which
    /// is what `__LoadINI` does for the C++.
    pub fn info_mut(&mut self) -> &mut NetHeartbeatInfo {
        &mut self.info
    }

    /// `last_heart_` — the interval the last heartbeat went out with.
    pub fn last_heart(&self) -> u32 {
        self.last_heart
    }

    /// `cur_heart_`.
    pub fn cur_heart(&self) -> u32 {
        self.cur_heart
    }

    /// `success_heart_count_`.
    pub fn success_heart_count(&self) -> u32 {
        self.success_heart_count
    }

    /// `OnHeartbeatStart()` — a noop went out at `now`, and its answer is what
    /// the next `on_heart_result` is about.
    pub fn on_heartbeat_start(&mut self, now: u64) {
        self.noop_start_tick = Some(now);
        self.is_wait_heart_response = true;
    }

    /// `OnLongLinkEstablished()` — a new TCP: the network is looked at again
    /// (a different one starts from scratch) and the computation restarts from
    /// the short end.
    pub fn on_longlink_established(&mut self, net_detail: &str, net_type: i32) {
        if net_detail.is_empty() {
            self.info.clear();
        } else if net_detail != self.info.net_detail {
            self.info.clear();
            self.info.net_detail = net_detail.to_owned();
            self.info.net_type = net_type;
        }
        self.success_heart_count = 0;
        self.pre_heart = MIN_HEART_INTERVAL;
        self.cur_heart = MIN_HEART_INTERVAL;
    }

    /// `OnLongLinkDisconnect()` — the TCP is gone, which counts as a heartbeat
    /// that never answered.
    pub fn on_longlink_disconnect(&mut self, now: i64) {
        self.on_heart_result(false, false, now);
        self.info.succ_heart_count = 0;

        // a network that never settled keeps whatever it had probed
        if !self.info.is_stable {
            return;
        }
        self.last_heart = MIN_HEART_INTERVAL;
    }

    /// `OnHeartResult(success, fail_of_timeout)` — one heartbeat answered (or
    /// did not), at `now` seconds.
    ///
    /// A result nobody is waiting for is ignored, which is why a heartbeat
    /// only counts between [`SmartHeartbeat::on_heartbeat_start`] and its
    /// answer.
    pub fn on_heart_result(&mut self, success: bool, fail_of_timeout: bool, now: i64) {
        if !self.is_wait_heart_response {
            return;
        }

        if self.report.is_some()
            && !success
            && self.success_heart_count >= NET_STABLE_TEST_COUNT
            && self.info.is_stable
        {
            self.report(SmartHeartBeatAction::Disconnect, fail_of_timeout);
        }

        self.pre_heart = self.cur_heart;
        self.cur_heart = self.last_heart;
        self.is_wait_heart_response = false;

        if !self.info.has_network() {
            return;
        }

        if success {
            self.success_heart_count += 1;
        }

        // the first few heartbeats only decide whether the network can hold
        // the minimum at all
        if self.success_heart_count <= NET_STABLE_TEST_COUNT {
            self.info.min_heart_fail_count = if success {
                0
            } else {
                self.info.min_heart_fail_count + 1
            };
            // `report_smart_heart_ && min_heart_fail_count_ >= 6 &&
            // ::isNetworkConnected()`, in the C++'s order — and the count is
            // reset *inside* that branch: with nobody to report to, or with a
            // device that has no network to be bad, the C++ leaves it where it
            // is. The port used to reset it either way.
            if self.report.is_some()
                && self.info.min_heart_fail_count >= BAD_NETWORK_FAIL_COUNT
                && self.is_network_connected()
            {
                self.report(SmartHeartBeatAction::BadNetwork, false);
                self.info.min_heart_fail_count = 0;
            }
            return;
        }

        if self.last_heart != self.info.cur_heart {
            return;
        }

        if success {
            if self.last_heart == self.pre_heart {
                self.info.succ_heart_count += 1;
                self.info.fail_heart_count = 0;
            }
        } else {
            if fail_of_timeout {
                self.info.succ_heart_count = 0;
            }
            self.info.fail_heart_count += 1;
        }

        if success && self.info.is_stable {
            // the biggest interval there is: nothing left to try
            if self.info.cur_heart >= MAX_HEART_INTERVAL - SUCCESS_STEP {
                return;
            }
            // a record that has not moved for a week is worth one more probe
            if now - self.info.last_modify_time >= PROBE_BIGGER_HEART_AGE
                && self.info.cur_heart < MAX_HEART_INTERVAL - SUCCESS_STEP
            {
                self.info.cur_heart += SUCCESS_STEP;
                self.info.succ_heart_count = 0;
                self.info.is_stable = false;
                self.info.fail_heart_count = 0;
                self.report(SmartHeartBeatAction::ReCalc, false);
                self.save(now);
            }
            return;
        }

        if success {
            if self.info.succ_heart_count >= BASE_SUCC_COUNT {
                if self.info.cur_heart >= MAX_HEART_INTERVAL - SUCCESS_STEP {
                    self.info.cur_heart = MAX_HEART_INTERVAL - SUCCESS_STEP;
                    self.info.succ_heart_count = 0;
                    self.info.is_stable = true;
                    self.info.heart_type = self.heart_type();
                    self.report(SmartHeartBeatAction::CalcEnd, false);
                } else {
                    self.info.succ_heart_count = 0;
                    // a network that dozes is not probed step by step: the
                    // only interval that works on one is the biggest
                    self.info.cur_heart = if self.is_doze_style() {
                        MAX_HEART_INTERVAL - SUCCESS_STEP
                    } else {
                        std::cmp::min(
                            MAX_HEART_INTERVAL - SUCCESS_STEP,
                            self.info.cur_heart + HEART_STEP,
                        )
                    };
                }
            }
        } else {
            // a failure on the minimum says nothing about any bigger interval
            if self.last_heart == MIN_HEART_INTERVAL {
                return;
            }

            if self.info.fail_heart_count >= MAX_HEART_FAIL_COUNT {
                if self.info.is_stable {
                    self.info.cur_heart = MIN_HEART_INTERVAL;
                    self.info.succ_heart_count = 0;
                    self.info.is_stable = false;
                    self.report(SmartHeartBeatAction::ReCalc, true);
                    self.info.fail_heart_count = 0;
                } else {
                    self.info.cur_heart = if self.is_doze_style() {
                        MIN_HEART_INTERVAL
                    } else if self.info.cur_heart - HEART_STEP - SUCCESS_STEP > MIN_HEART_INTERVAL {
                        self.info.cur_heart - HEART_STEP - SUCCESS_STEP
                    } else {
                        MIN_HEART_INTERVAL
                    };
                    self.info.succ_heart_count = 0;
                    self.info.fail_heart_count = 0;
                    self.info.is_stable = true;
                    self.info.heart_type = self.heart_type();
                    self.report(SmartHeartBeatAction::CalcEnd, false);
                }
            }
        }

        // `__DumpHeartInfo()` and `__SaveINI()`
        self.save(now);
    }

    /// `__SaveINI()` without the file: what the C++ leaves behind is the
    /// `modifyTime` it stamps and the record itself, and the record is
    /// [`SmartHeartbeat::info`] — so this is the stamp, and it is the host
    /// that keeps it.
    fn save(&mut self, now: i64) {
        if self.info.has_network() {
            self.info.last_modify_time = now;
        }
    }

    /// `GetNextHeartbeatInterval()` — the interval the next noop goes out
    /// with. `is_active` is `ActiveLogic::Instance()->IsActive()`.
    pub fn get_next_heartbeat_interval(&mut self, is_active: bool) -> u32 {
        // an interval set from outside wins over anything computed
        let outer = outer_setted_heart();
        if outer >= 0 {
            self.last_heart = outer as u32;
            return self.last_heart;
        }

        // while the app is in the foreground there is nothing to save
        if is_active {
            self.last_heart = MIN_HEART_INTERVAL;
            return MIN_HEART_INTERVAL;
        }

        if self.success_heart_count < NET_STABLE_TEST_COUNT || !self.info.has_network() {
            self.last_heart = MIN_HEART_INTERVAL;
            return MIN_HEART_INTERVAL;
        }

        self.last_heart = self.info.cur_heart;

        // a network that started dozing cannot be trusted with the interval
        // that was computed before it did
        if self.is_doze_style()
            && self.info.heart_type != SmartHeartBeatType::DozeModeHeart
            && self.last_heart != MAX_HEART_INTERVAL - SUCCESS_STEP
        {
            self.info.cur_heart = MIN_HEART_INTERVAL;
            self.last_heart = MIN_HEART_INTERVAL;
        }

        // and one outside the range is replaced by the short end
        if self.last_heart >= MAX_HEART_INTERVAL || self.last_heart < MIN_HEART_INTERVAL {
            self.info.cur_heart = MIN_HEART_INTERVAL;
            self.last_heart = MIN_HEART_INTERVAL;
        }

        self.last_heart
    }

    /// `JudgeDozeStyle()` — whether the network delivers the noop answer when
    /// it is due: MIUI hands alarms out on five-minute marks, so a heartbeat
    /// that came back [`DOZE_JUDGE_WINDOW`] away from its interval is a
    /// network that was asleep.
    ///
    /// `is_mobile` is `kMobile == getNetInfo()` and `is_active` is
    /// `ActiveLogic::Instance()->IsActive()`. `now` is the tick count, and it
    /// is only judged against a heartbeat that was actually started
    /// ([`SmartHeartbeat::on_heartbeat_start`]); the tick is spent on it, so
    /// the next judgement needs a new one.
    pub fn judge_doze_style(&mut self, now: u64, is_mobile: bool, is_active: bool) {
        if is_active {
            return;
        }
        let Some(started) = self.noop_start_tick else {
            return;
        };
        if !is_mobile {
            return;
        }

        let span = now.saturating_sub(started);
        if span.abs_diff(self.last_heart as u64) >= DOZE_JUDGE_WINDOW as u64 {
            self.doze_mode_count += 1;
            self.normal_mode_count = std::cmp::max(self.normal_mode_count - 1, 0);
        } else {
            self.normal_mode_count += 1;
            self.doze_mode_count = std::cmp::max(self.doze_mode_count - 1, 0);
        }
        self.noop_start_tick = None;
    }

    /// Whether the network looks like it dozes.
    pub fn is_doze_style(&self) -> bool {
        self.doze_mode_count >= 2 && self.doze_mode_count > 2 * self.normal_mode_count
    }

    /// `heart_type_` for a network that just settled.
    fn heart_type(&self) -> SmartHeartBeatType {
        if self.is_doze_style() {
            SmartHeartBeatType::DozeModeHeart
        } else {
            SmartHeartBeatType::SmartHeartBeat
        }
    }

    fn report(&mut self, action: SmartHeartBeatAction, fail_of_timeout: bool) {
        if let Some(report) = self.report.as_mut() {
            report(action, &self.info, fail_of_timeout);
        }
    }

    fn is_network_connected(&mut self) -> bool {
        self.is_network_connected
            .as_mut()
            .is_none_or(|is_network_connected| is_network_connected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `MinHeartInterval` is where every computation starts, so most of these
    /// have to get past the `NetStableTestCount` heartbeats first.
    fn established(hb: &mut SmartHeartbeat) {
        hb.on_longlink_established("wifi-home", 1);
    }

    /// `NetStableTestCount` heartbeats that answered, and one that settles the
    /// count: `success_heart_count` has to be *above* the threshold.
    fn stable(hb: &mut SmartHeartbeat) {
        for _ in 0..=NET_STABLE_TEST_COUNT {
            hb.on_heartbeat_start(0);
            hb.on_heart_result(true, false, 0);
        }
    }

    /// `outer_setted_heart_` is one value for the whole process, and
    /// [`with_outer_heart`] holds the crate's turn while it moves it — so a test
    /// that *reads* it has to take the same turn, or it reads the interval
    /// another test set. These tests run in parallel.
    fn next_interval(hb: &mut SmartHeartbeat, is_active: bool) -> u32 {
        let _guard = crate::test_lock();
        hb.get_next_heartbeat_interval(is_active)
    }

    /// One heartbeat the way the long link drives it: it asks what interval to
    /// wait, sends the noop, and reports the answer at `now`.
    fn heartbeat_at(hb: &mut SmartHeartbeat, success: bool, now: i64) {
        next_interval(hb, false);
        hb.on_heartbeat_start(0);
        hb.on_heart_result(success, false, now);
    }

    fn heartbeat(hb: &mut SmartHeartbeat, success: bool) {
        heartbeat_at(hb, success, 0)
    }

    #[test]
    fn a_new_heartbeat_starts_at_the_short_end() {
        let mut hb = SmartHeartbeat::new();
        assert_eq!(hb.last_heart(), MIN_HEART_INTERVAL);
        assert_eq!(hb.cur_heart(), MIN_HEART_INTERVAL);
        // no network yet, and no heartbeat to judge
        assert!(!hb.info().has_network());
        assert!(!hb.is_doze_style());
        assert_eq!(next_interval(&mut hb, false), MIN_HEART_INTERVAL);
    }

    /// `outer_setted_heart_` is process-wide, so the test that moves it takes
    /// the crate's turn: [`crate::test_lock`].
    fn with_outer_heart<R>(heart: i32, f: impl FnOnce() -> R) -> R {
        let guard = crate::test_lock();
        let previous = outer_setted_heart();
        set_heartbeat(heart);
        let result = f();
        set_heartbeat(previous);
        drop(guard);
        result
    }

    #[test]
    fn an_interval_from_outside_wins() {
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);
        with_outer_heart(120_000, || {
            assert_eq!(hb.get_next_heartbeat_interval(false), 120_000);
            assert_eq!(hb.last_heart(), 120_000);
        });

        // `SetHeartBeat` also resets the noop interval of the C++ (`0`)
        with_outer_heart(-1, || {
            assert_eq!(hb.get_next_heartbeat_interval(false), MIN_HEART_INTERVAL);
        });
    }

    /// `TrigNooping` is `SmartHeartbeat::SetHeartBeat(0)` in the C++, and the
    /// C++ honours a `0`: `outer_setted_heart_ != -1 && >= 0`. It is `-1`, not
    /// `0`, that means "no interval from outside".
    #[test]
    fn a_zero_from_outside_is_an_interval_like_any_other() {
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);
        stable(&mut hb);
        with_outer_heart(0, || {
            assert_eq!(outer_setted_heart(), 0);
            assert_eq!(hb.get_next_heartbeat_interval(false), 0);
            assert_eq!(hb.last_heart(), 0, "a computed interval loses to it");
        });
    }

    #[test]
    fn an_active_app_always_gets_the_short_end() {
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);
        stable(&mut hb);
        // even a computed interval is dropped while the app is in front
        hb.info_mut().cur_heart = MAX_HEART_INTERVAL - SUCCESS_STEP;
        assert_eq!(next_interval(&mut hb, true), MIN_HEART_INTERVAL);
        assert_eq!(next_interval(&mut hb, false), hb.info().cur_heart);
    }

    #[test]
    fn the_first_heartbeats_only_test_the_minimum() {
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);

        // two answers, then a failure: the minimum is what failed
        heartbeat(&mut hb, true);
        heartbeat(&mut hb, true);
        heartbeat(&mut hb, false);
        assert_eq!(hb.info().min_heart_fail_count, 1);
        heartbeat(&mut hb, true);
        assert_eq!(hb.info().min_heart_fail_count, 0, "a success resets it");
        assert_eq!(hb.info().cur_heart, MIN_HEART_INTERVAL);
    }

    #[test]
    fn a_network_that_cannot_hold_the_minimum_is_reported() {
        let mut hb = SmartHeartbeat::new();
        let mut actions = Vec::new();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&seen);
        hb.set_report(move |action, info, timeout| {
            sink.lock().unwrap().push((action, info.cur_heart, timeout));
        });
        established(&mut hb);

        for _ in 0..BAD_NETWORK_FAIL_COUNT {
            heartbeat(&mut hb, false);
        }
        actions.extend(seen.lock().unwrap().iter().copied());
        assert_eq!(
            actions,
            vec![(SmartHeartBeatAction::BadNetwork, MIN_HEART_INTERVAL, false)],
            "one bad-network report, and the counter is reset after it"
        );
        assert_eq!(hb.info().min_heart_fail_count, 0);
    }

    /// A network nobody is told about is not reported, and the count the C++
    /// only resets *inside* that branch is left where it is.
    #[test]
    fn the_bad_network_report_is_the_two_questions_the_c_plus_plus_asks() {
        // nobody listening: `report_smart_heart_ == NULL`, so the C++ neither
        // reports nor resets `min_heart_fail_count_`
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);
        for _ in 0..BAD_NETWORK_FAIL_COUNT {
            heartbeat(&mut hb, false);
        }
        assert_eq!(
            hb.info().min_heart_fail_count,
            BAD_NETWORK_FAIL_COUNT,
            "the C++ leaves the count where it is when it does not report"
        );

        // ... and a listener, on a device with no network at all:
        // `::isNetworkConnected()` is false, so six heartbeats that went
        // unanswered say nothing about the network
        let mut hb = SmartHeartbeat::new();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&seen);
        hb.set_report(move |action, _info, _timeout| {
            sink.lock().unwrap().push(action);
        });
        hb.set_is_network_connected(|| false);
        established(&mut hb);
        for _ in 0..BAD_NETWORK_FAIL_COUNT {
            heartbeat(&mut hb, false);
        }
        assert!(
            seen.lock().unwrap().is_empty(),
            "a device with no network is not a bad network"
        );
        assert_eq!(hb.info().min_heart_fail_count, BAD_NETWORK_FAIL_COUNT);

        // ... and once the device is back on one, the report is the one the
        // port has always made
        hb.set_is_network_connected(|| true);
        heartbeat(&mut hb, false);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![SmartHeartBeatAction::BadNetwork]
        );
        assert_eq!(hb.info().min_heart_fail_count, 0, "reset after reporting");
    }

    #[test]
    fn successes_push_the_interval_up_a_step_at_a_time() {
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);
        stable(&mut hb);

        // `BaseSuccCount` answers on one interval make the next one bigger
        for _ in 0..BASE_SUCC_COUNT {
            heartbeat(&mut hb, true);
        }
        assert_eq!(hb.info().cur_heart, MIN_HEART_INTERVAL + HEART_STEP);
        assert!(!hb.info().is_stable, "still looking");
    }

    #[test]
    fn the_biggest_interval_is_where_it_settles() {
        let mut hb = SmartHeartbeat::new();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&seen);
        hb.set_report(move |action, info, _| sink.lock().unwrap().push((action, info.cur_heart)));
        established(&mut hb);
        stable(&mut hb);
        hb.info_mut().cur_heart = MAX_HEART_INTERVAL - SUCCESS_STEP;

        for _ in 0..BASE_SUCC_COUNT {
            heartbeat(&mut hb, true);
        }
        assert_eq!(hb.info().cur_heart, MAX_HEART_INTERVAL - SUCCESS_STEP);
        assert!(hb.info().is_stable);
        assert_eq!(hb.info().heart_type, SmartHeartBeatType::SmartHeartBeat);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![(
                SmartHeartBeatAction::CalcEnd,
                MAX_HEART_INTERVAL - SUCCESS_STEP
            )]
        );

        // and a stable interval at the ceiling is never probed again
        let before = seen.lock().unwrap().len();
        heartbeat(&mut hb, true);
        assert_eq!(seen.lock().unwrap().len(), before);
    }

    #[test]
    fn failures_pull_the_interval_back_down() {
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);
        stable(&mut hb);
        hb.info_mut().cur_heart = MIN_HEART_INTERVAL + 2 * HEART_STEP;
        hb.info_mut().is_stable = false;

        // `MaxHeartFailCount` failures on one interval
        for _ in 0..MAX_HEART_FAIL_COUNT {
            heartbeat(&mut hb, false);
        }
        assert_eq!(
            hb.info().cur_heart,
            MIN_HEART_INTERVAL + HEART_STEP - SUCCESS_STEP,
            "a step and a margin down"
        );
        assert!(hb.info().is_stable, "the backoff settles the network");
        assert_eq!(hb.info().heart_type, SmartHeartBeatType::SmartHeartBeat);
    }

    #[test]
    fn a_failure_on_the_minimum_says_nothing() {
        let mut hb = SmartHeartbeat::new();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&seen);
        hb.set_report(move |action, _, _| sink.lock().unwrap().push(action));
        established(&mut hb);
        stable(&mut hb);

        // the heartbeat went out on the minimum, so a failure says nothing
        // about any bigger interval: nothing is settled, nothing recomputed
        heartbeat(&mut hb, false);
        assert_eq!(hb.info().cur_heart, MIN_HEART_INTERVAL);
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_stable_interval_that_stops_answering_is_given_up_on() {
        let mut hb = SmartHeartbeat::new();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&seen);
        hb.set_report(move |action, info, timeout| {
            sink.lock().unwrap().push((action, info.cur_heart, timeout));
        });
        established(&mut hb);
        stable(&mut hb);
        hb.info_mut().cur_heart = MIN_HEART_INTERVAL + HEART_STEP;
        hb.info_mut().is_stable = true;

        // a stable network that stops answering is reported once per heartbeat
        // that fails, and then given up on
        for _ in 0..MAX_HEART_FAIL_COUNT {
            heartbeat(&mut hb, false);
        }
        assert_eq!(hb.info().cur_heart, MIN_HEART_INTERVAL);
        assert!(!hb.info().is_stable);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                (
                    SmartHeartBeatAction::Disconnect,
                    MIN_HEART_INTERVAL + HEART_STEP,
                    false
                ),
                (
                    SmartHeartBeatAction::Disconnect,
                    MIN_HEART_INTERVAL + HEART_STEP,
                    false
                ),
                (SmartHeartBeatAction::ReCalc, MIN_HEART_INTERVAL, true),
            ]
        );
    }

    #[test]
    fn a_record_that_has_not_moved_for_a_week_is_probed_again() {
        let mut hb = SmartHeartbeat::new();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&seen);
        hb.set_report(move |action, info, _| sink.lock().unwrap().push((action, info.cur_heart)));
        established(&mut hb);
        stable(&mut hb);
        hb.info_mut().cur_heart = MIN_HEART_INTERVAL + HEART_STEP;
        hb.info_mut().is_stable = true;
        hb.info_mut().last_modify_time = 0;

        heartbeat_at(&mut hb, true, PROBE_BIGGER_HEART_AGE);
        assert_eq!(
            hb.info().cur_heart,
            MIN_HEART_INTERVAL + HEART_STEP + SUCCESS_STEP,
            "one margin bigger"
        );
        assert!(!hb.info().is_stable, "and it is computed again");
        assert_eq!(
            *seen.lock().unwrap(),
            vec![(
                SmartHeartBeatAction::ReCalc,
                MIN_HEART_INTERVAL + HEART_STEP + SUCCESS_STEP
            )]
        );

        // a record that moved today is left alone
        let before = hb.info().cur_heart;
        hb.info_mut().is_stable = true;
        hb.info_mut().last_modify_time = PROBE_BIGGER_HEART_AGE;
        heartbeat(&mut hb, true);
        assert_eq!(hb.info().cur_heart, before);
    }

    #[test]
    fn a_result_nobody_is_waiting_for_is_ignored() {
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);
        hb.on_heart_result(true, false, 0);
        assert_eq!(hb.success_heart_count(), 0);
        assert_eq!(hb.last_heart(), MIN_HEART_INTERVAL);
    }

    #[test]
    fn a_heartbeat_that_does_not_match_the_interval_is_ignored() {
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);
        stable(&mut hb);

        // the record has moved on, but the heartbeat that is being answered
        // went out on the minimum: the result says nothing about either
        let before = hb.info().succ_heart_count;
        hb.info_mut().cur_heart = MIN_HEART_INTERVAL + HEART_STEP;
        hb.on_heartbeat_start(0);
        hb.on_heart_result(true, false, 0);
        assert_eq!(hb.info().succ_heart_count, before);
    }

    #[test]
    fn a_disconnect_is_a_heartbeat_that_never_answered() {
        let mut hb = SmartHeartbeat::new();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&seen);
        hb.set_report(move |action, _, timeout| sink.lock().unwrap().push((action, timeout)));
        established(&mut hb);
        stable(&mut hb);
        hb.info_mut().is_stable = true;

        hb.on_heartbeat_start(0);
        hb.on_longlink_disconnect(0);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![(SmartHeartBeatAction::Disconnect, false)]
        );
        assert_eq!(hb.last_heart(), MIN_HEART_INTERVAL);

        // a network that never settled keeps its interval
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);
        stable(&mut hb);
        hb.info_mut().is_stable = false;
        hb.info_mut().cur_heart = MIN_HEART_INTERVAL + HEART_STEP;
        assert_eq!(
            next_interval(&mut hb, false),
            MIN_HEART_INTERVAL + HEART_STEP
        );
        hb.on_longlink_disconnect(0);
        assert_ne!(hb.last_heart(), MIN_HEART_INTERVAL);
    }

    #[test]
    fn a_new_network_starts_from_scratch_and_the_same_one_does_not() {
        let mut hb = SmartHeartbeat::new();
        hb.on_longlink_established("wifi-home", 1);
        hb.info_mut().cur_heart = MIN_HEART_INTERVAL + HEART_STEP;
        stable(&mut hb);

        // same network: the record stays
        hb.on_longlink_established("wifi-home", 1);
        assert_eq!(hb.info().cur_heart, MIN_HEART_INTERVAL + HEART_STEP);
        assert_eq!(hb.success_heart_count(), 0, "but the count restarts");

        // a different one: back to the defaults
        hb.on_longlink_established("4g", 2);
        assert_eq!(hb.info().cur_heart, MIN_HEART_INTERVAL);
        assert_eq!(hb.info().net_type, 2);

        // and no network at all
        hb.on_longlink_established("", NO_NET);
        assert!(!hb.info().has_network());
        assert_eq!(hb.info().net_type, NO_NET);

        // with no network nothing is computed
        heartbeat(&mut hb, true);
        assert_eq!(hb.info().cur_heart, MIN_HEART_INTERVAL);
    }

    #[test]
    fn a_network_that_delivers_late_looks_like_it_dozes() {
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);

        // a mobile network, in the background, that answered the noop far from
        // when it was due
        hb.on_heartbeat_start(1_000);
        hb.judge_doze_style(
            1_000 + MIN_HEART_INTERVAL as u64 + DOZE_JUDGE_WINDOW as u64,
            true,
            false,
        );
        assert!(!hb.is_doze_style(), "once is not a pattern");

        hb.on_heartbeat_start(2_000);
        hb.judge_doze_style(
            2_000 + MIN_HEART_INTERVAL as u64 + DOZE_JUDGE_WINDOW as u64,
            true,
            false,
        );
        assert!(hb.is_doze_style());

        // an answer that came back on time counts the other way
        hb.on_heartbeat_start(3_000);
        hb.judge_doze_style(3_000 + MIN_HEART_INTERVAL as u64, true, false);
        assert!(!hb.is_doze_style());
    }

    #[test]
    fn the_doze_judgement_needs_a_mobile_network_and_a_background_app() {
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);
        let late = |hb: &mut SmartHeartbeat, is_mobile: bool, is_active: bool| {
            hb.on_heartbeat_start(1_000);
            hb.judge_doze_style(
                1_000 + MIN_HEART_INTERVAL as u64 + DOZE_JUDGE_WINDOW as u64,
                is_mobile,
                is_active,
            );
        };

        for _ in 0..3 {
            late(&mut hb, false, false);
            late(&mut hb, true, true);
        }
        assert!(!hb.is_doze_style(), "neither was judged at all");
    }

    #[test]
    fn a_dozing_network_skips_straight_to_the_biggest_interval() {
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);
        stable(&mut hb);

        // two late answers on a mobile network
        for index in 0..2 {
            hb.on_heartbeat_start(index * 1_000);
            hb.judge_doze_style(
                index * 1_000 + MIN_HEART_INTERVAL as u64 + DOZE_JUDGE_WINDOW as u64,
                true,
                false,
            );
        }
        assert!(hb.is_doze_style());

        for _ in 0..BASE_SUCC_COUNT {
            heartbeat(&mut hb, true);
        }
        assert_eq!(hb.info().cur_heart, MAX_HEART_INTERVAL - SUCCESS_STEP);
    }

    #[test]
    fn an_interval_outside_the_range_is_replaced() {
        let mut hb = SmartHeartbeat::new();
        established(&mut hb);
        stable(&mut hb);

        hb.info_mut().cur_heart = MAX_HEART_INTERVAL + 1;
        assert_eq!(next_interval(&mut hb, false), MIN_HEART_INTERVAL);

        hb.info_mut().cur_heart = MIN_HEART_INTERVAL - 1;
        assert_eq!(next_interval(&mut hb, false), MIN_HEART_INTERVAL);
    }

    #[test]
    fn the_record_of_a_network_can_be_handed_back() {
        let mut hb = SmartHeartbeat::new();
        let mut kept = NetHeartbeatInfo::new();
        kept.net_detail = "wifi-home".to_owned();
        kept.net_type = 1;
        kept.cur_heart = MIN_HEART_INTERVAL + HEART_STEP;
        kept.is_stable = true;
        kept.last_modify_time = 1_700_000_000;
        *hb.info_mut() = kept.clone();

        hb.on_longlink_established("wifi-home", 1);
        assert_eq!(hb.info().cur_heart, MIN_HEART_INTERVAL + HEART_STEP);
        assert_eq!(hb.info().last_modify_time, 1_700_000_000);

        // and one that is dropped goes back to the defaults
        hb.info_mut().clear();
        assert_eq!(*hb.info(), NetHeartbeatInfo::new());
    }
}
