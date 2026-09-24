//! Configuration and record types — port of `mars/xlog/appender.h`
//! (`TAppenderMode`, `TCompressMode`, `TFileIOAction`, `XLogConfig`) and of the
//! `XLoggerInfo` struct in `mars/comm/xlogger/xloggerbase.h`.
//!
//! The Rust `XLogConfig` mirrors the C++ one field for field, except that
//! `cachedir_` (a `std::string` where the empty string means "not set") becomes
//! an `Option<PathBuf>` and `cache_days_` becomes an unsigned value.

use std::fmt;
use std::path::PathBuf;

use mars_xlog_buffer::CompressMode;

/// `mars::xlog::TAppenderMode`.
///
/// `Async` buffers records in the mmap cache and lets a background thread
/// write them out; `Sync` writes every record straight to the log file on the
/// calling thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppenderMode {
    /// `kAppenderAsync` — buffer + writer thread.
    Async,
    /// `kAppenderSync` — write on the caller's thread.
    Sync,
}

/// `mars::comm::TLogLevel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// `kLevelVerbose`
    Verbose = 0,
    /// `kLevelDebug`
    Debug = 1,
    /// `kLevelInfo`
    Info = 2,
    /// `kLevelWarn`
    Warn = 3,
    /// `kLevelError`
    Error = 4,
    /// `kLevelFatal`
    Fatal = 5,
    /// `kLevelNone` — nothing is logged. `TLogLevel` has it, and
    /// `Xlog.LEVEL_NONE` passes it straight through, so the port needs it too:
    /// without the variant `LEVEL_NONE` collapsed onto `Fatal` and kept the
    /// worst records while dropping everything else.
    None = 6,
}

/// `mars::xlog::TFileIOAction`.
///
/// Reported by [`crate::appender_oneshot_flush`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIoAction {
    /// `kActionNone`
    None = 0,
    /// `kActionSuccess`
    Success = 1,
    /// `kActionUnnecessary` — there was nothing to flush.
    Unnecessary = 2,
    /// `kActionOpenFailed`
    OpenFailed = 3,
    /// `kActionReadFailed`
    ReadFailed = 4,
    /// `kActionWriteFailed`
    WriteFailed = 5,
    /// `kActionCloseFailed`
    CloseFailed = 6,
    /// `kActionRemoveFailed`
    RemoveFailed = 7,
}

/// `mars::xlog::XLogConfig`.
#[derive(Debug, Clone)]
pub struct XLogConfig {
    /// `mode_`
    pub mode: AppenderMode,
    /// `logdir_`
    pub logdir: PathBuf,
    /// `nameprefix_`
    pub nameprefix: String,
    /// `pub_key_` — 128 hex chars of the *server* public key; empty means
    /// "no encryption".
    pub pub_key: String,
    /// `compress_mode_`
    pub compress_mode: CompressMode,
    /// `compress_level_`
    pub compress_level: i32,
    /// `cachedir_` — `None` (C++: empty string) means "write straight to
    /// `logdir`".
    pub cachedir: Option<PathBuf>,
    /// `cache_days_`
    pub cache_days: u32,
}

impl Default for XLogConfig {
    /// Matches the C++ in-class initialisers (`logdir_` defaults to `./log`
    /// through the callers, `nameprefix_` to `Mars`, `compress_mode_` to
    /// `kZlib`, `compress_level_` to 6).
    fn default() -> Self {
        Self {
            mode: AppenderMode::Async,
            logdir: PathBuf::from("./log"),
            nameprefix: "Mars".to_owned(),
            pub_key: String::new(),
            compress_mode: CompressMode::Zlib,
            compress_level: 6,
            cachedir: None,
            cache_days: 0,
        }
    }
}

/// `mars::comm::XLoggerInfo`.
///
/// Optional strings are `None` where the C++ uses a possibly-`NULL`
/// `const char*`.
#[derive(Debug, Clone)]
pub struct XLoggerInfo {
    /// `level`
    pub level: LogLevel,
    /// `tag`
    pub tag: Option<String>,
    /// `filename`
    pub filename: Option<String>,
    /// `func_name`
    pub func_name: Option<String>,
    /// `line`
    pub line: i32,
    /// `pid`
    pub pid: i64,
    /// `tid`
    pub tid: i64,
    /// `maintid`
    pub maintid: i64,
    /// `timeval` as `(tv_sec, tv_usec)`.
    pub timeval: (i64, i64),
}

impl Default for XLoggerInfo {
    /// The C++ `XLOGGER_INFO_INITIALIZER` (all zero / `NULL`, level verbose).
    fn default() -> Self {
        Self {
            level: LogLevel::Verbose,
            tag: None,
            filename: None,
            func_name: None,
            line: 0,
            pid: 0,
            tid: 0,
            maintid: 0,
            timeval: (0, 0),
        }
    }
}

/// The error type of [`crate::appender_open`].
///
/// The C++ `appender_open` returns `void` and silently ignores failures; the
/// Rust port reports them instead.
#[derive(Debug, Clone)]
pub struct AppenderError(pub String);

impl fmt::Display for AppenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AppenderError {}

impl From<std::io::Error> for AppenderError {
    fn from(err: std::io::Error) -> Self {
        AppenderError(err.to_string())
    }
}

impl From<String> for AppenderError {
    fn from(msg: String) -> Self {
        AppenderError(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_cpp_initializers() {
        let c = XLogConfig::default();
        assert_eq!(c.mode, AppenderMode::Async);
        assert_eq!(c.logdir, PathBuf::from("./log"));
        assert_eq!(c.nameprefix, "Mars");
        assert!(c.pub_key.is_empty());
        assert_eq!(c.compress_mode, CompressMode::Zlib);
        assert_eq!(c.compress_level, 6);
        assert!(c.cachedir.is_none());
        assert_eq!(c.cache_days, 0);
    }

    #[test]
    fn enum_discriminants_match_cpp() {
        assert_eq!(AppenderMode::Async as i32, 0);
        assert_eq!(AppenderMode::Sync as i32, 1);
        assert_eq!(LogLevel::Verbose as i32, 0);
        assert_eq!(LogLevel::Fatal as i32, 5);
        assert_eq!(FileIoAction::None as i32, 0);
        assert_eq!(FileIoAction::RemoveFailed as i32, 7);
    }

    #[test]
    fn appender_error_is_an_error() {
        let e: Box<dyn std::error::Error> = Box::new(AppenderError("boom".to_owned()));
        assert_eq!(e.to_string(), "boom");
    }
}
