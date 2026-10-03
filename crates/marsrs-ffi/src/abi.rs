//! The exported `extern "C"` symbols.
//!
//! Every function in this module is a mechanical translation of one C++ entry
//! point: null-check the pointers, convert to the Rust types of
//! `marsrs-appender`, delegate. Nothing else — the level filter is
//! `marsrs-appender`'s (one store, shared with every instance), the identity
//! fields of a record come from [`crate::state`], and all pointer handling from
//! [`crate::cstr`].

use std::borrow::Cow;
use std::ffi::{c_char, c_int, c_longlong, c_uchar, c_uint, c_ulonglong};
use std::path::Path;

use marsrs_appender::{
    category_set_max_alive_duration as set_max_alive_duration,
    category_set_max_file_size as set_max_file_size, current_log_path, flush_now, request_flush,
    set_console_log_open, set_level, AppenderMode, LogLevel, XLogConfig, XLoggerInfo,
    XloggerHandle,
};
use marsrs_buffer::CompressMode;

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
            cstr::ptr_to_string_lossy(cfg.name_prefix),
            cstr::ptr_to_str_or_empty(cfg.pub_key),
            cstr::ptr_to_path_buf(cfg.cache_dir),
        )
    };

    if log_dir.as_os_str().is_empty() {
        return Err(MARS_XLOG_ERR_EMPTY_LOG_DIR);
    }

    // A non-positive compress level keeps the appender's default. An empty
    // prefix must stay empty: the C++ `XLogConfig::nameprefix_` has no default
    // either, so it produces `.mmap3` / `_YYYYMMDD.xlog` and cache discovery is
    // prefix-based — substituting "Mars" would stop the Rust port from draining
    // (or being drained by) a C++ process's cache file. The prefix does go
    // through UTF-8, `XLogConfig` storing a `String`, so a non-UTF-8 one is
    // converted lossily and not to the empty prefix, which is the name of
    // another appender and not of a prefix that failed; the directories above
    // are byte-exact.
    let defaults = XLogConfig::default();
    Ok(XLogConfig {
        mode,
        logdir: log_dir,
        nameprefix: name_prefix,
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

/// The directory the appender of `instance` writes its files into.
///
/// The only spelling there is: a process-wide question would be about an
/// appender no symbol of this ABI opens, so `0` — and every handle no
/// appender is open for — answers [`MARS_XLOG_ERR_NO_PATH`].
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
pub unsafe extern "C" fn mars_xlog_current_log_path_instance(
    instance: c_longlong,
    out: *mut c_char,
    len: c_uint,
) -> c_int {
    guard(MARS_XLOG_ERR_PANIC, || {
        if out.is_null() {
            return MARS_XLOG_ERR_NULL_OUT;
        }
        if len == 0 {
            return MARS_XLOG_ERR_NO_SPACE;
        }

        let Some(path) = current_log_path(instance as u64) else {
            return MARS_XLOG_ERR_NO_PATH;
        };

        // SAFETY: `out` is non-null (checked above) and the caller promises
        // `len` writable bytes.
        unsafe { write_path_into(path_to_bytes(&path), out as *mut c_uchar, len) }
    })
}
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
///
/// The non-empty case borrows: the C++ `XLoggerInfo` holds the caller's
/// `const char*` as-is, and copying it into a `String` was one allocation per
/// field per record.
fn opt_string(value: &str) -> Option<Cow<'_, str>> {
    if value.is_empty() {
        None
    } else {
        Some(Cow::Borrowed(value))
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
/// This is the C counterpart of `mars::xlog::NewXloggerInstance`: each
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

        marsrs_appender::new_xlogger_instance(&rust_config, level) as c_longlong
    })
}

/// The handle registered for `name_prefix`, or `0` when there is none.
///
/// The prefix is read the way [`mars_xlog_new_instance`] read the one it
/// registered — lossily — so one that is not UTF-8 names the appender it
/// opened. Read as text, the whole of such a prefix is `""`, which is no
/// prefix at all: an appender that was open was answered with `0`.
///
/// # Safety
///
/// `name_prefix` must be null, or a NUL-terminated C string that stays alive for the duration of
/// the call.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_get_instance(name_prefix: *const c_char) -> c_longlong {
    guard(0, || {
        // SAFETY: null is reported as an empty prefix by the helper.
        let prefix = unsafe { cstr::ptr_to_string_lossy(name_prefix) };
        marsrs_appender::get_xlogger_instance(&prefix) as c_longlong
    })
}

/// Releases the instance registered for `name_prefix` and closes its appender.
/// Prefer [`mars_xlog_release_instance_of`] when the caller holds the handle;
/// this legacy prefix-only entry point cannot distinguish a stale close from a
/// newly opened instance with the same prefix.
///
/// The prefix is read the way the open read it, so an appender whose prefix is
/// not UTF-8 is one this closes: read as text, the whole of such a prefix is
/// `""`, and the appender — its mmap, its files and its writer — stayed open
/// for as long as the process did.
///
/// # Safety
///
/// `name_prefix` must be null, or a NUL-terminated C string that stays alive for the duration of
/// the call.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_release_instance(name_prefix: *const c_char) {
    let _ = guard(0, || {
        // SAFETY: null is reported as an empty prefix by the helper.
        let prefix = unsafe { cstr::ptr_to_string_lossy(name_prefix) };
        marsrs_appender::release_xlogger_instance(&prefix);
        0
    });
}

/// Releases the instance registered for `name_prefix`, and only while it is
/// still the one `instance` names.
///
/// [`mars_xlog_release_instance`] takes the prefix and not the handle, so a
/// caller that asks the registry which handle the prefix answers and then
/// releases is answered twice and not once: an `open` of the same prefix that
/// lands between the two is handed a handle of its own, and the release closes
/// *that* appender — the one the caller asked about is already gone and the one
/// it just closed is another's. This is the question and the release under one
/// lock, so nothing can land between them, which is also what makes a second
/// `close` of one prefix a no-op however many `Xlog`s hold its handle.
///
/// # Safety
///
/// `name_prefix` must be null, or a NUL-terminated C string that stays alive for the duration of
/// the call.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_release_instance_of(
    name_prefix: *const c_char,
    instance: c_longlong,
) {
    let _ = guard(0, || {
        // SAFETY: null is reported as an empty prefix by the helper.
        let prefix = unsafe { cstr::ptr_to_string_lossy(name_prefix) };
        marsrs_appender::release_xlogger_instance_of(&prefix, instance as XloggerHandle);
        0
    });
}

/// Writes through a specific instance.
///
/// The instance's own level decides: a record below it is dropped. A handle
/// that is not one writes nothing, `0` among them — no symbol of this ABI
/// installs a process-wide appender for it to write through.
///
/// A null `log` is the C++'s own `NULL == _log`: the write happens, at
/// `Fatal`, and says so — which is the one case a caller is told about a
/// body it never gave. An empty one is not written at all.
///
/// # Safety
///
/// `tag`, `filename`, `func_name` and `log` must each be null, or a NUL-terminated C string
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
        let (tag, filename, func_name) = unsafe {
            (
                cstr::ptr_to_str_or_empty(tag),
                cstr::ptr_to_str_or_empty(filename),
                cstr::ptr_to_str_or_empty(func_name),
            )
        };
        // A body that is not UTF-8 keeps what it can of itself, the way a
        // prefix does: `ptr_to_str_or_empty` answers `""` for the whole of
        // one whose single byte fails to decode, and `""` is what the check
        // below drops — a GBK message would have disappeared instead of
        // being written. A null one is kept as `None`, which is the `NULL ==
        // _log` the appender promotes. Borrowed while the body is UTF-8, so
        // the write itself still costs no allocation.
        let log = if log.is_null() {
            None
        } else {
            // SAFETY: `log` is non-null and — per the caller's contract —
            // points to a valid NUL-terminated string that outlives this call.
            Some(match unsafe { cstr::ptr_to_str(log) } {
                Some(log) => Cow::Borrowed(log),
                None => Cow::Owned(unsafe { cstr::ptr_to_string_lossy(log) }),
            })
        };
        // An empty body writes nothing. A null one is not empty but absent,
        // and it is written: `NULL == _log` is the appender's own promotion.
        if log.as_deref() == Some("") {
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
            // Borrowed, like the C++ `const char*` fields: one `String` per
            // field per record was three allocations a `.xlog` write never
            // needed.
            tag: opt_string(tag),
            filename: opt_string(filename),
            func_name: opt_string(func_name),
            line,
            // -1 makes the category fill these in from the OS.
            pid: -1,
            tid: -1,
            maintid: -1,
            timeval: state::now_timeval(),
            trace_log: 0,
        };
        marsrs_appender::xlogger_write(instance as u64, Some(&info), log.as_deref());
        0
    });
}

/// `mars::xlog::IsEnabledFor` — `1` when the instance would write this level.
///
/// `level_ <= _level`, on the **`TLogLevel`** the caller passed and not on a
/// collapsed copy of it: the C++ casts it (`(TLogLevel)_level`,
/// `Java2C_Xlog.cc`) and never checks it, so a level of `-1` is asked about as
/// `-1` — below `Verbose`, and answered `0` — and not as the `Verbose` the
/// *filter* would make of it.
///
/// A level no record can carry is answered `0`, whatever the filter is:
/// `MARS_LEVEL_NONE` (6) is a filter that drops everything, and the write
/// refuses it as a record's level, so `1` here would promise a caller a write
/// that never happens.
#[no_mangle]
pub extern "C" fn mars_xlog_is_enabled_for(instance: c_longlong, level: c_int) -> c_int {
    guard(0, || c_int::from(enabled_for(instance as u64, level)))
}

/// Whether `handle` would write a record of the raw level `level`
/// (`xlogger_IsEnabledFor` for handle `0`, `XloggerCategory::IsEnabledFor` for
/// an instance).
fn enabled_for(handle: u64, level: c_int) -> bool {
    // A record has no level outside `Verbose..=Fatal`, and the write refuses
    // one that is outside it, so this does too: the question is whether the
    // instance would write, and nothing is written at such a level.
    let Some(_) = to_log_level(level) else {
        return false;
    };
    match marsrs_appender::get_level(handle) {
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
    guard(-1, || match marsrs_appender::get_level(instance as u64) {
        Some(level) => level as c_int,
        None => -1,
    })
}

/// `mars::xlog::SetLevel` for an instance: the level [`mars_xlog_get_level`]
/// and [`mars_xlog_is_enabled_for`] answer, and the one the instance's own
/// writes are filtered against. A no-op for `0`, which is no instance.
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

/// `mars::xlog::SetAppenderMode` for an instance.
#[no_mangle]
pub extern "C" fn mars_xlog_set_mode_instance(instance: c_longlong, mode: c_int) {
    let _ = guard(0, || {
        let mode = match mode {
            x if x == MarsAppenderMode::Async as c_int => AppenderMode::Async,
            x if x == MarsAppenderMode::Sync as c_int => AppenderMode::Sync,
            _ => return 0,
        };
        marsrs_appender::set_appender_mode(instance as u64, mode);
        0
    });
}

/// Drains one instance, signalled: the writer thread is told it may take what
/// is in the cache to the file, and this returns at once.
///
/// The instance matters: each of them owns an appender of its own, so a drain
/// asked of one instance is nobody else's records. A handle that is not one —
/// `0` among them — drains nothing.
#[no_mangle]
pub extern "C" fn mars_xlog_request_flush_instance(instance: c_longlong) {
    let _ = guard(0, || {
        request_flush(instance as u64);
        0
    });
}

/// Drains one instance on the calling thread: that instance's records are on
/// the disk when this returns, which [`mars_xlog_request_flush_instance`] does
/// not promise. A handle that is not one — `0` among them — drains nothing.
#[no_mangle]
pub extern "C" fn mars_xlog_flush_now_instance(instance: c_longlong) {
    let _ = guard(0, || {
        flush_now(instance as u64);
        0
    });
}

/// `mars::xlog::SetConsoleLogOpen` for an instance.
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
    guard(false, || {
        set_max_alive_duration(instance as u64, seconds.max(0) as u64)
    });
}

/// `mars::xlog::appender_make_logfile_name` — the log file *name* for the day
/// `timespan` days ago (0 = today), whether or not it exists yet.
///
/// The C++ fills a `std::vector` (the log-dir file and, when a cache dir is
/// configured and the file exists, its cache-dir twin); a C caller walks the
/// same list with `index`, starting at `0` and stopping at
/// [`MARS_XLOG_ERR_NO_PATH`].
///
/// `instance` is a handle from [`mars_xlog_new_instance`]; a handle no appender
/// is open for answers [`MARS_XLOG_ERR_NO_PATH`] from the first `index` on. See
/// [`mars_xlog_current_log_path_instance`] for the `out` contract.
///
/// # Safety
///
/// `out` must be null or point to at least `len` writable bytes; both are
/// checked.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_make_logfile_name_instance(
    instance: c_longlong,
    timespan: c_int,
    index: c_uint,
    out: *mut c_char,
    len: c_uint,
) -> c_int {
    guard(MARS_XLOG_ERR_PANIC, || {
        let paths =
            marsrs_appender::current_log_file_names(instance as XloggerHandle, i64::from(timespan));
        // SAFETY: `out`/`len` are checked inside `path_at`.
        unsafe { path_at(&paths, index, out, len) }
    })
}

/// `mars::xlog::appender_getfilepath_from_timespan` — the log files that
/// *exist* for the day `timespan` days ago (0 = today).
///
/// Same protocol as [`mars_xlog_make_logfile_name_instance`]: walk `index` from `0`
/// until it answers [`MARS_XLOG_ERR_NO_PATH`].
///
/// # Safety
///
/// `out` must be null or point to at least `len` writable bytes; both are
/// checked.
#[no_mangle]
pub unsafe extern "C" fn mars_xlog_getfilepath_from_timespan_instance(
    instance: c_longlong,
    timespan: c_int,
    index: c_uint,
    out: *mut c_char,
    len: c_uint,
) -> c_int {
    guard(MARS_XLOG_ERR_PANIC, || {
        let paths =
            marsrs_appender::current_log_files(instance as XloggerHandle, i64::from(timespan));
        // SAFETY: `out`/`len` are checked inside `path_at`.
        unsafe { path_at(&paths, index, out, len) }
    })
}

/// Copies `paths[index]` into the caller's buffer; [`MARS_XLOG_ERR_NO_PATH`]
/// when the list is shorter than `index`.
///
/// # Safety
///
/// `out` must be null, or point to at least `len` writable bytes that stay
/// alive for the duration of the call. Both are checked one frame down, in
/// [`write_path_into`], which is what answers `MARS_XLOG_ERR_NULL_OUT` and
/// `MARS_XLOG_ERR_NO_SPACE`.
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

    use std::fs;
    use std::mem::offset_of;
    use std::path::Path;

    /// The header these tests are read against: the one a C caller includes,
    /// and not a copy of it, which is what made the two below assert a number
    /// the header had already stopped agreeing with.
    fn header() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("include/mars_xlog.h");
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    }

    /// The value the header gives `variant`, of the enum it ends with `ty`.
    ///
    /// The block is found by the `} ty;` that closes it and not by the variant
    /// alone, so a variant named after another one's beginning —
    /// `MarsLevelVerbose` in `MarsLevelVerboseX` — is not taken for it.
    fn header_enum_value(header: &str, ty: &str, variant: &str) -> c_int {
        let end = header
            .find(&format!("}} {ty};"))
            .unwrap_or_else(|| panic!("include/mars_xlog.h declares no `{ty}`"));
        let start = header[..end]
            .rfind("typedef enum")
            .unwrap_or_else(|| panic!("`{ty}` in include/mars_xlog.h is not a `typedef enum`"));
        let block = &header[start..end];
        let at = block
            .find(&format!("{variant} ="))
            .unwrap_or_else(|| panic!("`{ty}` in include/mars_xlog.h has no `{variant}`"));
        let digits: String = block[at + variant.len() + 1..]
            .trim_start()
            .trim_start_matches('=')
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        assert!(
            !digits.is_empty(),
            "`{ty}`'s `{variant}` in include/mars_xlog.h has no value"
        );
        digits
            .parse()
            .unwrap_or_else(|e| panic!("`{ty}`'s `{variant}` is not an int: {digits} ({e})"))
    }

    /// The fields of `MarsXLogConfig` in the order the header declares them,
    /// comments and all: a field whose comment names another field with a `;`
    /// in it — `int mode; /* MarsAppenderMode; ... */` — is one a naive parse
    /// read as two.
    fn header_config_fields(header: &str) -> Vec<&str> {
        let end = header
            .find("} MarsXLogConfig;")
            .unwrap_or_else(|| panic!("include/mars_xlog.h declares no `MarsXLogConfig`"));
        let start = header[..end]
            .rfind("typedef struct")
            .unwrap_or_else(|| panic!("`MarsXLogConfig` is not a `typedef struct`"));
        let block = &header[start + "typedef struct".len()..end];
        let mut fields = Vec::new();
        for line in block.lines() {
            // The comment is what carries the `;` that is not a field's.
            let code = match line.find("/*") {
                Some(at) => &line[..at],
                None => line,
            };
            let code = code.trim();
            let Some(name) = code.strip_suffix(';') else {
                continue;
            };
            // `const char* name_prefix`, and the type in front of it.
            let name = name.rsplit([' ', '*']).next().unwrap_or(name);
            assert!(!name.is_empty(), "a field of MarsXLogConfig has no name");
            fields.push(name);
        }
        assert!(
            !fields.is_empty(),
            "no field of MarsXLogConfig found in include/mars_xlog.h"
        );
        fields
    }

    /// Every variant of the three enums is the integer the header spells for
    /// it, which is the only place a C caller reads them from: an enum here
    /// and a `typedef enum` there are the same numbers or a C caller asks for
    /// `Sync` and gets `Async`.
    #[test]
    fn abi_enums_match_the_c_header() {
        let header = header();
        for (ty, variant, value) in [
            (
                "MarsAppenderMode",
                "MarsAppenderAsync",
                MarsAppenderMode::Async as c_int,
            ),
            (
                "MarsAppenderMode",
                "MarsAppenderSync",
                MarsAppenderMode::Sync as c_int,
            ),
            (
                "MarsCompressMode",
                "MarsCompressZlib",
                MarsCompressMode::Zlib as c_int,
            ),
            (
                "MarsCompressMode",
                "MarsCompressZstd",
                MarsCompressMode::Zstd as c_int,
            ),
            (
                "MarsLogLevel",
                "MarsLevelVerbose",
                MarsLogLevel::Verbose as c_int,
            ),
            (
                "MarsLogLevel",
                "MarsLevelDebug",
                MarsLogLevel::Debug as c_int,
            ),
            ("MarsLogLevel", "MarsLevelInfo", MarsLogLevel::Info as c_int),
            ("MarsLogLevel", "MarsLevelWarn", MarsLogLevel::Warn as c_int),
            (
                "MarsLogLevel",
                "MarsLevelError",
                MarsLogLevel::Error as c_int,
            ),
            (
                "MarsLogLevel",
                "MarsLevelFatal",
                MarsLogLevel::Fatal as c_int,
            ),
        ] {
            assert_eq!(
                header_enum_value(&header, ty, variant),
                value as c_int,
                "include/mars_xlog.h out of sync: `{ty}`'s `{variant}`"
            );
        }
        // `MARS_LEVEL_NONE` is a `#define` beside the enum and not a variant of
        // it, because no record carries it: it is a filter, and only a filter.
        assert!(
            header.contains("#define MARS_LEVEL_NONE 6"),
            "include/mars_xlog.h out of sync: `MARS_LEVEL_NONE` is 6"
        );
        let none: c_int = header
            .lines()
            .find_map(|line| line.strip_prefix("#define MARS_LEVEL_NONE "))
            .expect("include/mars_xlog.h has no `MARS_LEVEL_NONE`")
            .trim()
            .parse()
            .expect("`MARS_LEVEL_NONE` in include/mars_xlog.h is not an int");
        assert_eq!(
            to_log_level(none),
            None,
            "`MARS_LEVEL_NONE` must not produce a record"
        );
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

    /// The fields of `MarsXLogConfig` are the header's, in the header's order:
    /// an aggregate initialiser on the C side — `{ .mode = 1, .log_dir = dir }`
    /// — names them by position, so a Rust field that moved is a config a C
    /// caller reads as one it never wrote.
    ///
    /// The offsets are the compiler's, and not numbers written down beside the
    /// struct: it is the order they are in that is asserted, and an order
    /// written twice is one that is checked against itself.
    #[test]
    fn config_layout_is_c_compatible() {
        let header = header();
        let fields = header_config_fields(&header);
        let offsets: Vec<(&str, usize)> = vec![
            ("mode", offset_of!(MarsXLogConfig, mode)),
            ("log_dir", offset_of!(MarsXLogConfig, log_dir)),
            ("name_prefix", offset_of!(MarsXLogConfig, name_prefix)),
            ("pub_key", offset_of!(MarsXLogConfig, pub_key)),
            ("compress_mode", offset_of!(MarsXLogConfig, compress_mode)),
            ("compress_level", offset_of!(MarsXLogConfig, compress_level)),
            ("cache_dir", offset_of!(MarsXLogConfig, cache_dir)),
            ("cache_days", offset_of!(MarsXLogConfig, cache_days)),
        ];
        let mine: Vec<&str> = {
            let mut mine: Vec<(&str, usize)> = offsets.clone();
            mine.sort_by_key(|(_, offset)| *offset);
            mine.into_iter().map(|(name, _)| name).collect()
        };
        assert_eq!(
            mine, fields,
            "include/mars_xlog.h and src/abi.rs lay MarsXLogConfig out differently"
        );
        // One field, one offset: a struct the compiler padded into the same
        // place twice is one the C side reads as a shorter config.
        let unique: std::collections::BTreeSet<usize> =
            offsets.iter().map(|(_, offset)| *offset).collect();
        assert_eq!(
            unique.len(),
            offsets.len(),
            "two fields of MarsXLogConfig are at the same offset"
        );
    }

    #[test]
    fn a_prefix_that_is_not_utf8_stays_a_prefix() {
        let log_dir = c"/tmp/xlog";
        let name_prefix = c"app\xffname";
        let cfg = MarsXLogConfig {
            mode: MarsAppenderMode::Async as c_int,
            log_dir: log_dir.as_ptr(),
            name_prefix: name_prefix.as_ptr(),
            pub_key: std::ptr::null(),
            compress_mode: MarsCompressMode::Zlib as c_int,
            compress_level: 0,
            cache_dir: std::ptr::null(),
            cache_days: 0,
        };
        // SAFETY: every pointer of `cfg` is null or a NUL-terminated string
        // that outlives this call.
        let config = unsafe { to_xlog_config(&cfg) }.unwrap();
        // Not `""`: the appender refuses an empty prefix, so a name that
        // failed to decode would have opened nothing at all.
        assert_eq!(config.nameprefix, "app\u{fffd}name");
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
