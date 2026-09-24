//! The exported `extern "C"` symbols.
//!
//! Every function in this module is a mechanical translation of one C++ entry
//! point: null-check the pointers, convert to the Rust types of
//! `mars-xlog-appender`, delegate. Nothing else — all policy (level filtering,
//! identity fields) lives in [`crate::state`], all pointer handling in
//! [`crate::cstr`].

use std::ffi::{c_char, c_int, c_longlong, c_uchar, c_uint, c_ulonglong};
use std::path::Path;

use mars_xlog_appender::{
    appender_close, appender_flush, appender_flush_sync, appender_get_current_log_path,
    appender_open, appender_set_console_log, appender_set_max_alive_duration,
    appender_set_max_file_size, appender_write, AppenderMode, LogLevel, XLogConfig, XLoggerInfo,
};
use mars_xlog_buffer::CompressMode;

use crate::cstr;
use crate::error::*;
use crate::guard;
use crate::state;

/// `mars::xlog::TAppenderMode` (appender.h) as a C enum.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarsAppenderMode {
    /// `kAppenderAsync` — records are handed to a writer thread.
    Async = 0,
    /// `kAppenderSync` — records are written on the calling thread.
    Sync = 1,
}

/// `mars::xlog::TCompressMode` (appender.h) as a C enum.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarsCompressMode {
    /// `kZlib` — raw DEFLATE, one `Z_SYNC_FLUSH` per record.
    Zlib = 0,
    /// `kZstd` — streaming zstd, one `ZSTD_e_flush` per record.
    Zstd = 1,
}

/// `TLogLevel` (mars/comm/xlogger/xloggerbase.h) as a C enum.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MarsLogLevel {
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
}

/// `mars::xlog::XLogConfig` (appender.h) with C strings. Layout must stay
/// identical to `MarsXLogConfig` in `include/mars_xlog.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MarsXLogConfig {
    /// [`MarsAppenderMode`]; anything else is rejected with
    /// [`MARS_XLOG_ERR_BAD_MODE`].
    pub mode: c_int,
    /// Mandatory log directory; created if missing.
    pub log_dir: *const c_char,
    /// Log file name prefix; null / empty is used verbatim, as in the C++ `XLogConfig`.
    pub name_prefix: *const c_char,
    /// 128 hex chars of ECDH pubkey; null / empty disables encryption.
    pub pub_key: *const c_char,
    /// [`MarsCompressMode`]; anything else is rejected with
    /// [`MARS_XLOG_ERR_BAD_COMPRESS`].
    pub compress_mode: c_int,
    /// `<= 0` keeps the appender default (6).
    pub compress_level: c_int,
    /// mmap cache directory; null / empty means "use `log_dir`".
    pub cache_dir: *const c_char,
    /// Days of cache to keep; `< 0` is treated as 0.
    pub cache_days: c_int,
}

/// `mars::xlog::appender_open(const XLogConfig&)`.
///
/// Opens the process-wide appender. `config` may be null (the call is then a
/// no-op that reports [`MARS_XLOG_ERR_NULL_CONFIG`]); every string in it may
/// also be null, in which case it is treated as empty.
///
/// @return [`MARS_XLOG_OK`] on success, otherwise a negative
/// `MARS_XLOG_ERR_*` code.
// The pointer is null-checked (and otherwise only read through the audited
// `cstr` helpers), so this symbol is total and stays a *safe* `extern "C" fn`
// exactly as the C header declares it; clippy's "mark it unsafe" advice would
// push the burden onto every C/C++ caller without buying any safety.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn mars_xlog_open(config: *const MarsXLogConfig) -> c_int {
    guard(MARS_XLOG_ERR_PANIC, || {
        // SAFETY: `config` may be null (checked inside `ptr_to_ref`); otherwise
        // the caller guarantees a valid, aligned, initialised `MarsXLogConfig`
        // that stays alive for the duration of this call.
        let Some(cfg) = (unsafe { cstr::ptr_to_ref(config) }) else {
            return MARS_XLOG_ERR_NULL_CONFIG;
        };

        let mode = match cfg.mode {
            x if x == MarsAppenderMode::Async as c_int => AppenderMode::Async,
            x if x == MarsAppenderMode::Sync as c_int => AppenderMode::Sync,
            _ => return MARS_XLOG_ERR_BAD_MODE,
        };

        let compress_mode = match cfg.compress_mode {
            x if x == MarsCompressMode::Zlib as c_int => CompressMode::Zlib,
            x if x == MarsCompressMode::Zstd as c_int => CompressMode::Zstd,
            _ => return MARS_XLOG_ERR_BAD_COMPRESS,
        };

        // SAFETY: every field pointer is null-checked inside the helper and
        // otherwise points to a caller-owned NUL-terminated string.
        let (log_dir, name_prefix, pub_key, cache_dir) = unsafe {
            (
                cstr::ptr_to_path_buf(cfg.log_dir),
                cstr::ptr_to_str_or_empty(cfg.name_prefix),
                cstr::ptr_to_str_or_empty(cfg.pub_key),
                cstr::ptr_to_path_buf(cfg.cache_dir),
            )
        };

        if log_dir.as_os_str().is_empty() {
            return MARS_XLOG_ERR_EMPTY_LOG_DIR;
        }

        // Non-positive level falls back to the default. `name_prefix` still
        // goes through UTF-8 (XLogConfig stores a String), so a non-UTF-8
        // prefix is converted lossily — noted in the header; the directories
        // above are byte-exact.
        // An empty prefix must
        // stay empty: the C++ `XLogConfig::nameprefix_` has no default, so it
        // produces `.mmap3` / `_YYYYMMDD.xlog` and cache discovery is
        // prefix-based — substituting "Mars" would stop the Rust port from
        // draining (or being drained by) a C++ process's cache file.
        let defaults = XLogConfig::default();
        let rust_config = XLogConfig {
            mode,
            logdir: log_dir,
            nameprefix: name_prefix.to_string(),
            pub_key: pub_key.to_string(),
            compress_mode,
            compress_level: if cfg.compress_level > 0 {
                cfg.compress_level
            } else {
                defaults.compress_level
            },
            cachedir: if cache_dir.as_os_str().is_empty() {
                None
            } else {
                Some(cache_dir)
            },
            cache_days: cfg.cache_days.max(0) as u32,
        };

        match appender_open(rust_config) {
            Ok(()) => MARS_XLOG_OK,
            Err(err) => {
                // The C++ returned `void` here and silently did nothing; surfacing
                // the reason on stderr is the only diagnostic a host process gets.
                eprintln!("[mars-xlog-ffi] appender_open failed: {err}");
                MARS_XLOG_ERR_APPENDER
            }
        }
    })
}

/// `mars::xlog::XloggerWrite(...)` (xlogger_interface.h) plus the
/// `xlogger_IsEnabledFor` gate that `Java2C_Xlog.cc::logWrite` performs before
/// it.
///
/// `tag`, `filename`, `func_name` and `message` may be null; null and invalid
/// UTF-8 become an empty string. The record is dropped when `level` is outside
/// `MarsLevelVerbose..=MarsLevelFatal` (that is what C++ `kLevelNone` means) or
/// below the level set by [`mars_xlog_set_level`].
// See `mars_xlog_open`: every pointer argument is null-checked before use.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn mars_xlog_write(
    level: c_int,
    tag: *const c_char,
    filename: *const c_char,
    func_name: *const c_char,
    line: c_int,
    message: *const c_char,
) {
    guard((), || {
        if !state::level_enabled(level) {
            return;
        }
        let Some(level) = to_log_level(level) else {
            // `kLevelNone` (6) and anything else out of range: log nothing.
            return;
        };

        // SAFETY: each pointer is null-checked inside the helper and otherwise
        // points to a caller-owned NUL-terminated string.
        let (tag, filename, func_name, message) = unsafe {
            (
                cstr::ptr_to_str_or_empty(tag),
                cstr::ptr_to_str_or_empty(filename),
                cstr::ptr_to_str_or_empty(func_name),
                cstr::ptr_to_str_or_empty(message),
            )
        };

        let info = XLoggerInfo {
            level,
            tag: opt_string(tag),
            filename: opt_string(filename),
            func_name: opt_string(func_name),
            line,
            pid: state::pid(),
            tid: state::tid(),
            maintid: state::main_tid(),
            timeval: state::now_timeval(),
        };

        // The appender returns whether anything was written; a C caller has no
        // channel for it (the C++ `xlogger_AssertP` family did not check either).
        let _written = appender_write(Some(&info), message);
    });
}

/// `mars::xlog::appender_flush()` — asks the async writer thread to drain.
#[no_mangle]
pub extern "C" fn mars_xlog_flush() {
    guard((), appender_flush);
}

/// `mars::xlog::appender_flush_sync()` — flushes and waits for the drain.
#[no_mangle]
pub extern "C" fn mars_xlog_flush_sync() {
    guard((), appender_flush_sync);
}

/// `mars::xlog::appender_close()`.
#[no_mangle]
pub extern "C" fn mars_xlog_close() {
    guard((), appender_close);
}

/// `xlogger_SetLevel(TLogLevel)` — sets the minimum level [`mars_xlog_write`]
/// forwards to the appender. Negative values clamp to `MarsLevelVerbose`;
/// `MARS_LEVEL_NONE` (6) or higher disables logging completely.
#[no_mangle]
pub extern "C" fn mars_xlog_set_level(level: c_int) {
    guard((), || state::set_min_level(level));
}

/// `mars::xlog::appender_set_console_log(bool)`; any non-zero `open` is `true`.
#[no_mangle]
pub extern "C" fn mars_xlog_set_console_log(open: c_int) {
    guard((), || appender_set_console_log(open != 0));
}

/// `mars::xlog::appender_set_max_file_size(uint64_t)`; 0 means "never split".
#[no_mangle]
pub extern "C" fn mars_xlog_set_max_file_size(bytes: c_ulonglong) {
    guard((), || appender_set_max_file_size(bytes));
}

/// `mars::xlog::appender_set_max_alive_duration(long)`; negative clamps to 0.
#[no_mangle]
pub extern "C" fn mars_xlog_set_max_alive_duration(seconds: c_longlong) {
    guard(
        (),
        || appender_set_max_alive_duration(seconds.max(0) as u64),
    );
}

/// `mars::xlog::appender_get_current_log_path(char*, unsigned int)`.
///
/// Copies the NUL-terminated path of the log file currently being appended to
/// into `out`, which must have room for `len` bytes.
///
/// @return the number of bytes written excluding the terminating NUL, or
/// [`MARS_XLOG_ERR_NULL_OUT`], [`MARS_XLOG_ERR_NO_SPACE`] (including `len == 0`)
/// or [`MARS_XLOG_ERR_NO_PATH`].
#[no_mangle]
pub extern "C" fn mars_xlog_current_log_path(out: *mut c_char, len: c_uint) -> c_int {
    guard(MARS_XLOG_ERR_PANIC, || {
        if out.is_null() {
            return MARS_XLOG_ERR_NULL_OUT;
        }
        if len == 0 {
            return MARS_XLOG_ERR_NO_SPACE;
        }

        let Some(path) = appender_get_current_log_path() else {
            return MARS_XLOG_ERR_NO_PATH;
        };

        // SAFETY: `out` is non-null (checked above) and the caller promises
        // `len` writable bytes.
        unsafe { write_path_into(path_to_bytes(&path), out as *mut c_uchar, len) }
    })
}

/// Copies `bytes` plus a terminating NUL into the caller's buffer.
///
/// # Safety
///
/// `out` must be non-null and point to at least `len` writable bytes.
unsafe fn write_path_into(bytes: Vec<u8>, out: *mut c_uchar, len: c_uint) -> c_int {
    if out.is_null() {
        return MARS_XLOG_ERR_NULL_OUT;
    }
    if len == 0 {
        return MARS_XLOG_ERR_NO_SPACE;
    }
    // `+ 1` for the terminating NUL.
    if bytes.len() + 1 > len as usize {
        return MARS_XLOG_ERR_NO_SPACE;
    }

    // SAFETY: `len >= 1` and `bytes.len() + 1 <= len` were checked above, so
    // both the copy and the NUL write stay in the caller's buffer. The slice is
    // sized by what is written, not by what the caller claimed.
    unsafe {
        let dst = std::slice::from_raw_parts_mut(out, bytes.len() + 1);
        dst[..bytes.len()].copy_from_slice(&bytes);
        dst[bytes.len()] = 0;
    }

    bytes.len() as c_int
}

/// Maps a raw `TLogLevel` onto [`LogLevel`]; `None` for anything outside
/// `Verbose..=Fatal` (e.g. C++ `kLevelNone`).
fn to_log_level(level: c_int) -> Option<LogLevel> {
    let level = level as i64;
    if level == MarsLogLevel::Verbose as i64 {
        Some(LogLevel::Verbose)
    } else if level == MarsLogLevel::Debug as i64 {
        Some(LogLevel::Debug)
    } else if level == MarsLogLevel::Info as i64 {
        Some(LogLevel::Info)
    } else if level == MarsLogLevel::Warn as i64 {
        Some(LogLevel::Warn)
    } else if level == MarsLogLevel::Error as i64 {
        Some(LogLevel::Error)
    } else if level == MarsLogLevel::Fatal as i64 {
        Some(LogLevel::Fatal)
    } else {
        None
    }
}

/// Empty strings become `None` so the formatter omits the `[tag]` /
/// `[file:line, func]` fields, matching the C++ behaviour for a null `char*`.
fn opt_string(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// The path as raw bytes for the C buffer. Non-UTF-8 paths are lossily
/// converted on platforms without `OsStrExt` (a log directory containing such a
/// path is a caller bug, not a reason to fail the query).
fn path_to_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().into_owned().into_bytes()
    }
}

/// Creates a logger instance with its own appender.
///
/// This is the C counterpart of `mars::xlog::NewXloggerInstance`: unlike
/// [`mars_xlog_open`], which configures the single process-wide appender, each
/// instance gets its own log directory, prefix, key, mode and cache file.
///
/// Returns the instance handle, or `0` when `config` is null / invalid or the
/// appender cannot be opened. A level outside `0..=6` is reported as
/// [`MARS_XLOG_ERR_BAD_MODE`]-style failure by returning `0`.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn mars_xlog_new_instance(
    config: *const MarsXLogConfig,
    level: c_int,
) -> c_longlong {
    guard(0, || {
        // SAFETY: null-checked inside `ptr_to_ref`.
        let Some(cfg) = (unsafe { cstr::ptr_to_ref(config) }) else {
            return 0;
        };
        let mode = match cfg.mode {
            x if x == MarsAppenderMode::Async as c_int => AppenderMode::Async,
            x if x == MarsAppenderMode::Sync as c_int => AppenderMode::Sync,
            _ => return 0,
        };
        let compress_mode = match cfg.compress_mode {
            x if x == MarsCompressMode::Zlib as c_int => CompressMode::Zlib,
            x if x == MarsCompressMode::Zstd as c_int => CompressMode::Zstd,
            _ => return 0,
        };
        let level = match to_log_level(level) {
            Some(level) => level,
            None => return 0,
        };

        // SAFETY: every field pointer is null-checked inside the helpers.
        let (log_dir, name_prefix, pub_key, cache_dir) = unsafe {
            (
                cstr::ptr_to_path_buf(cfg.log_dir),
                cstr::ptr_to_str_or_empty(cfg.name_prefix),
                cstr::ptr_to_str_or_empty(cfg.pub_key),
                cstr::ptr_to_path_buf(cfg.cache_dir),
            )
        };
        if log_dir.as_os_str().is_empty() {
            return 0;
        }

        let rust_config = XLogConfig {
            mode,
            logdir: log_dir,
            // An empty prefix stays empty, as in the C++ XLogConfig.
            nameprefix: name_prefix.to_string(),
            pub_key: pub_key.to_string(),
            compress_mode,
            compress_level: if cfg.compress_level > 0 {
                cfg.compress_level
            } else {
                XLogConfig::default().compress_level
            },
            cachedir: if cache_dir.as_os_str().is_empty() {
                None
            } else {
                Some(cache_dir)
            },
            cache_days: cfg.cache_days.max(0) as u32,
        };
        mars_xlog_appender::new_xlogger_instance(&rust_config, level) as c_longlong
    })
}

/// The handle registered for `name_prefix`, or `0` when there is none.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn mars_xlog_get_instance(name_prefix: *const c_char) -> c_longlong {
    guard(0, || {
        // SAFETY: null is reported as an empty string by the helper.
        let prefix = unsafe { cstr::ptr_to_str_or_empty(name_prefix) };
        mars_xlog_appender::get_xlogger_instance(prefix) as c_longlong
    })
}

/// Releases the instance registered for `name_prefix` and closes its appender.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn mars_xlog_release_instance(name_prefix: *const c_char) {
    let _ = guard(0, || {
        // SAFETY: null is reported as an empty string by the helper.
        let prefix = unsafe { cstr::ptr_to_str_or_empty(name_prefix) };
        mars_xlog_appender::release_xlogger_instance(prefix);
        0
    });
}

/// Writes through a specific instance (`0` = the process-wide appender).
///
/// Unlike [`mars_xlog_write`], this honours the instance's own level: a record
/// below it is dropped, and an unknown non-zero handle writes nothing.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn mars_xlog_write_instance(
    instance: c_longlong,
    level: c_int,
    tag: *const c_char,
    filename: *const c_char,
    func_name: *const c_char,
    line: c_int,
    log: *const c_char,
) {
    let _ = guard(0, || {
        // SAFETY: every pointer is null-checked inside the helpers.
        let (tag, filename, func_name, log) = unsafe {
            (
                cstr::ptr_to_str_or_empty(tag),
                cstr::ptr_to_str_or_empty(filename),
                cstr::ptr_to_str_or_empty(func_name),
                cstr::ptr_to_str_or_empty(log),
            )
        };
        if log.is_empty() {
            return 0;
        }
        let info = XLoggerInfo {
            level: to_log_level(level).unwrap_or(LogLevel::Verbose),
            tag: Some(tag.to_owned()),
            filename: Some(filename.to_owned()),
            func_name: Some(func_name.to_owned()),
            line,
            // -1 makes the category fill these in from the OS.
            pid: -1,
            tid: -1,
            maintid: -1,
            timeval: state::now_timeval(),
        };
        mars_xlog_appender::xlogger_write(instance as u64, Some(&info), Some(log));
        0
    });
}

/// `mars::xlog::IsEnabledFor` — `0` when the instance would drop this level.
#[no_mangle]
pub extern "C" fn mars_xlog_is_enabled_for(instance: c_longlong, level: c_int) -> c_int {
    guard(0, || match to_log_level(level) {
        Some(level) => c_int::from(mars_xlog_appender::is_enabled_for(instance as u64, level)),
        None => 0,
    })
}

/// `mars::xlog::GetLevel` — the instance's level, or `-1` when the handle is
/// unknown.
#[no_mangle]
pub extern "C" fn mars_xlog_get_level(instance: c_longlong) -> c_int {
    guard(-1, || {
        match mars_xlog_appender::get_level(instance as u64) {
            Some(level) => level as c_int,
            None => -1,
        }
    })
}

/// `mars::xlog::SetLevel` for an instance (`0` = the default logger).
#[no_mangle]
pub extern "C" fn mars_xlog_set_level_instance(instance: c_longlong, level: c_int) {
    let _ = guard(0, || {
        if let Some(level) = to_log_level(level) {
            mars_xlog_appender::set_level(instance as u64, level);
        }
        0
    });
}

/// `mars::xlog::appender_setmode` — switches the process-wide appender
/// between async and sync.
#[no_mangle]
pub extern "C" fn mars_xlog_set_mode(mode: c_int) {
    let _ = guard(0, || {
        let mode = match mode {
            x if x == MarsAppenderMode::Async as c_int => AppenderMode::Async,
            x if x == MarsAppenderMode::Sync as c_int => AppenderMode::Sync,
            _ => return 0,
        };
        mars_xlog_appender::appender_set_mode(mode);
        0
    });
}

/// `mars::xlog::SetAppenderMode` for an instance.
#[no_mangle]
pub extern "C" fn mars_xlog_set_mode_instance(instance: c_longlong, mode: c_int) {
    let _ = guard(0, || {
        let mode = match mode {
            x if x == MarsAppenderMode::Async as c_int => AppenderMode::Async,
            x if x == MarsAppenderMode::Sync as c_int => AppenderMode::Sync,
            _ => return 0,
        };
        mars_xlog_appender::set_appender_mode(instance as u64, mode);
        0
    });
}

/// Drains an instance (`sync` non-zero waits for the write to complete).
#[no_mangle]
pub extern "C" fn mars_xlog_flush_instance(instance: c_longlong, sync: c_int) {
    let _ = guard(0, || {
        mars_xlog_appender::flush(instance as u64, sync != 0);
        0
    });
}

/// The cache directory of an instance; see [`mars_xlog_current_log_path`] for
/// the buffer contract (`0` when there is none).
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn mars_xlog_current_log_cache_path(out: *mut c_uchar, len: c_uint) -> c_int {
    guard(MARS_XLOG_ERR_PANIC, || {
        if out.is_null() {
            return MARS_XLOG_ERR_NULL_OUT;
        }
        let Some(path) = mars_xlog_appender::appender_get_current_log_cache_path() else {
            return MARS_XLOG_ERR_NO_PATH;
        };
        // SAFETY: the caller's buffer contract is the same as for
        // `mars_xlog_current_log_path`.
        unsafe { write_path_into(path.as_os_str().as_encoded_bytes().to_vec(), out, len) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_enums_match_the_c_header() {
        assert_eq!(MarsAppenderMode::Async as c_int, 0);
        assert_eq!(MarsAppenderMode::Sync as c_int, 1);
        assert_eq!(MarsCompressMode::Zlib as c_int, 0);
        assert_eq!(MarsCompressMode::Zstd as c_int, 1);
        assert_eq!(MarsLogLevel::Verbose as c_int, 0);
        assert_eq!(MarsLogLevel::Debug as c_int, 1);
        assert_eq!(MarsLogLevel::Info as c_int, 2);
        assert_eq!(MarsLogLevel::Warn as c_int, 3);
        assert_eq!(MarsLogLevel::Error as c_int, 4);
        assert_eq!(MarsLogLevel::Fatal as c_int, 5);
    }

    #[test]
    fn level_mapping_is_total_over_the_valid_range() {
        for (raw, expected) in [
            (0, LogLevel::Verbose),
            (1, LogLevel::Debug),
            (2, LogLevel::Info),
            (3, LogLevel::Warn),
            (4, LogLevel::Error),
            (5, LogLevel::Fatal),
        ] {
            assert_eq!(to_log_level(raw), Some(expected));
        }
        assert_eq!(
            to_log_level(6),
            None,
            "kLevelNone must not produce a record"
        );
        assert_eq!(to_log_level(-1), None);
        assert_eq!(to_log_level(99), None);
    }

    #[test]
    fn empty_strings_become_none() {
        assert_eq!(opt_string(""), None);
        assert_eq!(opt_string("tag").as_deref(), Some("tag"));
    }

    #[test]
    fn config_layout_is_c_compatible() {
        // The C header's field order (`mode`, `log_dir`, `name_prefix`,
        // `pub_key`, `compress_mode`, `compress_level`, `cache_dir`,
        // `cache_days`) is what `mars::xlog::XLogConfig` uses too, so a C
        // caller's aggregate initialiser lands on the right fields.
        let cfg = MarsXLogConfig {
            mode: 1,
            log_dir: std::ptr::null(),
            name_prefix: std::ptr::null(),
            pub_key: std::ptr::null(),
            compress_mode: 0,
            compress_level: 6,
            cache_dir: std::ptr::null(),
            cache_days: 3,
        };
        assert_eq!(cfg.mode, MarsAppenderMode::Sync as c_int);
        assert_eq!(cfg.compress_mode, MarsCompressMode::Zlib as c_int);
        assert_eq!(cfg.cache_days, 3);
    }

    #[test]
    fn path_to_bytes_is_utf8_on_unix() {
        #[cfg(unix)]
        assert_eq!(
            path_to_bytes(Path::new("/tmp/a.xlog")),
            b"/tmp/a.xlog".to_vec()
        );
    }
}
