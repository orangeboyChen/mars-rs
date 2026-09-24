//! JNI bindings for the Rust port of Mars xlog.
//!
//! Every symbol here is the counterpart of one `native` method in
//! `com.tencent.mars.xlog.Xlog` (and of one function in
//! `mars/xlog/jni/Java2C_Xlog.cc`), so the Java side does not have to change:
//!
//! | Java                      | this crate                                    |
//! |---------------------------|-----------------------------------------------|
//! | `appenderOpen`            | `Java_…_appenderOpen`                         |
//! | `appenderClose`           | `Java_…_appenderClose`                        |
//! | `appenderFlush`           | `Java_…_appenderFlush`                        |
//! | `newXlogInstance`         | [`mars_xlog_appender::new_xlogger_instance`]  |
//! | `getXlogInstance`         | [`mars_xlog_appender::get_xlogger_instance`]  |
//! | `releaseXlogInstance`     | [`mars_xlog_appender::release_xlogger_instance`] |
//! | `logWrite` / `logWrite2`  | [`mars_xlog_appender::xlogger_write`]         |
//! | `getLogLevel`/`setLogLevel` | [`mars_xlog_appender::get_level`] / `set_level` |
//! | `setAppenderMode`         | [`mars_xlog_appender::set_appender_mode`]     |
//! | `setConsoleLogOpen`       | [`mars_xlog_appender::set_console_log_open`]  |
//! | `setMaxFileSize`          | `set_max_file_size`                           |
//! | `setMaxAliveTime`         | `set_max_alive_duration`                      |
//!
//! # Panic safety
//!
//! A panic unwinding into the JVM is undefined behaviour, so every entry point
//! runs inside a `catch_unwind` guard, which catches it and drops the call.

use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, jint, jlong};
use jni::JNIEnv;

use mars_xlog_appender::{
    category_set_max_alive_duration as set_max_alive_duration,
    category_set_max_file_size as set_max_file_size, flush, get_level, get_xlogger_instance,
    new_xlogger_instance, release_xlogger_instance, set_appender_mode, set_console_log_open,
    set_level, xlogger_write, AppenderMode, CompressMode, LogLevel, XLogConfig, XLoggerInfo,
    DEFAULT_HANDLE,
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

/// Runs `f`, catching any panic: unwinding into the JVM is UB.
fn guard<R: Default>(f: impl FnOnce() -> R) -> R {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(_) => {
            let _ = writeln!(std::io::stderr(), "marsxlog: JNI call panicked");
            R::default()
        }
    }
}

use std::io::Write;

fn int_field(env: &mut JNIEnv<'_>, obj: &JObject<'_>, name: &str) -> i32 {
    guard(|| {
        env.get_field(obj, name, "I")
            .and_then(|value| value.i())
            .unwrap_or(0)
    })
}

fn long_field(env: &mut JNIEnv<'_>, obj: &JObject<'_>, name: &str) -> i64 {
    guard(|| {
        env.get_field(obj, name, "J")
            .and_then(|value| value.j())
            .unwrap_or(0)
    })
}

fn string_field(env: &mut JNIEnv<'_>, obj: &JObject<'_>, name: &str) -> String {
    guard(|| {
        let Ok(field) = env.get_field(obj, name, "Ljava/lang/String;") else {
            return String::new();
        };
        let Ok(object) = field.l() else {
            return String::new();
        };
        if object.is_null() {
            return String::new();
        }
        let jstring = JString::from(object);
        let Ok(java_str) = env.get_string(&jstring) else {
            return String::new();
        };
        java_str.to_string_lossy().into_owned()
    })
}

/// Reads `com.tencent.mars.xlog.Xlog$XLogConfig`.
fn config_from_java(env: &mut JNIEnv<'_>, config: &JObject<'_>) -> Option<(XLogConfig, LogLevel)> {
    if config.is_null() {
        return None;
    }

    let mode = match int_field(env, config, "mode") {
        0 => AppenderMode::Async,
        1 => AppenderMode::Sync,
        _ => return None,
    };
    let compress_mode = match int_field(env, config, "compressmode") {
        0 => CompressMode::Zlib,
        _ => CompressMode::Zstd,
    };

    let logdir = string_field(env, config, "logdir");
    if logdir.is_empty() {
        return None;
    }
    let cachedir = string_field(env, config, "cachedir");

    Some((
        XLogConfig {
            mode,
            logdir: std::path::PathBuf::from(logdir),
            nameprefix: string_field(env, config, "nameprefix"),
            pub_key: string_field(env, config, "pubkey"),
            compress_mode,
            compress_level: int_field(env, config, "compresslevel"),
            cachedir: if cachedir.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(cachedir))
            },
            cache_days: int_field(env, config, "cachedays").max(0) as u32,
        },
        level_from_java(int_field(env, config, "level")),
    ))
}

fn java_string(env: &mut JNIEnv<'_>, value: &JObject<'_>) -> String {
    guard(|| {
        if value.is_null() {
            return String::new();
        }
        // `JObject` is a borrowed handle: re-wrap the same raw reference
        // without taking ownership of the local ref.
        let jstring = unsafe { JString::from_raw(value.as_raw()) };
        env.get_string(&jstring)
            .map(|java_str| java_str.to_string_lossy().into_owned())
            .unwrap_or_default()
    })
}

/// `appender_open` plus the level of the Java config, i.e. what
/// `Java2C_Xlog.cc` did with `appender_open(config); xlogger_SetLevel(level);`.
///
/// `XLogConfig` carries no level, and the level lives in the category registry
/// rather than in the appender, so it is applied even when the open failed
/// (already open, unusable log directory): the C++ called `xlogger_SetLevel`
/// unconditionally, and `Log.v()/d()` in Java are gated on
/// `Xlog.getLogLevel(0)`, which reads exactly this value.
fn open_appender(config: XLogConfig, level: LogLevel) {
    let _ = mars_xlog_appender::appender_open(config);
    set_level(DEFAULT_HANDLE, level);
}

/// `Xlog.appenderOpen`.
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_appenderOpen<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    config: JObject<'local>,
) {
    guard(|| {
        let Some((config, level)) = config_from_java(&mut env, &config) else {
            return;
        };
        open_appender(config, level);
    })
}

/// `Xlog.appenderClose`.
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_appenderClose<'local>(
    _env: JNIEnv<'local>,
    _this: JObject<'local>,
) {
    guard(mars_xlog_appender::appender_close)
}

/// `Xlog.appenderFlush`.
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_appenderFlush<'local>(
    _env: JNIEnv<'local>,
    _this: JObject<'local>,
    instance: jlong,
    is_sync: jboolean,
) {
    guard(|| flush(instance as u64, is_sync != 0))
}

/// `Xlog.newXlogInstance` — returns the handle, or `0` on a bad config.
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_newXlogInstance<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    config: JObject<'local>,
) -> jlong {
    guard(|| match config_from_java(&mut env, &config) {
        Some((config, level)) => new_xlogger_instance(&config, level) as jlong,
        None => 0,
    })
}

/// `Xlog.getXlogInstance` — the handle for `nameprefix`, or `0`.
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_getXlogInstance<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    nameprefix: JString<'local>,
) -> jlong {
    guard(|| {
        let prefix = env
            .get_string(&nameprefix)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        get_xlogger_instance(&prefix) as jlong
    })
}

/// `Xlog.releaseXlogInstance`.
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_releaseXlogInstance<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    nameprefix: JString<'local>,
) {
    guard(|| {
        let prefix = env
            .get_string(&nameprefix)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        release_xlogger_instance(&prefix);
    })
}

/// `Xlog.logWrite` — writes through the process-wide appender.
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_logWrite<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    info: JObject<'local>,
    log: JString<'local>,
) {
    guard(|| {
        let log = env
            .get_string(&log)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        if info.is_null() {
            xlogger_write(DEFAULT_HANDLE, None, Some(&log));
            return;
        }
        let info = XLoggerInfo {
            level: level_from_java(int_field(&mut env, &info, "level")),
            tag: Some(string_field(&mut env, &info, "tag")),
            filename: Some(string_field(&mut env, &info, "filename")),
            func_name: Some(string_field(&mut env, &info, "funcname")),
            line: int_field(&mut env, &info, "line"),
            // -1 makes the category fill these in from the OS; Java passes real
            // values, which the port keeps.
            pid: long_field(&mut env, &info, "pid"),
            tid: long_field(&mut env, &info, "tid"),
            maintid: long_field(&mut env, &info, "maintid"),
            timeval: now_timeval(),
        };
        xlogger_write(DEFAULT_HANDLE, Some(&info), Some(&log));
    })
}

/// `Xlog.logWrite2` — writes through a specific instance.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_logWrite2<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    instance: jlong,
    level: jint,
    tag: JObject<'local>,
    filename: JObject<'local>,
    funcname: JObject<'local>,
    line: jint,
    pid: jint,
    tid: jlong,
    maintid: jlong,
    log: JString<'local>,
) {
    guard(|| {
        let log = env
            .get_string(&log)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        let info = XLoggerInfo {
            level: level_from_java(level),
            tag: Some(java_string(&mut env, &tag)),
            filename: Some(java_string(&mut env, &filename)),
            func_name: Some(java_string(&mut env, &funcname)),
            line,
            pid: pid as i64,
            tid,
            maintid,
            timeval: now_timeval(),
        };
        xlogger_write(instance as u64, Some(&info), Some(&log));
    })
}

/// `Xlog.getLogLevel` — `-1` for an unknown handle.
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_getLogLevel(
    _env: JNIEnv<'_>,
    _this: JObject<'_>,
    instance: jlong,
) -> jint {
    guard(|| match get_level(instance as u64) {
        Some(level) => level_to_java(level),
        None => -1,
    })
}

/// `Xlog.setLogLevel`.
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_setLogLevel(
    _env: JNIEnv<'_>,
    _this: JObject<'_>,
    instance: jlong,
    level: jint,
) {
    guard(|| set_level(instance as u64, level_from_java(level)))
}

/// `Xlog.setAppenderMode`.
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_setAppenderMode(
    _env: JNIEnv<'_>,
    _this: JObject<'_>,
    instance: jlong,
    mode: jint,
) {
    guard(|| {
        let mode = match mode {
            0 => AppenderMode::Async,
            1 => AppenderMode::Sync,
            _ => return,
        };
        set_appender_mode(instance as u64, mode);
    })
}

/// `Xlog.setConsoleLogOpen`.
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_setConsoleLogOpen(
    _env: JNIEnv<'_>,
    _this: JObject<'_>,
    instance: jlong,
    is_open: jboolean,
) {
    guard(|| set_console_log_open(instance as u64, is_open != 0))
}

/// `Xlog.setMaxFileSize`.
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_setMaxFileSize(
    _env: JNIEnv<'_>,
    _this: JObject<'_>,
    instance: jlong,
    size: jlong,
) {
    guard(|| set_max_file_size(instance as u64, size.max(0) as u64))
}

/// `Xlog.setMaxAliveTime`.
#[no_mangle]
pub extern "system" fn Java_com_tencent_mars_xlog_Xlog_setMaxAliveTime(
    _env: JNIEnv<'_>,
    _this: JObject<'_>,
    instance: jlong,
    seconds: jlong,
) {
    guard(|| set_max_alive_duration(instance as u64, seconds.max(0) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mars_xlog_appender::is_enabled_for;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// The process-wide appender is a singleton, so the tests that open it
    /// must not run concurrently.
    fn singleton() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn logdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mars-xlog-jni-{tag}-{}", std::process::id()));
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
        mars_xlog_appender::appender_close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The C++ called `xlogger_SetLevel` whatever `appender_open` returned, so
    /// a second `appenderOpen` still moves the level even though the singleton
    /// refuses to open twice.
    #[test]
    fn the_level_is_applied_when_the_appender_is_already_open() {
        let _guard = singleton();
        let dir = logdir("reopen");
        assert!(mars_xlog_appender::appender_open(config(&dir)).is_ok());
        set_level(DEFAULT_HANDLE, LogLevel::Info);
        assert!(mars_xlog_appender::appender_open(config(&dir)).is_err());
        open_appender(config(&dir), LogLevel::Error);
        assert_eq!(get_level(DEFAULT_HANDLE), Some(LogLevel::Error));
        mars_xlog_appender::appender_close();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
