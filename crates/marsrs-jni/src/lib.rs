//! JNI bindings for the Rust port of Mars xlog.
//!
//! Every symbol here is the counterpart of one `native` method in
//! `io.github.orangeboychen.marsrs.xlog.Xlog` (and of one function in
//! `mars/xlog/jni/Java2C_Xlog.cc`), so the Java side does not have to change:
//!
//! | Java                      | this crate                                    |
//! |---------------------------|-----------------------------------------------|
//! | `appenderOpen`            | `Java_…_appenderOpen`                         |
//! | `appenderClose`           | `Java_…_appenderClose`                        |
//! | `appenderFlush`           | `Java_…_appenderFlush`                        |
//! | `newXlogInstance`         | [`marsrs_appender::new_xlogger_instance`]  |
//! | `getXlogInstance`         | [`marsrs_appender::get_xlogger_instance`]  |
//! | `releaseXlogInstance`     | [`marsrs_appender::release_xlogger_instance`] |
//! | `logWrite`               | [`marsrs_appender::xlogger_write`]         |
//! | `write`                  | [`marsrs_appender::is_enabled_for`] + `xlogger_write` |
//! | `getLogLevel`/`setLogLevel` | [`marsrs_appender::get_level`] / `set_level` |
//! | `setAppenderMode`         | [`marsrs_appender::set_appender_mode`]     |
//! | `setConsoleLogOpen`       | [`marsrs_appender::set_console_log_open`]  |
//! | `setMaxFileSize`          | `set_max_file_size`                           |
//! | `setMaxAliveTime`         | `set_max_alive_duration`                      |
//!
//! The same seam covers the rest of the Java api: [`stn`] is
//! `io.github.orangeboychen.marsrs.stn.StnLogic`, and what it reaches is the net core of
//! [`marsrs_stn`] — one value for the whole process, like the C++'s `NetCore`
//! singleton — rather than a copy of it; [`sdt`], [`app_logic`],
//! [`platform_comm`], [`alarm`] and [`wakerlock`] are the same for their own
//! Java classes.
//!
//! # Panic safety
//!
//! A panic unwinding into the JVM is undefined behaviour, so every entry point
//! runs inside a `catch_unwind` guard, which catches it and drops the call.

use jni::sys::{jint, jlong};

use std::borrow::Cow;

use marsrs_appender::{
    category_set_max_alive_duration as set_max_alive_duration,
    category_set_max_file_size as set_max_file_size, flush_now, get_level, get_xlogger_instance,
    is_enabled_for, new_xlogger_instance, release_xlogger_instance, set_appender_mode,
    set_console_log_open, set_level, signal_flush, xlogger_write, AppenderMode, LogLevel,
    XLogConfig, XLoggerInfo, DEFAULT_HANDLE,
};

/// `gettimeofday(&info.timeval, NULL)` — seconds + microseconds since the
/// epoch, the same value the FFI seam stamps on every record.
fn now_timeval() -> (i64, i64) {
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => (duration.as_secs() as i64, duration.subsec_micros() as i64),
        Err(_) => (0, 0),
    }
}

/// `Xlog.LEVEL_*` — Java's 0..=5 map onto `TLogLevel` and `LEVEL_NONE` onto
/// `kLevelNone`. A negative level logs everything, exactly like
/// `(TLogLevel)-1` did in the C++.
fn level_from_java(level: jint) -> LogLevel {
    match level {
        0 => LogLevel::Verbose,
        1 => LogLevel::Debug,
        2 => LogLevel::Info,
        3 => LogLevel::Warn,
        4 => LogLevel::Error,
        5 => LogLevel::Fatal,
        6 => LogLevel::None,
        _ if level < 0 => LogLevel::Verbose,
        _ => LogLevel::Fatal,
    }
}

fn level_to_java(level: LogLevel) -> jint {
    level as jint
}

/// `Xlog.AppednerModeAsync/Sync` — anything else is ignored, like the C++.
fn appender_mode_from_java(mode: jint) -> Option<AppenderMode> {
    match mode {
        0 => Some(AppenderMode::Async),
        1 => Some(AppenderMode::Sync),
        _ => None,
    }
}

/// Runs `f`, catching any panic: unwinding into the JVM is UB.
fn guard<R: Default>(f: impl FnOnce() -> R) -> R {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(_) => {
            let _ = writeln!(std::io::stderr(), "marsrsxlog: JNI call panicked");
            R::default()
        }
    }
}

use std::io::Write;

/// `appender_open` plus the level of the Java config, i.e. what
/// `Java2C_Xlog.cc` did with `appender_open(config); xlogger_SetLevel(level);`.
///
/// `XLogConfig` carries no level, and the level lives in the category registry
/// rather than in the appender, so it is applied even when the open failed
/// (already open, unusable log directory): the C++ called `xlogger_SetLevel`
/// unconditionally, and `Log.v()/d()` in Java are gated on
/// `Xlog.getLogLevel(0)`, which reads exactly this value.
fn open_appender(config: XLogConfig, level: LogLevel) {
    let _ = marsrs_appender::appender_open(config);
    set_level(DEFAULT_HANDLE, level);
}

/// `Xlog.appenderClose` body: everything but the JNI plumbing, so it can be
/// unit tested without a JVM.
pub(crate) fn close_impl() {
    marsrs_appender::appender_close()
}

/// `Xlog.appenderFlush` body.
pub(crate) fn flush_impl(instance: u64, is_sync: bool) {
    if is_sync {
        flush_now(instance);
    } else {
        signal_flush(instance);
    }
}

/// `Xlog.newXlogInstance` body — `0` is the "bad config" answer.
pub(crate) fn new_instance_impl(config: XLogConfig, level: LogLevel) -> jlong {
    new_xlogger_instance(&config, level) as jlong
}

/// `Xlog.getXlogInstance` body.
pub(crate) fn get_instance_impl(prefix: &str) -> jlong {
    get_xlogger_instance(prefix) as jlong
}

/// `Xlog.releaseXlogInstance` body.
pub(crate) fn release_instance_impl(prefix: &str) {
    release_xlogger_instance(prefix)
}

/// `Xlog.logWrite` body — `None` writes with the category defaults.
pub(crate) fn log_write_impl(info: Option<XLoggerInfo>, log: &str) -> bool {
    xlogger_write(DEFAULT_HANDLE, info.as_ref(), Some(log))
}

/// `Xlog.write` body — the one record, one JNI call write of the Kotlin API.
///
/// [`marsrs_appender::is_enabled_for`] is asked *before* the record is formatted, which is what
/// gives the level something to say on the process-wide appender: the C++'s
/// `xlogger_Write` — what handle `0` reaches — has no level filter of its own,
/// so a record of a level the appender is above reached the file anyway. Asking
/// from Kotlin, the other way to keep the promise "a record less severe than
/// the level is dropped", costs a second JNI call (`getLogLevel`) per line.
///
/// The pid, the tid and the main tid are `-1`, the "fill these in from the OS"
/// the C++ project's Java spelled for itself only in part: what its `Log` hands
/// over is a `Thread.id`, which is not a tid. There is no filename, function or
/// line to carry either, because Java has no `__FILE__` — the C++ project's
/// `Log` passes `""` and `0` for them, and always did.
pub(crate) fn write_impl(instance: u64, level: LogLevel, tag: Cow<'_, str>, log: &str) -> bool {
    if !is_enabled_for(instance, level) {
        return false;
    }
    let info = XLoggerInfo {
        level,
        tag: Some(tag),
        filename: None,
        func_name: None,
        line: 0,
        pid: -1,
        tid: -1,
        maintid: -1,
        timeval: now_timeval(),
        trace_log: 0,
    };
    xlogger_write(instance, Some(&info), Some(log))
}

/// `Xlog.getLogLevel` body — `-1` for an unknown handle.
pub(crate) fn get_level_impl(instance: u64) -> jint {
    match get_level(instance) {
        Some(level) => level_to_java(level),
        None => -1,
    }
}

/// `Xlog.setLogLevel` body.
pub(crate) fn set_level_impl(instance: u64, level: jint) {
    set_level(instance, level_from_java(level))
}

/// `Xlog.setAppenderMode` body — an unknown mode is ignored.
pub(crate) fn set_appender_mode_impl(instance: u64, mode: jint) {
    if let Some(mode) = appender_mode_from_java(mode) {
        set_appender_mode(instance, mode);
    }
}

/// `Xlog.setConsoleLogOpen` body.
pub(crate) fn set_console_log_open_impl(instance: u64, is_open: bool) {
    set_console_log_open(instance, is_open)
}

/// `Xlog.setMaxFileSize` body — a negative size means "never split".
pub(crate) fn set_max_file_size_impl(instance: u64, size: jlong) {
    set_max_file_size(instance, size.max(0) as u64)
}

/// `Xlog.setMaxAliveTime` body.
pub(crate) fn set_max_alive_time_impl(instance: u64, seconds: jlong) {
    set_max_alive_duration(instance, seconds.max(0) as u64)
}

/// The `native` methods of `io.github.orangeboychen.marsrs.stn.StnLogic`.
pub mod stn;

// `io.github.orangeboychen.marsrs.stn.StnLogic$ICallBack` — the app STN asks when the app is
// Java. The module's own doc says the rest of it: a `///` here would push its
// links out of the module they resolve in.
pub mod stn_c2java;

/// The `native` methods of `io.github.orangeboychen.marsrs.sdt.SdtLogic`.
pub mod sdt;

/// `io.github.orangeboychen.marsrs.comm.Alarm` — the timer the app broadcasts into.
pub mod alarm;

/// `io.github.orangeboychen.marsrs.comm.WakerLock` — the wake lock the platform holds.
pub mod wakerlock;

// `io.github.orangeboychen.marsrs.app.AppLogic` — what the app is. The module's own doc says
// it all, and its links resolve in the module: a `///` here would push them out
// of it, which is why this one is a `//` too.
pub mod app_logic;

// `mars/app/src/traffic_statistics.cc` — how much traffic the app has cost the
// device. The module's own doc says it all, and its links resolve in the
// module: a `///` here would push them out of it, which is why this one is a
// `//` too.
pub mod traffic_statistics;

// `io.github.orangeboychen.marsrs.comm.PlatformComm$C2Java` — what the platform answers. The
// module's own doc says it all, and its links resolve in the module: a `///`
// here would push them out of it, which is why this one is a `//` too.
pub mod platform_comm;

// `io.github.orangeboychen.marsrs.BaseEvent` — what the app tells the port happened to it.
// The module's own doc says it all, and its links resolve in the module: a
// `///` here would push them out of it, which is why this one is a `//` too.
pub mod baseevent;

/// The `Java_…_*` symbols the library exports: every `native` method, and the
/// readers beside them that turn Java objects into Rust values.
pub mod jni_bridge;

/// The state of this crate is process-wide — one alarm set, one long-link
/// address, one appender — so the tests need **one** lock for the whole crate,
/// not one per module: `alarm`'s reset would otherwise drop an id another
/// module is holding.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use marsrs_appender::is_enabled_for;
    use std::sync::MutexGuard;

    /// The process-wide appender is a singleton, so the tests that open it
    /// must not run concurrently — and not only with each other: the other
    /// modules open and close the same appender and drop the same net core
    /// under [`crate::test_lock`], so a lock of this module's own would guard
    /// half of what these tests touch.
    fn singleton() -> MutexGuard<'static, ()> {
        crate::test_lock()
    }

    /// The lock is the crate's one and not a second one of this module's: a
    /// test that opens the singleton appender here used to be able to run
    /// alongside one that closes it in another module.
    #[test]
    fn the_lock_the_appender_tests_take_is_the_crates_one() {
        use std::sync::mpsc::{channel, RecvTimeoutError};
        use std::time::Duration;

        let (ready_tx, ready_rx) = channel();
        let (let_go_tx, let_go_rx) = channel();
        let holder = std::thread::spawn(move || {
            let _guard = crate::test_lock();
            ready_tx.send(()).unwrap();
            let_go_rx.recv().unwrap();
        });
        // the crate's lock is held from here on
        ready_rx.recv().unwrap();

        let (took_tx, took_rx) = channel();
        let taker = std::thread::spawn(move || {
            let _guard = singleton();
            took_tx.send(()).unwrap();
        });
        assert_eq!(
            took_rx.recv_timeout(Duration::from_millis(500)),
            Err(RecvTimeoutError::Timeout),
            "`singleton()` is a lock of its own, so it does not wait for the crate's"
        );

        let_go_tx.send(()).unwrap();
        holder.join().unwrap();
        took_rx.recv().unwrap();
        taker.join().unwrap();
    }

    fn logdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("marsrs-jni-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn config(dir: &std::path::Path) -> XLogConfig {
        XLogConfig {
            logdir: dir.to_path_buf(),
            nameprefix: "Mars".to_owned(),
            ..XLogConfig::default()
        }
    }

    #[test]
    fn level_from_java_maps_the_xlog_constants() {
        assert_eq!(level_from_java(0), LogLevel::Verbose);
        assert_eq!(level_from_java(1), LogLevel::Debug);
        assert_eq!(level_from_java(2), LogLevel::Info);
        assert_eq!(level_from_java(3), LogLevel::Warn);
        assert_eq!(level_from_java(4), LogLevel::Error);
        assert_eq!(level_from_java(5), LogLevel::Fatal);
        assert_eq!(level_from_java(6), LogLevel::None);
        // `(TLogLevel)-1` logged everything in the C++.
        assert_eq!(level_from_java(-1), LogLevel::Verbose);
    }

    #[test]
    fn level_none_disables_every_record() {
        let _guard = singleton();
        set_level(DEFAULT_HANDLE, LogLevel::None);
        assert!(!is_enabled_for(DEFAULT_HANDLE, LogLevel::Fatal));
        assert_eq!(level_to_java(get_level(DEFAULT_HANDLE).unwrap()), 6);
        set_level(DEFAULT_HANDLE, LogLevel::Info);
    }

    #[test]
    fn appender_open_applies_the_config_level_to_the_default_logger() {
        let _guard = singleton();
        let dir = logdir("level");
        set_level(DEFAULT_HANDLE, LogLevel::Verbose);
        open_appender(config(&dir), LogLevel::Warn);
        // `Xlog.getLogLevel(0)` reads exactly this value.
        assert_eq!(get_level(DEFAULT_HANDLE), Some(LogLevel::Warn));
        assert!(!is_enabled_for(DEFAULT_HANDLE, LogLevel::Info));
        marsrs_appender::appender_close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The C++ called `xlogger_SetLevel` whatever `appender_open` returned, so
    /// a second `appenderOpen` still moves the level even though the singleton
    /// refuses to open twice.
    #[test]
    fn every_jni_body_is_reachable_without_a_jvm() {
        let _guard = singleton();
        let dir = logdir("bodies");
        set_level(DEFAULT_HANDLE, LogLevel::Verbose);

        // open + write + flush + close, the whole lifecycle
        open_appender(config(&dir), LogLevel::Debug);
        assert_eq!(
            get_level_impl(DEFAULT_HANDLE),
            level_to_java(LogLevel::Debug)
        );
        assert!(log_write_impl(None, "no info"));
        let info = XLoggerInfo {
            level: LogLevel::Info,
            tag: Some("Net".into()),
            filename: Some("main.rs".into()),
            func_name: Some("run".into()),
            line: 42,
            pid: -1,
            tid: -1,
            maintid: -1,
            timeval: now_timeval(),
            trace_log: 0,
        };
        assert!(log_write_impl(Some(info), "with info"));
        assert!(write_impl(
            DEFAULT_HANDLE,
            LogLevel::Info,
            "Net".into(),
            "one call"
        ));
        flush_impl(DEFAULT_HANDLE, false);
        flush_impl(DEFAULT_HANDLE, true);
        close_impl();
        // closing twice is harmless, like the C++ appender_close()
        close_impl();

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `write` is the write of the Kotlin API, and the level has to mean
    /// something there: handle `0` reaches the C++'s `xlogger_Write`, which has
    /// no filter of its own, so the filter `Xlog.i(tag, message)` promises is
    /// this one.
    #[test]
    fn write_drops_a_record_the_level_is_above() {
        let _guard = singleton();
        let dir = logdir("write-level");
        open_appender(config(&dir), LogLevel::Warn);

        assert!(!write_impl(
            DEFAULT_HANDLE,
            LogLevel::Info,
            "Net".into(),
            "dropped"
        ));
        assert!(write_impl(
            DEFAULT_HANDLE,
            LogLevel::Warn,
            "Net".into(),
            "kept"
        ));
        assert!(write_impl(
            DEFAULT_HANDLE,
            LogLevel::Error,
            "Net".into(),
            "kept"
        ));
        // an unknown handle writes nothing, like `xlogger_write`
        assert!(!write_impl(
            0xdead_beef,
            LogLevel::Fatal,
            "Net".into(),
            "nowhere"
        ));

        marsrs_appender::appender_close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn instances_are_created_looked_up_and_released() {
        // the instances of the process-wide appender are one registry, and
        // the prefix this test looks them up by is the one the other tests
        // open the singleton with
        let _guard = singleton();
        let dir = logdir("instances");
        let config = config(&dir);
        let handle = new_instance_impl(config.clone(), LogLevel::Info);
        assert_ne!(handle, 0);
        assert_eq!(get_instance_impl("Mars"), handle);
        // an unknown prefix has no handle
        assert_eq!(get_instance_impl("nope"), 0);
        // a config without a log dir is refused
        let broken = XLogConfig {
            logdir: std::path::PathBuf::new(),
            ..config.clone()
        };
        assert_eq!(new_instance_impl(broken, LogLevel::Info), 0);
        release_instance_impl("Mars");
        assert_eq!(get_instance_impl("Mars"), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_setters_accept_the_java_values() {
        // every setter here moves the state of the default handle, which the
        // other tests read: without the singleton lock this test used to race
        // against them and leave the level/mode it happened to set last.
        let _guard = singleton();
        set_level_impl(DEFAULT_HANDLE, 3);
        assert_eq!(
            get_level_impl(DEFAULT_HANDLE),
            level_to_java(LogLevel::Warn)
        );
        // an unknown handle has no level
        assert_eq!(get_level_impl(0xdead_beef), -1);

        set_appender_mode_impl(DEFAULT_HANDLE, 1);
        set_appender_mode_impl(DEFAULT_HANDLE, 0);
        // an out-of-range mode is ignored instead of panicking
        set_appender_mode_impl(DEFAULT_HANDLE, 9);

        set_console_log_open_impl(DEFAULT_HANDLE, true);
        set_console_log_open_impl(DEFAULT_HANDLE, false);

        // negatives clamp to "unlimited", like the C++
        set_max_file_size_impl(DEFAULT_HANDLE, -1);
        set_max_file_size_impl(DEFAULT_HANDLE, 1024);
        set_max_alive_time_impl(DEFAULT_HANDLE, -1);
        set_max_alive_time_impl(DEFAULT_HANDLE, 3600);
    }

    #[test]
    fn appender_mode_from_java_only_accepts_the_two_modes() {
        assert_eq!(appender_mode_from_java(0), Some(AppenderMode::Async));
        assert_eq!(appender_mode_from_java(1), Some(AppenderMode::Sync));
        assert_eq!(appender_mode_from_java(2), None);
        assert_eq!(appender_mode_from_java(-1), None);
    }

    #[test]
    fn the_level_is_applied_when_the_appender_is_already_open() {
        let _guard = singleton();
        let dir = logdir("reopen");
        assert!(marsrs_appender::appender_open(config(&dir)).is_ok());
        set_level(DEFAULT_HANDLE, LogLevel::Info);
        assert!(marsrs_appender::appender_open(config(&dir)).is_err());
        open_appender(config(&dir), LogLevel::Error);
        assert_eq!(get_level(DEFAULT_HANDLE), Some(LogLevel::Error));
        marsrs_appender::appender_close();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
