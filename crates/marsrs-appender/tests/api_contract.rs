//! Compile-time check that `marsrs-appender` keeps exposing the API an app,
//! the FFI crate and the JNI crate are written against. Every public function
//! — and every member of [`Xlog`] — is coerced to a function pointer of its
//! expected signature, so a signature drift fails the build instead of the
//! FFI crate downstream.
//!
//! What is checked is the shape an app sees, and it is three things: the
//! [`Xlog`] an app holds, the process-wide `appender_open` / `appender_close`
//! the C ABI installs the default logger with, and the handle of
//! [`marsrs_appender::category`] — a handle and not an object, because neither
//! the C ABI nor JNI has one to hold. The write, the drain and the four setters
//! of the process-wide appender are deliberately *not* here: they are the
//! category's at `DEFAULT_HANDLE`, and a second spelling of them is what this
//! test would otherwise pin down twice.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use marsrs_appender::{
    appender_close, appender_get_current_log_cache_path, appender_get_current_log_path,
    appender_getfilepath_from_timespan, appender_make_logfile_name, appender_oneshot_flush,
    appender_open, category_set_max_alive_duration, category_set_max_file_size, current_log_path,
    flush, flush_all, flush_now, flush_now_all, get_filter, get_level, get_xlogger_instance,
    is_enabled_for, log_formater, new_xlogger_instance, release_xlogger_instance, request_flush,
    request_flush_all, set_appender_mode, set_console_log_open, set_filter, set_level,
    xlogger_assert, xlogger_assert_p, xlogger_write, AppenderError, AppenderMode, FileIoAction,
    Flush, LogLevel, XLogConfig, XLoggerInfo, Xlog, XloggerFilter, XloggerHandle, DEFAULT_HANDLE,
};
use marsrs_buffer::CompressMode;
use marsrs_core::PtrBuffer;

#[test]
fn the_object_is_the_api_an_app_takes() {
    let _: fn(XLogConfig, LogLevel) -> Result<Xlog, AppenderError> = Xlog::open;
    // The second writer over one prefix: the appender layer, which the
    // `*_instance` family used to be the public face of.
    let _: fn(XLogConfig, LogLevel) -> Result<Xlog, AppenderError> = Xlog::open_unregistered;
    let _: fn(&Xlog) -> &str = Xlog::name_prefix;
    let _: fn(&Xlog) -> bool = Xlog::is_open;
    let _: fn(&Xlog) -> Option<LogLevel> = Xlog::level;
    let _: fn(&Xlog, LogLevel) = Xlog::set_level;
    let _: fn(&Xlog) -> AppenderMode = Xlog::mode;
    let _: fn(&Xlog, AppenderMode) = Xlog::set_mode;
    let _: fn(&Xlog) -> bool = Xlog::console_log_enabled;
    let _: fn(&Xlog, bool) = Xlog::set_console_log_enabled;
    let _: fn(&Xlog) -> u64 = Xlog::max_file_size_bytes;
    let _: fn(&Xlog, u64) = Xlog::set_max_file_size_bytes;
    let _: fn(&Xlog) -> u64 = Xlog::max_alive_time_seconds;
    let _: fn(&Xlog, u64) = Xlog::set_max_alive_time_seconds;
    let _: fn(&Xlog, LogLevel) -> bool = Xlog::is_loggable;
    let _: for<'a, 'b> fn(&Xlog, LogLevel, &'a str, &'b str) -> bool = Xlog::log;
    let _: for<'a, 'b, 'c> fn(&Xlog, Option<&'a XLoggerInfo<'b>>, &'c str) -> bool =
        Xlog::log_with_info;
    let _: for<'a, 'b> fn(&Xlog, &'a str, &'b str) -> bool = Xlog::v;
    let _: for<'a, 'b> fn(&Xlog, &'a str, &'b str) -> bool = Xlog::d;
    let _: for<'a, 'b> fn(&Xlog, &'a str, &'b str) -> bool = Xlog::i;
    let _: for<'a, 'b> fn(&Xlog, &'a str, &'b str) -> bool = Xlog::w;
    let _: for<'a, 'b> fn(&Xlog, &'a str, &'b str) -> bool = Xlog::e;
    let _: for<'a, 'b> fn(&Xlog, &'a str, &'b str) -> bool = Xlog::f;
    let _: fn(&Xlog) = Xlog::request_flush;
    let _: fn(&Xlog) = Xlog::flush_now;
    let _: fn(&Xlog) -> Flush = Xlog::flush;
    let _: fn(&Xlog) -> Option<PathBuf> = Xlog::current_log_path;
    let _: fn(&Xlog) = Xlog::close;
}

#[test]
fn function_signatures_match_the_contract() {
    // The process-wide appender the C ABI and JNI install and drop. Its write,
    // its drain and its four setters are `category`'s at `DEFAULT_HANDLE`.
    let _: fn(XLogConfig) -> Result<(), AppenderError> = appender_open;
    let _: fn() = appender_close;
    let _: fn() -> Option<PathBuf> = appender_get_current_log_path;
    let _: fn() -> Option<PathBuf> = appender_get_current_log_cache_path;
    let _: for<'a> fn(&'a XLogConfig) -> FileIoAction = appender_oneshot_flush;
    let _: for<'a> fn(i64, &'a str, &'a Path) -> Vec<PathBuf> = appender_make_logfile_name;
    let _: for<'a> fn(i64, &'a str, &'a Path) -> Vec<PathBuf> = appender_getfilepath_from_timespan;
    let _: for<'a, 'b, 'c, 'd> fn(Option<&'a XLoggerInfo>, Option<&'b str>, &'c mut PtrBuffer<'d>) =
        log_formater;

    // The handle: what the C ABI and the JNI bridge are written over.
    let _: for<'a> fn(&'a XLogConfig, LogLevel) -> XloggerHandle = new_xlogger_instance;
    let _: for<'a> fn(&'a str) -> XloggerHandle = get_xlogger_instance;
    let _: for<'a> fn(&'a str) = release_xlogger_instance;
    let _: for<'a, 'b, 'c> fn(XloggerHandle, Option<&'a XLoggerInfo<'b>>, Option<&'c str>) -> bool =
        xlogger_write;
    let _: for<'a> fn(Option<&'a XLoggerInfo>, &'a str, &'a str) -> bool = xlogger_assert;
    let _: for<'a> fn(Option<&'a XLoggerInfo>, &'a str, std::fmt::Arguments<'a>) -> bool =
        xlogger_assert_p;
    let _: fn(XloggerHandle, LogLevel) -> bool = is_enabled_for;
    let _: fn(XloggerHandle) -> Option<LogLevel> = get_level;
    let _: fn(XloggerHandle, LogLevel) = set_level;
    let _: fn(XloggerHandle, AppenderMode) = set_appender_mode;
    let _: fn(XloggerHandle, bool) = set_console_log_open;
    let _: fn(XloggerHandle, u64) = category_set_max_file_size;
    let _: fn(XloggerHandle, u64) = category_set_max_alive_duration;
    let _: fn(XloggerHandle) = request_flush;
    let _: fn(XloggerHandle) = flush_now;
    let _: fn(XloggerHandle) -> Flush = flush;
    let _: fn() = request_flush_all;
    let _: fn() = flush_now_all;
    let _: fn() -> Flush = flush_all;
    let _: fn(XloggerHandle) -> Option<PathBuf> = current_log_path;
    let _: fn(Option<XloggerFilter>) = set_filter;
    let _: fn() -> Option<XloggerFilter> = get_filter;

    // `DEFAULT_HANDLE` is what the process-wide appender answers to.
    let _: fn() -> XloggerHandle = || DEFAULT_HANDLE;
}

#[test]
fn struct_and_enum_shapes_match_the_contract() {
    // `XLogConfig` field-for-field.
    let config = XLogConfig {
        mode: AppenderMode::Async,
        logdir: PathBuf::from("log"),
        nameprefix: "Mars".to_owned(),
        pub_key: String::new(),
        compress_mode: CompressMode::Zlib,
        compress_level: 6,
        cachedir: None,
        cache_days: 0,
    };
    assert_eq!(config.compress_level, XLogConfig::default().compress_level);

    // `XLoggerInfo` field-for-field.
    let info = XLoggerInfo {
        level: LogLevel::Fatal,
        tag: None,
        filename: None,
        func_name: None,
        line: 0,
        pid: 0,
        tid: 0,
        maintid: 0,
        timeval: (0, 0),
        trace_log: 0,
    };
    assert_eq!(info.level, LogLevel::Fatal);
    assert_eq!(info.timeval, (0, 0));

    // The three string fields are `Cow`, like the `const char*` they port:
    // borrowed for a caller that already has the bytes (the FFI / JNI
    // boundary), owned for one that built them.
    let owned = String::from("owned");
    let info = XLoggerInfo {
        tag: Some(Cow::Borrowed("borrowed")),
        filename: Some(Cow::Owned(owned.clone())),
        func_name: None,
        ..XLoggerInfo::default()
    };
    assert_eq!(info.tag.as_deref(), Some("borrowed"));
    assert_eq!(info.filename.as_deref(), Some("owned"));
    assert_eq!(info.func_name.as_deref(), None);

    // `AppenderError(pub String)` + `Display` + `Error`.
    let err = AppenderError("boom".to_owned());
    assert_eq!(err.0, "boom");
    let boxed: Box<dyn std::error::Error> = Box::new(err);
    assert_eq!(boxed.to_string(), "boom");

    // Discriminants.
    assert_eq!(LogLevel::Verbose as i32, 0);
    assert_eq!(LogLevel::Debug as i32, 1);
    assert_eq!(LogLevel::Info as i32, 2);
    assert_eq!(LogLevel::Warn as i32, 3);
    assert_eq!(LogLevel::Error as i32, 4);
    assert_eq!(LogLevel::Fatal as i32, 5);
    assert_eq!(FileIoAction::None as i32, 0);
    assert_eq!(FileIoAction::Success as i32, 1);
    assert_eq!(FileIoAction::Unnecessary as i32, 2);
    assert_eq!(FileIoAction::OpenFailed as i32, 3);
    assert_eq!(FileIoAction::ReadFailed as i32, 4);
    assert_eq!(FileIoAction::WriteFailed as i32, 5);
    assert_eq!(FileIoAction::CloseFailed as i32, 6);
    assert_eq!(FileIoAction::RemoveFailed as i32, 7);
}
