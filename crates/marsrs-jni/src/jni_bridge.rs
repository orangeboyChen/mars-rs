//! The JVM boundary: the entry points Java calls and the readers that turn
//! Java objects into Rust values.
//!
//! Everything here needs a live `Env`, so it can only be exercised from Java
//! (the Android AAR CI builds). The logic behind it lives in the crate root and
//! is covered by `cargo test`, which is why this file is left out of the
//! coverage measurement.

use jni::objects::{
    JByteArray, JClass, JIntArray, JObject, JObjectArray, JString, JValue, JValueOwned,
};
use jni::signature::MethodSignature;
use jni::strings::{JNIStr, MUTF8Chars};
use jni::sys::{jboolean, jint, jlong, jobject, JNI_VERSION_1_6};
use jni::{jni_sig, jni_str};
use jni::{Env, EnvUnowned, JavaVM};

use marsrs_appender::{AppenderMode, CompressMode, LogLevel, XLogConfig, XLoggerInfo};
use marsrs_stn::{CgiProfile, Task};

use std::borrow::Cow;
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
    clear_task_impl, due_time_impl, gen_sequence_id_impl, gen_task_id_impl,
    get_load_libraries_impl, has_task_impl, keep_signalling_impl, makesure_longlink_connected_impl,
    redo_task_impl, reset_and_init_encoder_version_impl, reset_impl, run_pending_impl,
    set_backup_ips_impl, set_client_version_impl, set_debug_ip_impl, set_longlink_svr_addr_impl,
    set_shortlink_svr_addr_impl, set_signalling_strategy_impl, start_task_impl,
    stop_signalling_impl, stop_task_impl, touch_tasks_impl, trig_nooping_impl,
};

use crate::{
    close_impl, flush_impl, get_instance_impl, get_level_impl, guard, level_from_java,
    log_write_impl, new_instance_impl, now_timeval, open_appender, release_instance_impl,
    set_appender_mode_impl, set_console_log_open_impl, set_level_impl, set_max_alive_time_impl,
    set_max_file_size_impl, write_impl,
};

/// Runs `f` with an [`Env`], which is what an entry point has to go through
/// now: the handle the JVM hands a native method is an [`EnvUnowned`], and
/// none of the calls below live on it — they live on the `Env` borrowed from
/// it for the length of the call.
///
/// [`guard`] still holds inside: a panic or a JNI error is the default value
/// of `T`, never an unwind into the JVM, and the panic still says so on
/// stderr.
fn guard_env<'local, T, F>(env: &mut EnvUnowned<'local>, f: F) -> T
where
    T: Default,
    F: FnOnce(&mut Env<'local>) -> T,
{
    match env
        .with_env(|env| Ok::<_, jni::errors::Error>(guard(|| f(env))))
        .into_outcome()
    {
        jni::Outcome::Ok(value) => value,
        jni::Outcome::Err(_) | jni::Outcome::Panic(_) => T::default(),
    }
}

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
///
/// The VM arrives as the raw `JavaVM*` the JNI header declares, because a
/// `jni::JavaVM` by value is not FFI-safe to hand across the boundary.
///
/// # Safety
///
/// `vm` has to be the live VM the JVM hands `JNI_OnLoad`: nothing checks it
/// here, and it is kept until the process goes away.
#[no_mangle]
pub unsafe extern "system" fn JNI_OnLoad(
    vm: *mut jni::sys::JavaVM,
    _reserved: *mut std::ffi::c_void,
) -> jint {
    let _ = VM.set(JavaVM::from_raw(vm));
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
        // Attaching lends the `Env` to a closure now rather than handing back
        // a guard, so the whole call moves inside it.
        let _ = vm.attach_current_thread(|env| {
            let class = env.find_class(jni_str!("io/github/orangeboychen/marsrs/sdt/SdtLogic"))?;
            let message = JObject::from(env.new_string(&json)?);
            env.call_static_method(
                class,
                jni_str!("reportSignalDetectResults"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::Object(&message)],
            )?;
            Ok::<_, jni::errors::Error>(())
        });
    })
}

fn int_field(env: &mut Env<'_>, obj: &JObject<'_>, name: &JNIStr) -> i32 {
    guard(|| {
        env.get_field(obj, name, jni_sig!("I"))
            .and_then(|value| value.i())
            .unwrap_or(0)
    })
}

fn long_field(env: &mut Env<'_>, obj: &JObject<'_>, name: &JNIStr) -> i64 {
    guard(|| {
        env.get_field(obj, name, jni_sig!("J"))
            .and_then(|value| value.j())
            .unwrap_or(0)
    })
}

/// The `java.lang.String` behind `value`, or `None` when it is null.
///
/// A handle to *borrow* the characters from (see [`borrowed_str`]), not a copy:
/// `Java2C_Xlog.cc` hands the appender the `char*` of a `ScopedJstring`, and a
/// `String` per field per record was three allocations it never makes.
fn java_string_handle<'local>(
    env: &Env<'local>,
    value: &JObject<'local>,
) -> Option<JString<'local>> {
    if value.is_null() {
        return None;
    }
    // `JObject` is a borrowed handle: re-wrap the same raw reference without
    // taking ownership of the local ref.
    Some(unsafe { JString::from_raw(env, value.as_raw()) })
}

/// The characters of a borrowed `MUTF8Chars`, or `""` when it is absent.
///
/// Valid UTF-8 comes back borrowed (Java's modified UTF-8 only differs for
/// supplementary characters, and those are converted lossily, exactly like the
/// old `to_string_lossy().into_owned()` did).
fn borrowed_str<'a>(java_str: Option<&'a MUTF8Chars<'a, &JString<'a>>>) -> Cow<'a, str> {
    java_str.map_or(Cow::Borrowed(""), |java_str| java_str.to_str())
}

/// [`java_string_handle`] for a `java.lang.String` *field* of `obj`.
fn string_field_handle<'local>(
    env: &mut Env<'local>,
    obj: &JObject<'_>,
    name: &JNIStr,
) -> Option<JString<'local>> {
    guard(|| {
        let Ok(field) = env.get_field(obj, name, jni_sig!("Ljava/lang/String;")) else {
            return None;
        };
        let Ok(object) = field.l() else {
            return None;
        };
        java_string_handle(env, &object)
    })
}

fn string_field(env: &mut Env<'_>, obj: &JObject<'_>, name: &JNIStr) -> String {
    guard(|| {
        let Ok(field) = env.get_field(obj, name, jni_sig!("Ljava/lang/String;")) else {
            return String::new();
        };
        let Ok(object) = field.l() else {
            return String::new();
        };
        if object.is_null() {
            return String::new();
        }
        let jstring = unsafe { JString::from_raw(env, object.as_raw()) };
        let Ok(java_str) = jstring.mutf8_chars(env) else {
            return String::new();
        };
        java_str.to_str().into_owned()
    })
}

/// Reads `io.github.orangeboychen.marsrs.xlog.Xlog$XLogConfig`.
fn config_from_java(env: &mut Env<'_>, config: &JObject<'_>) -> Option<(XLogConfig, LogLevel)> {
    if config.is_null() {
        return None;
    }

    let mode = match int_field(env, config, jni_str!("mode")) {
        0 => AppenderMode::Async,
        1 => AppenderMode::Sync,
        _ => return None,
    };
    let compress_mode = match int_field(env, config, jni_str!("compressmode")) {
        0 => CompressMode::Zlib,
        _ => CompressMode::Zstd,
    };

    let logdir = string_field(env, config, jni_str!("logdir"));
    if logdir.is_empty() {
        return None;
    }
    let cachedir = string_field(env, config, jni_str!("cachedir"));

    Some((
        XLogConfig {
            mode,
            logdir: std::path::PathBuf::from(logdir),
            nameprefix: string_field(env, config, jni_str!("nameprefix")),
            pub_key: string_field(env, config, jni_str!("pubkey")),
            compress_mode,
            compress_level: int_field(env, config, jni_str!("compresslevel")),
            cachedir: if cachedir.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(cachedir))
            },
            cache_days: int_field(env, config, jni_str!("cachedays")).max(0) as u32,
        },
        level_from_java(int_field(env, config, jni_str!("level"))),
    ))
}

fn java_string(env: &mut Env<'_>, value: &JObject<'_>) -> String {
    guard(|| {
        if value.is_null() {
            return String::new();
        }
        // `JObject` is a borrowed handle: re-wrap the same raw reference
        // without taking ownership of the local ref.
        let jstring = unsafe { JString::from_raw(env, value.as_raw()) };
        jstring
            .mutf8_chars(env)
            .map(|java_str| java_str.to_str().into_owned())
            .unwrap_or_default()
    })
}

#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_appenderOpen<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    config: JObject<'local>,
) {
    guard_env(&mut env, |env| {
        let Some((config, level)) = config_from_java(env, &config) else {
            return;
        };
        open_appender(config, level);
    })
}

/// `Xlog.appenderClose`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_appenderClose<'local>(
    _env: EnvUnowned<'local>,
    _this: JObject<'local>,
) {
    guard(close_impl)
}

/// `Xlog.appenderFlush`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_appenderFlush<'local>(
    _env: EnvUnowned<'local>,
    _this: JObject<'local>,
    instance: jlong,
    is_sync: jboolean,
) {
    guard(|| flush_impl(instance as u64, is_sync))
}

/// `Xlog.newXlogInstance` — returns the handle, or `0` on a bad config.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_newXlogInstance<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    config: JObject<'local>,
) -> jlong {
    guard_env(&mut env, |env| match config_from_java(env, &config) {
        Some((config, level)) => new_instance_impl(config, level),
        None => 0,
    })
}

/// `Xlog.getXlogInstance` — the handle for `nameprefix`, or `0`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_getXlogInstance<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    nameprefix: JString<'local>,
) -> jlong {
    guard_env(&mut env, |env| {
        let prefix = nameprefix
            .mutf8_chars(env)
            .map(|value| value.to_str().into_owned())
            .unwrap_or_default();
        get_instance_impl(&prefix)
    })
}

/// `Xlog.releaseXlogInstance`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_releaseXlogInstance<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    nameprefix: JString<'local>,
) {
    guard_env(&mut env, |env| {
        let prefix = nameprefix
            .mutf8_chars(env)
            .map(|value| value.to_str().into_owned())
            .unwrap_or_default();
        release_instance_impl(&prefix);
    })
}

/// `Xlog.logWrite` — writes through the process-wide appender.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_logWrite<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    info: JObject<'local>,
    log: JString<'local>,
) {
    guard_env(&mut env, |env| {
        let log = log
            .mutf8_chars(env)
            .map(|value| value.to_str().into_owned())
            .unwrap_or_default();
        if info.is_null() {
            log_write_impl(None, &log);
            return;
        }
        let level = level_from_java(int_field(env, &info, jni_str!("level")));
        let line = int_field(env, &info, jni_str!("line"));
        // -1 makes the category fill these in from the OS; Java passes real
        // values, which the port keeps.
        let pid = long_field(env, &info, jni_str!("pid"));
        let tid = long_field(env, &info, jni_str!("tid"));
        let maintid = long_field(env, &info, jni_str!("maintid"));

        // The three strings are borrowed from the JVM instead of copied into
        // `String`s: an `MUTF8Chars` keeps the characters alive for as long as
        // the record needs them, which is this call.
        let tag = string_field_handle(env, &info, jni_str!("tag"));
        let filename = string_field_handle(env, &info, jni_str!("filename"));
        let funcname = string_field_handle(env, &info, jni_str!("funcname"));
        let tag = tag.as_ref().and_then(|value| value.mutf8_chars(env).ok());
        let filename = filename
            .as_ref()
            .and_then(|value| value.mutf8_chars(env).ok());
        let funcname = funcname
            .as_ref()
            .and_then(|value| value.mutf8_chars(env).ok());

        let info = XLoggerInfo {
            level,
            tag: Some(borrowed_str(tag.as_ref())),
            filename: Some(borrowed_str(filename.as_ref())),
            func_name: Some(borrowed_str(funcname.as_ref())),
            line,
            pid,
            tid,
            maintid,
            timeval: now_timeval(),
        };
        log_write_impl(Some(info), &log);
    })
}

/// `Xlog.write` — the write of the Kotlin API: one record in one JNI call,
/// with the level filter in `write_impl` and no `XLoggerInfo` for the
/// caller to fill in.
///
/// `Xlog.logWrite` is the write the C++ project's Java spelled, and it is what
/// the process-wide appender still writes through; `Xlog` writes through this
/// one now, whichever API asked.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_write<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    instance: jlong,
    level: jint,
    tag: JString<'local>,
    log: JString<'local>,
) {
    guard_env(&mut env, |env| {
        let log = log
            .mutf8_chars(env)
            .map(|value| value.to_str().into_owned())
            .unwrap_or_default();
        // Borrowed from the JVM, like `logWrite` does: a `String` per record is
        // an allocation `Java2C_Xlog.cc` never makes.
        let tag = java_string_handle(env, tag.as_ref());
        let tag = tag.as_ref().and_then(|value| value.mutf8_chars(env).ok());
        let _ = write_impl(
            instance as u64,
            level_from_java(level),
            borrowed_str(tag.as_ref()),
            &log,
        );
    })
}

/// `Xlog.getLogLevel` — `-1` for an unknown handle.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_getLogLevel(
    _env: EnvUnowned<'_>,
    _this: JObject<'_>,
    instance: jlong,
) -> jint {
    guard(|| get_level_impl(instance as u64))
}

/// `Xlog.setLogLevel`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_setLogLevel(
    _env: EnvUnowned<'_>,
    _this: JObject<'_>,
    instance: jlong,
    level: jint,
) {
    guard(|| set_level_impl(instance as u64, level))
}

/// `Xlog.setAppenderMode`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_setAppenderMode(
    _env: EnvUnowned<'_>,
    _this: JObject<'_>,
    instance: jlong,
    mode: jint,
) {
    guard(|| set_appender_mode_impl(instance as u64, mode))
}

/// `Xlog.setConsoleLogOpen`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_setConsoleLogOpen(
    _env: EnvUnowned<'_>,
    _this: JObject<'_>,
    instance: jlong,
    is_open: jboolean,
) {
    guard(|| set_console_log_open_impl(instance as u64, is_open))
}

/// `Xlog.setMaxFileSize`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_setMaxFileSize(
    _env: EnvUnowned<'_>,
    _this: JObject<'_>,
    instance: jlong,
    size: jlong,
) {
    guard(|| set_max_file_size_impl(instance as u64, size))
}

/// `Xlog.setMaxAliveTime`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_xlog_Xlog_setMaxAliveTime(
    _env: EnvUnowned<'_>,
    _this: JObject<'_>,
    instance: jlong,
    seconds: jlong,
) {
    guard(|| set_max_alive_time_impl(instance as u64, seconds))
}

// #################### io.github.orangeboychen.marsrs.stn.StnLogic ####################

/// Reads a Java `int[]`.
fn int_array(env: &mut Env<'_>, array: &JObject<'_>) -> Vec<i32> {
    if array.is_null() {
        return Vec::new();
    }
    // Re-borrow the handle as an `int[]` without touching the local ref.
    let array = unsafe { JIntArray::from_raw(env, array.as_raw()) };
    let Ok(len) = array.len(env) else {
        return Vec::new();
    };
    if len == 0 {
        return Vec::new();
    }
    let mut ports = vec![0; len];
    if array.get_region(env, 0, &mut ports).is_err() {
        return Vec::new();
    }
    ports
}

/// Reads a Java `String[]`.
fn string_array(env: &mut Env<'_>, array: &JObject<'_>) -> Vec<String> {
    if array.is_null() {
        return Vec::new();
    }
    let array = unsafe { JObjectArray::<'_, JObject<'_>>::from_raw(env, array.as_raw()) };
    let Ok(len) = array.len(env) else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for index in 0..len {
        let Ok(element) = array.get_element(env, index) else {
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
fn string_list(env: &mut Env<'_>, list: &JObject<'_>) -> Vec<String> {
    if list.is_null() {
        return Vec::new();
    }
    let Ok(len) = env
        .call_method(list, jni_str!("size"), jni_sig!("()I"), &[])
        .and_then(|value| value.i())
    else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for index in 0..len {
        let Ok(element) = env.call_method(
            list,
            jni_str!("get"),
            jni_sig!("(I)Ljava/lang/Object;"),
            &[JValue::Int(index)],
        ) else {
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
fn string_map(env: &mut Env<'_>, map: &JObject<'_>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if map.is_null() {
        return out;
    }
    let Ok(entries) = env.call_method(
        map,
        jni_str!("entrySet"),
        jni_sig!("()Ljava/util/Set;"),
        &[],
    ) else {
        return out;
    };
    let Ok(entries) = entries.l() else {
        return out;
    };
    let Ok(iterator) = env.call_method(
        &entries,
        jni_str!("iterator"),
        jni_sig!("()Ljava/util/Iterator;"),
        &[],
    ) else {
        return out;
    };
    let Ok(iterator) = iterator.l() else {
        return out;
    };
    loop {
        let Ok(has_next) = env
            .call_method(&iterator, jni_str!("hasNext"), jni_sig!("()Z"), &[])
            .and_then(|value| value.z())
        else {
            return out;
        };
        if !has_next {
            return out;
        }
        let Ok(entry) = env.call_method(
            &iterator,
            jni_str!("next"),
            jni_sig!("()Ljava/lang/Object;"),
            &[],
        ) else {
            return out;
        };
        let Ok(entry) = entry.l() else {
            return out;
        };
        let key = env
            .call_method(
                &entry,
                jni_str!("getKey"),
                jni_sig!("()Ljava/lang/Object;"),
                &[],
            )
            .and_then(|value| value.l())
            .map(|key| java_string(env, &key))
            .unwrap_or_default();
        let value = env
            .call_method(
                &entry,
                jni_str!("getValue"),
                jni_sig!("()Ljava/lang/Object;"),
                &[],
            )
            .and_then(|value| value.l())
            .map(|value| java_string(env, &value))
            .unwrap_or_default();
        out.insert(key, value);
    }
}

/// Reads `io.github.orangeboychen.marsrs.stn.StnLogic$Task` into a [`Task`]. The fields the
/// C++ reads are the ones below; the rest of the Java class has no counterpart
/// in the port's `Task`.
fn task_from_java(env: &mut Env<'_>, task: &JObject<'_>) -> Option<Task> {
    if task.is_null() {
        return None;
    }
    let taskid = int_field(env, task, jni_str!("taskID"));
    if taskid < 0 {
        return None;
    }
    let cmdid = int_field(env, task, jni_str!("cmdID"));
    let mut parsed = Task::new(taskid as u32, cmdid as u32);

    parsed.channel_select = int_field(env, task, jni_str!("channelSelect"));
    parsed.cgi = string_field(env, task, jni_str!("cgi"));
    parsed.send_only = bool_field(env, task, jni_str!("sendOnly"));
    parsed.need_authed = bool_field(env, task, jni_str!("needAuthed"));
    parsed.limit_flow = bool_field(env, task, jni_str!("limitFlow"));
    parsed.limit_frequency = bool_field(env, task, jni_str!("limitFrequency"));
    parsed.channel_strategy = int_field(env, task, jni_str!("channelStrategy"));
    parsed.network_status_sensitive = bool_field(env, task, jni_str!("networkStatusSensitive"));
    parsed.priority = int_field(env, task, jni_str!("priority"));
    parsed.retry_count = int_field(env, task, jni_str!("retryCount"));
    parsed.server_process_cost = int_field(env, task, jni_str!("serverProcessCost"));
    parsed.total_timeout = int_field(env, task, jni_str!("totalTimeout"));
    parsed.report_arg = string_field(env, task, jni_str!("reportArg"));
    parsed.long_polling = bool_field(env, task, jni_str!("longPolling"));
    parsed.long_polling_timeout = int_field(env, task, jni_str!("longPollingTimeout"));
    parsed.client_sequence_id = int_field(env, task, jni_str!("clientSequenceId")) as u16;
    parsed.shortlink_host_list = match env.get_field(
        task,
        jni_str!("shortLinkHostList"),
        jni_sig!("Ljava/util/List;"),
    ) {
        Ok(field) => field
            .l()
            .map(|list| string_list(env, &list))
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    parsed.headers = match env.get_field(task, jni_str!("headers"), jni_sig!("Ljava/util/Map;")) {
        Ok(field) => field
            .l()
            .map(|map| string_map(env, &map))
            .unwrap_or_default(),
        Err(_) => BTreeMap::new(),
    };
    Some(parsed)
}

fn bool_field(env: &mut Env<'_>, obj: &JObject<'_>, name: &JNIStr) -> bool {
    guard(|| {
        env.get_field(obj, name, jni_sig!("Z"))
            .and_then(|value| value.z())
            .unwrap_or(false)
    })
}

/// Builds a Java `ArrayList<String>`.
fn string_array_list(env: &mut Env<'_>, values: &[String]) -> jobject {
    let Ok(class) = env.find_class(jni_str!("java/util/ArrayList")) else {
        return std::ptr::null_mut();
    };
    let Ok(list) = env.new_object(class, jni_sig!("()V"), &[]) else {
        return std::ptr::null_mut();
    };
    for value in values {
        let Ok(text) = env.new_string(value) else {
            continue;
        };
        let _ = env.call_method(
            &list,
            jni_str!("add"),
            jni_sig!("(Ljava/lang/Object;)Z"),
            &[JValue::Object(&JObject::from(text))],
        );
    }
    list.into_raw()
}

/// `StnLogic.reset`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_reset<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    guard(reset_impl)
}

/// `StnLogic.resetAndInitEncoderVersion`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_resetAndInitEncoderVersion<
    'local,
>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    version: jint,
    name: JString<'local>,
) {
    guard_env(&mut env, |env| {
        let name = name
            .mutf8_chars(env)
            .map(|value| value.to_str().into_owned())
            .unwrap_or_default();
        reset_and_init_encoder_version_impl(version, &name);
    })
}

/// `StnLogic.setLonglinkSvrAddr` — the `int[]` arrives as a plain object.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_setLonglinkSvrAddr<
    'local,
>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    host: JString<'local>,
    ports: JObject<'local>,
    debug_ip: JString<'local>,
) {
    guard_env(&mut env, |env| {
        let host = host
            .mutf8_chars(env)
            .map(|value| value.to_str().into_owned())
            .unwrap_or_default();
        let debug_ip = debug_ip
            .mutf8_chars(env)
            .map(|value| value.to_str().into_owned())
            .unwrap_or_default();
        set_longlink_svr_addr_impl(&host, &int_array(env, &ports), &debug_ip);
    })
}

/// `StnLogic.setShortlinkSvrAddr`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_setShortlinkSvrAddr<
    'local,
>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    port: jint,
    debug_ip: JString<'local>,
) {
    guard_env(&mut env, |env| {
        let debug_ip = debug_ip
            .mutf8_chars(env)
            .map(|value| value.to_str().into_owned())
            .unwrap_or_default();
        set_shortlink_svr_addr_impl(port, &debug_ip);
    })
}

/// `StnLogic.setDebugIP`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_setDebugIP<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    host: JString<'local>,
    ip: JString<'local>,
) {
    guard_env(&mut env, |env| {
        let host = host
            .mutf8_chars(env)
            .map(|value| value.to_str().into_owned())
            .unwrap_or_default();
        let ip = ip
            .mutf8_chars(env)
            .map(|value| value.to_str().into_owned())
            .unwrap_or_default();
        set_debug_ip_impl(&host, &ip);
    })
}

/// `StnLogic.setBackupIPs`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_setBackupIPs<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    host: JString<'local>,
    ips: JObject<'local>,
) {
    guard_env(&mut env, |env| {
        let host = host
            .mutf8_chars(env)
            .map(|value| value.to_str().into_owned())
            .unwrap_or_default();
        set_backup_ips_impl(&host, &string_array(env, &ips));
    })
}

/// `StnLogic.startTask`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_startTask<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    task: JObject<'local>,
) {
    guard_env(&mut env, |env| {
        if let Some(task) = task_from_java(env, &task) {
            start_task_impl(task);
        }
    })
}

/// `StnLogic.stopTask`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_stopTask<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    taskid: jint,
) {
    guard(|| {
        stop_task_impl(taskid.max(0) as u32);
    })
}

/// `StnLogic.hasTask`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_hasTask<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    taskid: jint,
) -> jboolean {
    guard(|| has_task_impl(taskid.max(0) as u32))
}

/// `StnLogic.redoTask`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_redoTask<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    guard(|| {
        redo_task_impl();
    })
}

/// `StnLogic.touchTasks`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_touchTasks<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    guard(touch_tasks_impl)
}

/// `StnLogic.clearTask`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_clearTask<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    guard(|| {
        clear_task_impl();
    })
}

/// `StnLogic.runPending` — what the C++'s message queue thread would have done,
/// as one pass the host's loop makes.
///
/// The C++ runs this on a thread of its own; this port has none, so it is the
/// app's loop that calls it — `StnLogic.dueTime` is when. Declared by no Java of
/// the C++'s, because the C++ has no need of one: its own thread is the caller.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_runPending<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    guard(run_pending_impl)
}

/// `StnLogic.dueTime` — when the host's loop is to call `StnLogic.runPending`
/// again, as a `gettickcount()`; `-1` is nothing to wait for.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_dueTime<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> jlong {
    guard(|| due_time_impl().map_or(-1, |due| due as jlong))
}

/// `StnLogic.makesureLongLinkConnected`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_makesureLongLinkConnected<
    'local,
>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    guard(|| {
        makesure_longlink_connected_impl();
    })
}

/// `StnLogic.setSignallingStrategy`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_setSignallingStrategy<
    'local,
>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    period: jlong,
    keep_time: jlong,
) {
    guard(|| set_signalling_strategy_impl(period, keep_time))
}

/// `StnLogic.keepSignalling`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_keepSignalling<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    guard(keep_signalling_impl)
}

/// `StnLogic.stopSignalling`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_stopSignalling<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    guard(stop_signalling_impl)
}

/// `StnLogic.setClientVersion`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_setClientVersion<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    version: jint,
) {
    guard(|| set_client_version_impl(version.max(0) as u32))
}

/// `StnLogic.genTaskID`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_genTaskID<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> jint {
    guard(|| gen_task_id_impl() as jint)
}

/// `StnLogic.genSequenceId` — declared by the C++ but not by the Java class.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_genSequenceId<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> jint {
    guard(|| gen_sequence_id_impl() as jint)
}

/// `StnLogic.trigNooping` — declared by the C++ but not by the Java class.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_trigNooping<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    guard(trig_nooping_impl)
}

/// `StnLogic.getLoadLibraries`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_stn_StnLogic_getLoadLibraries<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> jobject {
    guard_env(&mut env, |env| {
        string_array_list(env, &get_load_libraries_impl())
    })
}

// #################### the questions STN asks Java ####################

/// `StnLogic` — the class the C++'s thirteen C2Java calls are static methods
/// of. Every one of them forwards to the `ICallBack` the app handed to
/// `setCallBack`, which is why the native side never keeps the app itself.
const STN_CALLBACK: &JNIStr = jni_str!("io/github/orangeboychen/marsrs/stn/StnLogic");

/// `StnLogic$CgiProfile` — the object `onTaskEnd` is handed.
const STN_CGI_PROFILE: &JNIStr = jni_str!("io/github/orangeboychen/marsrs/stn/StnLogic$CgiProfile");

/// `getLongLinkIdentifyCheckBuffer(String, ByteArrayOutputStream,
/// ByteArrayOutputStream, int[])`, kept up here because it does not fit on the
/// line it is called from.
const GET_LONG_LINK_IDENTIFY_CHECK_BUFFER_SIG: MethodSignature = jni_sig!(
    "(Ljava/lang/String;Ljava/io/ByteArrayOutputStream;Ljava/io/ByteArrayOutputStream;[I)I"
);

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
        vm.attach_current_thread(|env| -> jni::errors::Result<Answer> {
            let Ok(class) = env.find_class(STN_CALLBACK) else {
                return Ok(Answer::Nothing);
            };
            Ok(ask_stn(env, class, question))
        })
        .unwrap_or(Answer::Nothing)
    })
}

fn ask_stn<'a>(env: &mut Env<'a>, class: JClass<'a>, question: Question) -> Answer {
    match question {
        Question::MakesureAuthed { host } => {
            let Ok(host) = env.new_string(&host) else {
                return Answer::Nothing;
            };
            let host = JObject::from(host);
            let called = env.call_static_method(
                class,
                jni_str!("makesureAuthed"),
                jni_sig!("(Ljava/lang/String;)Z"),
                &[JValue::Object(&host)],
            );
            Answer::Yes(bool_of(called))
        }
        Question::TrafficData { send, recv } => {
            let _ = env.call_static_method(
                class,
                jni_str!("trafficData"),
                jni_sig!("(II)V"),
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
                jni_str!("onNewDns"),
                jni_sig!("(Ljava/lang/String;)[Ljava/lang/String;"),
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
                jni_str!("onPush"),
                jni_sig!("(Ljava/lang/String;II[B)V"),
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
                jni_str!("req2Buf"),
                jni_sig!(
                    "(ILjava/lang/Object;Ljava/io/ByteArrayOutputStream;[IILjava/lang/String;I)Z"
                ),
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
                jni_str!("buf2Resp"),
                jni_sig!("(ILjava/lang/Object;[B[II[I)I"),
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
                class, jni_str!("onTaskEnd"),
                jni_sig!("(ILjava/lang/Object;IILio/github/orangeboychen/marsrs/stn/StnLogic$CgiProfile;)I"),
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
                jni_str!("reportConnectStatus"),
                jni_sig!("(II)V"),
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
                jni_str!("getLongLinkIdentifyCheckBuffer"),
                GET_LONG_LINK_IDENTIFY_CHECK_BUFFER_SIG,
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
                jni_str!("onLongLinkIdentifyResp"),
                jni_sig!("(Ljava/lang/String;[B[B)Z"),
                &[
                    JValue::Object(&channel_id),
                    JValue::Object(&response),
                    JValue::Object(&hash),
                ],
            );
            Answer::Yes(bool_of(called))
        }
        Question::RequestSync => {
            let _ = env.call_static_method(class, jni_str!("requestDoSync"), jni_sig!("()V"), &[]);
            Answer::Nothing
        }
        Question::NetCheckShortLinkHosts => {
            let called = env.call_static_method(
                class,
                jni_str!("requestNetCheckShortLinkHosts"),
                jni_sig!("()[Ljava/lang/String;"),
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
                jni_str!("reportTaskProfile"),
                jni_sig!("(Ljava/lang/String;)V"),
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
fn string_of(env: &mut Env<'_>, called: jni::errors::Result<JValueOwned>) -> String {
    let Some(object) = object_of(called) else {
        return String::new();
    };
    let jstring = unsafe { JString::from_raw(env, object.as_raw()) };
    let Ok(java) = jstring.mutf8_chars(env) else {
        return String::new();
    };
    java.to_str().into_owned()
}

/// A `String[]` Java answered with.
fn strings_of(env: &mut Env<'_>, called: jni::errors::Result<JValueOwned>) -> Vec<String> {
    let Ok(array) = called.and_then(|value| value.l()) else {
        return Vec::new();
    };
    string_array(env, &array)
}

/// An `int[]` Java writes an answer into — the C++'s `env->NewIntArray`, which
/// is two ints wide for `req2Buf` and one everywhere else.
fn int_out<'a>(env: &mut Env<'a>, len: i32) -> Option<JIntArray<'a>> {
    env.new_int_array(len.max(0) as usize).ok()
}

fn int_at(env: &mut Env<'_>, array: &JIntArray<'_>, index: usize) -> i32 {
    let mut values = vec![0; index + 1];
    if array.get_region(env, 0, &mut values).is_err() {
        return 0;
    }
    values[index]
}

/// A `byte[]` argument — `null` for an empty one, which is what the C++ hands
/// over when the buffer it is carrying has nothing in it.
fn bytes_argument<'a>(env: &mut Env<'a>, bytes: &[u8]) -> JObject<'a> {
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
fn byte_stream<'a>(env: &mut Env<'a>) -> Option<JObject<'a>> {
    let Ok(class) = env.find_class(jni_str!("java/io/ByteArrayOutputStream")) else {
        return None;
    };
    env.new_object(class, jni_sig!("()V"), &[]).ok()
}

/// `toByteArray()` of one — empty for a stream Java never wrote to, which is
/// what the C++ ends up with too.
fn bytes_of(env: &mut Env<'_>, stream: &JObject<'_>) -> Vec<u8> {
    let Ok(bytes) = env.call_method(stream, jni_str!("toByteArray"), jni_sig!("()[B"), &[]) else {
        return Vec::new();
    };
    let Ok(bytes) = bytes.l() else {
        return Vec::new();
    };
    env.convert_byte_array(unsafe { JByteArray::from_raw(env, bytes.as_raw()) })
        .unwrap_or_default()
}

/// `StnLogic$CgiProfile`, the object `onTaskEnd` is handed: the C++'s ten
/// `SetLongField`/`SetIntField` calls.
///
/// Not read: `startHandshakeTime` and `handshakeSuccessfulTime`, which stay at
/// `0` — the port's [`CgiProfile`] keeps no tls handshake, because nothing in
/// the port writes one. Nor is the connect's `nettype`, which the Java class
/// has no field for.
fn cgi_profile<'a>(env: &mut Env<'a>, profile: &CgiProfile) -> Option<JObject<'a>> {
    let Ok(class) = env.find_class(STN_CGI_PROFILE) else {
        return None;
    };
    let Ok(object) = env.new_object(class, jni_sig!("()V"), &[]) else {
        return None;
    };
    for (name, value) in [
        (jni_str!("taskStartTime"), profile.start_time as i64),
        (
            jni_str!("startConnectTime"),
            profile.start_connect_time as i64,
        ),
        (
            jni_str!("connectSuccessfulTime"),
            profile.connect_successful_time as i64,
        ),
        (
            jni_str!("startSendPacketTime"),
            profile.start_send_packet_time as i64,
        ),
        (
            jni_str!("startReadPacketTime"),
            profile.start_read_packet_time as i64,
        ),
        (
            jni_str!("readPacketFinishedTime"),
            profile.read_packet_finished_time as i64,
        ),
        (jni_str!("rtt"), profile.rtt as i64),
    ] {
        let _ = env.set_field(&object, name, jni_sig!("J"), JValue::Long(value));
    }
    for (name, value) in [
        (jni_str!("channelType"), profile.channel_type),
        (jni_str!("protocolType"), profile.transport_protocol),
    ] {
        let _ = env.set_field(&object, name, jni_sig!("I"), JValue::Int(value));
    }
    Some(object)
}

// #################### the questions the app is asked ####################

/// `AppLogic` — the class the C++'s four C2Java calls are static methods of.
/// Unlike [`STN_CALLBACK`], it forwards nothing: what it answers is the app's
/// own, which is why the native side never keeps the app itself.
const APP_LOGIC: &JNIStr = jni_str!("io/github/orangeboychen/marsrs/app/AppLogic");

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
        vm.attach_current_thread(|env| -> jni::errors::Result<AppAnswer> {
            let Ok(class) = env.find_class(APP_LOGIC) else {
                return Ok(AppAnswer::Nothing);
            };
            Ok(ask_app(env, class, question))
        })
        .unwrap_or(AppAnswer::Nothing)
    })
}

fn ask_app<'a>(env: &mut Env<'a>, class: JClass<'a>, question: AppQuestion) -> AppAnswer {
    match question {
        AppQuestion::AppFilePath => {
            let called = env.call_static_method(
                class,
                jni_str!("getAppFilePath"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            );
            AppAnswer::Path(string_of(env, called))
        }
        AppQuestion::AccountInfo => {
            // `AppLogic$AccountInfo` — `uin` and `userName`.
            let called = env.call_static_method(
                class,
                jni_str!("getAccountInfo"),
                jni_sig!("()Lio/github/orangeboychen/marsrs/app/AppLogic$AccountInfo;"),
                &[],
            );
            let Some(account) = object_of(called) else {
                return AppAnswer::Nothing;
            };
            AppAnswer::Account(AccountInfo::new(
                long_field(env, &account, jni_str!("uin")),
                string_field(env, &account, jni_str!("userName")),
            ))
        }
        AppQuestion::ClientVersion => {
            let called =
                env.call_static_method(class, jni_str!("getClientVersion"), jni_sig!("()I"), &[]);
            AppAnswer::Version(int_of(called))
        }
        AppQuestion::DeviceInfo => {
            // `AppLogic$DeviceInfo` — `devicename` and `devicetype`.
            let called = env.call_static_method(
                class,
                jni_str!("getDeviceType"),
                jni_sig!("()Lio/github/orangeboychen/marsrs/app/AppLogic$DeviceInfo;"),
                &[],
            );
            let Some(device) = object_of(called) else {
                return AppAnswer::Nothing;
            };
            AppAnswer::Device(DeviceInfo::new(
                string_field(env, &device, jni_str!("devicename")),
                string_field(env, &device, jni_str!("devicetype")),
            ))
        }
    }
}

// #################### the questions the platform is asked ####################

/// `PlatformComm$C2Java` — the class the C++'s nine C2Java calls are static
/// methods of.
const PLATFORM_COMM: &JNIStr = jni_str!("io/github/orangeboychen/marsrs/comm/PlatformComm$C2Java");

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
        vm.attach_current_thread(|env| -> jni::errors::Result<PlatformAnswer> {
            let Ok(class) = env.find_class(PLATFORM_COMM) else {
                return Ok(PlatformAnswer::Nothing);
            };
            Ok(ask_platform(env, class, question))
        })
        .unwrap_or(PlatformAnswer::Nothing)
    })
}

fn ask_platform<'a>(
    env: &mut Env<'a>,
    class: JClass<'a>,
    question: PlatformQuestion,
) -> PlatformAnswer {
    match question {
        PlatformQuestion::NetInfo => {
            let called =
                env.call_static_method(class, jni_str!("getNetInfo"), jni_sig!("()I"), &[]);
            PlatformAnswer::NetInfo(NetInfo::of(int_of(called)))
        }
        PlatformQuestion::StatisticsNetType => {
            let called = env.call_static_method(
                class,
                jni_str!("getStatisticsNetType"),
                jni_sig!("()I"),
                &[],
            );
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
                jni_str!("getProxyInfo"),
                jni_sig!("(Ljava/lang/StringBuffer;)I"),
                &[argument],
            );
            let port = int_of(called);
            let called = env.call_method(
                &buffer,
                jni_str!("toString"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            );
            PlatformAnswer::Proxy {
                port,
                host: string_of(env, called),
            }
        }
        PlatformQuestion::WifiInfo => {
            // `PlatformComm$WifiInfo` — `ssid` and `bssid`.
            let called = env.call_static_method(
                class,
                jni_str!("getCurWifiInfo"),
                jni_sig!("()Lio/github/orangeboychen/marsrs/comm/PlatformComm$WifiInfo;"),
                &[],
            );
            let Some(wifi) = object_of(called) else {
                return PlatformAnswer::Nothing;
            };
            PlatformAnswer::Wifi(Some(WifiInfo {
                ssid: string_field(env, &wifi, jni_str!("ssid")),
                bssid: string_field(env, &wifi, jni_str!("bssid")),
            }))
        }
        PlatformQuestion::SimInfo => {
            // `PlatformComm$SIMInfo` — `ispCode` and `ispName`, both strings:
            // the Java writes `"" + ispCode`.
            let called = env.call_static_method(
                class,
                jni_str!("getCurSIMInfo"),
                jni_sig!("()Lio/github/orangeboychen/marsrs/comm/PlatformComm$SIMInfo;"),
                &[],
            );
            let Some(sim) = object_of(called) else {
                return PlatformAnswer::Nothing;
            };
            PlatformAnswer::Sim(Some(SimInfo {
                isp_code: string_field(env, &sim, jni_str!("ispCode")),
                isp_name: string_field(env, &sim, jni_str!("ispName")),
            }))
        }
        PlatformQuestion::ApnInfo => {
            // `PlatformComm$APNInfo` — `netType`, `subNetType` and `extraInfo`.
            let called = env.call_static_method(
                class,
                jni_str!("getAPNInfo"),
                jni_sig!("()Lio/github/orangeboychen/marsrs/comm/PlatformComm$APNInfo;"),
                &[],
            );
            let Some(apn) = object_of(called) else {
                return PlatformAnswer::Nothing;
            };
            PlatformAnswer::Apn(Some(ApnInfo {
                net_type: int_field(env, &apn, jni_str!("netType")),
                sub_net_type: int_field(env, &apn, jni_str!("subNetType")),
                extra_info: string_field(env, &apn, jni_str!("extraInfo")),
            }))
        }
        PlatformQuestion::RadioAccessNetwork => {
            let called = env.call_static_method(
                class,
                jni_str!("getCurRadioAccessNetworkInfo"),
                jni_sig!("()I"),
                &[],
            );
            PlatformAnswer::RadioAccessNetwork(int_of(called))
        }
        PlatformQuestion::Signal { wifi } => {
            let called = env.call_static_method(
                class,
                jni_str!("getSignal"),
                jni_sig!("(Z)J"),
                &[JValue::Bool(wifi)],
            );
            PlatformAnswer::Signal(long_of(called))
        }
        PlatformQuestion::NetworkConnected => {
            let called =
                env.call_static_method(class, jni_str!("isNetworkConnected"), jni_sig!("()Z"), &[]);
            PlatformAnswer::Connected(bool_of(called))
        }
    }
}

/// A `StringBuffer` Java writes an answer into — the C++'s own
/// `NewObject(StringBuffer)`, handed to `getProxyInfo` and read back with
/// `toString()`.
fn string_buffer<'a>(env: &mut Env<'a>) -> Option<JObject<'a>> {
    let Ok(class) = env.find_class(jni_str!("java/lang/StringBuffer")) else {
        return None;
    };
    env.new_object(class, jni_sig!("()V"), &[]).ok()
}

/// A `J` Java answered with — `0` for a call that could not be made.
fn long_of(called: jni::errors::Result<JValueOwned>) -> i64 {
    called.and_then(|value| value.j()).unwrap_or(0)
}

// #################### io.github.orangeboychen.marsrs.BaseEvent ####################

/// `BaseEvent.onCreate` — the app is up, which is when the net core is made.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_BaseEvent_onCreate<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    guard(|| {
        on_create_impl();
    })
}

/// `BaseEvent.onInitConfigBeforeOnCreate`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_BaseEvent_onInitConfigBeforeOnCreate<
    'local,
>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    packer_encoder_version: jint,
) {
    guard(|| on_init_config_before_on_create_impl(packer_encoder_version))
}

/// `BaseEvent.onDestroy`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_BaseEvent_onDestroy<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    guard(|| {
        on_destroy_impl();
    })
}

/// `BaseEvent.onForeground`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_BaseEvent_onForeground<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    is_foreground: jboolean,
) {
    guard(|| on_foreground_impl(is_foreground))
}

/// `BaseEvent.onNetworkChange`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_BaseEvent_onNetworkChange<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    guard(on_network_change_impl)
}

/// `BaseEvent.onSingalCrash` — the signal number is not the port's to handle,
/// so it is not read; what the crash does is close the appender.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_BaseEvent_onSingalCrash<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _sig: jint,
) {
    guard(|| on_signal_crash_impl(_sig))
}

/// `BaseEvent.onExceptionCrash`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_BaseEvent_onExceptionCrash<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    guard(on_exception_crash_impl)
}

// #################### io.github.orangeboychen.marsrs.sdt.SdtLogic ####################

/// `SdtLogic.setHttpNetcheckCGI`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_sdt_SdtLogic_setHttpNetcheckCGI<
    'local,
>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    cgi: JString<'local>,
) {
    guard_env(&mut env, |env| {
        let cgi = cgi
            .mutf8_chars(env)
            .map(|value| value.to_str().into_owned())
            .unwrap_or_default();
        set_http_netcheck_cgi_impl(&cgi);
    })
}

/// `SdtLogic.getLoadLibraries`.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_sdt_SdtLogic_getLoadLibraries<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> jobject {
    guard_env(&mut env, |env| string_array_list(env, &sdt_libraries()))
}

// #################### io.github.orangeboychen.marsrs.comm.Alarm ####################

/// `Alarm.onAlarm(id)` — the one `native` method of the Java class, called from
/// `onReceive` once the broadcast found the id.
#[no_mangle]
pub extern "system" fn Java_io_github_orangeboychen_marsrs_comm_Alarm_onAlarm<'local>(
    _env: EnvUnowned<'local>,
    _this: JObject<'local>,
    id: jlong,
) {
    guard(|| {
        on_alarm_impl(id);
    })
}
