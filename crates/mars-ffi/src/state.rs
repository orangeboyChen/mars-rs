//! Process-wide knobs that have no counterpart in `mars-appender`.
//!
//! Two groups of state live here:
//!
//! 1. the **level filter** ([`set_min_level`] / [`level_enabled`]), which is the
//!    port of `xlogger_SetLevel` / `xlogger_IsEnabledFor` from
//!    `mars/comm/xlogger/xlogger.cc`. The appender itself is level-agnostic
//!    (it formats whatever it is handed), so the gate belongs to the seam.
//! 2. the **identity fields** of `XLoggerInfo` (`pid` / `tid` / `maintid` /
//!    `timeval`) that the C++ filled in from `xlogger_pid()` and friends.
//!
//! Everything is lock-free and `Send + Sync`; there is no `unsafe` here.

use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Level below which records are dropped. Mirrors the C++ global in
/// `xlogger.cc`; `MarsLevelVerbose` (0) means "log everything".
static MIN_LEVEL: AtomicI32 = AtomicI32::new(0);

/// `xlogger_maintid()` — see [`mars_appender::main_thread_id`], which
/// captures the first caller once so worker threads still report the real main
/// thread id.
/// Sets the minimum level that [`level_enabled`] lets through.
///
/// Levels below `kLevelVerbose` (i.e. negative) clamp to "log everything"; any
/// value above `kLevelNone` (6) disables logging altogether, exactly like
/// `xlogger_SetLevel(kLevelNone)`.
pub fn set_min_level(level: i32) {
    MIN_LEVEL.store(level.max(0), Ordering::Relaxed);
}

/// The currently configured minimum level.
pub fn min_level() -> i32 {
    MIN_LEVEL.load(Ordering::Relaxed)
}

/// Port of `xlogger_IsEnabledFor(TLogLevel)`: `true` when `level` is at or above
/// the configured minimum.
pub fn level_enabled(level: i32) -> bool {
    level >= min_level()
}

/// `xlogger_pid()` — the OS process id.
pub fn pid() -> i64 {
    std::process::id() as i64
}

/// `xlogger_tid()` — the OS thread id, so records written through the FFI can
/// be correlated with the ones the C++ wrote in the same process.
pub fn tid() -> i64 {
    mars_appender::thread_id()
}

/// `xlogger_maintid()` — the OS id of the process main thread.
pub fn main_tid() -> i64 {
    mars_appender::main_thread_id()
}

/// `gettimeofday(&info.timeval, NULL)` — seconds + microseconds since the epoch.
pub fn now_timeval() -> (i64, i64) {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_micros() as i64),
        // The system clock is before 1970 (or absurdly skewed); report the epoch
        // rather than failing the write.
        Err(_) => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// The level filter is process-global, so the tests that touch it must not
    /// run concurrently.
    fn level_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    #[test]
    fn default_logs_everything() {
        let _guard = level_lock();
        set_min_level(0);
        for level in 0..=5 {
            assert!(
                level_enabled(level),
                "level {level} should pass the default filter"
            );
        }
    }

    #[test]
    fn higher_level_gates_lower_records() {
        let _guard = level_lock();
        set_min_level(3);
        assert!(!level_enabled(0));
        assert!(!level_enabled(2));
        assert!(level_enabled(3));
        assert!(level_enabled(5));
        set_min_level(0);
    }

    #[test]
    fn level_none_disables_everything() {
        let _guard = level_lock();
        set_min_level(6);
        for level in 0..=5 {
            assert!(
                !level_enabled(level),
                "level {level} should be filtered out"
            );
        }
        set_min_level(0);
    }

    #[test]
    fn negative_level_clamps_to_verbose() {
        let _guard = level_lock();
        set_min_level(-7);
        assert_eq!(min_level(), 0);
        assert!(level_enabled(0));
    }

    #[test]
    fn identity_fields_are_sane() {
        assert!(pid() > 0);
        let this_tid = tid();
        assert!(this_tid >= 1);
        assert_eq!(
            this_tid,
            tid(),
            "the thread id must be stable within a thread"
        );
        assert!(main_tid() >= 1);

        let (sec, usec) = now_timeval();
        assert!(sec > 1_600_000_000, "clock looks wrong: {sec}");
        assert!(usec < 1_000_000);
    }

    #[test]
    fn tid_is_per_thread() {
        let first = tid();
        let other = std::thread::spawn(tid).join().unwrap();
        assert_ne!(first, other);
    }
}
