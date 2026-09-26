//! The exported `extern "C"` symbols.
//!
//! Every function in this module is a mechanical translation of one C++ entry
//! point: null-check the pointers, convert to the Rust types of
//! `mars-appender`, delegate. Nothing else — the level filter is
//! `mars-appender`'s (one store, shared with every instance), the identity
//! fields of a record come from [`crate::state`], and all pointer handling from
//! [`crate::cstr`].

use std::ffi::{c_char, c_int, c_longlong, c_uchar, c_uint, c_ulonglong};
use std::path::Path;

use mars_appender::{
    appender_close, appender_flush, appender_flush_sync, appender_get_current_log_path,
    appender_open, appender_set_console_log, appender_set_max_alive_duration,
    appender_set_max_file_size, appender_write,
    category_set_max_alive_duration as set_max_alive_duration,
    category_set_max_file_size as set_max_file_size, flush_all, set_console_log_open, set_level,
    AppenderMode, LogLevel, XLogConfig, XLoggerInfo, DEFAULT_HANDLE,
};
use mars_buffer::CompressMode;

use crate::cstr;
pub use crate::error::*;
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
///
/// # Safety
///
/// `config` must be null, or point to an initialised `MarsXLogConfig` that stays alive for the
/// duration of the call; every `char*` in it must be null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_open(config: *const MarsXLogConfig) -> c_int {
    guard(MARS_XLOG_ERR_PANIC, || {
        // SAFETY: `config` may be null (checked inside `ptr_to_ref`); otherwise
        // the caller guarantees a valid, aligned, initialised `MarsXLogConfig`
        // that stays alive for the duration of this call.
        let Some(cfg) = (unsafe { cstr::ptr_to_ref(config) }) else {
            return MARS_XLOG_ERR_NULL_CONFIG;
        };
        // SAFETY: `cfg` is the caller's valid config, as above.
        let rust_config = match unsafe { to_xlog_config(cfg) } {
            Ok(config) => config,
            Err(code) => return code,
        };

        match appender_open(rust_config) {
            Ok(()) => MARS_XLOG_OK,
            Err(err) => {
                // The C++ returned `void` here and silently did nothing; surfacing
                // the reason on stderr is the only diagnostic a host process gets.
                eprintln!("[mars-ffi] appender_open failed: {err}");
                MARS_XLOG_ERR_APPENDER
            }
        }
    })
}

/// The Rust config behind a C one, or the `MARS_XLOG_ERR_*` code that makes it
/// unusable: a mode outside [`MarsAppenderMode`], a compress mode outside
/// [`MarsCompressMode`], or an empty `log_dir`.
///
/// # Safety
///
/// `cfg` must point to a valid, aligned, initialised `MarsXLogConfig` that
/// stays alive for the duration of this call.
unsafe fn to_xlog_config(cfg: &MarsXLogConfig) -> Result<XLogConfig, c_int> {
    let mode = match cfg.mode {
        x if x == MarsAppenderMode::Async as c_int => AppenderMode::Async,
        x if x == MarsAppenderMode::Sync as c_int => AppenderMode::Sync,
        _ => return Err(MARS_XLOG_ERR_BAD_MODE),
    };

    let compress_mode = match cfg.compress_mode {
        x if x == MarsCompressMode::Zlib as c_int => CompressMode::Zlib,
        x if x == MarsCompressMode::Zstd as c_int => CompressMode::Zstd,
        _ => return Err(MARS_XLOG_ERR_BAD_COMPRESS),
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
        return Err(MARS_XLOG_ERR_EMPTY_LOG_DIR);
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
    Ok(XLogConfig {
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
///
/// # Safety
///
/// `tag`, `filename`, `func_name` and `message` must each be null, or a NUL-terminated C string
/// that stays alive for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_write(
    level: c_int,
    tag: *const c_char,
    filename: *const c_char,
    func_name: *const c_char,
    line: c_int,
    message: *const c_char,
) {
    guard((), || {
        // A record of a level that is not `Verbose..=Fatal` — `kLevelNone`, or
        // a negative one — is dropped, the way `xlogger_IsEnabledFor` answers
        // for it.
        let Some(level) = to_log_level(level) else {
            return;
        };
        if !enabled_for(DEFAULT_HANDLE, level as c_int) {
            return;
        }

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

/// `xlogger_SetLevel(TLogLevel)` — the minimum level a record must have to be
/// written. `MARS_LEVEL_NONE` (6) or higher disables logging completely, and a
/// negative value is "log everything", the way `(TLogLevel)-1` was in the C++.
///
/// It is the **default logger's** level: `0` is the process-wide appender, and
/// [`mars_xlog_get_level`], [`mars_xlog_is_enabled_for`] and
/// [`mars_xlog_write_instance`] all read that one. A level of its own here
/// would have the ABI answer two different levels for the same logger.
#[no_mangle]
pub extern "C" fn mars_xlog_set_level(level: c_int) {
    guard((), || set_level(DEFAULT_HANDLE, to_filter_level(level)));
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
///
/// # Safety
///
/// `out` must be null, or point to at least `len` writable bytes that stay alive for the duration
/// of the call.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_current_log_path(out: *mut c_char, len: c_uint) -> c_int {
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
/// `Verbose..=Fatal`. Those are the levels a *record* can have: `kLevelNone`
/// (6) is not one of them, and the C++'s own `levelStrings[]` has no string for
/// it either.
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

/// What a raw `TLogLevel` means as a *filter*, which is what
/// `xlogger_SetLevel((TLogLevel)_level)` does with it: the C++ casts straight
/// to the enum, so `kLevelNone` (6) — and anything above it — disables logging
/// altogether, and a negative value is "everything", which is
/// [`LogLevel::Verbose`].
fn to_filter_level(level: c_int) -> LogLevel {
    match to_log_level(level) {
        Some(level) => level,
        None if level < 0 => LogLevel::Verbose,
        None => LogLevel::None,
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
/// appender cannot be opened. A level outside `0..=5` is accepted: the C++
/// casts it, so `MARS_LEVEL_NONE` (6) is "an instance that logs nothing".
///
/// # Safety
///
/// `config` must be null, or point to an initialised `MarsXLogConfig` that stays alive for the
/// duration of the call; every `char*` in it must be null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_new_instance(
    config: *const MarsXLogConfig,
    level: c_int,
) -> c_longlong {
    guard(0, || {
        // SAFETY: null-checked inside `ptr_to_ref`.
        let Some(cfg) = (unsafe { cstr::ptr_to_ref(config) }) else {
            return 0;
        };
        // SAFETY: `cfg` is the caller's valid config, as above. A config the
        // appender cannot use has no instance, which is what `0` means.
        let Ok(rust_config) = (unsafe { to_xlog_config(cfg) }) else {
            return 0;
        };
        // `NewXloggerInstance(_config, (TLogLevel)_level)`: the level is cast,
        // never checked — `MARS_LEVEL_NONE` (6) is the level a caller starts an
        // instance at when it wants it silent, and it used to be refused here,
        // which silently gave back handle `0`, the appender-less default.
        let level = to_filter_level(level);

        mars_appender::new_xlogger_instance(&rust_config, level) as c_longlong
    })
}

/// The handle registered for `name_prefix`, or `0` when there is none.
///
/// # Safety
///
/// `name_prefix` must be null, or a NUL-terminated C string that stays alive for the duration of
/// the call.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_get_instance(name_prefix: *const c_char) -> c_longlong {
    guard(0, || {
        // SAFETY: null is reported as an empty string by the helper.
        let prefix = unsafe { cstr::ptr_to_str_or_empty(name_prefix) };
        mars_appender::get_xlogger_instance(prefix) as c_longlong
    })
}

/// Releases the instance registered for `name_prefix` and closes its appender.
///
/// # Safety
///
/// `name_prefix` must be null, or a NUL-terminated C string that stays alive for the duration of
/// the call.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_release_instance(name_prefix: *const c_char) {
    let _ = guard(0, || {
        // SAFETY: null is reported as an empty string by the helper.
        let prefix = unsafe { cstr::ptr_to_str_or_empty(name_prefix) };
        mars_appender::release_xlogger_instance(prefix);
        0
    });
}

/// Writes through a specific instance (`0` = the process-wide appender).
///
/// The instance's own level decides: a record below it is dropped, and an
/// unknown non-zero handle writes nothing. For `0` that level is the one
/// [`mars_xlog_set_level`] set, the same one [`mars_xlog_write`] asks.
///
/// # Safety
///
/// `tag`, `filename`, `func_name` and `message` must each be null, or a NUL-terminated C string
/// that stays alive for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_write_instance(
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
        // A record's level is `Verbose..=Fatal` here as well: `kLevelNone` is
        // nothing to write, not a verbose record, which is what turning it into
        // one would make it.
        let Some(level) = to_log_level(level) else {
            return 0;
        };
        let info = XLoggerInfo {
            level,
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
        mars_appender::xlogger_write(instance as u64, Some(&info), Some(log));
        0
    });
}

/// `mars::xlog::IsEnabledFor` — `1` when the instance would write this level.
///
/// `level_ <= _level`, on the **raw** `TLogLevel` the caller passed: the C++
/// casts it (`(TLogLevel)_level`, `Java2C_Xlog.cc`) and never checks it, so
/// `MARS_LEVEL_NONE` (6) is a level a caller may ask *about* — and asking about
/// it is not the same as asking about `Fatal`, which is what collapsing it onto
/// `Fatal` (or answering nothing) used to do.
#[no_mangle]
pub extern "C" fn mars_xlog_is_enabled_for(instance: c_longlong, level: c_int) -> c_int {
    guard(0, || c_int::from(enabled_for(instance as u64, level)))
}

/// Whether `handle` would write a record of the raw level `level`
/// (`xlogger_IsEnabledFor` for handle `0`, `XloggerCategory::IsEnabledFor` for
/// an instance).
fn enabled_for(handle: u64, level: c_int) -> bool {
    match mars_appender::get_level(handle) {
        Some(stored) => (stored as i32) <= level,
        // A handle that is not one: nothing is written through it, so nothing
        // is enabled for it either.
        None => false,
    }
}

/// `mars::xlog::GetLevel` — the instance's level, or `-1` when the handle is
/// unknown.
#[no_mangle]
pub extern "C" fn mars_xlog_get_level(instance: c_longlong) -> c_int {
    guard(-1, || match mars_appender::get_level(instance as u64) {
        Some(level) => level as c_int,
        None => -1,
    })
}

/// `mars::xlog::SetLevel` for an instance (`0` = the default logger).
///
/// `kLevelNone` (6) and anything above it disables the instance, and a
/// negative level logs everything: the C++ casts the value straight to
/// `TLogLevel`, and a level outside `Verbose..=Fatal` is not one to answer
/// nothing at all to.
#[no_mangle]
pub extern "C" fn mars_xlog_set_level_instance(instance: c_longlong, level: c_int) {
    let _ = guard(0, || {
        set_level(instance as u64, to_filter_level(level));
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
        mars_appender::appender_set_mode(mode);
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
        mars_appender::set_appender_mode(instance as u64, mode);
        0
    });
}

/// Drains an instance (`sync` non-zero waits for the write to complete).
#[no_mangle]
pub extern "C" fn mars_xlog_flush_instance(instance: c_longlong, sync: c_int) {
    let _ = guard(0, || {
        mars_appender::flush(instance as u64, sync != 0);
        0
    });
}

/// `mars::xlog::FlushAll` — drains the process-wide appender *and* every
/// instance (`sync` non-zero waits for the write to complete).
///
/// The instances matter: each of them owns an appender of its own, so a caller
/// that flushes before collecting logs or suspending misses their records
/// otherwise.
#[no_mangle]
pub extern "C" fn mars_xlog_flush_all(sync: c_int) {
    guard((), || flush_all(sync != 0));
}

/// `mars::xlog::SetConsoleLogOpen` for an instance (`0` = the default logger).
#[no_mangle]
pub extern "C" fn mars_xlog_set_console_log_instance(instance: c_longlong, open: c_int) {
    guard((), || set_console_log_open(instance as u64, open != 0));
}

/// `mars::xlog::SetMaxFileSize` for an instance; `0` means "never split".
#[no_mangle]
pub extern "C" fn mars_xlog_set_max_file_size_instance(instance: c_longlong, bytes: c_ulonglong) {
    guard((), || set_max_file_size(instance as u64, bytes));
}

/// `mars::xlog::SetMaxAliveTime` for an instance; negative clamps to 0.
#[no_mangle]
pub extern "C" fn mars_xlog_set_max_alive_duration_instance(
    instance: c_longlong,
    seconds: c_longlong,
) {
    guard((), || {
        set_max_alive_duration(instance as u64, seconds.max(0) as u64)
    });
}

/// The cache directory of an instance; see [`mars_xlog_current_log_path`] for
/// the buffer contract (`0` when there is none).
///
/// # Safety
///
/// `out` must be null, or point to at least `len` writable bytes that stay alive for the duration
/// of the call.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_current_log_cache_path(out: *mut c_uchar, len: c_uint) -> c_int {
    guard(MARS_XLOG_ERR_PANIC, || {
        if out.is_null() {
            return MARS_XLOG_ERR_NULL_OUT;
        }
        let Some(path) = mars_appender::appender_get_current_log_cache_path() else {
            return MARS_XLOG_ERR_NO_PATH;
        };
        // SAFETY: the caller's buffer contract is the same as for
        // `mars_xlog_current_log_path`.
        unsafe { write_path_into(path.as_os_str().as_encoded_bytes().to_vec(), out, len) }
    })
}

/// `mars::xlog::appender_oneshot_flush` — drains an `<prefix>.mmap3` that
/// another process left behind, without opening an appender.
///
/// This is the "another process died with a full cache" recovery path, and it
/// refuses to run for a directory an appender of this process already owns
/// ([`MARS_XLOG_OK`] plus `kActionUnnecessary`): reading that cache file
/// mid-write and unlinking it loses every record the live appender buffers
/// afterwards.
///
/// @return the `TFileIOAction` the recovery ended in — one of
/// `kActionNone` (0) … `kActionRemoveFailed` (7) — or a negative
/// `MARS_XLOG_ERR_*` code when `config` is unusable.
///
/// # Safety
///
/// `config` must be null, or point to an initialised `MarsXLogConfig` that stays alive for the
/// duration of the call; every `char*` in it must be null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_oneshot_flush(config: *const MarsXLogConfig) -> c_int {
    guard(MARS_XLOG_ERR_PANIC, || {
        // SAFETY: null-checked inside `ptr_to_ref`.
        let Some(cfg) = (unsafe { cstr::ptr_to_ref(config) }) else {
            return MARS_XLOG_ERR_NULL_CONFIG;
        };
        // SAFETY: `cfg` is the caller's valid config, as above.
        let rust_config = match unsafe { to_xlog_config(cfg) } {
            Ok(config) => config,
            Err(code) => return code,
        };
        mars_appender::appender_oneshot_flush(&rust_config) as c_int
    })
}

/// `mars::xlog::appender_make_logfile_name` — the log file *name* for the day
/// `timespan` days ago (0 = today), whether or not it exists yet.
///
/// The C++ fills a `std::vector` (the log-dir file and, when a cache dir is
/// configured and the file exists, its cache-dir twin); a C caller walks the
/// same list with `index`, starting at `0` and stopping at
/// [`MARS_XLOG_ERR_NO_PATH`].
///
/// `prefix` and `log_dir` may be null (an empty `log_dir` yields no name at
/// all). See [`mars_xlog_current_log_path`] for the `out` contract.
///
/// # Safety
///
/// `prefix` and `log_dir` must each be null or a NUL-terminated C string, and `out` must be null
/// or point to at least `len` writable bytes; all of them stay alive for the duration of the
/// call.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_make_logfile_name(
    timespan: c_int,
    prefix: *const c_char,
    log_dir: *const c_char,
    index: c_uint,
    out: *mut c_char,
    len: c_uint,
) -> c_int {
    guard(MARS_XLOG_ERR_PANIC, || {
        // SAFETY: both pointers are null-checked inside the helpers.
        let (prefix, log_dir) = unsafe {
            (
                cstr::ptr_to_str_or_empty(prefix),
                cstr::ptr_to_path_buf(log_dir),
            )
        };
        let paths =
            mars_appender::appender_make_logfile_name(i64::from(timespan), prefix, &log_dir);
        // SAFETY: `out`/`len` are checked inside `path_at`.
        unsafe { path_at(&paths, index, out, len) }
    })
}

/// `mars::xlog::appender_getfilepath_from_timespan` — the log files that
/// *exist* for the day `timespan` days ago (0 = today).
///
/// Same protocol as [`mars_xlog_make_logfile_name`]: walk `index` from `0`
/// until it answers [`MARS_XLOG_ERR_NO_PATH`].
///
/// # Safety
///
/// `prefix` and `log_dir` must each be null or a NUL-terminated C string, and `out` must be null
/// or point to at least `len` writable bytes; all of them stay alive for the duration of the
/// call.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_getfilepath_from_timespan(
    timespan: c_int,
    prefix: *const c_char,
    log_dir: *const c_char,
    index: c_uint,
    out: *mut c_char,
    len: c_uint,
) -> c_int {
    guard(MARS_XLOG_ERR_PANIC, || {
        // SAFETY: both pointers are null-checked inside the helpers.
        let (prefix, log_dir) = unsafe {
            (
                cstr::ptr_to_str_or_empty(prefix),
                cstr::ptr_to_path_buf(log_dir),
            )
        };
        let paths = mars_appender::appender_getfilepath_from_timespan(
            i64::from(timespan),
            prefix,
            &log_dir,
        );
        // SAFETY: `out`/`len` are checked inside `path_at`.
        unsafe { path_at(&paths, index, out, len) }
    })
}

/// Copies `paths[index]` into the caller's buffer; [`MARS_XLOG_ERR_NO_PATH`]
/// when the list is shorter than `index`.
///
/// # Safety
///
/// `out` must be null or point to at least `len` writable bytes (both are
/// checked).
unsafe fn path_at(
    paths: &[std::path::PathBuf],
    index: c_uint,
    out: *mut c_char,
    len: c_uint,
) -> c_int {
    match paths.get(index as usize) {
        Some(path) => unsafe { write_path_into(path_to_bytes(path), out as *mut c_uchar, len) },
        None => MARS_XLOG_ERR_NO_PATH,
    }
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
