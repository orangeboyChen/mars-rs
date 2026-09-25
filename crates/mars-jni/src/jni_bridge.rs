//! The JVM boundary: the entry points Java calls and the readers that turn
//! Java objects into Rust values.
//!
//! Everything here needs a live `JNIEnv`, so it can only be exercised from Java
//! (the Android AAR CI builds). The logic behind it lives in the crate root and
//! is covered by `cargo test`, which is why this file is left out of the
//! coverage measurement.

use jni::objects::{JClass, JIntArray, JObject, JObjectArray, JString, JValue};
use jni::sys::{jboolean, jint, jlong, jobject, JNI_VERSION_1_6};
use jni::{JNIEnv, JavaVM};

use mars_appender::{AppenderMode, CompressMode, LogLevel, XLogConfig, XLoggerInfo};
use mars_stn::Task;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::alarm::on_alarm_impl;
use crate::sdt::{get_load_libraries_impl as sdt_libraries, set_http_netcheck_cgi_impl};
use crate::stn::{
    clear_task_impl, gen_sequence_id_impl, gen_task_id_impl, get_load_libraries_impl,
    has_task_impl, keep_signalling_impl, makesure_longlink_connected_impl, redo_task_impl,
    reset_and_init_encoder_version_impl, reset_impl, set_backup_ips_impl, set_client_version_impl,
    set_debug_ip_impl, set_longlink_svr_addr_impl, set_shortlink_svr_addr_impl,
    set_signalling_strategy_impl, start_task_impl, stop_signalling_impl, stop_task_impl,
    touch_tasks_impl, trig_nooping_impl,
};

use crate::{
    close_impl, flush_impl, get_instance_impl, get_level_impl, guard, level_from_java,
    log_write2_impl, log_write_impl, new_instance_impl, now_timeval, open_appender,
    release_instance_impl, set_appender_mode_impl, set_console_log_open_impl, set_level_impl,
    set_max_alive_time_impl, set_max_file_size_impl,
};

/// The VM the library was loaded into.
///
/// Every other call in this file goes Java -> Rust, but two of them go the
/// other way: `SdtLogic.reportSignalDetectResults` is a static *Java* method
/// the native side calls when a diagnosis finishes, and calling it needs a VM
/// to attach a thread to. `System.loadLibrary` calls `JNI_OnLoad`, which is
/// where this is set.
static VM: OnceLock<JavaVM> = OnceLock::new();

/// `JNI_OnLoad` — `System.loadLibrary` calls it, and it is the only place the
/// library can get hold of the VM.
#[no_mangle]
pub extern "system" fn JNI_OnLoad(vm: JavaVM, _reserved: *mut std::ffi::c_void) -> jint {
    let _ = VM.set(vm);
    JNI_VERSION_1_6 as jint
}

/// `SdtLogic.reportSignalDetectResults(String)` — the C2Java call at the end of
/// a diagnosis, the port of `mars::sdt::ReportNetCheckResult`.
///
/// Without a VM (a unit test, or a host that linked the library instead of
/// loading it from Java) there is nobody to tell, and the report stays where
/// [`crate::sdt`] recorded it; nothing here panics into Rust either way.
pub fn report_signal_detect_results(json: String) {
    guard(|| {
        let Some(vm) = VM.get() else {
            return;
        };
        let Ok(mut env) = vm.attach_current_thread() else {
            return;
        };
        let Ok(class) = env.find_class("io/github/marsrs/sdt/SdtLogic") else {
            return;
        };
        let Ok(message) = env.new_string(&json) else {
            return;
        };
        let argument = JObject::from(message);
        let argument = JValue::Object(&argument);
        let _ = env.call_static_method(
            class,
            "reportSignalDetectResults",
            "(Ljava/lang/String;)V",
            &[argument],
        );
    })
}

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

// #################### io.github.marsrs.stn.StnLogic ####################

/// Reads a Java `int[]`.
fn int_array(env: &mut JNIEnv<'_>, array: &JObject<'_>) -> Vec<i32> {
    if array.is_null() {
        return Vec::new();
    }
    // Re-borrow the handle as an `int[]` without touching the local ref.
    let array: &JIntArray = array.into();
    let Ok(len) = env.get_array_length(array) else {
        return Vec::new();
    };
    if len <= 0 {
        return Vec::new();
    }
    let mut ports = vec![0; len as usize];
    if env.get_int_array_region(array, 0, &mut ports).is_err() {
        return Vec::new();
    }
    ports
}

/// Reads a Java `String[]`.
fn string_array(env: &mut JNIEnv<'_>, array: &JObject<'_>) -> Vec<String> {
    if array.is_null() {
        return Vec::new();
    }
    let array: &JObjectArray = array.into();
    let Ok(len) = env.get_array_length(array) else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for index in 0..len {
        let Ok(element) = env.get_object_array_element(array, index) else {
            continue;
        };
        if element.is_null() {
            continue;
        }
        values.push(java_string(env, &element));
    }
    values
}

/// Reads a Java `List<String>` the way the C++ reads `shortLinkHostList`: with
/// `size()` and `get(int)`.
fn string_list(env: &mut JNIEnv<'_>, list: &JObject<'_>) -> Vec<String> {
    if list.is_null() {
        return Vec::new();
    }
    let Ok(len) = env
        .call_method(list, "size", "()I", &[])
        .and_then(|value| value.i())
    else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for index in 0..len {
        let Ok(element) =
            env.call_method(list, "get", "(I)Ljava/lang/Object;", &[JValue::Int(index)])
        else {
            continue;
        };
        let Ok(element) = element.l() else {
            continue;
        };
        if element.is_null() {
            continue;
        }
        values.push(java_string(env, &element));
    }
    values
}

/// Reads a Java `Map<String, String>` through its `entrySet`, the way the C++
/// `JNU_JObject2Map` does.
fn string_map(env: &mut JNIEnv<'_>, map: &JObject<'_>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if map.is_null() {
        return out;
    }
    let Ok(entries) = env.call_method(map, "entrySet", "()Ljava/util/Set;", &[]) else {
        return out;
    };
    let Ok(entries) = entries.l() else {
        return out;
    };
    let Ok(iterator) = env.call_method(&entries, "iterator", "()Ljava/util/Iterator;", &[]) else {
        return out;
    };
    let Ok(iterator) = iterator.l() else {
        return out;
    };
    loop {
        let Ok(has_next) = env
            .call_method(&iterator, "hasNext", "()Z", &[])
            .and_then(|value| value.z())
        else {
            return out;
        };
        if !has_next {
            return out;
        }
        let Ok(entry) = env.call_method(&iterator, "next", "()Ljava/lang/Object;", &[]) else {
            return out;
        };
        let Ok(entry) = entry.l() else {
            return out;
        };
        let key = env
            .call_method(&entry, "getKey", "()Ljava/lang/Object;", &[])
            .and_then(|value| value.l())
            .map(|key| java_string(env, &key))
            .unwrap_or_default();
        let value = env
            .call_method(&entry, "getValue", "()Ljava/lang/Object;", &[])
            .and_then(|value| value.l())
            .map(|value| java_string(env, &value))
            .unwrap_or_default();
        out.insert(key, value);
    }
}

/// Reads `io.github.marsrs.stn.StnLogic$Task` into a [`Task`]. The fields the
/// C++ reads are the ones below; the rest of the Java class has no counterpart
/// in the port's `Task`.
fn task_from_java(env: &mut JNIEnv<'_>, task: &JObject<'_>) -> Option<Task> {
    if task.is_null() {
        return None;
    }
    let taskid = int_field(env, task, "taskID");
    if taskid < 0 {
        return None;
    }
    let cmdid = int_field(env, task, "cmdID");
    let mut parsed = Task::new(taskid as u32, cmdid as u32);

    parsed.channel_select = int_field(env, task, "channelSelect");
    parsed.cgi = string_field(env, task, "cgi");
    parsed.send_only = bool_field(env, task, "sendOnly");
    parsed.need_authed = bool_field(env, task, "needAuthed");
    parsed.limit_flow = bool_field(env, task, "limitFlow");
    parsed.limit_frequency = bool_field(env, task, "limitFrequency");
    parsed.channel_strategy = int_field(env, task, "channelStrategy");
    parsed.network_status_sensitive = bool_field(env, task, "networkStatusSensitive");
    parsed.priority = int_field(env, task, "priority");
    parsed.retry_count = int_field(env, task, "retryCount");
    parsed.server_process_cost = int_field(env, task, "serverProcessCost");
    parsed.total_timeout = int_field(env, task, "totalTimeout");
    parsed.report_arg = string_field(env, task, "reportArg");
    parsed.long_polling = bool_field(env, task, "longPolling");
    parsed.long_polling_timeout = int_field(env, task, "longPollingTimeout");
    parsed.client_sequence_id = int_field(env, task, "clientSequenceId") as u16;
    parsed.shortlink_host_list = match env.get_field(task, "shortLinkHostList", "Ljava/util/List;")
    {
        Ok(field) => field
            .l()
            .map(|list| string_list(env, &list))
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    parsed.headers = match env.get_field(task, "headers", "Ljava/util/Map;") {
        Ok(field) => field
            .l()
            .map(|map| string_map(env, &map))
            .unwrap_or_default(),
        Err(_) => BTreeMap::new(),
    };
    Some(parsed)
}

fn bool_field(env: &mut JNIEnv<'_>, obj: &JObject<'_>, name: &str) -> bool {
    guard(|| {
        env.get_field(obj, name, "Z")
            .and_then(|value| value.z())
            .unwrap_or(false)
    })
}

/// Builds a Java `ArrayList<String>`.
fn string_array_list(env: &mut JNIEnv<'_>, values: &[String]) -> jobject {
    let Ok(class) = env.find_class("java/util/ArrayList") else {
        return std::ptr::null_mut();
    };
    let Ok(list) = env.new_object(class, "()V", &[]) else {
        return std::ptr::null_mut();
    };
    for value in values {
        let Ok(text) = env.new_string(value) else {
            continue;
        };
        let _ = env.call_method(
            &list,
            "add",
            "(Ljava/lang/Object;)Z",
            &[JValue::Object(&JObject::from(text))],
        );
    }
    list.into_raw()
}

/// `StnLogic.reset`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_reset<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    guard(reset_impl)
}

/// `StnLogic.resetAndInitEncoderVersion`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_resetAndInitEncoderVersion<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    version: jint,
    name: JString<'local>,
) {
    guard(|| {
        let name = env
            .get_string(&name)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        reset_and_init_encoder_version_impl(version, &name);
    })
}

/// `StnLogic.setLonglinkSvrAddr` — the `int[]` arrives as a plain object.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_setLonglinkSvrAddr<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host: JString<'local>,
    ports: JObject<'local>,
    debug_ip: JString<'local>,
) {
    guard(|| {
        let host = env
            .get_string(&host)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        let debug_ip = env
            .get_string(&debug_ip)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        set_longlink_svr_addr_impl(&host, &int_array(&mut env, &ports), &debug_ip);
    })
}

/// `StnLogic.setShortlinkSvrAddr`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_setShortlinkSvrAddr<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    port: jint,
    debug_ip: JString<'local>,
) {
    guard(|| {
        let debug_ip = env
            .get_string(&debug_ip)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        set_shortlink_svr_addr_impl(port, &debug_ip);
    })
}

/// `StnLogic.setDebugIP`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_setDebugIP<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host: JString<'local>,
    ip: JString<'local>,
) {
    guard(|| {
        let host = env
            .get_string(&host)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ip = env
            .get_string(&ip)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        set_debug_ip_impl(&host, &ip);
    })
}

/// `StnLogic.setBackupIPs`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_setBackupIPs<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    host: JString<'local>,
    ips: JObject<'local>,
) {
    guard(|| {
        let host = env
            .get_string(&host)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        set_backup_ips_impl(&host, &string_array(&mut env, &ips));
    })
}

/// `StnLogic.startTask`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_startTask<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    task: JObject<'local>,
) {
    guard(|| {
        if let Some(task) = task_from_java(&mut env, &task) {
            start_task_impl(task);
        }
    })
}

/// `StnLogic.stopTask`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_stopTask<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    taskid: jint,
) {
    guard(|| {
        stop_task_impl(taskid.max(0) as u32);
    })
}

/// `StnLogic.hasTask`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_hasTask<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    taskid: jint,
) -> jboolean {
    guard(|| has_task_impl(taskid.max(0) as u32) as jboolean)
}

/// `StnLogic.redoTask`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_redoTask<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    guard(|| {
        redo_task_impl();
    })
}

/// `StnLogic.touchTasks` — how many tasks there are.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_touchTasks<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jint {
    guard(|| touch_tasks_impl() as jint)
}

/// `StnLogic.clearTask`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_clearTask<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    guard(|| {
        clear_task_impl();
    })
}

/// `StnLogic.makesureLongLinkConnected`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_makesureLongLinkConnected<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    guard(|| {
        makesure_longlink_connected_impl();
    })
}

/// `StnLogic.setSignallingStrategy`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_setSignallingStrategy<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    period: jlong,
    keep_time: jlong,
) {
    guard(|| set_signalling_strategy_impl(period, keep_time))
}

/// `StnLogic.keepSignalling`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_keepSignalling<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    guard(keep_signalling_impl)
}

/// `StnLogic.stopSignalling`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_stopSignalling<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    guard(stop_signalling_impl)
}

/// `StnLogic.setClientVersion`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_setClientVersion<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    version: jint,
) {
    guard(|| set_client_version_impl(version.max(0) as u32))
}

/// `StnLogic.genTaskID`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_genTaskID<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jint {
    guard(|| gen_task_id_impl() as jint)
}

/// `StnLogic.genSequenceId` — declared by the C++ but not by the Java class.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_genSequenceId<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jint {
    guard(|| gen_sequence_id_impl() as jint)
}

/// `StnLogic.trigNooping` — declared by the C++ but not by the Java class.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_trigNooping<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    guard(trig_nooping_impl)
}

/// `StnLogic.getLoadLibraries`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_getLoadLibraries<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jobject {
    guard(|| string_array_list(&mut env, &get_load_libraries_impl()))
}

// #################### io.github.marsrs.sdt.SdtLogic ####################

/// `SdtLogic.setHttpNetcheckCGI`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_sdt_SdtLogic_setHttpNetcheckCGI<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    cgi: JString<'local>,
) {
    guard(|| {
        let cgi = env
            .get_string(&cgi)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        set_http_netcheck_cgi_impl(&cgi);
    })
}

/// `SdtLogic.getLoadLibraries`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_sdt_SdtLogic_getLoadLibraries<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jobject {
    guard(|| string_array_list(&mut env, &sdt_libraries()))
}

// #################### io.github.marsrs.comm.Alarm ####################

/// `Alarm.onAlarm(id)` — the one `native` method of the Java class, called from
/// `onReceive` once the broadcast found the id.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_comm_Alarm_onAlarm<'local>(
    _env: JNIEnv<'local>,
    _this: JObject<'local>,
    id: jlong,
) {
    guard(|| {
        on_alarm_impl(id);
    })
}
