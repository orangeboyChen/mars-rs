//! The JVM boundary: the entry points Java calls and the readers that turn
//! Java objects into Rust values.
//!
//! Everything here needs a live `JNIEnv`, so it can only be exercised from Java
//! (the Android AAR CI builds). The logic behind it lives in the crate root and
//! is covered by `cargo test`, which is why this file is left out of the
//! coverage measurement.

use jni::objects::{
    JByteArray, JClass, JIntArray, JObject, JObjectArray, JString, JValue, JValueOwned,
};
use jni::sys::{jboolean, jint, jlong, jobject, JNI_VERSION_1_6};
use jni::{JNIEnv, JavaVM};

use mars_appender::{AppenderMode, CompressMode, LogLevel, XLogConfig, XLoggerInfo};
use mars_stn::{CgiProfile, Task};

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::stn_c2java::{Answer, Question};

use crate::app_logic::{AccountInfo, Answer as AppAnswer, DeviceInfo, Question as AppQuestion};
use crate::platform_comm::{
    Answer as PlatformAnswer, ApnInfo, NetInfo, NetType, Question as PlatformQuestion, SimInfo,
    WifiInfo,
};

use crate::alarm::on_alarm_impl;
use crate::baseevent::{
    on_create_impl, on_destroy_impl, on_exception_crash_impl, on_foreground_impl,
    on_init_config_before_on_create_impl, on_network_change_impl, on_signal_crash_impl,
};
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

/// `StnLogic.touchTasks`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_stn_StnLogic_touchTasks<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    guard(touch_tasks_impl)
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

// #################### the questions STN asks Java ####################

/// `StnLogic` — the class the C++'s thirteen C2Java calls are static methods
/// of. Every one of them forwards to the `ICallBack` the app handed to
/// `setCallBack`, which is why the native side never keeps the app itself.
const STN_CALLBACK: &str = "io/github/marsrs/stn/StnLogic";

/// `StnLogic$CgiProfile` — the object `onTaskEnd` is handed.
const STN_CGI_PROFILE: &str = "io/github/marsrs/stn/StnLogic$CgiProfile";

/// One of the thirteen `C2Java_*` functions of
/// `com_tencent_mars_stn_StnLogic_C2Java.cc`, i.e. what
/// [`crate::stn_c2java::JavaApp::jvm`] asks: attach the thread, call the one
/// static method, and read the answer out of what Java handed back — a
/// `ByteArrayOutputStream`, an `int[]`, a `String[]`.
///
/// Without a VM (a host that linked the library instead of loading it from
/// Java) there is nobody to ask, and the answer is the one STN takes when the
/// app said nothing. Nothing here panics into Rust either way.
pub(crate) fn ask_java(question: Question) -> Answer {
    guard(|| {
        let Some(vm) = VM.get() else {
            return Answer::Nothing;
        };
        let Ok(mut env) = vm.attach_current_thread() else {
            return Answer::Nothing;
        };
        let Ok(class) = env.find_class(STN_CALLBACK) else {
            return Answer::Nothing;
        };
        ask_stn(&mut env, class, question)
    })
}

fn ask_stn<'a>(env: &mut JNIEnv<'a>, class: JClass<'a>, question: Question) -> Answer {
    match question {
        Question::MakesureAuthed { host } => {
            let Ok(host) = env.new_string(&host) else {
                return Answer::Nothing;
            };
            let host = JObject::from(host);
            let called = env.call_static_method(
                class,
                "makesureAuthed",
                "(Ljava/lang/String;)Z",
                &[JValue::Object(&host)],
            );
            Answer::Yes(bool_of(called))
        }
        Question::TrafficData { send, recv } => {
            let _ = env.call_static_method(
                class,
                "trafficData",
                "(II)V",
                &[JValue::Int(send as jint), JValue::Int(recv as jint)],
            );
            Answer::Nothing
        }
        Question::OnNewDns { host } => {
            let Ok(host) = env.new_string(&host) else {
                return Answer::Nothing;
            };
            let host = JObject::from(host);
            let called = env.call_static_method(
                class,
                "onNewDns",
                "(Ljava/lang/String;)[Ljava/lang/String;",
                &[JValue::Object(&host)],
            );
            Answer::Ips(strings_of(env, called))
        }
        Question::OnPush {
            channel_id,
            cmdid,
            taskid,
            body,
        } => {
            let Ok(channel_id) = env.new_string(&channel_id) else {
                return Answer::Nothing;
            };
            let channel_id = JObject::from(channel_id);
            let body = bytes_argument(env, &body);
            let _ = env.call_static_method(
                class,
                "onPush",
                "(Ljava/lang/String;II[B)V",
                &[
                    JValue::Object(&channel_id),
                    JValue::Int(cmdid as jint),
                    JValue::Int(taskid as jint),
                    JValue::Object(&body),
                ],
            );
            Answer::Nothing
        }
        Question::Req2Buf {
            taskid,
            channel_select,
            host,
            sequence,
        } => {
            let (Some(stream), Some(errcode)) = (byte_stream(env), int_out(env, 2)) else {
                return Answer::Nothing;
            };
            let Ok(host) = env.new_string(&host) else {
                return Answer::Nothing;
            };
            let host = JObject::from(host);
            let user_context = JObject::null();
            let errcode_argument = errcode.as_ref();
            let called = env.call_static_method(
                class,
                "req2Buf",
                "(ILjava/lang/Object;Ljava/io/ByteArrayOutputStream;[IILjava/lang/String;I)Z",
                &[
                    JValue::Int(taskid as jint),
                    JValue::Object(&user_context),
                    JValue::Object(&stream),
                    JValue::Object(errcode_argument),
                    JValue::Int(channel_select),
                    JValue::Object(&host),
                    JValue::Int(sequence as jint),
                ],
            );
            if !bool_of(called) {
                return Answer::Encoded(Err(int_at(env, &errcode, 0)));
            }
            Answer::Encoded(Ok(bytes_of(env, &stream)))
        }
        Question::Buf2Resp {
            taskid,
            body,
            channel_select,
        } => {
            let (Some(errcode), Some(sequence)) = (int_out(env, 1), int_out(env, 1)) else {
                return Answer::Nothing;
            };
            let body = bytes_argument(env, &body);
            let user_context = JObject::null();
            let errcode_argument = errcode.as_ref();
            let sequence_argument = sequence.as_ref();
            let called = env.call_static_method(
                class,
                "buf2Resp",
                "(ILjava/lang/Object;[B[II[I)I",
                &[
                    JValue::Int(taskid as jint),
                    JValue::Object(&user_context),
                    JValue::Object(&body),
                    JValue::Object(errcode_argument),
                    JValue::Int(channel_select),
                    JValue::Object(sequence_argument),
                ],
            );
            Answer::Decoded {
                handle: int_of(called),
                err_code: int_at(env, &errcode, 0),
            }
        }
        Question::OnTaskEnd {
            taskid,
            err_type,
            err_code,
            profile,
        } => {
            let Some(profile) = cgi_profile(env, &profile) else {
                return Answer::Nothing;
            };
            let user_context = JObject::null();
            let called = env.call_static_method(
                class,
                "onTaskEnd",
                "(ILjava/lang/Object;IILio/github/marsrs/stn/StnLogic$CgiProfile;)I",
                &[
                    JValue::Int(taskid as jint),
                    JValue::Object(&user_context),
                    JValue::Int(err_type as jint),
                    JValue::Int(err_code),
                    JValue::Object(&profile),
                ],
            );
            Answer::Ended(int_of(called))
        }
        Question::ReportConnectStatus { all, longlink } => {
            let _ = env.call_static_method(
                class,
                "reportConnectStatus",
                "(II)V",
                &[JValue::Int(all as jint), JValue::Int(longlink as jint)],
            );
            Answer::Nothing
        }
        Question::IdentifyCheckBuffer { channel_id } => {
            let (Some(buffer), Some(hash), Some(cmdids)) =
                (byte_stream(env), byte_stream(env), int_out(env, 1))
            else {
                return Answer::Nothing;
            };
            let Ok(channel_id) = env.new_string(&channel_id) else {
                return Answer::Nothing;
            };
            let channel_id = JObject::from(channel_id);
            let buffer_argument = buffer.as_ref();
            let hash_argument = hash.as_ref();
            let cmdids_argument = cmdids.as_ref();
            let called = env.call_static_method(
                class,
                "getLongLinkIdentifyCheckBuffer",
                concat!(
                    "(Ljava/lang/String;Ljava/io/ByteArrayOutputStream;",
                    "Ljava/io/ByteArrayOutputStream;[I)I"
                ),
                &[
                    JValue::Object(&channel_id),
                    JValue::Object(buffer_argument),
                    JValue::Object(hash_argument),
                    JValue::Object(cmdids_argument),
                ],
            );
            Answer::Identified {
                mode: int_of(called),
                buffer: bytes_of(env, buffer_argument),
                hash: bytes_of(env, hash_argument),
                cmdid: int_at(env, &cmdids, 0).max(0) as u32,
            }
        }
        Question::IdentifyResponse {
            channel_id,
            response,
            hash,
        } => {
            let Ok(channel_id) = env.new_string(&channel_id) else {
                return Answer::Nothing;
            };
            let channel_id = JObject::from(channel_id);
            let response = bytes_argument(env, &response);
            let hash = bytes_argument(env, &hash);
            let called = env.call_static_method(
                class,
                "onLongLinkIdentifyResp",
                "(Ljava/lang/String;[B[B)Z",
                &[
                    JValue::Object(&channel_id),
                    JValue::Object(&response),
                    JValue::Object(&hash),
                ],
            );
            Answer::Yes(bool_of(called))
        }
        Question::RequestSync => {
            let _ = env.call_static_method(class, "requestDoSync", "()V", &[]);
            Answer::Nothing
        }
        Question::NetCheckShortLinkHosts => {
            let called = env.call_static_method(
                class,
                "requestNetCheckShortLinkHosts",
                "()[Ljava/lang/String;",
                &[],
            );
            Answer::Ips(strings_of(env, called))
        }
        Question::ReportTaskProfile { json } => {
            let Ok(json) = env.new_string(&json) else {
                return Answer::Nothing;
            };
            let json = JObject::from(json);
            let _ = env.call_static_method(
                class,
                "reportTaskProfile",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&json)],
            );
            Answer::Nothing
        }
    }
}

/// A `Z` Java answered with — `false` for a call that could not be made.
fn bool_of(called: jni::errors::Result<JValueOwned>) -> bool {
    called.and_then(|value| value.z()).unwrap_or(false)
}

/// An `I` Java answered with — `0` for a call that could not be made.
fn int_of(called: jni::errors::Result<JValueOwned>) -> i32 {
    called.and_then(|value| value.i()).unwrap_or(0)
}

/// An `L` Java answered with — nothing for a call that could not be made, or
/// for one that answered `null`, which is the C++'s own `NULL` check.
fn object_of<'a>(called: jni::errors::Result<JValueOwned<'a>>) -> Option<JObject<'a>> {
    let object = called.and_then(|value| value.l()).ok()?;
    (!object.is_null()).then_some(object)
}

/// A `String` Java answered with — empty for a call that could not be made, or
/// for one that answered `null`, which is the C++'s `""` too.
fn string_of(env: &mut JNIEnv<'_>, called: jni::errors::Result<JValueOwned>) -> String {
    let Some(object) = object_of(called) else {
        return String::new();
    };
    let jstring = JString::from(object);
    let Ok(java) = env.get_string(&jstring) else {
        return String::new();
    };
    java.to_string_lossy().into_owned()
}

/// A `String[]` Java answered with.
fn strings_of(env: &mut JNIEnv<'_>, called: jni::errors::Result<JValueOwned>) -> Vec<String> {
    let Ok(array) = called.and_then(|value| value.l()) else {
        return Vec::new();
    };
    string_array(env, &array)
}

/// An `int[]` Java writes an answer into — the C++'s `env->NewIntArray`, which
/// is two ints wide for `req2Buf` and one everywhere else.
fn int_out<'a>(env: &mut JNIEnv<'a>, len: i32) -> Option<JIntArray<'a>> {
    env.new_int_array(len).ok()
}

fn int_at(env: &mut JNIEnv<'_>, array: &JIntArray<'_>, index: usize) -> i32 {
    let mut values = vec![0; index + 1];
    if env.get_int_array_region(array, 0, &mut values).is_err() {
        return 0;
    }
    values[index]
}

/// A `byte[]` argument — `null` for an empty one, which is what the C++ hands
/// over when the buffer it is carrying has nothing in it.
fn bytes_argument<'a>(env: &mut JNIEnv<'a>, bytes: &[u8]) -> JObject<'a> {
    if bytes.is_empty() {
        return JObject::null();
    }
    match env.byte_array_from_slice(bytes) {
        Ok(array) => JObject::from(array),
        Err(_) => JObject::null(),
    }
}

/// A `ByteArrayOutputStream` — the C++ hands one to a call that answers bytes
/// and reads it back with `toByteArray()`.
fn byte_stream<'a>(env: &mut JNIEnv<'a>) -> Option<JObject<'a>> {
    let Ok(class) = env.find_class("java/io/ByteArrayOutputStream") else {
        return None;
    };
    env.new_object(class, "()V", &[]).ok()
}

/// `toByteArray()` of one — empty for a stream Java never wrote to, which is
/// what the C++ ends up with too.
fn bytes_of(env: &mut JNIEnv<'_>, stream: &JObject<'_>) -> Vec<u8> {
    let Ok(bytes) = env.call_method(stream, "toByteArray", "()[B", &[]) else {
        return Vec::new();
    };
    let Ok(bytes) = bytes.l() else {
        return Vec::new();
    };
    env.convert_byte_array(JByteArray::from(bytes))
        .unwrap_or_default()
}

/// `StnLogic$CgiProfile`, the object `onTaskEnd` is handed: the C++'s ten
/// `SetLongField`/`SetIntField` calls.
///
/// Not read: `startHandshakeTime` and `handshakeSuccessfulTime`, which stay at
/// `0` — the port's [`CgiProfile`] keeps no tls handshake, because nothing in
/// the port writes one. Nor is the connect's `nettype`, which the Java class
/// has no field for.
fn cgi_profile<'a>(env: &mut JNIEnv<'a>, profile: &CgiProfile) -> Option<JObject<'a>> {
    let Ok(class) = env.find_class(STN_CGI_PROFILE) else {
        return None;
    };
    let Ok(object) = env.new_object(class, "()V", &[]) else {
        return None;
    };
    for (name, value) in [
        ("taskStartTime", profile.start_time as i64),
        ("startConnectTime", profile.start_connect_time as i64),
        (
            "connectSuccessfulTime",
            profile.connect_successful_time as i64,
        ),
        ("startSendPacketTime", profile.start_send_packet_time as i64),
        ("startReadPacketTime", profile.start_read_packet_time as i64),
        (
            "readPacketFinishedTime",
            profile.read_packet_finished_time as i64,
        ),
        ("rtt", profile.rtt as i64),
    ] {
        let _ = env.set_field(&object, name, "J", JValue::Long(value));
    }
    for (name, value) in [
        ("channelType", profile.channel_type),
        ("protocolType", profile.transport_protocol),
    ] {
        let _ = env.set_field(&object, name, "I", JValue::Int(value));
    }
    Some(object)
}

// #################### the questions the app is asked ####################

/// `AppLogic` — the class the C++'s four C2Java calls are static methods of.
/// Unlike [`STN_CALLBACK`], it forwards nothing: what it answers is the app's
/// own, which is why the native side never keeps the app itself.
const APP_LOGIC: &str = "io/github/marsrs/app/AppLogic";

/// One of the four `C2Java_*` functions of
/// `com_tencent_mars_app_AppLogic_C2Java.cc`, i.e. what
/// [`crate::app_logic::Ask::jvm`] asks: attach the thread, call the one static
/// method, and read the answer out of what Java handed back — a `String`, an
/// `int`, and the two objects `getAccountInfo` and `getDeviceType` answer with.
///
/// Without a VM (a host that linked the library instead of loading it from
/// Java) there is nobody to ask, and the answer is the one the port reads when
/// the app said nothing: no directory, nobody logged in, no version, no device.
/// Nothing here panics into Rust either way.
pub(crate) fn ask_app_logic(question: AppQuestion) -> AppAnswer {
    guard(|| {
        let Some(vm) = VM.get() else {
            return AppAnswer::Nothing;
        };
        let Ok(mut env) = vm.attach_current_thread() else {
            return AppAnswer::Nothing;
        };
        let Ok(class) = env.find_class(APP_LOGIC) else {
            return AppAnswer::Nothing;
        };
        ask_app(&mut env, class, question)
    })
}

fn ask_app<'a>(env: &mut JNIEnv<'a>, class: JClass<'a>, question: AppQuestion) -> AppAnswer {
    match question {
        AppQuestion::AppFilePath => {
            let called =
                env.call_static_method(class, "getAppFilePath", "()Ljava/lang/String;", &[]);
            AppAnswer::Path(string_of(env, called))
        }
        AppQuestion::AccountInfo => {
            // `AppLogic$AccountInfo` — `uin` and `userName`.
            let called = env.call_static_method(
                class,
                "getAccountInfo",
                "()Lio/github/marsrs/app/AppLogic$AccountInfo;",
                &[],
            );
            let Some(account) = object_of(called) else {
                return AppAnswer::Nothing;
            };
            AppAnswer::Account(AccountInfo::new(
                long_field(env, &account, "uin"),
                string_field(env, &account, "userName"),
            ))
        }
        AppQuestion::ClientVersion => {
            let called = env.call_static_method(class, "getClientVersion", "()I", &[]);
            AppAnswer::Version(int_of(called))
        }
        AppQuestion::DeviceInfo => {
            // `AppLogic$DeviceInfo` — `devicename` and `devicetype`.
            let called = env.call_static_method(
                class,
                "getDeviceType",
                "()Lio/github/marsrs/app/AppLogic$DeviceInfo;",
                &[],
            );
            let Some(device) = object_of(called) else {
                return AppAnswer::Nothing;
            };
            AppAnswer::Device(DeviceInfo::new(
                string_field(env, &device, "devicename"),
                string_field(env, &device, "devicetype"),
            ))
        }
    }
}

// #################### the questions the platform is asked ####################

/// `PlatformComm$C2Java` — the class the C++'s nine C2Java calls are static
/// methods of.
const PLATFORM_COMM: &str = "io/github/marsrs/comm/PlatformComm$C2Java";

/// One of the nine functions of `mars/comm/jni/platform_comm.cc`, i.e. what
/// [`crate::platform_comm::Ask::jvm`] asks: attach the thread, call the one
/// static method, and read the answer out of what Java handed back — an `int`,
/// a `long`, a `StringBuffer` the host is written into, and the three objects
/// `getCurWifiInfo`, `getCurSIMInfo` and `getAPNInfo` answer with.
///
/// Without a VM (a host that linked the library instead of loading it from
/// Java) there is nobody to ask, and the answer is the one the port reads when
/// the platform said nothing: offline, no proxy, no wifi, no SIM, no access
/// point, no signal. Nothing here panics into Rust either way.
pub(crate) fn ask_platform_comm(question: PlatformQuestion) -> PlatformAnswer {
    guard(|| {
        let Some(vm) = VM.get() else {
            return PlatformAnswer::Nothing;
        };
        let Ok(mut env) = vm.attach_current_thread() else {
            return PlatformAnswer::Nothing;
        };
        let Ok(class) = env.find_class(PLATFORM_COMM) else {
            return PlatformAnswer::Nothing;
        };
        ask_platform(&mut env, class, question)
    })
}

fn ask_platform<'a>(
    env: &mut JNIEnv<'a>,
    class: JClass<'a>,
    question: PlatformQuestion,
) -> PlatformAnswer {
    match question {
        PlatformQuestion::NetInfo => {
            let called = env.call_static_method(class, "getNetInfo", "()I", &[]);
            PlatformAnswer::NetInfo(NetInfo::of(int_of(called)))
        }
        PlatformQuestion::StatisticsNetType => {
            let called = env.call_static_method(class, "getStatisticsNetType", "()I", &[]);
            PlatformAnswer::StatisticsNetType(NetType::of(int_of(called)))
        }
        PlatformQuestion::ProxyInfo => {
            // the host comes back in the buffer Java was handed, the port in
            // what the call answered
            let Some(buffer) = string_buffer(env) else {
                return PlatformAnswer::Nothing;
            };
            let argument = JValue::Object(buffer.as_ref());
            let called = env.call_static_method(
                class,
                "getProxyInfo",
                "(Ljava/lang/StringBuffer;)I",
                &[argument],
            );
            let port = int_of(called);
            let called = env.call_method(&buffer, "toString", "()Ljava/lang/String;", &[]);
            PlatformAnswer::Proxy {
                port,
                host: string_of(env, called),
            }
        }
        PlatformQuestion::WifiInfo => {
            // `PlatformComm$WifiInfo` — `ssid` and `bssid`.
            let called = env.call_static_method(
                class,
                "getCurWifiInfo",
                "()Lio/github/marsrs/comm/PlatformComm$WifiInfo;",
                &[],
            );
            let Some(wifi) = object_of(called) else {
                return PlatformAnswer::Nothing;
            };
            PlatformAnswer::Wifi(Some(WifiInfo {
                ssid: string_field(env, &wifi, "ssid"),
                bssid: string_field(env, &wifi, "bssid"),
            }))
        }
        PlatformQuestion::SimInfo => {
            // `PlatformComm$SIMInfo` — `ispCode` and `ispName`, both strings:
            // the Java writes `"" + ispCode`.
            let called = env.call_static_method(
                class,
                "getCurSIMInfo",
                "()Lio/github/marsrs/comm/PlatformComm$SIMInfo;",
                &[],
            );
            let Some(sim) = object_of(called) else {
                return PlatformAnswer::Nothing;
            };
            PlatformAnswer::Sim(Some(SimInfo {
                isp_code: string_field(env, &sim, "ispCode"),
                isp_name: string_field(env, &sim, "ispName"),
            }))
        }
        PlatformQuestion::ApnInfo => {
            // `PlatformComm$APNInfo` — `netType`, `subNetType` and `extraInfo`.
            let called = env.call_static_method(
                class,
                "getAPNInfo",
                "()Lio/github/marsrs/comm/PlatformComm$APNInfo;",
                &[],
            );
            let Some(apn) = object_of(called) else {
                return PlatformAnswer::Nothing;
            };
            PlatformAnswer::Apn(Some(ApnInfo {
                net_type: int_field(env, &apn, "netType"),
                sub_net_type: int_field(env, &apn, "subNetType"),
                extra_info: string_field(env, &apn, "extraInfo"),
            }))
        }
        PlatformQuestion::RadioAccessNetwork => {
            let called = env.call_static_method(class, "getCurRadioAccessNetworkInfo", "()I", &[]);
            PlatformAnswer::RadioAccessNetwork(int_of(called))
        }
        PlatformQuestion::Signal { wifi } => {
            let called = env.call_static_method(
                class,
                "getSignal",
                "(Z)J",
                &[JValue::Bool(wifi as jboolean)],
            );
            PlatformAnswer::Signal(long_of(called))
        }
        PlatformQuestion::NetworkConnected => {
            let called = env.call_static_method(class, "isNetworkConnected", "()Z", &[]);
            PlatformAnswer::Connected(bool_of(called))
        }
    }
}

/// A `StringBuffer` Java writes an answer into — the C++'s own
/// `NewObject(StringBuffer)`, handed to `getProxyInfo` and read back with
/// `toString()`.
fn string_buffer<'a>(env: &mut JNIEnv<'a>) -> Option<JObject<'a>> {
    let Ok(class) = env.find_class("java/lang/StringBuffer") else {
        return None;
    };
    env.new_object(class, "()V", &[]).ok()
}

/// A `J` Java answered with — `0` for a call that could not be made.
fn long_of(called: jni::errors::Result<JValueOwned>) -> i64 {
    called.and_then(|value| value.j()).unwrap_or(0)
}

// #################### io.github.marsrs.BaseEvent ####################

/// `BaseEvent.onCreate` — the app is up, which is when the net core is made.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_BaseEvent_onCreate<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    guard(|| {
        on_create_impl();
    })
}

/// `BaseEvent.onInitConfigBeforeOnCreate`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_BaseEvent_onInitConfigBeforeOnCreate<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    packer_encoder_version: jint,
) {
    guard(|| on_init_config_before_on_create_impl(packer_encoder_version))
}

/// `BaseEvent.onDestroy`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_BaseEvent_onDestroy<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    guard(|| {
        on_destroy_impl();
    })
}

/// `BaseEvent.onForeground`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_BaseEvent_onForeground<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    is_foreground: jboolean,
) {
    guard(|| on_foreground_impl(is_foreground != 0))
}

/// `BaseEvent.onNetworkChange`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_BaseEvent_onNetworkChange<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    guard(on_network_change_impl)
}

/// `BaseEvent.onSingalCrash` — the signal number is not the port's to handle,
/// so it is not read; what the crash does is close the appender.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_BaseEvent_onSingalCrash<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    _sig: jint,
) {
    guard(|| on_signal_crash_impl(_sig))
}

/// `BaseEvent.onExceptionCrash`.
#[no_mangle]
pub extern "system" fn Java_io_github_marsrs_BaseEvent_onExceptionCrash<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    guard(on_exception_crash_impl)
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
