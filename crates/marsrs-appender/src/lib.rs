//! `marsrs-appender` — Rust port of the Mars xlog *appender* layer:
//!
//! | C++                                        | Rust                                            |
//! |--------------------------------------------|-------------------------------------------------|
//! | `mars/xlog/src/appender.cc`                | `appender` + the free functions in this crate     |
//! | `mars/xlog/appender.h`                     | `config`                                         |
//! | `mars/xlog/src/formater.cc`                | `formater`                                       |
//! | `mars/xlog/src/xlogger_interface.cc`       | `category`                                      |
//! | `mars/xlog/unix/ConsoleLog.cc`             | `console`                                        |
//! | `mars/xlog/appender.h` file helpers        | `file_util`                                      |
//!
//! # One appender per instance, and no process-wide one
//!
//! The C++ keeps `static XloggerAppender* sg_default_appender` (plus
//! `sg_max_byte_size`, `sg_max_alive_time`, `sg_default_console_log_open`) in
//! file scope, and every `mars::xlog::` free function writes through it. This
//! port has no such appender: nothing installs one, so there is no state for a
//! free function to reach and none of them is here.
//!
//! What is here is the table the C++'s `XloggerAppender::NewInstance` builds:
//! an appender per handle, opened from its config alone. Every call is asked of
//! a handle — or of the [`Xlog`] that holds one — and every table entry is
//! behind a `Mutex`, so a call is safe from any thread and needs no `unsafe`.
//! A write takes an [`std::sync::Arc`] clone out of the table and lets the
//! guard go before it formats or writes, which is what the C++ does:
//! `xlogger_appender` reads `sg_default_appender` without touching `sg_mutex`.
//! Holding it across the write turned `N` logging threads into one; the clone
//! cannot dangle, because a concurrent `close` drops the table's own reference
//! and the writer's keeps the appender (already closed, so the write is a
//! no-op) alive. The C++ deletes the appender under a concurrent write instead.
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
//! # [`Xlog`], and the seam under it
//!
//! [`Xlog`] is what an app takes. What is left public beside it is the
//! plumbing the other crates of the port are written over, and neither of them
//! is an app's spelling of anything:
//!
//! * [`Xlog::open`] opens the appender of a `namePrefix`, which is one
//!   appender to [`category`]: a second `Xlog` of a prefix the first opened
//!   writes through the same appender, and `close` on either closes it.
//! * [`category`] is the handle table the C ABI (`marsrs-ffi`) and the JNI
//!   bridge (`marsrs-jni`) are written over — a handle and not an object,
//!   because neither of the two has one to hold.
//! * [`DEFAULT_HANDLE`] — `0` — names no appender at all. It is what an open
//!   that failed answers, and every call asked of it is a no-op: a caller
//!   that wants a logger of its own opens one and holds the handle.
//!
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
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

pub use category::{
    current_log_path, flush, flush_now, get_filter, get_level, get_xlogger_instance,
    is_enabled_for, new_xlogger_instance, release_xlogger_instance, request_flush,
    set_appender_mode, set_console_log_open, set_filter, set_level,
    set_max_alive_duration as category_set_max_alive_duration,
    set_max_file_size as category_set_max_file_size, xlogger_write, XloggerCategory, XloggerFilter,
    XloggerHandle, DEFAULT_HANDLE,
};
pub use config::{AppenderError, AppenderMode, LogLevel, XLogConfig, XLoggerInfo};
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

/// `XloggerAppender::NewInstance` — one appender per `XloggerCategory`, as in
/// `mars/xlog/src/xlogger_interface.cc`, and the only appenders there are:
/// nothing installs a process-wide one, so handle `0` names no appender at all.
static INSTANCES: OnceLock<Mutex<Instances>> = OnceLock::new();

/// Opaque id of an appender created by `appender_open_instance`, and the one
/// the `*_instance` calls take.
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

/// One instance, as an [`Arc`] clone: a write holds a clone and not the
/// table's lock.
fn instance(id: AppenderId) -> Option<Arc<Appender>> {
    lock_instances().map.get(&id).cloned()
}

/// The cache file an instance claimed, if any.
///
/// Which slot an instance got is decided by the appender at open time and is
/// not a function of the config alone — another process can hold slot 0 — so
/// the path has to be read back from the appender.
fn instance_cache_path(id: AppenderId) -> Option<PathBuf> {
    instance(id).and_then(|appender| appender.claimed_cache_path())
}

/// Opens the appender of an instance.
///
/// Every instance gets its own log directory, prefix, key, mode and cache
/// file, like the C++ `XloggerAppender::NewInstance`. Like the C++, an
/// instance is **not** given the process-wide settings the C++ keeps for its
/// own default: `NewInstance(_config, 0)` builds it with no split size, so an
/// instance keeps its own 10 day expiry and starts with console logging off.
/// Use the `*_instance` functions below to change that.
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
    // config alone. There is no process-wide setting to inherit, because
    // nothing installs the process-wide appender they would come from.
    let appender = Appender::open(config, 0, 0)?;

    let mut instances = lock_instances();
    let id = instances.next;
    instances.next += 1;
    instances.map.insert(id, Arc::new(appender));
    Ok(id)
}

/// Closes and drops the instance; unknown ids are ignored.
///
/// The table's lock is let go before the close and not after it: a close
/// flushes, hands the file's buffer to the OS and waits for the writer thread
/// to finish, and every other logger in the process asks this table for its
/// own appender on every record. Holding it across a close is a stall of all
/// of them for the length of someone else's drain.
pub(crate) fn appender_close_instance(id: AppenderId) {
    let appender = lock_instances().map.remove(&id);
    if let Some(appender) = appender {
        appender.close();
    }
}

/// Writes through a specific instance. `false` when the id is unknown, or the
/// appender is closed — including one closed by another thread while this
/// record was being written, which is why the answer is the write's own and
/// not a flag read before it.
pub(crate) fn appender_write_instance(
    id: AppenderId,
    info: Option<&XLoggerInfo>,
    logbody: &str,
) -> bool {
    // A clone, so the instance table's lock is not held while the record is
    // written: N logging threads then run in parallel instead of queueing.
    let Some(appender) = instance(id) else {
        return false;
    };
    appender.write(info, logbody)
}

/// Asks the writer thread to drain one instance, and returns at once.
///
/// The drain is the writer thread's, and nothing here says when it is over:
/// a drain that has not happened yet loses nothing, because what is in the
/// cache is in a file the kernel holds. Unknown ids are ignored.
pub(crate) fn appender_request_flush_instance(id: AppenderId) {
    if let Some(appender) = instance(id) {
        appender.wake_writer();
    }
}

/// Drains one instance on the calling thread.
///
/// The records are on the disk when this returns, and the log file's own
/// buffer has been handed to the OS with them. Unknown ids are ignored.
pub(crate) fn appender_flush_now_instance(id: AppenderId) {
    if let Some(appender) = instance(id) {
        appender.flush_sync();
    }
}

/// [`appender_flush_now_instance`] for a caller that can wait without holding
/// a thread: the [`Flush`] this hands back drains that instance on a thread of
/// its own and is Ready when the records are on the disk.
///
/// Nothing drains until the future is polled, and dropping it does not stop a
/// drain that has started. A future that drains nothing when the id is
/// unknown.
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

/// Sets the expiry for a specific instance; `false` for an unknown id, or
/// when the appender refused the value for being below one day.
pub(crate) fn appender_set_max_alive_duration_instance(id: AppenderId, secs: u64) -> bool {
    instance(id).is_some_and(|appender| appender.set_max_alive_duration(secs))
}

/// The log directory of a specific instance; `None` for an unknown id.
pub(crate) fn appender_get_current_log_path_instance(id: AppenderId) -> Option<PathBuf> {
    instance(id).and_then(|appender| appender.current_log_path())
}

/// The appender of `handle`: the one that knows the prefix and the directory
/// a day of files is named out of. `None` for a handle nothing was opened
/// for — an unknown handle, and [`DEFAULT_HANDLE`], which names no appender
/// at all.
fn appender_of(handle: XloggerHandle) -> Option<Arc<Appender>> {
    use crate::category::target;
    match target(handle) {
        crate::category::Target::Instance(id) => instance(id),
        crate::category::Target::Gone => None,
    }
}

/// The log files of the day `timespan` days ago that are *there* — of the
/// appender of `handle`, and out of its own prefix and directory.
pub fn current_log_files(handle: XloggerHandle, timespan: i64) -> Vec<PathBuf> {
    let Some(appender) = appender_of(handle) else {
        return Vec::new();
    };
    appender.getfilepath_from_timespan(timespan, &appender.nameprefix())
}

/// The names of the log files of the day `timespan` days ago, whether or not
/// they are there yet — of the appender of `handle`.
pub fn current_log_file_names(handle: XloggerHandle, timespan: i64) -> Vec<PathBuf> {
    let Some(appender) = appender_of(handle) else {
        return Vec::new();
    };
    appender.make_logfile_name(timespan, &appender.nameprefix())
}

#[cfg(test)]
pub(crate) mod test_lock {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// The instance table and the files under it are process-wide, so the
    /// tests that open, close or assert on them must not run concurrently with
    /// each other.
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
