//! `marsrs-appender` — Rust port of the Mars xlog *appender* layer:
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
//! The `Mutex` there guards the *slot*, not the appender: a write takes an
//! [`std::sync::Arc`] clone out of it and lets the guard go before it formats
//! or writes, which is what the C++ does — `xlogger_appender` reads
//! `sg_default_appender` without touching `sg_mutex`. Holding it across the
//! write turned `N` logging threads into one; the clone cannot dangle, because
//! a concurrent [`appender_close`] drops the slot's own reference and the
//! writer's keeps the appender (already closed, so the write is a no-op) alive.
//! The C++ deletes the appender under a concurrent write instead.
//!
//! ```no_run
//! use marsrs_appender::{LogLevel, XLogConfig, Xlog};
//!
//! let mut config = XLogConfig::default();
//! config.logdir = std::path::PathBuf::from("/tmp/mars-log");
//!
//! let xlog = Xlog::open(config, LogLevel::Info).unwrap();
//! xlog.i("startup", "hello from mars");
//! xlog.flush_now();
//! ```
//!
//! # The process-wide appender, and the two seams over it
//!
//! [`Xlog`] is what an app takes. What is left public beside it is the
//! plumbing the other crates of the port are written over, and neither of them
//! is an app's spelling of anything:
//!
//! * [`appender_open`] and [`appender_close`] install and drop the
//!   process-wide appender, which is what handle [`DEFAULT_HANDLE`] means. The
//!   write, the drain and the four setters of that appender are *not* here:
//!   they are [`category`]'s at [`DEFAULT_HANDLE`], which is one spelling and
//!   not two.
//! * [`category`] is the handle table the C ABI (`marsrs-ffi`) and the JNI
//!   bridge (`marsrs-jni`) are written over — a handle and not an object,
//!   because neither of the two has one to hold.
//! * [`Xlog::open_unregistered`] opens an appender that no prefix is
//!   registered for: a prefix is one appender to [`category`], and two copies
//!   of the library linked into one process — a React Native module beside the
//!   Kotlin one — are two writers over one prefix, which is the one shape
//!   [`Xlog::open`] cannot answer. The `*_instance` family it is written over
//!   is `pub(crate)` for the same reason the process-wide one is.
//! # Not ported (out of the contract's scope)
//!
//! * `appender.cc`'s `g_log_write_callback` hook, the per-record mirror a host
//!   process can install.
//! * On a write error the C++ appends a record through `log_buff_`; the port
//!   does the same but reports the failure on the console only.
//!
//! # Where the port does not do what the C++ does
//!
//! The C++ mmaps `<prefix>.mmap3` whatever else is doing, so two processes —
//! or two copies of the C++ linked into one — write through the same 150 KiB
//! region with their own idea of its length: records are lost, and each flush
//! either writes what the other buffered or clears it before the other gets
//! there. The port gives every writer a cache file of its own
//! (`<prefix>.mmap3`, then `<prefix>_1.mmap3` …) and, around the steps that
//! move more than one file, takes the lock of the log they share —
//! `<logdir>/<prefix>.lock`, which is theirs whatever cache directory each of
//! them was given — so a log two writers share is still complete: every record
//! of both, in a file that still decodes end to end.
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
mod flush;
mod formater;
mod sys;
mod xlog;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

pub use category::{
    current_log_path, flush, flush_all, flush_now, flush_now_all, get_filter, get_level,
    get_xlogger_instance, is_enabled_for, new_xlogger_instance, release_xlogger_instance,
    request_flush, request_flush_all, set_appender_mode, set_console_log_open, set_filter,
    set_level, set_max_alive_duration as category_set_max_alive_duration,
    set_max_file_size as category_set_max_file_size, xlogger_assert, xlogger_assert_p,
    xlogger_write, XloggerCategory, XloggerFilter, XloggerHandle, XloggerScopeTracer,
    DEFAULT_HANDLE,
};
pub use config::{AppenderError, AppenderMode, FileIoAction, LogLevel, XLogConfig, XLoggerInfo};
pub use console::{get_console_fun, set_console_fun, ConsoleFun};
pub use dump::xlogger_memory_dump;
pub use flush::Flush;
pub use formater::log_formater;
/// Re-exported so callers (and the FFI layer) do not have to depend on
/// `marsrs-buffer` just to build a [`XLogConfig`].
pub use marsrs_buffer::CompressMode;
pub use sys::{available_space, main_thread_id, space_info, thread_id};
/// The appender an app holds: `Xlog.open(config)` in Kotlin, in Dart and in
/// TypeScript, and `Xlog::open(config)` here.
pub use xlog::Xlog;

use appender::Appender;

/// `static XloggerAppender* sg_default_appender` (plus `sg_release_guard`).
///
/// The `Mutex` is the slot's, not the appender's: [`current`] hands out an
/// [`Arc`] clone so a write does not hold it.
static APPENDER: OnceLock<Mutex<Option<Arc<Appender>>>> = OnceLock::new();
/// `static uint64_t sg_max_byte_size`.
static MAX_FILE_SIZE: AtomicU64 = AtomicU64::new(0);
/// `static long sg_max_alive_time` (0 = "not set": the appender keeps its
/// 10 day default).
static MAX_ALIVE_TIME: AtomicU64 = AtomicU64::new(0);
/// `static bool sg_default_console_log_open`.
static CONSOLE_LOG_OPEN: AtomicBool = AtomicBool::new(false);

fn slot() -> &'static Mutex<Option<Arc<Appender>>> {
    APPENDER.get_or_init(|| Mutex::new(None))
}

/// How many times the slot has been filled or emptied.
///
/// Bumped by `appender_open` / `appender_close`, so that [`current`] can tell
/// whether the appender it cached is still the open one with one shared atomic
/// load instead of a turn through the slot's lock.
static GENERATION: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// This thread's [`current`]: the appender, tagged with the
    /// [`GENERATION`] it was read at.
    static CURRENT: std::cell::RefCell<Option<(u64, Arc<Appender>)>> =
        const { std::cell::RefCell::new(None) };
}

/// The open appender, if any, as an [`Arc`] clone — so that the caller can do
/// what it wants with it without holding the slot's lock, which is what the
/// C++'s unlocked `sg_default_appender` read amounts to.
///
/// Cached per thread against [`GENERATION`]: a record asks for the appender at
/// least once, and a mutex per record is a mutex every logging thread in the
/// process contends for — one cache line they all write, which is the one
/// thing the write path is otherwise built to avoid (the formatting runs
/// unlocked, and the C++ reads its `sg_default_appender` with no lock at all).
/// The generation is a load nobody writes to, so N threads no longer share a
/// line here; the lock is taken only when the appender actually changed.
fn current() -> Option<Arc<Appender>> {
    let generation = GENERATION.load(Ordering::Acquire);
    CURRENT.with(|cell| {
        let mut cached = cell.borrow_mut();
        if let Some((cached_generation, appender)) = &*cached {
            if *cached_generation == generation {
                return Some(Arc::clone(appender));
            }
        }

        // Either nothing was cached yet, or the appender behind it was closed
        // or replaced: ask the slot. `None` is cached as `None`, so a closed
        // appender is not held alive by every thread that ever logged.
        let fresh = slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        *cached = fresh
            .as_ref()
            .map(|appender| (generation, Arc::clone(appender)));
        fresh
    })
}

/// `XloggerAppender::NewInstance` — one appender per `XloggerCategory`, as in
/// `mars/xlog/src/xlogger_interface.cc`. The process-wide default above stays
/// what `appender_open` creates and what handle `0` writes through.
static INSTANCES: OnceLock<Mutex<Instances>> = OnceLock::new();

/// Opaque id of an appender created by `appender_open_instance` — the one
/// [`Xlog::open_unregistered`] holds, and the one the `*_instance` calls take.
pub type AppenderId = u64;

struct Instances {
    next: AppenderId,
    map: HashMap<AppenderId, Arc<Appender>>,
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

/// One instance, as an [`Arc`] clone: the same reason [`current`] hands out a
/// clone instead of a reference.
fn instance(id: AppenderId) -> Option<Arc<Appender>> {
    lock_instances().map.get(&id).cloned()
}

/// The cache file an instance claimed, if any.
///
/// Which slot an instance got is decided by [`appender::claim_cache_slot`] at
/// open time and is not a function of the config alone — another process can
/// hold slot 0 — so the path has to be read back from the appender.
fn instance_cache_path(id: AppenderId) -> Option<PathBuf> {
    instance(id).and_then(|appender| appender.claimed_cache_path())
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
/// # Errors
///
/// Propagates [`AppenderError`] when the directory cannot be created or
/// the appender cannot be opened.
pub(crate) fn appender_open_instance(config: XLogConfig) -> Result<AppenderId, AppenderError> {
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
    instances.map.insert(id, Arc::new(appender));
    Ok(id)
}

/// Closes and drops the instance; unknown ids are ignored.
pub(crate) fn appender_close_instance(id: AppenderId) {
    if let Some(appender) = lock_instances().map.remove(&id) {
        appender.close();
    }
}

/// Writes through a specific instance. `false` when the id is unknown or the
/// appender is closed.
pub(crate) fn appender_write_instance(
    id: AppenderId,
    info: Option<&XLoggerInfo>,
    logbody: &str,
) -> bool {
    // Like [`current`]: take a clone and write outside the table's lock.
    let Some(appender) = instance(id) else {
        return false;
    };
    let closed = appender.is_closed();
    appender.write(info, logbody);
    !closed
}

/// Asks the writer thread to drain one instance, and returns at once.
///
/// `appender_request_flush` for one instance: the drain is the writer
/// thread's, and nothing here says when it is over. Unknown ids are ignored.
pub(crate) fn appender_request_flush_instance(id: AppenderId) {
    if let Some(appender) = instance(id) {
        appender.wake_writer();
    }
}

/// Drains one instance on the calling thread.
///
/// `appender_flush_now` for one instance: the records are on the disk when
/// this returns. Unknown ids are ignored.
pub(crate) fn appender_flush_now_instance(id: AppenderId) {
    if let Some(appender) = instance(id) {
        appender.flush_sync();
    }
}

/// [`appender_flush_now_instance`] for a caller that can wait without holding
/// a thread: the [`Flush`] this hands back drains that instance on a thread of
/// its own and is Ready when the records are on the disk.
///
/// `appender_flush` for one instance, and with the same two caveats: nothing
/// drains until the future is polled, and dropping it does not stop a drain
/// that has started. A future that drains nothing when the id is unknown.
pub(crate) fn appender_flush_instance(id: AppenderId) -> Flush {
    let appender = instance(id);
    Flush::new(move || {
        if let Some(appender) = appender {
            appender.flush_sync();
        }
    })
}

/// Sets the mode of a specific instance.
pub(crate) fn appender_set_mode_instance(id: AppenderId, mode: AppenderMode) {
    if let Some(appender) = instance(id) {
        let _ = appender.set_mode(mode);
    }
}

/// Sets console logging for a specific instance.
pub(crate) fn appender_set_console_log_instance(id: AppenderId, open: bool) {
    if let Some(appender) = instance(id) {
        appender.set_console_log(open);
    }
}

/// Sets the split size for a specific instance.
pub(crate) fn appender_set_max_file_size_instance(id: AppenderId, bytes: u64) {
    if let Some(appender) = instance(id) {
        appender.set_max_file_size(bytes);
    }
}

/// Sets the expiry for a specific instance (values below one day are ignored).
pub(crate) fn appender_set_max_alive_duration_instance(id: AppenderId, secs: u64) {
    if let Some(appender) = instance(id) {
        appender.set_max_alive_duration(secs);
    }
}

/// The log directory of a specific instance; `None` for an unknown id.
pub(crate) fn appender_get_current_log_path_instance(id: AppenderId) -> Option<PathBuf> {
    instance(id).and_then(|appender| appender.current_log_path())
}

fn lock_slot() -> MutexGuard<'static, Option<Arc<Appender>>> {
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
    *slot = Some(Arc::new(appender));
    // After the slot, so that a thread that sees the new generation also sees
    // the appender that came with it — and one that does not yet keeps using
    // whatever it had, which is still a perfectly good appender.
    GENERATION.fetch_add(1, Ordering::AcqRel);
    Ok(())
}

/// Asks the writer thread to drain the cache, and returns at once.
///
/// The drain is the writer thread's, and nothing here says when it is over:
/// what is in the cache stays in a file the kernel holds until then, so a
/// drain that has not happened yet loses nothing. What this is for is a drain
/// an app wants soon and does not want to wait for — on a timer, say.
/// `appender_flush_now` is the call that comes back with the records on the
/// disk, and `appender_flush` is the one an async caller awaits.
///
/// A no-op when no appender is open.
pub(crate) fn appender_request_flush() {
    if let Some(appender) = current() {
        appender.wake_writer();
    }
}

/// Drains the cache on the calling thread, and hands the log file's own buffer
/// to the OS.
///
/// The records are on the disk when this returns, and what it costs is the
/// time the drain takes. The buffer, in both modes: the C++ leaves the last
/// few KiB in the `FILE*` here, so a reader in another process could not see
/// them yet. This is the call to make before the log files are read, copied or
/// uploaded, and the one a caller with an executor reaches for
/// `appender_flush` instead of.
///
/// A no-op when no appender is open or when it is already closed; in
/// [`AppenderMode::Sync`] there is no cache to drain, but the file buffer is
/// flushed all the same.
pub(crate) fn appender_flush_now() {
    if let Some(appender) = current() {
        appender.flush_sync();
    }
}

/// `appender_flush_now` for a caller that can wait without holding a thread:
/// the [`Flush`] this hands back drains on a thread of its own and is Ready
/// when the records are on the disk.
///
/// ```no_run
/// # async fn drain() -> Result<(), marsrs_appender::AppenderError> {
/// let xlog = marsrs_appender::Xlog::open(Default::default(), marsrs_appender::LogLevel::Info)?;
/// xlog.flush().await;
/// # Ok(())
/// # }
/// ```
///
/// Nothing drains until the future is polled, so this is the one to `await`
/// and not to drop: a caller that wants the drain whatever happens wants
/// `appender_flush_now`. It is not cancelled with the task that asked for
/// it, either — the drain is already running on a thread holding its own
/// handle on the appender.
///
/// A future that drains nothing when no appender is open, which is what keeps
/// a caller from having to match on the appender being there.
pub(crate) fn appender_flush() -> Flush {
    // Resolved here, on the thread that asked: `current` is a per-thread
    // cache, so the closure cannot ask for the appender once it is on the
    // other thread.
    let appender = current();
    Flush::new(move || {
        if let Some(appender) = appender {
            appender.flush_sync();
        }
    })
}

/// `mars::xlog::appender_close`.
///
/// Writes the closing banner, drains whatever is left in the cache, stops the
/// writer thread and drops the appender. Safe to call when nothing is open.
pub fn appender_close() {
    // Taken out of the slot first, so the close does not run under the slot's
    // lock: a write that is already under way keeps its own Arc and finds the
    // appender closed instead of finding it gone.
    let Some(appender) = lock_slot().take() else {
        return;
    };
    GENERATION.fetch_add(1, Ordering::AcqRel);
    appender.close();
}

/// `mars::xlog::appender_setmode`.
pub(crate) fn appender_set_mode(mode: AppenderMode) {
    if let Some(appender) = current() {
        let _ = appender.set_mode(mode);
    }
}

/// `mars::xlog::appender_set_console_log`.
///
/// Remembered even when no appender is open, so a later [`appender_open`]
/// picks the setting up.
pub(crate) fn appender_set_console_log(open: bool) {
    CONSOLE_LOG_OPEN.store(open, Ordering::Relaxed);
    if let Some(appender) = current() {
        appender.set_console_log(open);
    }
}

/// `mars::xlog::appender_set_max_file_size`.
///
/// Remembered for the next [`appender_open`] as well.
pub(crate) fn appender_set_max_file_size(bytes: u64) {
    MAX_FILE_SIZE.store(bytes, Ordering::Relaxed);
    if let Some(appender) = current() {
        appender.set_max_file_size(bytes);
    }
}

/// `mars::xlog::appender_set_max_alive_duration`.
///
/// Values below one day are ignored (the C++ `kMinLogAliveTime` guard); the
/// default is 10 days.
pub(crate) fn appender_set_max_alive_duration(secs: u64) {
    MAX_ALIVE_TIME.store(secs, Ordering::Relaxed);
    if let Some(appender) = current() {
        appender.set_max_alive_duration(secs);
    }
}

/// `mars::xlog::appender_get_current_log_path` — the log directory.
///
/// `None` when no appender is open.
pub fn appender_get_current_log_path() -> Option<PathBuf> {
    current().and_then(|appender| appender.current_log_path())
}

/// `mars::xlog::appender_get_current_log_cache_path` — the cache directory.
///
/// `None` when no appender is open or when no `cachedir` is configured.
pub fn appender_get_current_log_cache_path() -> Option<PathBuf> {
    current().and_then(|appender| appender.current_log_cache_path())
}

/// `mars::xlog::appender_oneshot_flush`.
///
/// Drains the cache files no writer owns any more into the log file without
/// starting the appender — the "another process died with a full cache"
/// recovery path.
///
/// Which ones those are is the whole difficulty: a process that was killed
/// leaves exactly the file a running one has, and the C++ cannot tell them
/// apart, so it drains `<prefix>.mmap3` whatever else is doing — including a
/// second process that is mid-write, whose every later record then lands in an
/// unlinked inode. The port gives each writer a slot of its own and holds an
/// advisory lock on it for as long as the writer lives, so "no writer owns it"
/// is something that can actually be answered — by trying to take that lock.
pub fn appender_oneshot_flush(config: &XLogConfig) -> FileIoAction {
    use crate::appender::{
        cache_dir, cache_slot_path, dir_lock_path, mmap_file_path, MAX_CACHE_SLOTS,
    };

    if config.logdir.as_os_str().is_empty() {
        return FileIoAction::OpenFailed;
    }

    let dir = cache_dir(config).to_path_buf();
    // Whether a slot a dead writer left behind can be told from one a live
    // writer is using — which is asked of the cache directory, because that is
    // where the slots are. (The lock the drain itself is taken under is the
    // log's: see `appender::output_lock_path`.)
    let slot_lock_path = dir_lock_path(&dir, &config.nameprefix);
    let Ok(appender) = Appender::oneshot(
        config,
        MAX_FILE_SIZE.load(Ordering::Relaxed),
        MAX_ALIVE_TIME.load(Ordering::Relaxed) as i64,
    ) else {
        return FileIoAction::OpenFailed;
    };

    // Without a lock a live cache file cannot be told from a dead one, so all
    // that is left is the C++'s own behaviour: the single fixed name, drained
    // only when no appender of this process is using it.
    if !crate::sys::lock_excludes(&slot_lock_path) {
        if appender_get_current_log_path().is_some()
            || crate::category::instance_owns_mmap_path(&mmap_file_path(config))
        {
            return FileIoAction::Unnecessary;
        }
        // A lock nobody on this platform can take is not a reason not to read
        // the file: the drain reads it through this handle either way.
        let path = mmap_file_path(config);
        let Ok(mut file) = std::fs::File::open(&path) else {
            appender.close();
            return FileIoAction::OpenFailed;
        };
        let action = appender.treat_mapping_as_file_and_flush(&path, &mut file);
        appender.close();
        return action;
    }

    let mut action = FileIoAction::Unnecessary;
    // A failure is remembered on its own: one slot that recovers does not make
    // a later one that does not a success, and the caller — which is what
    // decides whether to try again — has to be able to see it.
    let mut failed: Option<FileIoAction> = None;
    for slot in 0..MAX_CACHE_SLOTS {
        let path = cache_slot_path(&dir, &config.nameprefix, slot);
        if !path.exists() {
            continue;
        }
        // Held for the whole drain and the unlink that follows it: the lock is
        // what says the slot is a dead writer's, and it has to still be ours
        // when the file goes away — and it is the handle the drain reads the
        // records through, for the reason `treat_mapping_as_file_and_flush`
        // gives.
        // A live writer still owns it — in another process, or in another
        // copy of this crate in this one.
        let Some(mut claim) = crate::appender::claim_dead_cache_slot(&path) else {
            continue;
        };
        match appender.treat_mapping_as_file_and_flush(&path, &mut claim) {
            FileIoAction::Success => action = FileIoAction::Success,
            FileIoAction::Unnecessary => {}
            // Every slot is tried, so any one of them says the same thing to
            // the caller: something is still unrecovered.
            other => {
                failed.get_or_insert(other);
            }
        }
    }
    appender.close();
    failed.unwrap_or(action)
}

/// `mars::xlog::xlogger_appender` / `XloggerAppender::Write`.
///
/// Returns `false` when no appender is open (or it is already closed).
///
/// In [`AppenderMode::Sync`] the record is written to the appender's file
/// buffer before this returns — the C++'s `fwrite` does the same, so a record
/// is not necessarily on disk until `appender_flush_now` runs, the file
/// fills up, or the appender is closed.
pub(crate) fn appender_write(info: Option<&XLoggerInfo>, logbody: &str) -> bool {
    // A clone, so the slot's lock is not held while the record is written: N
    // logging threads then run in parallel instead of queueing on the slot.
    let Some(appender) = current() else {
        return false;
    };
    let closed = appender.is_closed();
    appender.write(info, logbody);
    !closed
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
    match current().and_then(|appender| appender.current_log_path()) {
        Some(logdir) => dump::dump_to_logdir(bytes, &logdir),
        None => String::new(),
    }
}

/// `mars::xlog::appender_make_logfile_name`.
///
/// The log file names for the day `timespan` days ago (0 = today). Uses the
/// process-wide max file size set by `appender_set_max_file_size`.
pub fn appender_make_logfile_name(timespan: i64, prefix: &str, logdir: &Path) -> Vec<PathBuf> {
    // When an appender is open for exactly this directory its own lookup is
    // used, which (like the C++) also reports the matching cache-dir files.
    let appender = current();
    if let Some(appender) = appender.as_deref().and_then(|a| a.for_logdir(logdir)) {
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
    let appender = current();
    if let Some(appender) = appender.as_deref().and_then(|a| a.for_logdir(logdir)) {
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
        appender_request_flush();
        appender_flush_now();
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
