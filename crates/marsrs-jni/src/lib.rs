//! JNI bindings for the Rust port of Mars xlog.
//!
//! Every symbol here is the counterpart of one `native` method in
//! `io.github.orangeboychen.marsrs.xlog.Xlog` (and of one function in
//! `mars/xlog/jni/Java2C_Xlog.cc`), so the Java side does not have to change:
//!
//! | Java                      | this crate                                    |
//! |---------------------------|-----------------------------------------------|
//! | `appenderRequestFlush`     | `Java_…_appenderRequestFlush`                  |
//! | `appenderFlushNow`        | `Java_…_appenderFlushNow`                     |
//! | `newXlogInstance`         | [`marsrs_appender::new_xlogger_instance`]  |
//! | `releaseXlogInstanceOf`   | [`marsrs_appender::release_xlogger_instance_of`] |
//! | `write`                  | [`marsrs_appender::is_enabled_for`] + `xlogger_write` |
//! | `getLogLevel`/`setLogLevel` | [`marsrs_appender::get_level`] / `set_level` |
//! | `getCurrentLogPath`      | [`marsrs_appender::current_log_path`]       |
//! | `logFiles`/`logFileNames` | `current_log_files` / `current_log_file_names` |
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
    category_set_max_file_size as set_max_file_size, flush_now, get_level, is_enabled_for,
    new_xlogger_instance, release_xlogger_instance_of, request_flush, set_appender_mode,
    set_console_log_open, set_level, xlogger_write, AppenderMode, LogLevel, XLogConfig,
    XLoggerInfo,
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

/// `Xlog.appenderRequestFlush` body: the writer thread is told it may drain,
/// and this returns at once with no answer about when it is over.
pub(crate) fn request_flush_impl(instance: u64) {
    request_flush(instance);
}

/// `Xlog.appenderFlushNow` body: the drain is this thread's, so what was in the
/// cache is on the disk when it returns.
pub(crate) fn flush_now_impl(instance: u64) {
    flush_now(instance);
}

/// `Xlog.newXlogInstance` body — `0` is the "bad config" answer.
pub(crate) fn new_instance_impl(config: XLogConfig, level: LogLevel) -> jlong {
    new_xlogger_instance(&config, level) as jlong
}

/// `Xlog.releaseXlogInstanceOf` body.
pub(crate) fn release_instance_impl(prefix: &str, instance: u64) {
    release_xlogger_instance_of(prefix, instance)
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
/// the appender answers. There is no filename, function or line to carry
/// either, because Java has no `__FILE__` — the C++ project's `Log` passed
/// `""` and `0` for them, and always did.
pub(crate) fn write_impl(instance: u64, level: LogLevel, tag: Cow<'_, str>, log: &str) -> bool {
    // `Xlog.LEVEL_NONE` — `kLevelNone` — is what a level is *set* to when
    // nothing is to be logged, and it is not a level a record can have: the
    // C ABI writes no record of it either, and the level filter lets one
    // through because `level_ <= kLevelNone` holds for every level there is.
    // A caller that passes it to `write` means the record to be dropped, and
    // this is the one seam that can say so: `is_enabled_for` cannot.
    if level == LogLevel::None {
        return false;
    }
    // An empty body writes nothing, and an empty tag is a tag that was not
    // given, which is what the C ABI answers for the same call: every seam
    // but this one goes through it, so the same Kotlin on Android and on
    // iOS has to write the same record. A message that came out empty is a
    // line in the file that says nothing and cannot be told from one the
    // app wrote, and a `[tag]` field of no characters is a field the C ABI
    // leaves out.
    if log.is_empty() {
        return false;
    }
    if !is_enabled_for(instance, level) {
        return false;
    }
    let info = XLoggerInfo {
        level,
        tag: (!tag.is_empty()).then_some(tag),
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

/// `Xlog.getCurrentLogPath` body: the file the appender of `instance` is
/// writing to.
pub(crate) fn current_log_path_impl(instance: u64) -> Option<std::path::PathBuf> {
    marsrs_appender::current_log_path(instance)
}

/// `Xlog.logFiles` body: the day's files that are there.
pub(crate) fn log_files_impl(instance: u64, timespan: i64) -> Vec<std::path::PathBuf> {
    marsrs_appender::current_log_files(instance, timespan)
}

/// `Xlog.logFileNames` body: the day's names, whether or not they are there.
pub(crate) fn log_file_names_impl(instance: u64, timespan: i64) -> Vec<std::path::PathBuf> {
    marsrs_appender::current_log_file_names(instance, timespan)
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

/// `Xlog.setMaxAliveTime` body; a value below one day is the appender's to
/// refuse, and the answer the Rust api gives is the one Java's own `Xlog`
/// mirrors — see [`crate::set_max_alive_duration`].
pub(crate) fn set_max_alive_time_impl(instance: u64, seconds: jlong) {
    let _ = set_max_alive_duration(instance, seconds.max(0) as u64);
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
/// address, and one instance table every appender is looked up in — so the
/// tests need **one** lock for the whole crate, not one per module: `alarm`'s
/// reset would otherwise drop an id another module is holding.
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

    /// Every body of an entry point is reachable without a JVM: the JNI
    /// plumbing is the only thing a unit test cannot make, and the logic
    /// behind it lives here.
    #[test]
    fn every_jni_body_is_reachable_without_a_jvm() {
        let _guard = crate::test_lock();
        let dir = logdir("bodies");

        // open + write + flush + close, the whole lifecycle of an instance
        let instance = new_instance_impl(config(&dir), LogLevel::Debug);
        assert_ne!(instance, 0, "the appender took the configuration");
        assert_eq!(
            get_level_impl(instance as u64),
            level_to_java(LogLevel::Debug)
        );
        assert!(write_impl(
            instance as u64,
            LogLevel::Info,
            "Net".into(),
            "one call"
        ));

        // the appender's own file, and the day it belongs to — asked after a
        // drain, because a record an async appender is still holding is not in
        // the file yet
        flush_now_impl(instance as u64);
        assert_eq!(
            current_log_path_impl(instance as u64).as_deref(),
            Some(dir.as_path()),
            "the C++ names this question after the directory and not the file"
        );
        assert!(!log_files_impl(instance as u64, 0).is_empty());
        assert!(!log_file_names_impl(instance as u64, 0).is_empty());

        request_flush_impl(instance as u64);
        flush_now_impl(instance as u64);

        // a closed appender answers nothing, and closing twice is harmless
        release_instance_impl("Mars", instance as u64);
        release_instance_impl("Mars", instance as u64);
        assert_eq!(get_level_impl(instance as u64), -1);
        assert!(current_log_path_impl(instance as u64).is_none());
        assert!(log_files_impl(instance as u64, 0).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The two conversions a Java caller's numbers go through, both ways.
    #[test]
    fn levels_and_modes_round_trip() {
        for level in [LogLevel::Verbose, LogLevel::Info, LogLevel::Fatal] {
            assert_eq!(level, level_from_java(level_to_java(level)));
        }
        assert_eq!(
            appender_mode_from_java(AppenderMode::Sync as jint),
            Some(AppenderMode::Sync)
        );
        assert_eq!(appender_mode_from_java(99), None);
    }

    /// A record of the level that means "log nothing" is not written:
    /// `Xlog.LEVEL_NONE` is what `setLevel` is given to stop logging, and a
    /// `write` that carries it drops the record — which is what the C ABI does
    /// with the same level, and what `is_enabled_for` cannot do, since
    /// `level_ <= kLevelNone` holds for every level there is.
    #[test]
    fn a_record_of_the_level_that_disables_logging_is_not_written() {
        let _guard = crate::test_lock();
        let dir = logdir("level-none");

        let instance = new_instance_impl(config(&dir), LogLevel::Verbose) as u64;
        assert!(
            !write_impl(instance, LogLevel::None, "Net".into(), "not logged"),
            "a record of kLevelNone is not written"
        );
        assert!(write_impl(instance, LogLevel::Info, "Net".into(), "logged"));
        flush_now_impl(instance);

        let files = log_files_impl(instance, 0);
        assert!(!files.is_empty(), "the day has the file the record went to");
        // The body is compressed, so what is readable of the file is the
        // header and not the record — the one thing that is certain is that
        // the appender wrote one record and not two.
        let bytes = std::fs::read(&files[0]).expect("the file reads");
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("not logged"), "{text}");

        release_instance_impl("Mars", instance as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A record with nothing in it is not written: what the C ABI answers for
    /// the same call, and so what every seam but this one does. Two `Xlog`s of
    /// one source — an Android one going through here and an iOS one going
    /// through the C ABI — have to write the same file.
    ///
    /// The other half of that answer, a tag of no characters being a tag that
    /// was not given, is in the body above and not here: the body of a record
    /// is compressed, so a `.xlog` is not a file a test can read the `[tag]`
    /// field out of.
    #[test]
    fn an_empty_record_is_not_written() {
        let _guard = crate::test_lock();
        let dir = logdir("empty-record");

        let instance = new_instance_impl(config(&dir), LogLevel::Verbose) as u64;
        assert!(
            !write_impl(instance, LogLevel::Info, "Net".into(), ""),
            "a record with no body is not written"
        );
        // and the one beside it, which has a body, is
        assert!(write_impl(
            instance,
            LogLevel::Info,
            Cow::Borrowed(""),
            "tagged by nobody"
        ));
        flush_now_impl(instance);

        release_instance_impl("Mars", instance);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A panic inside a body is the default value of its answer, and not an
    /// unwind into the JVM.
    #[test]
    fn a_panic_inside_a_body_is_the_default_value() {
        let panicked: u64 = guard(|| panic!("no JVM to unwind into"));
        assert_eq!(panicked, 0);

        // a `()` body cannot be told apart by what it answers — the default
        // of `()` is the only value there is — so what is asserted is that
        // the body ran and the line after it was reached: the panic was
        // caught here, and not unwound into the JVM
        let mut ran = false;
        let _: () = guard(|| {
            ran = true;
            panic!("no JVM to unwind into")
        });
        assert!(ran);
    }
}
