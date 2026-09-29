//! The appender an app holds: the `Xlog` of Kotlin, of Swift, of TypeScript
//! and of `include/mars_xlog.hpp`, over the handle [`crate::category`] answers.
//!
//! Every platform of the port opens one of these and writes through it, and
//! Rust was the one that did not: what it had instead was three layers of free
//! functions — the process-wide `appender_write`, the
//! `appender_*_instance` family over an id of its own, and
//! [`crate::category`] over a handle — and an app that wanted a second logger
//! had to know which of the three to reach for. This is the one an app takes:
//!
//! ```no_run
//! use marsrs_appender::{LogLevel, XLogConfig, Xlog};
//!
//! let mut config = XLogConfig::default();
//! config.logdir = std::path::PathBuf::from("/tmp/mars-log");
//! config.nameprefix = "marsrs".to_owned();
//!
//! let xlog = Xlog::open(config, LogLevel::Info)?;
//! xlog.i("startup", "hello from mars");
//!
//! xlog.flush_now();          // the records are on the disk when this returns
//! // xlog.flush().await is the same drain off this thread
//! # Ok::<(), marsrs_appender::AppenderError>(())
//! ```
//!
//! What is underneath is the plumbing and not the API: [`crate::category`] is
//! the seam the C ABI and the JNI bridge are written over — a handle, because
//! neither of the two has an object to hold — and the process-wide
//! [`crate::appender_open`] is the one call that installs the appender handle
//! `0` means. An app takes [`Xlog`] and has no reason to name either.
//!
//! Two things are the object's own. Every member is safe to call from any
//! thread, and [`Xlog`] is `Send` and `Sync` — the four the appender has no
//! getter for are behind atomics rather than behind a borrow, so an `Xlog` in
//! an `Arc` is what a process that logs from many threads shares. And a
//! prefix is one appender: `Xlog::open` for a prefix that is already open
//! answers the appender that is open, so closing one `Xlog` closes what
//! another `Xlog` of the same prefix writes through.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use crate::category::{
    current_log_path as category_current_log_path, flush as category_flush,
    flush_now as category_flush_now, get_level, get_xlogger_instance, is_enabled_for,
    new_xlogger_instance, release_xlogger_instance, set_appender_mode, set_console_log_open,
    set_level, set_max_alive_duration as category_set_max_alive_duration,
    set_max_file_size as category_set_max_file_size, signal_flush as category_signal_flush,
    xlogger_write, XloggerHandle, DEFAULT_HANDLE,
};
use crate::config::{AppenderError, AppenderMode, LogLevel, XLogConfig, XLoggerInfo};
use crate::file_util::now_timeval;
use crate::flush::Flush;

/// An appender of an app's own: build one when the app starts, then write
/// through it from wherever there is something to say.
///
/// A write is a tag and a message, the pair every platform of the port spells;
/// the file, the function and the line of the record are empty unless
/// [`Xlog::log_with_info`] names them, which is what Kotlin writes too — Rust
/// has no `#file` to fill one in with. A record is dropped before anything is
/// formatted when its level is below [`Xlog::level`], so a message that is
/// expensive to build is worth an `if xlog.is_loggable(LogLevel::Debug)` first.
#[derive(Debug)]
pub struct Xlog {
    /// The handle [`crate::category`] answered; `DEFAULT_HANDLE` once
    /// [`Xlog::close`] ran, which is the appender of every other `Xlog` of
    /// this prefix as well.
    handle: AtomicU64,
    name_prefix: String,
    /// The four below are answers of this object's own and not of the
    /// appender's: the appender has no getter for them, so what they answer is
    /// the last value written through this `Xlog` — as on every platform.
    mode: AtomicU8,
    console_log_enabled: AtomicBool,
    max_file_size_bytes: AtomicU64,
    max_alive_time_seconds: AtomicU64,
}

impl Xlog {
    /// Opens an appender of its own: its own log directory, prefix, key, mode
    /// and cache file, all of them `config`'s.
    ///
    /// Asking for a prefix that is already open answers the appender that is
    /// open and not a second one.
    ///
    /// The level is beside the config and not in it, which is the one place
    /// this is not Kotlin's shape: `XLogConfig` is the C++'s `XLogConfig`, and
    /// the eight fields it has are the eight the C++ gives it — a ninth would
    /// be a field no other spelling of the config carries.
    ///
    /// # Errors
    ///
    /// * `config.logdir` or `config.nameprefix` is empty — both of them are
    ///   what an appender is known by;
    /// * the appender refused the config, which is what a directory it cannot
    ///   create comes to.
    pub fn open(config: XLogConfig, level: LogLevel) -> Result<Self, AppenderError> {
        if config.logdir.as_os_str().is_empty() {
            return Err(AppenderError("Xlog::open: logdir is empty".to_owned()));
        }
        if config.nameprefix.is_empty() {
            return Err(AppenderError("Xlog::open: nameprefix is empty".to_owned()));
        }

        let handle = new_xlogger_instance(&config, level);
        if handle == DEFAULT_HANDLE {
            return Err(AppenderError(format!(
                "Xlog::open: no appender was opened for '{}' in {}",
                config.nameprefix,
                config.logdir.display()
            )));
        }

        Ok(Self {
            handle: AtomicU64::new(handle),
            name_prefix: config.nameprefix,
            mode: AtomicU8::new(config.mode as u8),
            console_log_enabled: AtomicBool::new(false),
            max_file_size_bytes: AtomicU64::new(0),
            max_alive_time_seconds: AtomicU64::new(0),
        })
    }

    /// What every file of this appender starts with, and what it is known by.
    pub fn name_prefix(&self) -> &str {
        &self.name_prefix
    }

    /// Whether this appender is still open: `false` after [`Xlog::close`] — on
    /// this `Xlog` and on every other one of this prefix, which is the same
    /// appender and is closed with this one.
    pub fn is_open(&self) -> bool {
        self.handle() != DEFAULT_HANDLE
    }

    /// The level of this appender: a record less severe than this is dropped.
    ///
    /// Read from the appender and not mirrored here, so a level another part of
    /// the app set is the one this answers with. `None` once this `Xlog` is
    /// closed.
    pub fn level(&self) -> Option<LogLevel> {
        get_level(self.handle())
    }

    /// Moves the level; a no-op once this `Xlog` is closed.
    pub fn set_level(&self, level: LogLevel) {
        if self.is_open() {
            set_level(self.handle(), level);
        }
    }

    /// Whether a write reaches the file before it returns: what the
    /// [`XLogConfig`] gave, until this says otherwise. The appender has no
    /// getter for it, so this is the last value written through this `Xlog`.
    pub fn mode(&self) -> AppenderMode {
        if self.mode.load(Ordering::Relaxed) == AppenderMode::Sync as u8 {
            AppenderMode::Sync
        } else {
            AppenderMode::Async
        }
    }

    /// Switches async / sync; a no-op once this `Xlog` is closed.
    pub fn set_mode(&self, mode: AppenderMode) {
        self.mode.store(mode as u8, Ordering::Relaxed);
        if self.is_open() {
            set_appender_mode(self.handle(), mode);
        }
    }

    /// Whether the console prints the log too — off until an app turns it on.
    pub fn console_log_enabled(&self) -> bool {
        self.console_log_enabled.load(Ordering::Relaxed)
    }

    /// Mirrors every record to the console as well as to the file; a no-op once
    /// this `Xlog` is closed.
    pub fn set_console_log_enabled(&self, enabled: bool) {
        self.console_log_enabled.store(enabled, Ordering::Relaxed);
        if self.is_open() {
            set_console_log_open(self.handle(), enabled);
        }
    }

    /// How many bytes a log file may reach before it is closed and a new one
    /// opened; `0` is "never split".
    pub fn max_file_size_bytes(&self) -> u64 {
        self.max_file_size_bytes.load(Ordering::Relaxed)
    }

    /// Sets the split size; a no-op once this `Xlog` is closed.
    pub fn set_max_file_size_bytes(&self, bytes: u64) {
        self.max_file_size_bytes.store(bytes, Ordering::Relaxed);
        if self.is_open() {
            category_set_max_file_size(self.handle(), bytes);
        }
    }

    /// How many seconds a log file is kept; `0` is the C++'s own ten days.
    pub fn max_alive_time_seconds(&self) -> u64 {
        self.max_alive_time_seconds.load(Ordering::Relaxed)
    }

    /// Sets how long a log file is kept; a no-op once this `Xlog` is closed.
    pub fn set_max_alive_time_seconds(&self, seconds: u64) {
        self.max_alive_time_seconds
            .store(seconds, Ordering::Relaxed);
        if self.is_open() {
            category_set_max_alive_duration(self.handle(), seconds);
        }
    }

    /// Whether a record of `level` would be written: what an app asks before it
    /// builds a message that is expensive to build.
    pub fn is_loggable(&self, level: LogLevel) -> bool {
        self.is_open() && is_enabled_for(self.handle(), level)
    }

    /// Writes a record of `level`; `false` when this `Xlog` is closed or the
    /// level is below [`Xlog::level`].
    pub fn log(&self, level: LogLevel, tag: &str, message: &str) -> bool {
        self.log_with_info(
            Some(&XLoggerInfo {
                level,
                tag: Some(Cow::Borrowed(tag)),
                ..XLoggerInfo::default()
            }),
            message,
        )
    }

    /// [`Xlog::log`] with the record spelled out: the file, the function and
    /// the line of the call site go in the `XLoggerInfo`, which is what a
    /// caller that wants them in the record — and not only in the message —
    /// hands in. `None` is the default record, and it is the one
    /// [`Xlog::log`] hands in with a level and a tag on it.
    pub fn log_with_info(&self, info: Option<&XLoggerInfo<'_>>, message: &str) -> bool {
        if !self.is_open() {
            return false;
        }
        let mut info = match info {
            Some(info) => info.clone(),
            None => XLoggerInfo::default(),
        };
        // `-1` in all three is what asks the appender to fill the pid, the tid
        // and the main tid in from the OS, and `-1` is what a caller that set
        // them itself does not get overwritten with. `Default` answers `0` for
        // them, which is a record that then carries three wrong fields.
        if info.pid == 0 && info.tid == 0 && info.maintid == 0 {
            info.pid = -1;
            info.tid = -1;
            info.maintid = -1;
        }
        if info.timeval == (0, 0) {
            info.timeval = now_timeval();
        }
        xlogger_write(self.handle(), Some(&info), Some(message))
    }

    /// [`Xlog::log`] at [`LogLevel::Verbose`].
    pub fn v(&self, tag: &str, message: &str) -> bool {
        self.log(LogLevel::Verbose, tag, message)
    }

    /// [`Xlog::log`] at [`LogLevel::Debug`].
    pub fn d(&self, tag: &str, message: &str) -> bool {
        self.log(LogLevel::Debug, tag, message)
    }

    /// [`Xlog::log`] at [`LogLevel::Info`].
    pub fn i(&self, tag: &str, message: &str) -> bool {
        self.log(LogLevel::Info, tag, message)
    }

    /// [`Xlog::log`] at [`LogLevel::Warn`].
    pub fn w(&self, tag: &str, message: &str) -> bool {
        self.log(LogLevel::Warn, tag, message)
    }

    /// [`Xlog::log`] at [`LogLevel::Error`].
    pub fn e(&self, tag: &str, message: &str) -> bool {
        self.log(LogLevel::Error, tag, message)
    }

    /// [`Xlog::log`] at [`LogLevel::Fatal`].
    pub fn f(&self, tag: &str, message: &str) -> bool {
        self.log(LogLevel::Fatal, tag, message)
    }

    /// Asks the writer thread to take what is in the cache to the log file, and
    /// returns at once: nothing is in the file because this returned. What it is
    /// for is a drain an app wants soon and does not want to wait for — a
    /// record still in the cache sits in a file the kernel holds, so a drain
    /// that has not happened yet loses nothing.
    pub fn signal_flush(&self) {
        if self.is_open() {
            category_signal_flush(self.handle());
        }
    }

    /// Takes what is in the cache to the log file on the calling thread, and
    /// hands the file's own buffer to the OS: the records are on the disk when
    /// this returns, and what it costs is the time the drain takes.
    pub fn flush_now(&self) {
        if self.is_open() {
            category_flush_now(self.handle());
        }
    }

    /// [`Xlog::flush_now`] for a caller that can wait without holding a thread:
    /// the [`Flush`] this hands back drains on a thread of its own and is Ready
    /// when the records are on the disk.
    ///
    /// Nothing drains until it is awaited, and a `Flush` that is dropped
    /// unpolled drains nothing — a caller that wants the drain whatever happens
    /// wants [`Xlog::flush_now`].
    #[must_use = "a flush that is not awaited does not drain"]
    pub fn flush(&self) -> Flush {
        if self.is_open() {
            category_flush(self.handle())
        } else {
            Flush::noop()
        }
    }

    /// Closes this appender: drains what is left and drops it. Writing through
    /// this `Xlog` afterwards writes nothing, and asking for this prefix
    /// answers no handle at all. Safe to call twice, and safe to leave to the
    /// destructor of an `Xlog` that goes out of scope.
    pub fn close(&self) {
        let handle = self.handle.load(Ordering::Relaxed);
        if handle == DEFAULT_HANDLE {
            return;
        }
        // A prefix is one appender, so two `Xlog`s of one prefix hold one
        // handle between them — and releasing takes the prefix and not the
        // handle, which drops whatever the prefix answers *now*. Once this
        // object's twin closed the appender and a third one reopened the
        // prefix, releasing here would close an appender that is not ours.
        if get_xlogger_instance(&self.name_prefix) == handle {
            release_xlogger_instance(&self.name_prefix);
        }
        self.handle.store(DEFAULT_HANDLE, Ordering::Relaxed);
    }

    /// The directory this appender writes its files to — `XLogConfig::logdir`,
    /// and `None` once this `Xlog` is closed.
    ///
    /// The name is the C++'s (`XloggerAppender::GetCurrentLogPath`), and what
    /// the C++ answers with is the directory and not the file: a day's file is
    /// named for the day its records carry, so there is no one file to answer
    /// before the first record of the day is written.
    ///
    /// The one member no other platform carries: Kotlin, Swift, Dart and
    /// TypeScript answer no path at all, and the C++ only has the process-wide
    /// one. What wants it here is an upload path that reads the files back —
    /// `decode_log_file` and `marsrs-xlog`'s `xlog decode` both start from a
    /// path, and `appender_getfilepath_from_timespan` is the one that names a
    /// day's.
    pub fn current_log_path(&self) -> Option<std::path::PathBuf> {
        if self.is_open() {
            category_current_log_path(self.handle())
        } else {
            None
        }
    }

    fn handle(&self) -> XloggerHandle {
        self.handle.load(Ordering::Relaxed)
    }
}

impl Drop for Xlog {
    fn drop(&mut self) {
        self.close();
    }
}
