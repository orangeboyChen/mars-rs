//! `io.github.orangeboychen.marsrs.BaseEvent` — the seven `native` methods of
//! `com/tencent/mars/BaseEvent.java`, and the `mars/baseevent` behind them.
//!
//! `BaseEvent.java` is how the app tells Mars what happened to it: it was
//! created, it was destroyed, it went to the background, the network changed,
//! or it crashed. `mars/baseevent/jni/com_tencent_mars_BaseEvent.cc` turns
//! each of those into one call of `mars::baseevent`, and each of those fires
//! one of the seven signals of `mars/baseevent/baseprjevent.h`; who listens is
//! `mars/stn/stn_logic.cc`, which connects them all in one `BOOT_RUN_STARTUP`.
//!
//! Rust has no boot framework and no signals, so the seven calls go straight
//! where the signals went: the process-wide [`mars_stn::StnLogic`] of
//! [`crate::stn`] — `onCreate`, `onDestroy`, `onNetworkChange`,
//! `onInitConfigBeforeOnCreate` — and the one `ActiveLogic` there is, whose
//! state is what `onForeground` moves and what `SignalActive` used to
//! broadcast. The two crash calls close the appender, which is
//! `mars_appender`'s own business here.
//!
//! `ActiveLogic` is the small machine behind "is the app still in the
//! foreground?": it starts out backgrounded but active, a move to the
//! foreground makes it active again, and ten minutes in the background —
//! [`INACTIVE_TIMEOUT_MS`], an alarm started the moment the app leaves — is
//! what makes it inactive, which is what slows the anti-avalanche funnel down
//! to its background speed. The alarm is [`mars_comm::alarm::Alarm`], the same
//! one [`crate::alarm`] brings Java's broadcast back to: it is started with
//! its own id, so `Alarm.onAlarm(id)` finds it.
//!
//! Not ported: `addLoadModule` / `getLoadLibraries` (what
//! `StnLogic.getLoadLibraries` and `SdtLogic.getLoadLibraries` answer with is
//! a constant each, because this port is one library); `SignalForeground`
//! (what listens to it upstream — the long-link connect monitor and the
//! weak-network logic — is wired by the host here, through hooks of its own);
//! `OnNetworkDataChange` (the traffic the app counts goes to
//! [`mars_stn::StnLogic::traffic_data`]); `Alarm::SetType` (the platform alarm
//! has no type here, which is what [`crate::platform_comm::start_alarm_impl`]
//! does with it too).
//!
//! `OnAlarm` is not here either: `Alarm.onAlarm(id)` — the one `native` method
//! of `Alarm.java` — reaches [`crate::alarm::on_alarm_impl`] straight away,
//! which hands the id to the alarm that was started with it.
//!
//! Everything the JVM touches lives in [`crate::jni_bridge`]; what is here is
//! plain Rust and is covered by `cargo test`.

use std::sync::{Mutex, OnceLock};

use mars_comm::{alarm::Alarm, message_queue::get_def_message_queue, tickcount::gettickcount};

/// `ActiveLogic::INACTIVE_TIMEOUT` — ten minutes in the background is what
/// makes the app inactive.
pub const INACTIVE_TIMEOUT_MS: u32 = 10 * 60 * 1000;

/// `ActiveLogic` — whether the app is still doing something, which is what the
/// anti-avalanche check and the timing sync ask.
///
/// The C++'s is a class with two signals of its own (`SignalForeground`,
/// `SignalActive`) and a `Instance()` / `Release()` pair; this is the same
/// state, one value for the whole process, with the signals it used to fire
/// written as calls to the things that listened:
/// [`crate::stn::with_logic`]'s [`mars_stn::StnLogic::set_active`].
struct ActiveLogic {
    /// `isforeground_` — starts out `false`, like the C++.
    is_foreground: bool,
    /// `isactive_` — starts out `true`, like the C++.
    is_active: bool,
    /// `lastforegroundchangetime_`.
    last_foreground_change_time: u64,
    /// The alarm that runs [`on_inactive`] ten minutes after the app left the
    /// foreground — `alarm_` in the C++.
    alarm: Alarm,
}

impl ActiveLogic {
    /// `ActiveLogic::ActiveLogic()` — made with the inactivity alarm already
    /// running, the way the C++'s constructor starts it.
    fn new() -> Self {
        let mut logic = Self {
            is_foreground: false,
            is_active: true,
            last_foreground_change_time: gettickcount(),
            // the C++'s `Alarm(boost::bind(&ActiveLogic::__OnInActive, this),
            // false)`: the default queue, which is the one
            // `Alarm::onAlarmImpl` posts to as well.
            alarm: Alarm::new(get_def_message_queue(), on_inactive),
        };
        logic.start_inactive_alarm();
        logic
    }

    /// `ActiveLogic::OnForeground(_isforeground)`.
    fn on_foreground(&mut self, is_foreground: bool) {
        // the C++'s `if (_isforeground == isforeground_) return;`, and every
        // thing a change moves: a backgrounded app is active *again* until the
        // alarm says otherwise.
        if is_foreground == self.is_foreground {
            return;
        }
        let was_active = self.is_active;
        self.is_active = true;
        self.is_foreground = is_foreground;
        self.last_foreground_change_time = gettickcount();
        self.stop_inactive_alarm();
        if !is_foreground {
            self.start_inactive_alarm();
        }
        if was_active != self.is_active {
            self.signal_active();
        }
    }

    /// `ActiveLogic::__OnInActive()` — what the alarm runs. Ten minutes in the
    /// background is what makes the app inactive; in the foreground it never
    /// does, so the alarm has nothing to say.
    fn on_inactive(&mut self) {
        if !self.is_foreground {
            self.is_active = false;
        }
        self.signal_active();
    }

    /// `ActiveLogic::SwitchActiveStateForDebug(_active)` — sets it and then
    /// runs `__OnInActive`, exactly like the C++, which is why a debug "active"
    /// in the background does not stay active.
    fn switch_active_state_for_debug(&mut self, is_active: bool) {
        self.is_active = is_active;
        self.on_inactive();
    }

    /// `SignalActive(isactive)` — what the C++ broadcast is what the port
    /// calls: the net core is the only thing that listened.
    fn signal_active(&mut self) {
        let is_active = self.is_active;
        crate::stn::with_logic(|logic| logic.set_active(is_active));
    }

    /// `alarm_.Start(INACTIVE_TIMEOUT)` plus `startAlarm(type_, seq, after)`:
    /// the id Java's `Alarm` is started with is the alarm's own, which is how
    /// `Alarm.onAlarm(id)` finds it again.
    ///
    /// A start the alarm refuses — one is waiting already — leaves the id it
    /// was started with, which is the one Java's `Alarm` already has, so
    /// registering it again only answers `false`. The C++ logs and stops there
    /// (`if (!alarm_.Start(...))`); there is nothing here that stops.
    fn start_inactive_alarm(&mut self) {
        self.alarm.start(INACTIVE_TIMEOUT_MS);
        crate::alarm::alarm_start_impl(
            self.alarm.seq() as i64,
            INACTIVE_TIMEOUT_MS as i64,
            gettickcount(),
        );
    }

    /// `alarm_.Cancel()` plus `stopAlarm(seq_)`.
    fn stop_inactive_alarm(&mut self) {
        let id = self.alarm.seq() as i64;
        if id == 0 {
            return;
        }
        self.alarm.cancel();
        crate::alarm::alarm_stop_impl(id);
    }
}

/// The one `ActiveLogic` there is — `ActiveLogic::Instance()`, which the C++
/// makes the first time anything asks for it.
fn with_active<R>(f: impl FnOnce(&mut ActiveLogic) -> R) -> R {
    let mut logic = instance()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(logic.get_or_insert_with(ActiveLogic::new))
}

fn instance() -> &'static Mutex<Option<ActiveLogic>> {
    static LOGIC: OnceLock<Mutex<Option<ActiveLogic>>> = OnceLock::new();
    LOGIC.get_or_init(|| Mutex::new(None))
}

/// `ActiveLogic::Release()` — the state and the alarm with it are dropped.
///
/// The C++ does **not** call this from `onDestroy` ("others use activelogic
/// may crash after activelogic release"), and neither does
/// [`on_destroy_impl`]; it is here because it is the class's own teardown, and
/// because a test wants a fresh one.
pub fn release_impl() {
    let mut logic = instance()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *logic = None;
}

/// The alarm's target: `boost::bind(&ActiveLogic::__OnInActive, this)`. It runs
/// on whatever thread drains the queue, so it reaches the state the way any
/// other call does.
fn on_inactive() {
    with_active(ActiveLogic::on_inactive)
}

/// `BaseEvent.onCreate` — `mars::baseevent::OnCreate()`, which is the C++'s
/// `ActiveLogic::Instance(); StnManager::OnCreate();`.
///
/// `false` when there is a net core already, which is the C++'s
/// `if (!net_core_)`: [`crate::stn`] makes the process-wide STN the first time
/// anything asks for it, so this is what makes it when nothing has yet.
pub fn on_create_impl() -> bool {
    with_active(|_| ());
    crate::stn::with_logic(|logic| logic.create())
}

/// `BaseEvent.onInitConfigBeforeOnCreate` — the encoder version the net core is
/// made with, which the C++ therefore has to be told before `onCreate`.
pub fn on_init_config_before_on_create_impl(version: i32) {
    crate::stn::with_logic(|logic| logic.on_init_config_before_on_create(version))
}

/// `BaseEvent.onDestroy` — `mars::baseevent::OnDestroy()`, which drops the net
/// core. `false` when there was none, the C++'s "net core is nullptr. ignore
/// destroy".
///
/// `ActiveLogic` survives it on purpose — see [`release_impl`].
pub fn on_destroy_impl() -> bool {
    crate::stn::with_logic(mars_stn::StnLogic::destroy)
}

/// `BaseEvent.onForeground` — `mars::baseevent::OnForeground(_isforeground)`,
/// which is `ActiveLogic::OnForeground` in the C++.
pub fn on_foreground_impl(is_foreground: bool) {
    with_active(|logic| logic.on_foreground(is_foreground))
}

/// `BaseEvent.onNetworkChange` — `mars::baseevent::OnNetworkChange()`.
///
/// The C++ runs `OnPlatformNetworkChange()` before it fires the signal, which
/// is how it throws the network information it cached away; this port asks
/// [`crate::platform_comm`] on every read instead, so there is nothing to
/// throw away and the pre-change hook is empty.
pub fn on_network_change_impl() {
    crate::stn::with_logic(|logic| logic.on_network_change(|| {}))
}

/// `BaseEvent.onSingalCrash` — `mars::baseevent::OnSingalCrash(_sig)`.
///
/// What the signal reached is the appender: `mars::xlog::appender_close()`, so
/// what is buffered is flushed before the process goes. The signal number is
/// not the port's to handle, so it is not read.
pub fn on_signal_crash_impl(_sig: i32) {
    crate::close_impl()
}

/// `BaseEvent.onExceptionCrash` — `mars::baseevent::OnExceptionCrash()`, the
/// same close without a signal to name.
pub fn on_exception_crash_impl() {
    crate::close_impl()
}

/// `ActiveLogic::IsActive()` — whether the app is still doing something.
pub fn is_active_impl() -> bool {
    with_active(|logic| logic.is_active)
}

/// `ActiveLogic::IsForeground()`.
pub fn is_foreground_impl() -> bool {
    with_active(|logic| logic.is_foreground)
}

/// `ActiveLogic::LastForegroundChangeTime()` — when `onForeground` last moved,
/// in milliseconds since boot.
pub fn last_foreground_change_time_impl() -> u64 {
    with_active(|logic| logic.last_foreground_change_time)
}

/// `ActiveLogic::SwitchActiveStateForDebug(_active)`.
pub fn switch_active_state_for_debug_impl(is_active: bool) {
    with_active(|logic| logic.switch_active_state_for_debug(is_active))
}

/// The id the inactivity alarm is waiting on — [`crate::alarm`]'s, which is
/// Java's: `0` when no alarm is waiting, and otherwise the one
/// `Alarm.onAlarm(id)` brings back to it.
pub fn inactive_alarm_id_impl() -> i64 {
    with_active(|logic| logic.alarm.seq() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mars_comm::message_queue::{get_def_message_queue, RunLoop};
    use std::time::Duration;

    /// The state is process-wide, and so is the net core `onDestroy` drops: the
    /// samples take it in turn, and each one starts from a fresh
    /// `ActiveLogic`.
    fn isolated<R>(f: impl FnOnce() -> R) -> R {
        let guard = crate::test_lock();
        release_impl();
        let result = f();
        release_impl();
        drop(guard);
        result
    }

    #[test]
    fn a_fresh_app_is_backgrounded_and_active() {
        isolated(|| {
            assert!(!is_foreground_impl());
            assert!(is_active_impl());
            // the alarm the constructor started, waiting out the ten minutes
            assert_ne!(inactive_alarm_id_impl(), 0);
        })
    }

    #[test]
    fn the_foreground_is_what_the_app_said_and_when_it_said_it() {
        isolated(|| {
            let started = last_foreground_change_time_impl();
            on_foreground_impl(true);
            assert!(is_foreground_impl());
            assert!(is_active_impl());
            assert!(last_foreground_change_time_impl() >= started);
            // a foregrounded app has no inactivity alarm waiting
            assert_eq!(inactive_alarm_id_impl(), 0);

            on_foreground_impl(false);
            assert!(!is_foreground_impl());
            // leaving the foreground makes the app active again, and starts
            // the ten minutes over
            assert!(is_active_impl());
            assert_ne!(inactive_alarm_id_impl(), 0);
        })
    }

    #[test]
    fn a_foreground_the_app_did_not_change_is_not_a_change() {
        isolated(|| {
            // the C++'s `if (_isforeground == isforeground_) return;`
            on_foreground_impl(false);
            let changed = last_foreground_change_time_impl();
            let alarm = inactive_alarm_id_impl();
            on_foreground_impl(false);
            assert_eq!(last_foreground_change_time_impl(), changed);
            assert_eq!(inactive_alarm_id_impl(), alarm);
        })
    }

    /// Dispatches the default queue until the alarm's own message has run. The
    /// queue is the whole process's, so a message that is not this alarm's may
    /// be waiting on it ahead of this one.
    fn drain_alarms() {
        for _ in 0..10 {
            RunLoop::dispatch_timeout(get_def_message_queue(), Duration::from_millis(200));
            if !is_active_impl() {
                return;
            }
        }
    }

    /// The whole way an Android alarm travels: `Alarm.onAlarm(id)` finds the id
    /// the inactivity alarm was started with, and the alarm it wakes is what
    /// makes the app inactive.
    #[test]
    fn ten_minutes_in_the_background_is_what_makes_the_app_inactive() {
        isolated(|| {
            let id = inactive_alarm_id_impl();
            assert!(crate::alarm::on_alarm_impl(id));
            drain_alarms();
            assert!(!is_active_impl());
            // and coming back from that is what makes the app active again.
            // The alarm had already gone off, so there is none to cancel.
            on_foreground_impl(true);
            assert!(is_active_impl());
            on_foreground_impl(false);
            assert_ne!(inactive_alarm_id_impl(), 0);
        })
    }

    #[test]
    fn a_stale_alarm_and_a_debug_active_state() {
        isolated(|| {
            // the C++'s `SwitchActiveStateForDebug` runs `__OnInActive` after
            // setting the state, so a debug "active" in the background does
            // not hold
            switch_active_state_for_debug_impl(true);
            assert!(!is_active_impl());
            switch_active_state_for_debug_impl(false);
            assert!(!is_active_impl());
            // in the foreground it does, because `__OnInActive` has nothing
            // to say there
            on_foreground_impl(true);
            switch_active_state_for_debug_impl(true);
            assert!(is_active_impl());

            // no alarm was started for a foregrounded app, so a broadcast
            // that arrives anyway is a stale one
            assert!(!crate::alarm::on_alarm_impl(0));
        })
    }

    #[test]
    fn what_the_app_told_the_port_is_what_stn_hears() {
        isolated(|| {
            // the net core is process-wide and shared with [`crate::stn`]'s
            // own samples, so this one destroys and makes it under the lock
            assert!(on_destroy_impl());
            assert!(on_create_impl());
            // `onCreate` is what makes it, so a second one has nothing to do
            assert!(!on_create_impl());
            on_init_config_before_on_create_impl(200);
            on_network_change_impl();
            on_signal_crash_impl(11);
            on_exception_crash_impl();
        })
    }

    #[test]
    fn an_active_logic_a_panic_poisoned_keeps_answering() {
        isolated(|| {
            // a host that panicked while moving the app to the foreground must
            // not lock the port out of the state it left behind
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                with_active(|_| panic!("the app panicked"))
            }));
            assert!(panicked.is_err());

            on_foreground_impl(true);
            assert!(is_foreground_impl());
            // `Release()` keeps working with the lock poisoned
            release_impl();
        })
    }
}
