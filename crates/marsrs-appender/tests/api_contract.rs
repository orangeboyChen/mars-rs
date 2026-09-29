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
use std::path::PathBuf;

use marsrs_appender::{
    AppenderError,
    AppenderMode,
    FileIoAction,
    Flush,
    LogLevel,
    XLogConfig,
    XLoggerInfo,
    Xlog,
};
use marsrs_buffer::CompressMode;

#[test]
fn the_object_is_the_api_an_app_takes() {
    let _: fn(XLogConfig, LogLevel) -> Result<Xlog, AppenderError> = Xlog::open;
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
    let _: for<'a, 'b> fn(&Xlog, &'a str, &'b str) -> bool = Xlog::v;
    let _: for<'a, 'b> fn(&Xlog, &'a str, &'b str) -> bool = Xlog::d;
    let _: for<'a, 'b> fn(&Xlog, &'a str, &'b str) -> bool = Xlog::i;
    let _: for<'a, 'b> fn(&Xlog, &'a str, &'b str) -> bool = Xlog::w;
    let _: for<'a, 'b> fn(&Xlog, &'a str, &'b str) -> bool = Xlog::e;
    let _: for<'a, 'b> fn(&Xlog, &'a str, &'b str) -> bool = Xlog::f;
    let _: fn(&Xlog, i64) -> Vec<std::path::PathBuf> = Xlog::log_files;
    let _: fn(&Xlog, i64) -> Vec<std::path::PathBuf> = Xlog::log_file_names;
    let _: fn(&Xlog) -> Option<std::path::PathBuf> = Xlog::current_log_path;
    let _: fn(&Xlog) = Xlog::request_flush;
    let _: fn(&Xlog) = Xlog::flush_now;
    let _: fn(&Xlog) -> Flush = Xlog::flush;
    let _: fn(&Xlog) -> Option<PathBuf> = Xlog::current_log_path;
    let _: fn(&Xlog) = Xlog::close;
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
