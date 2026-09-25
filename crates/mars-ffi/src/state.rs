//! The fields of `XLoggerInfo` that the C++ filled in from `xlogger_pid()` and
//! friends, so that a record written through the FFI names the process and the
//! thread it came from the way the C++ would have.
//!
//! The **level filter** is not here: it lives in `mars-appender`'s default
//! logger (handle `0`, [`mars_appender::set_level`]), which is also what
//! answers `get_level` / `is_enabled_for` for every instance. Two stores would
//! answer two different levels for the same logger.
//!
//! Everything is lock-free and `Send + Sync`; there is no `unsafe` here.

use std::time::{SystemTime, UNIX_EPOCH};

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
