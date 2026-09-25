//! `io.github.marsrs.comm.Alarm` — the timer bookkeeping behind
//! `com/tencent/mars/comm/Alarm.java`.
//!
//! `Alarm.java` is a `BroadcastReceiver`: `start(id, after, context)` asks
//! Android's `AlarmManager` for a broadcast, `stop(id, context)` cancels it and
//! `onReceive` calls the one `native` method of the class, `onAlarm(long id)`,
//! which is where the C++ (and now the Rust) side hears about it.
//!
//! This module is that bookkeeping — the `TreeSet` of waiting alarms, keyed and
//! ordered by id, with `Alarm.java`'s rules:
//!
//! * a negative `after` is refused,
//! * an id that is already waiting is refused,
//! * `stop` answers `false` for an id that was never started,
//! * id `0` is never dispatched (`onReceive` ignores it).
//!
//! The pure-Rust timer is [`mars_comm::alarm`]; this is the Android-side one,
//! which fires through [`on_alarm_impl`](crate::alarm::on_alarm_impl).
//!
//! Everything the JVM touches lives in [`crate::jni_bridge`]; what is here is
//! plain Rust and is covered by `cargo test`.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use mars_comm::alarm::on_system_alarm;
use mars_comm::tickcount::gettickcount;

use crate::wakerlock::{wakerlock_is_locking_impl, wakerlock_lock_for_impl, wakerlock_new_impl};

/// `Alarm::kAlarmStartWakeupLook` — for how long the CPU is kept awake when a
/// platform alarm comes in. The C++ takes the wakelock *here*, on the thread
/// that heard the alarm, because acquiring it anywhere else fails.
const START_ALARM_WAKELOCK_MS: u64 = 1_000;

/// One waiting alarm, the Rust counterpart of the `Object[]` `Alarm.java` keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Waiting {
    /// When it is due, in milliseconds since boot — `curtime + after`.
    waittime: u64,
}

#[derive(Debug, Default)]
struct AlarmState {
    /// `alarm_waiting_set`, a `TreeSet` ordered by id.
    waiting: BTreeMap<i64, Waiting>,
    /// The ids `onAlarm` dispatched, in order.
    fired: Vec<i64>,
}

fn state() -> &'static Mutex<AlarmState> {
    static STATE: OnceLock<Mutex<AlarmState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(AlarmState::default()))
}

fn with_state<R>(f: impl FnOnce(&mut AlarmState) -> R) -> R {
    let mut state = state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut state)
}

/// `Alarm.start(id, after, context)` — the two checks that do not need a
/// `Context`. `waittime` is `curtime + after`, like the Java.
pub fn alarm_start_impl(id: i64, after: i64, curtime: u64) -> bool {
    if after < 0 {
        return false;
    }
    with_state(|state| match state.waiting.entry(id) {
        std::collections::btree_map::Entry::Occupied(_) => false,
        std::collections::btree_map::Entry::Vacant(slot) => {
            slot.insert(Waiting {
                waittime: curtime + after as u64,
            });
            true
        }
    })
}

/// `Alarm.stop(id, context)` — `false` for an id that was never started.
pub fn alarm_stop_impl(id: i64) -> bool {
    with_state(|state| state.waiting.remove(&id).is_some())
}

/// `Alarm.resetAlarm(context)` — how many alarms were dropped.
pub fn alarm_reset_impl() -> usize {
    with_state(|state| {
        let count = state.waiting.len();
        state.waiting.clear();
        count
    })
}

/// How many alarms are waiting, and whether one of them is `id`.
pub fn alarm_is_waiting_impl(id: i64) -> bool {
    with_state(|state| state.waiting.contains_key(&id))
}

/// `Alarm.onAlarm(id)` — the `native` method, called from `onReceive` once the
/// broadcast found the id. It answers whether the alarm was still waiting, so
/// the caller can tell a real alarm from a stale broadcast.
///
/// A real one is not just struck off the list: the id is handed to the alarm
/// subsystem (`Alarm::onAlarmImpl`), which is what wakes whatever the alarm
/// was started for — a retry, or the timer STN is waiting on. Striking it off
/// the list here and stopping there would accept an Android alarm without ever
/// running the work it was meant to wake.
pub fn on_alarm_impl(id: i64) -> bool {
    // `onReceive` returns early for id 0 and for a pid that is not ours
    if id == 0 {
        return false;
    }
    let fired = with_state(|state| {
        if state.waiting.remove(&id).is_some() {
            state.fired.push(id);
            true
        } else {
            false
        }
    });
    if !fired {
        return false;
    }
    on_system_alarm(id);
    // `Alarm::__StartWakeLock()`, on the alarm thread
    wakerlock_lock_for_impl(alarm_wakelock(), START_ALARM_WAKELOCK_MS, gettickcount());
    true
}

/// The wakelock `onAlarm` holds — a static one, like the C++ `WakeUpLock`.
fn alarm_wakelock() -> crate::wakerlock::WakerLockHandle {
    static LOCK: OnceLock<crate::wakerlock::WakerLockHandle> = OnceLock::new();
    *LOCK.get_or_init(wakerlock_new_impl)
}

/// Whether the wakelock `onAlarm` took is still held.
pub fn alarm_wakelock_is_locking_impl() -> bool {
    wakerlock_is_locking_impl(alarm_wakelock())
}

/// The ids `onAlarm` dispatched since the last call.
pub fn take_fired_impl() -> Vec<i64> {
    with_state(|state| std::mem::take(&mut state.fired))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated<R>(f: impl FnOnce() -> R) -> R {
        let guard = crate::test_lock();
        alarm_reset_impl();
        let _ = take_fired_impl();
        let result = f();
        alarm_reset_impl();
        let _ = take_fired_impl();
        drop(guard);
        result
    }

    #[test]
    fn an_alarm_is_started_is_waiting_and_fires_once() {
        isolated(|| {
            assert!(alarm_start_impl(1, 100, 1_000));
            assert!(alarm_is_waiting_impl(1));
            assert!(!alarm_is_waiting_impl(2));

            assert!(on_alarm_impl(1));
            assert_eq!(take_fired_impl(), vec![1]);
            // and the CPU is kept awake while the alarm is dispatched
            assert!(alarm_wakelock_is_locking_impl());

            // a second broadcast for the same id is stale, and it takes no
            // wakelock either
            assert!(!on_alarm_impl(1));
            assert!(take_fired_impl().is_empty());
        })
    }

    #[test]
    fn a_negative_after_and_a_duplicate_id_are_refused() {
        isolated(|| {
            assert!(!alarm_start_impl(1, -1, 0), "Alarm.java refuses after < 0");
            assert!(alarm_start_impl(1, 10, 0));
            assert!(!alarm_start_impl(1, 20, 0), "the id is already waiting");
        })
    }

    #[test]
    fn the_waittime_is_curtime_plus_after() {
        isolated(|| {
            assert!(alarm_start_impl(7, 500, 1_000));
            with_state(|state| assert_eq!(state.waiting[&7].waittime, 1_500));

            // `after == 0` is still a valid alarm, due right away
            assert!(alarm_start_impl(8, 0, 1_000));
            with_state(|state| assert_eq!(state.waiting[&8].waittime, 1_000));
        })
    }

    #[test]
    fn stop_answers_false_for_an_id_that_was_never_started() {
        isolated(|| {
            assert!(!alarm_stop_impl(1));
            assert!(alarm_start_impl(1, 10, 0));
            assert!(alarm_stop_impl(1));
            assert!(!alarm_stop_impl(1), "already stopped");
            // and a stopped alarm no longer fires
            assert!(!on_alarm_impl(1));
        })
    }

    #[test]
    fn the_id_java_never_dispatches_is_refused() {
        isolated(|| {
            assert!(alarm_start_impl(0, 10, 0));
            assert!(!on_alarm_impl(0), "onReceive ignores id 0");
            assert!(alarm_is_waiting_impl(0), "but it is still waiting");
        })
    }

    #[test]
    fn reset_drops_every_waiting_alarm() {
        isolated(|| {
            assert!(alarm_start_impl(1, 10, 0));
            assert!(alarm_start_impl(2, 10, 0));
            assert_eq!(alarm_reset_impl(), 2);
            assert_eq!(alarm_reset_impl(), 0);
            assert!(!on_alarm_impl(1));
        })
    }
}
