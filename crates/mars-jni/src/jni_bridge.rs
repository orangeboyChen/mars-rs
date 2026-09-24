//! The JVM boundary: the entry points Java calls and the readers that turn
//! Java objects into Rust values.
//!
//! Everything here needs a live `JNIEnv`, so it can only be exercised from Java
//! (the Android AAR CI builds). The logic behind it lives in the crate root and
//! is covered by `cargo test`, which is why this file is left out of the
//! coverage measurement.

use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, jint, jlong};
use jni::JNIEnv;

use mars_appender::{AppenderMode, CompressMode, LogLevel, XLogConfig, XLoggerInfo};

use crate::{
    close_impl, flush_impl, get_instance_impl, get_level_impl, guard, level_from_java,
    log_write2_impl, log_write_impl, new_instance_impl, now_timeval, open_appender,
    release_instance_impl, set_appender_mode_impl, set_console_log_open_impl, set_level_impl,
    set_max_alive_time_impl, set_max_file_size_impl,
};

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

/// Reads `io.github.marsrs.xlog.Xlog$XLogConfig`.
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

#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_appenderOpen<'local>(
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
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_appenderClose<'local>(
    _env: JNIEnv<'local>,
    _this: JObject<'local>,
) {
    guard(close_impl)
}

/// `Xlog.appenderFlush`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_appenderFlush<'local>(
    _env: JNIEnv<'local>,
    _this: JObject<'local>,
    instance: jlong,
    is_sync: jboolean,
) {
    guard(|| flush_impl(instance as u64, is_sync != 0))
}

/// `Xlog.newXlogInstance` — returns the handle, or `0` on a bad config.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_newXlogInstance<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    config: JObject<'local>,
) -> jlong {
    guard(|| match config_from_java(&mut env, &config) {
        Some((config, level)) => new_instance_impl(config, level),
        None => 0,
    })
}

/// `Xlog.getXlogInstance` — the handle for `nameprefix`, or `0`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_getXlogInstance<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    nameprefix: JString<'local>,
) -> jlong {
    guard(|| {
        let prefix = env
            .get_string(&nameprefix)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        get_instance_impl(&prefix)
    })
}

/// `Xlog.releaseXlogInstance`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_releaseXlogInstance<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    nameprefix: JString<'local>,
) {
    guard(|| {
        let prefix = env
            .get_string(&nameprefix)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        release_instance_impl(&prefix);
    })
}

/// `Xlog.logWrite` — writes through the process-wide appender.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_logWrite<'local>(
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
            log_write_impl(None, &log);
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
        log_write_impl(Some(info), &log);
    })
}

/// `Xlog.logWrite2` — writes through a specific instance.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_logWrite2<'local>(
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
        log_write2_impl(instance as u64, info, &log);
    })
}

/// `Xlog.getLogLevel` — `-1` for an unknown handle.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_getLogLevel(
    _env: JNIEnv<'_>,
    _this: JObject<'_>,
    instance: jlong,
) -> jint {
    guard(|| get_level_impl(instance as u64))
}

/// `Xlog.setLogLevel`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_setLogLevel(
    _env: JNIEnv<'_>,
    _this: JObject<'_>,
    instance: jlong,
    level: jint,
) {
    guard(|| set_level_impl(instance as u64, level))
}

/// `Xlog.setAppenderMode`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_setAppenderMode(
    _env: JNIEnv<'_>,
    _this: JObject<'_>,
    instance: jlong,
    mode: jint,
) {
    guard(|| set_appender_mode_impl(instance as u64, mode))
}

/// `Xlog.setConsoleLogOpen`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_setConsoleLogOpen(
    _env: JNIEnv<'_>,
    _this: JObject<'_>,
    instance: jlong,
    is_open: jboolean,
) {
    guard(|| set_console_log_open_impl(instance as u64, is_open != 0))
}

/// `Xlog.setMaxFileSize`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_setMaxFileSize(
    _env: JNIEnv<'_>,
    _this: JObject<'_>,
    instance: jlong,
    size: jlong,
) {
    guard(|| set_max_file_size_impl(instance as u64, size))
}

/// `Xlog.setMaxAliveTime`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_xlog_Xlog_setMaxAliveTime(
    _env: JNIEnv<'_>,
    _this: JObject<'_>,
    instance: jlong,
    seconds: jlong,
) {
    guard(|| set_max_alive_time_impl(instance as u64, seconds))
}
