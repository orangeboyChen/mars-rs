//! `mars::comm::XloggerCategory` plus the instance table in
//! `mars/xlog/src/xlogger_interface.cc`.
//!
//! | C++                                            | Rust                     |
//! |------------------------------------------------|--------------------------|
//! | `XloggerCategory`                              | [`XloggerCategory`]      |
//! | `NewXloggerInstance` / `GetXloggerInstance` /  | [`new_xlogger_instance`] |
//! | `ReleaseXloggerInstance`                       | …                        |
//! | `XloggerWrite` / `IsEnabledFor` / `GetLevel` / | the free functions below |
//! | `SetLevel` / `Flush` / `SetConsoleLogOpen`     |                          |
//!
//! # Handles instead of pointers
//!
//! The C++ hands out `XloggerCategory*` as a `uintptr_t` and casts it back on
//! every call — any stale pointer is undefined behaviour. The port hands out an
//! opaque [`XloggerHandle`] that is looked up in the instance table, so a stale
//! handle is a no-op instead of a wild write. Handle `0` keeps its C++ meaning:
//! "the default logger", i.e. the process-wide appender.
//!
//! # Instances
//!
//! Like the C++, every instance gets its own appender
//! (`appender_open_instance`), so two prefixes can write to two directories
//! with their own key, mode and cache file. Handle `0` keeps writing through
//! the process-wide appender opened by `appender_open`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::{
    appender_close_instance, appender_flush, appender_flush_instance, appender_flush_sync,
    appender_open_instance, appender_set_console_log, appender_set_console_log_instance,
    appender_set_max_alive_duration, appender_set_max_alive_duration_instance,
    appender_set_max_file_size, appender_set_max_file_size_instance, appender_set_mode,
    appender_set_mode_instance, appender_write, appender_write_instance, AppenderId, AppenderMode,
    LogLevel, XLogConfig, XLoggerInfo,
};

/// Opaque id of a [`XloggerCategory`]; `0` is the default logger.
pub type XloggerHandle = u64;

/// The default logger, i.e. "no instance" — calls go straight to the
/// process-wide appender.
pub const DEFAULT_HANDLE: XloggerHandle = 0;

/// `mars::comm::XloggerCategory`.
///
/// The C++ also stores an appender and a write callback; the port only needs
/// the level, because writing always goes through the process-wide appender
/// (see the module note).
#[derive(Debug, Clone, Copy)]
pub struct XloggerCategory {
    level: LogLevel,
    /// The C++ gives every instance its own `XloggerAppender`; `None` means
    /// "write through the process-wide default", which is what handle `0` does.
    appender: Option<AppenderId>,
}

impl Default for XloggerCategory {
    fn default() -> Self {
        Self {
            level: LogLevel::Verbose,
            appender: None,
        }
    }
}

impl XloggerCategory {
    /// `XloggerCategory::GetLevel`.
    pub fn level(&self) -> LogLevel {
        self.level
    }

    /// `XloggerCategory::SetLevel`.
    pub fn set_level(&mut self, level: LogLevel) {
        self.level = level;
    }

    /// `XloggerCategory::IsEnabledFor` — `level_ <= _level`.
    pub fn is_enabled_for(&self, level: LogLevel) -> bool {
        (self.level as i32) <= (level as i32)
    }

    /// `XloggerCategory::Write` — the level filter and the pid/tid fix-up of
    /// `__WriteImpl`.
    ///
    /// `log` of `None` mirrors the C++ `NULL == _log`: the record is written
    /// anyway, promoted to `Fatal` with a fixed message.
    pub fn write(&self, info: Option<&XLoggerInfo>, log: Option<&str>) -> bool {
        let mut info = info.cloned();

        if let Some(info) = info.as_ref() {
            if (info.level as i32) < (self.level as i32) {
                return false;
            }
        }

        // `-1 == pid && -1 == tid && -1 == maintid` means "fill these in".
        if let Some(info) = info.as_mut() {
            if info.pid == -1 && info.tid == -1 && info.maintid == -1 {
                info.pid = std::process::id() as i64;
                info.tid = crate::sys::thread_id();
                info.maintid = crate::sys::main_thread_id();
            }
        }

        match log {
            Some(log) => write_through(self.appender, info.as_ref(), log),
            None => {
                if let Some(info) = info.as_mut() {
                    info.level = LogLevel::Fatal;
                }
                write_through(self.appender, info.as_ref(), "NULL == _log")
            }
        }
    }
}

struct Registry {
    next: XloggerHandle,
    categories: HashMap<XloggerHandle, XloggerCategory>,
    by_prefix: HashMap<String, XloggerHandle>,
    /// The logger handle `0` selects: `SetLevel(0, ..)` in the C++ configures
    /// the process-wide level, so it has to be reachable.
    default: XloggerCategory,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            next: DEFAULT_HANDLE + 1,
            categories: HashMap::new(),
            by_prefix: HashMap::new(),
            default: XloggerCategory::default(),
        })
    })
}

/// Writes through the instance's own appender, or the process-wide default
/// when the category has none.
fn write_through(id: Option<AppenderId>, info: Option<&XLoggerInfo>, log: &str) -> bool {
    match id {
        Some(id) => appender_write_instance(id, info, log),
        None => appender_write(info, log),
    }
}

/// `mars::xlog::NewXloggerInstance`.
///
/// Registers a category for `config.nameprefix` (an existing prefix returns the
/// existing handle, like the C++) and opens the appender for `config`. Returns
/// [`DEFAULT_HANDLE`] when the config has no log dir or prefix, matching the
/// C++ `nullptr`.
pub fn new_xlogger_instance(config: &XLogConfig, level: LogLevel) -> XloggerHandle {
    if config.logdir.as_os_str().is_empty() || config.nameprefix.is_empty() {
        return DEFAULT_HANDLE;
    }

    // Fast path: already registered.
    if let Some(handle) = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .by_prefix
        .get(&config.nameprefix)
        .copied()
    {
        return handle;
    }

    // Each instance opens its own appender, like the C++ `NewXloggerInstance`.
    // That does `create_dir_all`, an mmap and several writes, so it happens
    // outside the registry lock — otherwise every other logger in the process
    // stalls for the whole of it.
    let appender = match appender_open_instance(config.clone()) {
        Ok(id) => Some(id),
        Err(_) => return DEFAULT_HANDLE,
    };

    let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    // Another thread may have registered the prefix while the lock was free.
    if let Some(handle) = registry.by_prefix.get(&config.nameprefix).copied() {
        // Ours is now redundant: close it rather than leak an appender nobody
        // can reach.
        if let Some(id) = appender {
            drop(registry);
            appender_close_instance(id);
        }
        return handle;
    }
    let handle = registry.next;
    registry.next += 1;
    let mut category = XloggerCategory::default();
    category.set_level(level);
    category.appender = appender;
    registry.categories.insert(handle, category);
    registry.by_prefix.insert(config.nameprefix.clone(), handle);
    handle
}

/// `mars::xlog::GetXloggerInstance` — the handle registered for `_nameprefix`.
pub fn get_xlogger_instance(nameprefix: &str) -> XloggerHandle {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .by_prefix
        .get(nameprefix)
        .copied()
        .unwrap_or(DEFAULT_HANDLE)
}

/// `mars::xlog::ReleaseXloggerInstance`.
pub fn release_xlogger_instance(nameprefix: &str) {
    let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    let Some(handle) = registry.by_prefix.remove(nameprefix) else {
        return;
    };
    let category = registry.categories.remove(&handle);
    drop(registry);

    // The C++ releases the instance's own appender here
    // (`XloggerAppender::DelayRelease`); ours closes under the instance lock,
    // so this is safe to do without holding the registry.
    if let Some(Some(id)) = category.map(|category| category.appender) {
        appender_close_instance(id);
    }
}

/// Looks a category up and returns a copy, so the caller never runs with the
/// registry lock held — writing a record does file I/O.
fn lookup(handle: XloggerHandle) -> Option<XloggerCategory> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .categories
        .get(&handle)
        .copied()
}

/// A copy of the default logger (handle `0`).
fn default_category() -> XloggerCategory {
    registry().lock().unwrap_or_else(|e| e.into_inner()).default
}

fn with_category_mut(handle: XloggerHandle, f: impl FnOnce(&mut XloggerCategory)) {
    if let Some(category) = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .categories
        .get_mut(&handle)
    {
        f(category);
    }
}

/// `mars::xlog::XloggerWrite`.
///
/// Handle `0` uses the default logger. An unknown non-zero handle (one whose
/// instance was released) writes nothing: the module promises that a stale
/// handle is a no-op, not a fall back to the default logger.
pub fn xlogger_write(handle: XloggerHandle, info: Option<&XLoggerInfo>, log: Option<&str>) -> bool {
    let category = if handle == DEFAULT_HANDLE {
        default_category()
    } else {
        match lookup(handle) {
            Some(category) => category,
            None => return false,
        }
    };
    category.write(info, log)
}

/// `mars::xlog::IsEnabledFor`.
///
/// `false` for an unknown non-zero handle, so nothing is written through it.
pub fn is_enabled_for(handle: XloggerHandle, level: LogLevel) -> bool {
    let category = if handle == DEFAULT_HANDLE {
        default_category()
    } else {
        match lookup(handle) {
            Some(category) => category,
            None => return false,
        }
    };
    category.is_enabled_for(level)
}

/// `mars::xlog::GetLevel`.
///
/// `None` for an unknown non-zero handle.
pub fn get_level(handle: XloggerHandle) -> Option<LogLevel> {
    if handle == DEFAULT_HANDLE {
        return Some(default_category().level());
    }
    lookup(handle).map(|category| category.level())
}

/// `mars::xlog::SetLevel`.
pub fn set_level(handle: XloggerHandle, level: LogLevel) {
    if handle == DEFAULT_HANDLE {
        registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .default
            .set_level(level);
        return;
    }
    with_category_mut(handle, |category| category.set_level(level));
}

/// `mars::xlog::SetAppenderMode` — applies to the instance's own appender.
pub fn set_appender_mode(handle: XloggerHandle, mode: AppenderMode) {
    match lookup(handle).and_then(|category| category.appender) {
        Some(id) => appender_set_mode_instance(id, mode),
        None => appender_set_mode(mode),
    }
}

/// `mars::xlog::SetMaxFileSize` — per instance, like
/// `xlogger_interface.cc`'s `SetMaxFileSize`.
pub fn set_max_file_size(handle: XloggerHandle, bytes: u64) {
    match lookup(handle).and_then(|category| category.appender) {
        Some(id) => appender_set_max_file_size_instance(id, bytes),
        None => appender_set_max_file_size(bytes),
    }
}

/// `mars::xlog::SetMaxAliveTime` — per instance.
pub fn set_max_alive_duration(handle: XloggerHandle, secs: u64) {
    match lookup(handle).and_then(|category| category.appender) {
        Some(id) => appender_set_max_alive_duration_instance(id, secs),
        None => appender_set_max_alive_duration(secs),
    }
}

/// `mars::xlog::Flush` — drains the instance's own appender.
pub fn flush(handle: XloggerHandle, sync: bool) {
    match lookup(handle).and_then(|category| category.appender) {
        Some(id) => appender_flush_instance(id, sync),
        None => {
            if sync {
                appender_flush_sync();
            } else {
                appender_flush();
            }
        }
    }
}

/// `mars::xlog::FlushAll`.
pub fn flush_all(sync: bool) {
    flush(DEFAULT_HANDLE, sync);
}

/// `mars::xlog::SetConsoleLogOpen` — per instance.
pub fn set_console_log_open(handle: XloggerHandle, open: bool) {
    match lookup(handle).and_then(|category| category.appender) {
        Some(id) => appender_set_console_log_instance(id, open),
        None => appender_set_console_log(open),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_lock::serial;
    use crate::AppenderMode;

    fn config(prefix: &str, dir: &std::path::Path) -> XLogConfig {
        XLogConfig {
            logdir: dir.to_path_buf(),
            nameprefix: prefix.to_owned(),
            ..XLogConfig::default()
        }
    }

    #[test]
    fn instance_table_is_keyed_by_prefix() {
        let _guard = serial();
        let dir = tempfile::tempdir().unwrap();
        let first = new_xlogger_instance(&config("p", dir.path()), LogLevel::Info);
        assert_ne!(first, DEFAULT_HANDLE);
        assert_eq!(get_xlogger_instance("p"), first);
        // A second call with the same prefix returns the existing handle.
        assert_eq!(
            new_xlogger_instance(&config("p", dir.path()), LogLevel::Info),
            first
        );
        assert_eq!(get_xlogger_instance("missing"), DEFAULT_HANDLE);

        release_xlogger_instance("p");
        assert_eq!(get_xlogger_instance("p"), DEFAULT_HANDLE);

        // The appender is a process-wide singleton and the other tests in this
        // crate assume it is closed, so put it back.
        crate::appender_close();
    }

    #[test]
    fn empty_logdir_or_prefix_is_rejected() {
        let _guard = serial();
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            new_xlogger_instance(&config("", dir.path()), LogLevel::Info),
            DEFAULT_HANDLE
        );
        assert_eq!(
            new_xlogger_instance(&config("prefix", std::path::Path::new("")), LogLevel::Info),
            DEFAULT_HANDLE
        );
    }

    #[test]
    fn a_second_prefix_shares_the_open_appender() {
        let _guard = serial();
        let dir = tempfile::tempdir().unwrap();
        let first = new_xlogger_instance(&config("one", dir.path()), LogLevel::Info);
        let second = new_xlogger_instance(&config("two", dir.path()), LogLevel::Warn);
        assert_ne!(first, DEFAULT_HANDLE);
        assert_ne!(second, DEFAULT_HANDLE);
        assert_ne!(first, second);
        assert_eq!(get_level(first), Some(LogLevel::Info));
        assert_eq!(get_level(second), Some(LogLevel::Warn));

        release_xlogger_instance("one");
        release_xlogger_instance("two");
        // The lifecycle can be repeated: both prefixes are gone again.
        assert_eq!(get_xlogger_instance("one"), DEFAULT_HANDLE);
        assert_eq!(get_xlogger_instance("two"), DEFAULT_HANDLE);
        let again = new_xlogger_instance(&config("one", dir.path()), LogLevel::Info);
        assert_ne!(again, DEFAULT_HANDLE);
        release_xlogger_instance("one");
    }

    /// Today's log file in `dir`, i.e. the only `*.xlog` there.
    fn log_text(dir: &std::path::Path) -> String {
        let mut text = String::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("xlog") {
                let bytes = std::fs::read(path).unwrap();
                // Sync mode stores the payload verbatim.
                text.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        text
    }

    #[test]
    fn instances_write_to_their_own_directory() {
        let _guard = serial();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();

        let one = new_xlogger_instance(&config("dir-one", first.path()), LogLevel::Verbose);
        let two = new_xlogger_instance(&config("dir-two", second.path()), LogLevel::Verbose);
        assert_ne!(one, DEFAULT_HANDLE);
        assert_ne!(two, DEFAULT_HANDLE);

        // Sync mode so the records are readable as text in the test.
        set_appender_mode(one, AppenderMode::Sync);
        set_appender_mode(two, AppenderMode::Sync);
        assert!(xlogger_write(one, None, Some("REC-INTO-ONE")));
        assert!(xlogger_write(two, None, Some("REC-INTO-TWO")));
        flush(one, true);
        flush(two, true);

        let first_text = log_text(first.path());
        let second_text = log_text(second.path());

        assert!(first_text.contains("REC-INTO-ONE"), "{first_text}");
        assert!(
            !first_text.contains("REC-INTO-TWO"),
            "instances share a file: {first_text}"
        );
        assert!(second_text.contains("REC-INTO-TWO"), "{second_text}");
        assert!(
            !second_text.contains("REC-INTO-ONE"),
            "instances share a file: {second_text}"
        );

        release_xlogger_instance("dir-one");
        release_xlogger_instance("dir-two");
    }

    #[test]
    fn an_instance_level_does_not_leak_into_another() {
        let _guard = serial();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let quiet = new_xlogger_instance(&config("quiet", first.path()), LogLevel::Error);
        let loud = new_xlogger_instance(&config("loud", second.path()), LogLevel::Verbose);

        assert!(!is_enabled_for(quiet, LogLevel::Info));
        assert!(is_enabled_for(loud, LogLevel::Info));
        // The gate is `_info->level < level_`, so it only applies when a
        // record actually carries a level (a `NULL` info always writes, in the
        // C++ as well).
        let info = XLoggerInfo {
            level: LogLevel::Info,
            ..XLoggerInfo::default()
        };
        assert!(!xlogger_write(quiet, Some(&info), Some("dropped")));
        assert!(xlogger_write(loud, Some(&info), Some("kept")));

        release_xlogger_instance("quiet");
        release_xlogger_instance("loud");
    }

    #[test]
    fn a_stale_handle_writes_nothing() {
        let _guard = serial();
        // A handle that was never registered behaves exactly like one whose
        // instance has been released: no write, no level, no fallback.
        const STALE: XloggerHandle = 999;
        assert!(!xlogger_write(STALE, None, Some("dropped")));
        assert!(!is_enabled_for(STALE, LogLevel::Fatal));
        assert_eq!(get_level(STALE), None);
    }

    #[test]
    fn the_default_handle_has_its_own_level() {
        let _guard = serial();
        set_level(DEFAULT_HANDLE, LogLevel::Error);
        assert_eq!(get_level(DEFAULT_HANDLE), Some(LogLevel::Error));
        assert!(!is_enabled_for(DEFAULT_HANDLE, LogLevel::Warn));
        assert!(is_enabled_for(DEFAULT_HANDLE, LogLevel::Error));
        set_level(DEFAULT_HANDLE, LogLevel::Verbose);
        assert!(is_enabled_for(DEFAULT_HANDLE, LogLevel::Verbose));
    }

    #[test]
    fn level_filter_follows_the_cpp_rule() {
        let mut category = XloggerCategory::default();
        category.set_level(LogLevel::Warn);
        assert!(category.is_enabled_for(LogLevel::Error));
        assert!(category.is_enabled_for(LogLevel::Warn));
        assert!(!category.is_enabled_for(LogLevel::Info));
        assert!(!category.is_enabled_for(LogLevel::Verbose));
        assert_eq!(get_level(12345), None, "unknown handle");
    }
}
