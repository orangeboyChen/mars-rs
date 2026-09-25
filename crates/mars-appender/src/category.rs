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

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, OnceLock};

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
    /// `TLogLevel level_ = kLevelNone` — a logger nobody has configured logs
    /// nothing: [`XloggerCategory::is_enabled_for`] answers `false` for every
    /// level, [`XloggerCategory::level`] answers [`LogLevel::None`], and only
    /// [`set_level`] moves it.
    fn default() -> Self {
        Self {
            level: LogLevel::None,
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
    /// `XloggerCategory::__WriteImpl`.
    ///
    /// This is the path of a handle that names an **instance**. `XloggerWrite(0,
    /// …)` does not come here: the C++ sends it to `xlogger_Write`, which has no
    /// level filter at all (see `write_default`).
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

        // `-1 == pid && -1 == tid && -1 == maintid` means "fill these in":
        // `XloggerCategory::__WriteImpl` asks for all three at once …
        if let Some(info) = info.as_mut() {
            if info.pid == -1 && info.tid == -1 && info.maintid == -1 {
                info.pid = std::process::id() as i64;
                info.tid = crate::sys::thread_id();
                info.maintid = crate::sys::main_thread_id();
            }
        }

        write_log(self.appender, &mut info, log)
    }
}

/// `__xlogger_Write_impl` — what `XloggerWrite(0, …)` reaches, i.e. the
/// `xlogger_Write` of `mars/comm/xlogger/xloggerbase.c`.
///
/// `xloggerbase.h` writes "no level filter" over the declaration, and the
/// implementation keeps the promise: `gs_level` is what `xlogger_IsEnabledFor`
/// answers *from*, and the write never looks at it. A record of any level goes
/// out through handle `0` whatever `SetLevel(0, …)` was given — the only thing
/// the level does there is answer [`is_enabled_for`].
///
/// `gs_level` also starts at `kLevelNone` and not at `kLevelVerbose`, so a
/// process that only ever called `appender_open` answers `false` for
/// [`LogLevel::Fatal`] and writes every record it is handed.
fn write_default(info: Option<&XLoggerInfo>, log: Option<&str>) -> bool {
    let mut info = info.cloned();

    // … while `xlogger_Write` fills each of the three in on its own.
    if let Some(info) = info.as_mut() {
        if info.pid == -1 {
            info.pid = std::process::id() as i64;
        }
        if info.tid == -1 {
            info.tid = crate::sys::thread_id();
        }
        if info.maintid == -1 {
            info.maintid = crate::sys::main_thread_id();
        }
    }

    write_log(None, &mut info, log)
}

/// What both paths end with: the `NULL == _log` promotion and the write itself.
fn write_log(
    appender: Option<AppenderId>,
    info: &mut Option<XLoggerInfo>,
    log: Option<&str>,
) -> bool {
    match log {
        Some(log) => write_through(appender, info.as_ref(), log),
        None => {
            if let Some(info) = info.as_mut() {
                info.level = LogLevel::Fatal;
            }
            write_through(appender, info.as_ref(), "NULL == _log")
        }
    }
}

struct Registry {
    next: XloggerHandle,
    categories: HashMap<XloggerHandle, XloggerCategory>,
    by_prefix: HashMap<String, XloggerHandle>,
    /// `<prefix>.mmap3` of every live instance: one-shot recovery reads and
    /// unlinks that file, so it has to stay away from an instance that owns it.
    mmap_paths: HashMap<XloggerHandle, PathBuf>,
    /// The prefixes whose appender is being opened right now.
    opening: HashSet<String>,
    /// The logger handle `0` selects: `SetLevel(0, ..)` in the C++ configures
    /// the process-wide level, so it has to be reachable.
    default: XloggerCategory,
}

/// Signalled whenever a prefix leaves [`Registry::opening`], so a thread that
/// asked for a prefix another thread is opening can wait for the answer
/// instead of opening it a second time.
fn opened() -> &'static Condvar {
    static OPENED: OnceLock<Condvar> = OnceLock::new();
    OPENED.get_or_init(Condvar::new)
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            next: DEFAULT_HANDLE + 1,
            categories: HashMap::new(),
            by_prefix: HashMap::new(),
            mmap_paths: HashMap::new(),
            opening: HashSet::new(),
            default: XloggerCategory::default(),
        })
    })
}

/// A prefix that this thread is opening: dropped when the open is over, which
/// is when the threads waiting for that prefix are woken.
///
/// It is a `Drop` and not a line at the end of the open because a panic inside
/// `appender_open_instance` would otherwise leave the prefix in
/// [`Registry::opening`] for good, and every other thread that ever asks for it
/// would wait for an answer that is never coming.
struct Opening(String);

impl Drop for Opening {
    fn drop(&mut self) {
        let mut guard = registry().lock().unwrap_or_else(|e| e.into_inner());
        guard.opening.remove(&self.0);
        drop(guard);
        opened().notify_all();
    }
}

/// Which appender a handle names.
enum Target {
    /// The instance's own appender.
    Instance(AppenderId),
    /// The process-wide default — what handle `0` means in the C++.
    Default,
    /// Nothing at all: a handle whose instance was released, or one that was
    /// never handed out. Calls through it are a no-op; they must not reach the
    /// process-wide appender, which belongs to handle `0`.
    Gone,
}

fn target(handle: XloggerHandle) -> Target {
    if handle == DEFAULT_HANDLE {
        return Target::Default;
    }
    match lookup(handle).and_then(|category| category.appender) {
        Some(id) => Target::Instance(id),
        None => Target::Gone,
    }
}

/// Whether a live instance owns the cache file at `path`.
///
/// `appender_oneshot_flush` asks this before it reads and unlinks
/// `<prefix>.mmap3`: doing that to an instance that is mid-write loses
/// everything the instance buffers afterwards.
pub fn instance_owns_mmap_path(path: &std::path::Path) -> bool {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .mmap_paths
        .values()
        .any(|owned| owned == path)
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

    // The C++ creates the whole instance under
    // `ScopedLock lock(GetGlobalMutex())` (`xlogger_interface.cc`), and this is
    // what that lock is for: two threads asking for the same prefix would
    // otherwise both miss the table and open two appenders over one
    // `<prefix>.mmap3`, and closing the redundant one clears the cache file the
    // other is still writing through.
    //
    // What the port does not take over is *how long* the lock is held. The
    // C++'s write path dereferences a pointer, but this registry's `lookup` is
    // on the write path, so holding it across a `create_dir_all`, an mmap and a
    // few writes would stall every other logger in the process — including
    // threads that are only logging. Only the prefix is reserved, and a thread
    // that asks for a prefix another thread is opening waits for it:
    let mut guard = registry().lock().unwrap_or_else(|e| e.into_inner());
    let opening = loop {
        if let Some(handle) = guard.by_prefix.get(&config.nameprefix).copied() {
            return handle;
        }
        if guard.opening.insert(config.nameprefix.clone()) {
            break Opening(config.nameprefix.clone());
        }
        guard = opened()
            .wait(guard)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    };
    drop(guard);

    // Each instance opens its own appender, like the C++ `NewXloggerInstance`.
    // A failed open releases the prefix on the way out.
    let appender = match appender_open_instance(config.clone()) {
        Ok(id) => Some(id),
        Err(_) => return DEFAULT_HANDLE,
    };

    let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    let handle = registry.next;
    registry.next += 1;
    let mut category = XloggerCategory::default();
    category.set_level(level);
    category.appender = appender;
    registry.categories.insert(handle, category);
    registry.by_prefix.insert(config.nameprefix.clone(), handle);
    registry
        .mmap_paths
        .insert(handle, crate::appender::mmap_file_path(config));
    drop(registry);
    // the handle is in the table, so whoever was waiting for the prefix finds
    // it now
    drop(opening);
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
    registry.mmap_paths.remove(&handle);
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
/// Handle `0` uses the default logger, i.e. the C++'s `xlogger_Write` — which
/// has no level filter (see `write_default`). An unknown non-zero handle (one
/// whose instance was released) writes nothing: the module promises that a
/// stale handle is a no-op, not a fall back to the default logger.
pub fn xlogger_write(handle: XloggerHandle, info: Option<&XLoggerInfo>, log: Option<&str>) -> bool {
    if handle == DEFAULT_HANDLE {
        return write_default(info, log);
    }
    match lookup(handle) {
        Some(category) => category.write(info, log),
        None => false,
    }
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
///
/// A handle whose instance is gone changes nothing: only [`DEFAULT_HANDLE`]
/// reaches the process-wide appender.
pub fn set_appender_mode(handle: XloggerHandle, mode: AppenderMode) {
    match target(handle) {
        Target::Instance(id) => appender_set_mode_instance(id, mode),
        Target::Default => appender_set_mode(mode),
        Target::Gone => {}
    }
}

/// `mars::xlog::SetMaxFileSize` — per instance, like
/// `xlogger_interface.cc`'s `SetMaxFileSize`.
pub fn set_max_file_size(handle: XloggerHandle, bytes: u64) {
    match target(handle) {
        Target::Instance(id) => appender_set_max_file_size_instance(id, bytes),
        Target::Default => appender_set_max_file_size(bytes),
        Target::Gone => {}
    }
}

/// `mars::xlog::SetMaxAliveTime` — per instance.
pub fn set_max_alive_duration(handle: XloggerHandle, secs: u64) {
    match target(handle) {
        Target::Instance(id) => appender_set_max_alive_duration_instance(id, secs),
        Target::Default => appender_set_max_alive_duration(secs),
        Target::Gone => {}
    }
}

/// `mars::xlog::Flush` — drains the instance's own appender.
pub fn flush(handle: XloggerHandle, sync: bool) {
    match target(handle) {
        Target::Instance(id) => appender_flush_instance(id, sync),
        Target::Default => {
            if sync {
                appender_flush_sync();
            } else {
                appender_flush();
            }
        }
        Target::Gone => {}
    }
}

/// `mars::xlog::FlushAll`.
///
/// Every registered instance has an appender of its own, so the C++'s
/// "flush everything" has to drain those too — a caller that flushes before
/// collecting logs or suspending would otherwise miss their records.
pub fn flush_all(sync: bool) {
    flush(DEFAULT_HANDLE, sync);

    let instances: Vec<AppenderId> = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .categories
        .values()
        .filter_map(|category| category.appender)
        .collect();
    for id in instances {
        appender_flush_instance(id, sync);
    }
}

/// `mars::xlog::SetConsoleLogOpen` — per instance.
pub fn set_console_log_open(handle: XloggerHandle, open: bool) {
    match target(handle) {
        Target::Instance(id) => appender_set_console_log_instance(id, open),
        Target::Default => appender_set_console_log(open),
        Target::Gone => {}
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

    /// An open that failed must not leave the prefix reserved: the thread that
    /// asked for it is gone, and every thread that asks afterwards would wait
    /// for an answer that is never coming.
    #[test]
    fn a_prefix_whose_open_failed_is_asked_for_again() {
        let _guard = serial();
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("not-a-dir");
        std::fs::write(&blocked, b"a file, not a dir").unwrap();
        let config = XLogConfig {
            logdir: blocked,
            nameprefix: "blocked".to_owned(),
            ..XLogConfig::default()
        };

        assert_eq!(
            new_xlogger_instance(&config, LogLevel::Info),
            DEFAULT_HANDLE,
            "the appender cannot be opened over a file"
        );
        // the second call is not left waiting for the first one's answer
        assert_eq!(
            new_xlogger_instance(&config, LogLevel::Info),
            DEFAULT_HANDLE
        );
        assert_eq!(get_xlogger_instance("blocked"), DEFAULT_HANDLE);
    }

    /// Two threads asking for the same prefix at once must open **one**
    /// appender between them: a second one over the same `<prefix>.mmap3` would
    /// be closed again, and `close` clears the cache file the first one is
    /// still writing through — which is why the C++ holds its mutex across
    /// `XloggerAppender::NewInstance`.
    #[test]
    fn threads_asking_for_the_same_prefix_at_once_open_one_appender() {
        let _guard = serial();
        let dir = tempfile::tempdir().unwrap();
        let config = config("race", dir.path());

        let handles: Vec<XloggerHandle> = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| new_xlogger_instance(&config, LogLevel::Verbose)))
                .collect();
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect()
        });
        assert!(handles.iter().all(|handle| *handle == handles[0]));
        assert_ne!(handles[0], DEFAULT_HANDLE);

        // one category, and the cache it writes through is the one it opened
        set_appender_mode(handles[0], AppenderMode::Sync);
        assert!(xlogger_write(handles[0], None, Some("REC-AFTER-THE-RACE")));
        flush(handles[0], true);
        let text = log_text(dir.path());
        assert!(text.contains("REC-AFTER-THE-RACE"), "{text}");

        release_xlogger_instance("race");
        crate::appender_close();
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
    fn flush_all_drains_the_instances_as_well() {
        let _guard = serial();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let one = new_xlogger_instance(&config("all-one", first.path()), LogLevel::Verbose);
        let two = new_xlogger_instance(&config("all-two", second.path()), LogLevel::Verbose);
        set_appender_mode(one, AppenderMode::Sync);
        set_appender_mode(two, AppenderMode::Sync);
        assert!(xlogger_write(one, None, Some("ALL-ONE")));
        assert!(xlogger_write(two, None, Some("ALL-TWO")));

        // Not one per-instance `flush`: `flush_all` has to reach both.
        flush_all(true);

        assert!(log_text(first.path()).contains("ALL-ONE"));
        assert!(log_text(second.path()).contains("ALL-TWO"));

        release_xlogger_instance("all-one");
        release_xlogger_instance("all-two");
    }

    #[test]
    fn a_stale_handle_leaves_the_default_appender_alone() {
        let _guard = serial();
        const STALE: XloggerHandle = 999;

        // Only handle `0` names the process-wide appender, so none of these
        // may reach it: a one byte file size would rotate on every record.
        crate::appender_close();
        set_appender_mode(STALE, AppenderMode::Async);
        set_max_file_size(STALE, 1);
        set_max_alive_duration(STALE, 1);
        set_console_log_open(STALE, true);
        flush(STALE, true);

        let dir = tempfile::tempdir().unwrap();
        crate::appender_open(config("default", dir.path())).unwrap();
        crate::appender_write(None, "first record");
        crate::appender_write(None, "second record");
        crate::appender_close();

        let files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".xlog"))
            .collect();
        assert_eq!(
            files.len(),
            1,
            "the default appender was resized: {files:?}"
        );

        // Put the process-wide settings back.
        set_max_file_size(DEFAULT_HANDLE, 0);
        set_console_log_open(DEFAULT_HANDLE, false);
    }

    #[test]
    fn one_shot_recovery_stays_away_from_an_instance_cache() {
        let _guard = serial();
        crate::appender_close();
        let dir = tempfile::tempdir().unwrap();
        let cfg = config("owned", dir.path());
        let handle = new_xlogger_instance(&cfg, LogLevel::Verbose);
        assert_ne!(handle, DEFAULT_HANDLE);

        // The instance owns `<prefix>.mmap3`: recovery must not read and
        // unlink it underneath a live appender.
        let mmap_path = crate::appender::mmap_file_path(&cfg);
        assert!(instance_owns_mmap_path(&mmap_path));
        assert_eq!(
            crate::appender_oneshot_flush(&cfg),
            crate::config::FileIoAction::Unnecessary
        );

        release_xlogger_instance("owned");
        assert!(!instance_owns_mmap_path(&mmap_path));
        crate::appender_close();
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
        // back to the level a fresh process starts with
        set_level(DEFAULT_HANDLE, LogLevel::None);
    }

    #[test]
    fn level_filter_follows_the_cpp_rule() {
        let mut category = XloggerCategory::default();
        // `TLogLevel level_ = kLevelNone`: a logger nobody configured answers
        // `false` for every level, `Fatal` included.
        assert_eq!(category.level(), LogLevel::None);
        assert!(!category.is_enabled_for(LogLevel::Fatal));
        category.set_level(LogLevel::Warn);
        assert!(category.is_enabled_for(LogLevel::Error));
        assert!(category.is_enabled_for(LogLevel::Warn));
        assert!(!category.is_enabled_for(LogLevel::Info));
        assert!(!category.is_enabled_for(LogLevel::Verbose));
        assert_eq!(get_level(12345), None, "unknown handle");
    }

    /// `xlogger_Write` has no level filter: the record goes out through handle
    /// `0` whatever `SetLevel(0, …)` says, because the level is only what
    /// `IsEnabledFor` answers from.
    #[test]
    fn a_record_through_the_default_logger_ignores_the_level() {
        let _guard = serial();
        let dir = tempfile::tempdir().unwrap();
        crate::appender_open(config("gate", dir.path())).unwrap();
        set_appender_mode(DEFAULT_HANDLE, AppenderMode::Sync);
        set_level(DEFAULT_HANDLE, LogLevel::Error);

        let info = XLoggerInfo {
            level: LogLevel::Verbose,
            ..XLoggerInfo::default()
        };
        assert!(!is_enabled_for(DEFAULT_HANDLE, LogLevel::Verbose));
        assert!(xlogger_write(
            DEFAULT_HANDLE,
            Some(&info),
            Some("WRITTEN-ANYWAY")
        ));
        flush(DEFAULT_HANDLE, true);

        assert!(
            log_text(dir.path()).contains("WRITTEN-ANYWAY"),
            "the record was filtered out: {}",
            log_text(dir.path())
        );

        crate::appender_close();
        // `kLevelNone`, the level a fresh process starts with.
        set_level(DEFAULT_HANDLE, LogLevel::None);
    }
}
