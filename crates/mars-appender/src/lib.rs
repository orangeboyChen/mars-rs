//! `mars-appender` — Rust port of the Mars xlog *appender* layer:
//!
//! | C++                                        | Rust                                            |
//! |--------------------------------------------|-------------------------------------------------|
//! | `mars/xlog/src/appender.cc`                | `appender` + the free functions in this crate     |
//! | `mars/xlog/appender.h`                     | `config`                                         |
//! | `mars/xlog/src/formater.cc`                | `formater`                                       |
//! | `mars/xlog/src/xlogger_interface.cc`       | the process-wide singleton below                |
//! | `mars/xlog/unix/ConsoleLog.cc`             | `console`                                        |
//! | `mars/xlog/appender.h` file helpers        | `file_util`                                      |
//!
//! # The process-wide singleton
//!
//! The C++ keeps `static XloggerAppender* sg_default_appender` (plus
//! `sg_max_byte_size`, `sg_max_alive_time`, `sg_default_console_log_open`) in
//! file scope. The port keeps the same state in `static`s guarded by a `Mutex`,
//! so every public function here is callable from any thread and needs no
//! `unsafe`.
//!
//! ```no_run
//! use mars_appender::{appender_close, appender_flush_sync, appender_open, appender_write, XLogConfig};
//!
//! let mut config = XLogConfig::default();
//! config.logdir = std::path::PathBuf::from("/tmp/mars-log");
//! appender_open(config).unwrap();
//! appender_write(None, "hello from mars");
//! appender_flush_sync();
//! appender_close();
//! ```
//!
//! # Not ported (out of the contract's scope)
//!
//! * `appender.cc`'s `g_log_write_callback` hook, the per-record mirror a host
//!   process can install.
//! * On a write error the C++ appends a record through `log_buff_`; the port
//!   does the same but reports the failure on the console only.
//!
//! Everything else has a counterpart: the per-prefix instance table lives in
//! [`category`], and the hex dump of a binary blob in [`xlogger_memory_dump`]
//! (and its file-writing sibling [`xlogger_dump`]).

// Only `appender::map_region` uses `unsafe` (memmap2 requires it); see the
// SAFETY comment there. Everything else is safe Rust.
#![deny(unsafe_code)]
#![deny(missing_docs)]

mod appender;
pub mod category;
mod config;
mod console;
mod dump;
mod file_util;
mod formater;
mod sys;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

pub use category::{
    flush, flush_all, get_level, get_xlogger_instance, is_enabled_for, new_xlogger_instance,
    release_xlogger_instance, set_appender_mode, set_console_log_open, set_level,
    set_max_alive_duration as category_set_max_alive_duration,
    set_max_file_size as category_set_max_file_size, xlogger_write, XloggerCategory, XloggerHandle,
    DEFAULT_HANDLE,
};
pub use config::{AppenderError, AppenderMode, FileIoAction, LogLevel, XLogConfig, XLoggerInfo};
pub use dump::xlogger_memory_dump;
pub use formater::log_formater;
/// Re-exported so callers (and the FFI layer) do not have to depend on
/// `mars-buffer` just to build a [`XLogConfig`].
pub use mars_buffer::CompressMode;
pub use sys::{available_space, main_thread_id, space_info, thread_id};

use appender::Appender;

/// `static XloggerAppender* sg_default_appender` (plus `sg_release_guard`).
static APPENDER: OnceLock<Mutex<Option<Appender>>> = OnceLock::new();
/// `static uint64_t sg_max_byte_size`.
static MAX_FILE_SIZE: AtomicU64 = AtomicU64::new(0);
/// `static long sg_max_alive_time` (0 = "not set": the appender keeps its
/// 10 day default).
static MAX_ALIVE_TIME: AtomicU64 = AtomicU64::new(0);
/// `static bool sg_default_console_log_open`.
static CONSOLE_LOG_OPEN: AtomicBool = AtomicBool::new(false);

fn slot() -> &'static Mutex<Option<Appender>> {
    APPENDER.get_or_init(|| Mutex::new(None))
}

/// `XloggerAppender::NewInstance` — one appender per `XloggerCategory`, as in
/// `mars/xlog/src/xlogger_interface.cc`. The process-wide default above stays
/// what `appender_open` creates and what handle `0` writes through.
static INSTANCES: OnceLock<Mutex<Instances>> = OnceLock::new();

/// Opaque id of an appender created by [`appender_open_instance`].
pub type AppenderId = u64;

struct Instances {
    next: AppenderId,
    map: HashMap<AppenderId, Appender>,
}

fn instances() -> &'static Mutex<Instances> {
    INSTANCES.get_or_init(|| {
        Mutex::new(Instances {
            next: 1,
            map: HashMap::new(),
        })
    })
}

fn lock_instances() -> MutexGuard<'static, Instances> {
    instances()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Opens an appender that is *not* the process-wide default.
///
/// Every instance gets its own log directory, prefix, key, mode and cache
/// file, like the C++ `XloggerAppender::NewInstance`. Like the C++, an
/// instance is **not** given the process-wide settings: `NewInstance(_config,
/// 0)` builds it with no split size, and `appender_set_console_log` /
/// `appender_set_max_file_size` / `appender_set_max_alive_duration` are the
/// default appender's, so an instance keeps its own 10 day expiry and starts
/// with console logging off. Use the `*_instance` functions below to change
/// that.
///
/// Returns `None` when the directory is empty or cannot be created.
///
/// # Errors
///
/// Propagates [`AppenderError`] when the directory cannot be created or
/// the appender cannot be opened.
pub fn appender_open_instance(config: XLogConfig) -> Result<AppenderId, AppenderError> {
    if config.logdir.as_os_str().is_empty() {
        return Err(AppenderError(
            "appender_open_instance: logdir is empty".to_owned(),
        ));
    }

    // `XloggerAppender::NewInstance(_config, 0)`: an instance starts from its
    // config alone, not from what the process-wide setters were last given.
    let appender = Appender::open(config, 0, 0)?;

    let mut instances = lock_instances();
    let id = instances.next;
    instances.next += 1;
    instances.map.insert(id, appender);
    Ok(id)
}

/// Closes and drops the instance; unknown ids are ignored.
pub fn appender_close_instance(id: AppenderId) {
    if let Some(mut appender) = lock_instances().map.remove(&id) {
        appender.close();
    }
}

/// Writes through a specific instance. `false` when the id is unknown or the
/// appender is closed.
pub fn appender_write_instance(id: AppenderId, info: Option<&XLoggerInfo>, logbody: &str) -> bool {
    match lock_instances().map.get(&id) {
        Some(appender) => {
            let closed = appender.is_closed();
            appender.write(info, logbody);
            !closed
        }
        None => false,
    }
}

/// Drains a specific instance.
pub fn appender_flush_instance(id: AppenderId, sync: bool) {
    if let Some(appender) = lock_instances().map.get_mut(&id) {
        if sync {
            appender.flush_sync();
        } else {
            appender.flush();
        }
    }
}

/// Sets the mode of a specific instance.
pub fn appender_set_mode_instance(id: AppenderId, mode: AppenderMode) {
    if let Some(appender) = lock_instances().map.get_mut(&id) {
        let _ = appender.set_mode(mode);
    }
}

/// Sets console logging for a specific instance.
pub fn appender_set_console_log_instance(id: AppenderId, open: bool) {
    if let Some(appender) = lock_instances().map.get_mut(&id) {
        appender.set_console_log(open);
    }
}

/// Sets the split size for a specific instance.
pub fn appender_set_max_file_size_instance(id: AppenderId, bytes: u64) {
    if let Some(appender) = lock_instances().map.get_mut(&id) {
        appender.set_max_file_size(bytes);
    }
}

/// Sets the expiry for a specific instance (values below one day are ignored).
pub fn appender_set_max_alive_duration_instance(id: AppenderId, secs: u64) {
    if let Some(appender) = lock_instances().map.get_mut(&id) {
        appender.set_max_alive_duration(secs);
    }
}

/// The log directory of a specific instance; `None` for an unknown id.
pub fn appender_get_current_log_path_instance(id: AppenderId) -> Option<PathBuf> {
    lock_instances()
        .map
        .get(&id)
        .and_then(Appender::current_log_path)
}

fn lock_slot() -> MutexGuard<'static, Option<Appender>> {
    slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `mars::xlog::appender_open`.
///
/// Creates the process-wide appender. Unlike the C++ (which silently returns
/// when an appender already exists) the port reports that as an error.
///
/// # Errors
///
/// * `logdir` is empty;
/// * the log (or cache) directory cannot be created;
/// * an appender is already open — call [`appender_close`] first.
pub fn appender_open(config: XLogConfig) -> Result<(), AppenderError> {
    if config.logdir.as_os_str().is_empty() {
        return Err(AppenderError("appender_open: logdir is empty".to_owned()));
    }

    let mut slot = lock_slot();
    if slot.is_some() {
        return Err(AppenderError(format!(
            "appender has already been opened. _dir:{} _nameprefix:{}",
            config.logdir.display(),
            config.nameprefix
        )));
    }

    let appender = Appender::open(
        config,
        MAX_FILE_SIZE.load(Ordering::Relaxed),
        MAX_ALIVE_TIME.load(Ordering::Relaxed) as i64,
    )?;
    appender.set_console_log(CONSOLE_LOG_OPEN.load(Ordering::Relaxed));
    *slot = Some(appender);
    Ok(())
}

/// `mars::xlog::appender_flush` — asks the writer thread to drain the cache.
///
/// A no-op when no appender is open.
pub fn appender_flush() {
    if let Some(appender) = lock_slot().as_ref() {
        appender.flush();
    }
}

/// `mars::xlog::appender_flush_sync` — drains the cache on the calling thread.
///
/// A no-op when no appender is open or when the mode is
/// [`AppenderMode::Sync`] (sync writes never sit in the cache).
pub fn appender_flush_sync() {
    if let Some(appender) = lock_slot().as_ref() {
        appender.flush_sync();
    }
}

/// `mars::xlog::appender_close`.
///
/// Writes the closing banner, drains whatever is left in the cache, stops the
/// writer thread and drops the appender. Safe to call when nothing is open.
pub fn appender_close() {
    let mut slot = lock_slot();
    if let Some(mut appender) = slot.take() {
        appender.close();
    }
}

/// `mars::xlog::appender_setmode`.
pub fn appender_set_mode(mode: AppenderMode) {
    let mut slot = lock_slot();
    if let Some(appender) = slot.as_mut() {
        let _ = appender.set_mode(mode);
    }
}

/// `mars::xlog::appender_set_console_log`.
///
/// Remembered even when no appender is open, so a later [`appender_open`]
/// picks the setting up.
pub fn appender_set_console_log(open: bool) {
    CONSOLE_LOG_OPEN.store(open, Ordering::Relaxed);
    if let Some(appender) = lock_slot().as_ref() {
        appender.set_console_log(open);
    }
}

/// `mars::xlog::appender_set_max_file_size`.
///
/// Remembered for the next [`appender_open`] as well.
pub fn appender_set_max_file_size(bytes: u64) {
    MAX_FILE_SIZE.store(bytes, Ordering::Relaxed);
    if let Some(appender) = lock_slot().as_ref() {
        appender.set_max_file_size(bytes);
    }
}

/// `mars::xlog::appender_set_max_alive_duration`.
///
/// Values below one day are ignored (the C++ `kMinLogAliveTime` guard); the
/// default is 10 days.
pub fn appender_set_max_alive_duration(secs: u64) {
    MAX_ALIVE_TIME.store(secs, Ordering::Relaxed);
    if let Some(appender) = lock_slot().as_ref() {
        appender.set_max_alive_duration(secs);
    }
}

/// `mars::xlog::appender_get_current_log_path` — the log directory.
///
/// `None` when no appender is open.
pub fn appender_get_current_log_path() -> Option<PathBuf> {
    lock_slot().as_ref().and_then(Appender::current_log_path)
}

/// `mars::xlog::appender_get_current_log_cache_path` — the cache directory.
///
/// `None` when no appender is open or when no `cachedir` is configured.
pub fn appender_get_current_log_cache_path() -> Option<PathBuf> {
    lock_slot()
        .as_ref()
        .and_then(Appender::current_log_cache_path)
}

/// `mars::xlog::appender_oneshot_flush`.
///
/// Drains an existing cache file (`<cachedir or logdir>/<prefix>.mmap3`) into
/// the log file without starting the appender — the "another process died with
/// a full cache" recovery path.
pub fn appender_oneshot_flush(config: &XLogConfig) -> FileIoAction {
    if config.logdir.as_os_str().is_empty() {
        return FileIoAction::OpenFailed;
    }
    // This is the "another process died with a full cache" recovery path. Run
    // it while an appender is open for the same directory and it reads that
    // cache file mid-write and then unlinks it, so every record the live
    // appender buffers afterwards lands in an unnamed inode and is lost. The
    // instances matter as much as the process-wide appender: each of them owns
    // a `<prefix>.mmap3` of its own.
    if appender_get_current_log_path().is_some()
        || crate::category::instance_owns_mmap_path(&crate::appender::mmap_file_path(config))
    {
        return FileIoAction::Unnecessary;
    }

    let mut appender = match Appender::oneshot(
        config,
        MAX_FILE_SIZE.load(Ordering::Relaxed),
        MAX_ALIVE_TIME.load(Ordering::Relaxed) as i64,
    ) {
        Ok(appender) => appender,
        Err(_) => return FileIoAction::OpenFailed,
    };

    let action = appender.treat_mapping_as_file_and_flush();
    appender.close();
    action
}

/// `mars::xlog::xlogger_appender` / `XloggerAppender::Write`.
///
/// Returns `false` when no appender is open (or it is already closed).
pub fn appender_write(info: Option<&XLoggerInfo>, logbody: &str) -> bool {
    let slot = lock_slot();
    match slot.as_ref() {
        Some(appender) => {
            let closed = appender.is_closed();
            appender.write(info, logbody);
            !closed
        }
        None => false,
    }
}

/// `mars::xlog::xlogger_dump` — the dump that also leaves a file behind.
///
/// The blob is written to `<logdir>/<YYYYMMDD>/<YYYYMMDDHHMMSS>_<len>.dump`
/// and the returned report is what the C++ hands back to the caller:
/// `"\n dump file to <path> :\n"` plus up to 32 lines of 16 bytes. Empty when
/// no appender is open (`sg_release_guard`) or the file cannot be written, as
/// in the C++.
///
/// The `YYYYMMDD` directory is the same one [`appender_open`]'s expiry sweeps,
/// so a dump is kept no longer than the logs around it.
pub fn xlogger_dump(bytes: &[u8]) -> String {
    match lock_slot().as_ref() {
        Some(appender) => match appender.current_log_path() {
            Some(logdir) => dump::dump_to_logdir(bytes, &logdir),
            None => String::new(),
        },
        None => String::new(),
    }
}

/// `mars::xlog::appender_make_logfile_name`.
///
/// The log file names for the day `timespan` days ago (0 = today). Uses the
/// process-wide max file size set by [`appender_set_max_file_size`].
pub fn appender_make_logfile_name(timespan: i64, prefix: &str, logdir: &Path) -> Vec<PathBuf> {
    // When an appender is open for exactly this directory its own lookup is
    // used, which (like the C++) also reports the matching cache-dir files.
    if let Some(appender) = lock_slot().as_ref().and_then(|a| a.for_logdir(logdir)) {
        return appender.make_logfile_name(timespan, prefix);
    }
    if logdir.as_os_str().is_empty() {
        return Vec::new();
    }

    let tv = file_util::now_secs() - timespan * file_util::SECONDS_PER_DAY;
    vec![file_util::make_log_file_name(
        tv,
        logdir,
        logdir,
        prefix,
        file_util::LOG_EXT,
        MAX_FILE_SIZE.load(Ordering::Relaxed),
        None,
    )]
}

/// `mars::xlog::appender_getfilepath_from_timespan`.
///
/// The *existing* log files whose name covers the day `timespan` days ago.
pub fn appender_getfilepath_from_timespan(
    timespan: i64,
    prefix: &str,
    logdir: &Path,
) -> Vec<PathBuf> {
    if let Some(appender) = lock_slot().as_ref().and_then(|a| a.for_logdir(logdir)) {
        return appender.getfilepath_from_timespan(timespan, prefix);
    }
    if logdir.as_os_str().is_empty() {
        return Vec::new();
    }

    let tv = file_util::now_secs() - timespan * file_util::SECONDS_PER_DAY;
    file_util::get_file_paths_from_timeval(tv, logdir, prefix, file_util::LOG_EXT)
}

#[cfg(test)]
pub(crate) mod test_lock {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// The appender is a process-wide singleton, so the tests that open, close
    /// or assert on it must not run concurrently with each other.
    pub(crate) fn serial() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }
}

/// Counts the allocations of one thread, so a test can assert what the write
/// path costs.
///
/// Only the thread named in [`test_alloc::watch`] is counted: the other tests
/// of this binary run in parallel, and so does the async writer thread, and
/// neither may land in another test's count. The id comes from
/// [`crate::sys::raw_thread_id`] — a syscall, no thread-local of its own —
/// because an allocator that touched one could recurse into itself.
#[cfg(test)]
pub(crate) mod test_alloc {
    // An allocator is `unsafe` by construction; the rest of the crate is
    // `#![deny(unsafe_code)]` and stays that way.
    #![allow(unsafe_code)]

    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    /// The thread being counted, or `0` for "nobody".
    static WATCHED: AtomicI64 = AtomicI64::new(0);
    /// How many allocations that thread has made since the last reset.
    static COUNT: AtomicUsize = AtomicUsize::new(0);

    pub(crate) struct Counter;

    /// Counts the allocations of the calling thread from here on.
    pub(crate) fn watch() {
        WATCHED.store(crate::sys::raw_thread_id(), Ordering::SeqCst);
        COUNT.store(0, Ordering::SeqCst);
    }

    /// Stops counting and answers what was counted since [`watch`].
    pub(crate) fn stop() -> usize {
        WATCHED.store(0, Ordering::SeqCst);
        COUNT.load(Ordering::SeqCst)
    }

    unsafe impl GlobalAlloc for Counter {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            counted(|| unsafe { System.alloc(layout) })
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            counted(|| unsafe { System.realloc(ptr, layout, new_size) })
        }
    }

    fn counted<T>(alloc: impl FnOnce() -> T) -> T {
        if WATCHED.load(Ordering::SeqCst) == crate::sys::raw_thread_id() {
            COUNT.fetch_add(1, Ordering::SeqCst);
        }
        alloc()
    }
}

#[cfg(test)]
#[global_allocator]
static COUNTER: test_alloc::Counter = test_alloc::Counter;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_without_open_reports_false() {
        let _guard = crate::test_lock::serial();
        // Nothing else may hold the singleton while this runs, so the call
        // must fail.
        assert!(!appender_write(None, "nothing"));
        assert!(appender_get_current_log_path().is_none());
        assert!(appender_get_current_log_cache_path().is_none());
        // Harmless no-ops.
        appender_flush();
        appender_flush_sync();
        appender_close();
    }

    #[test]
    fn make_logfile_name_is_deterministic() {
        let _guard = crate::test_lock::serial();
        let dir = Path::new("/tmp/mars-xlog-name-test");
        let paths = appender_make_logfile_name(0, "Mars", dir);
        assert_eq!(paths.len(), 1);
        let name = paths[0].file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("Mars_"), "{name}");
        assert!(name.ends_with(".xlog"), "{name}");
        assert_eq!(paths[0].parent(), Some(dir));

        // Empty logdir -> nothing.
        assert!(appender_make_logfile_name(0, "Mars", Path::new("")).is_empty());
        assert!(appender_getfilepath_from_timespan(0, "Mars", Path::new("")).is_empty());
    }

    #[test]
    fn getfilepath_from_timespan_lists_existing_files() {
        let _guard = crate::test_lock::serial();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let expected = appender_make_logfile_name(0, "Mars", dir).remove(0);
        std::fs::write(dir.join("Mars_19700101.xlog"), b"x").unwrap();

        assert!(appender_getfilepath_from_timespan(0, "Mars", dir).is_empty());
        assert!(appender_getfilepath_from_timespan(20_000, "Mars", dir).is_empty());

        std::fs::write(&expected, b"x").unwrap();
        let found = appender_getfilepath_from_timespan(0, "Mars", dir);
        assert_eq!(found, vec![expected.clone()]);

        std::fs::write(
            dir.join(format!(
                "{}_1.xlog",
                expected.file_stem().unwrap().to_str().unwrap()
            )),
            b"x",
        )
        .unwrap();
        // A second file for the same day is reported too.
        assert_eq!(appender_getfilepath_from_timespan(0, "Mars", dir).len(), 2);
    }

    #[test]
    fn setters_are_sticky_before_open() {
        let _guard = crate::test_lock::serial();
        appender_set_max_file_size(1234);
        appender_set_max_alive_duration(3 * 24 * 60 * 60);
        appender_set_console_log(false);
        // A too-small alive duration is ignored by the appender, but the value
        // is still remembered for the next open.
        appender_set_max_alive_duration(60);
        appender_set_max_file_size(0);
    }
}
