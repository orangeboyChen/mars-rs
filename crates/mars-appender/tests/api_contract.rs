//! Compile-time check that `mars-appender` keeps exposing the API the
//! FFI and JNI crates are written against. Every public function is coerced to
//! a function pointer of its expected signature, so a signature drift fails
//! the build instead of the FFI crate downstream.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use mars_appender::{
    appender_close, appender_flush, appender_flush_sync, appender_get_current_log_cache_path,
    appender_get_current_log_path, appender_getfilepath_from_timespan, appender_make_logfile_name,
    appender_oneshot_flush, appender_open, appender_set_console_log,
    appender_set_max_alive_duration, appender_set_max_file_size, appender_set_mode, appender_write,
    log_formater, AppenderError, AppenderMode, FileIoAction, LogLevel, XLogConfig, XLoggerInfo,
};
use mars_buffer::CompressMode;
use mars_core::PtrBuffer;

#[test]
fn function_signatures_match_the_contract() {
    let _: fn(XLogConfig) -> Result<(), AppenderError> = appender_open;
    let _: fn() = appender_flush;
    let _: fn() = appender_flush_sync;
    let _: fn() = appender_close;
    let _: fn(AppenderMode) = appender_set_mode;
    let _: fn(bool) = appender_set_console_log;
    let _: fn(u64) = appender_set_max_file_size;
    let _: fn(u64) = appender_set_max_alive_duration;
    let _: fn() -> Option<PathBuf> = appender_get_current_log_path;
    let _: fn() -> Option<PathBuf> = appender_get_current_log_cache_path;
    let _: for<'a> fn(&'a XLogConfig) -> FileIoAction = appender_oneshot_flush;
    let _: for<'a> fn(Option<&'a XLoggerInfo>, &str) -> bool = appender_write;
    let _: for<'a> fn(i64, &'a str, &'a Path) -> Vec<PathBuf> = appender_make_logfile_name;
    let _: for<'a> fn(i64, &'a str, &'a Path) -> Vec<PathBuf> = appender_getfilepath_from_timespan;
    let _: for<'a, 'b, 'c, 'd> fn(Option<&'a XLoggerInfo>, Option<&'b str>, &'c mut PtrBuffer<'d>) =
        log_formater;
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
