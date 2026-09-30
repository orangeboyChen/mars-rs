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
//! handle is a no-op instead of a wild write.
//!
//! # Instances
//!
//! Like the C++, every instance gets its own appender
//! (`appender_open_instance`), so two prefixes can write to two directories
//! with their own key, mode and cache file. There is no process-wide appender
//! beside them: the C++'s `sg_default_appender` is what its free functions
//! write through, and nothing here installs one, so [`DEFAULT_HANDLE`] — the
//! `0` an open that failed answers — names no appender and every call asked of
//! it does nothing.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, RwLock};

use crate::{
    appender_close_instance, appender_flush_instance, appender_flush_now_instance,
    appender_get_current_log_path_instance, appender_open_instance,
    appender_request_flush_instance, appender_set_console_log_instance,
    appender_set_max_alive_duration_instance, appender_set_max_file_size_instance,
    appender_set_mode_instance, appender_write_instance, AppenderId, AppenderMode, Flush, LogLevel,
    XLogConfig, XLoggerInfo,
};

/// Opaque id of a [`XloggerCategory`]; `0` is no logger at all.
pub type XloggerHandle = u64;

/// The handle that names no logger: what an open that failed answers, and
/// the one every call asked of it is a no-op for.
pub const DEFAULT_HANDLE: XloggerHandle = 0;

/// `xlogger_filter_t` of `mars/comm/xlogger/xloggerbase.h` — what an app
/// hands to [`set_filter`] to decide, per record, whether it is written.
///
/// The C++ takes a non-`const` `XLoggerInfo*`, and what it does with the
/// record afterwards is what the filter left behind: a filter may rewrite
/// the level, the tag or any other field and the record goes out rewritten.
/// The return is `<= 0` for "not this one", which is the C++'s
/// `if (filter && filter(&m_info, ...) <= 0) return;`.
pub type XloggerFilter = fn(&mut XLoggerInfo, &str) -> i32;

/// `gs_filter` of `mars/comm/xlogger/xloggerbase.c` — one filter for the
/// whole process, `None` until an app sets one.
static FILTER: RwLock<Option<XloggerFilter>> = RwLock::new(None);

/// `xlogger_SetFilter` — the filter every record is handed before it is
/// written; `None` takes it away again, which is the C++'s
/// `xlogger_SetFilter(NULL)`.
pub fn set_filter(filter: Option<XloggerFilter>) {
    *FILTER
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = filter;
}

/// `xlogger_GetFilter` — the filter a record is about to be handed to, if
/// an app set one.
pub fn get_filter() -> Option<XloggerFilter> {
    *FILTER
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The filter's own answer, which the C++ asks for from inside
/// `XLogger::~XLogger` (`mars/comm/xlogger/xlogger.cc`) — before it writes,
/// and before the level of an instance is asked.
///
/// A record with no message of its own is not handed to a filter: that is
/// the `NULL == _log` promotion, which upstream's streaming helper never
/// reaches either.
fn filtered_out(info: &mut XLoggerInfo, log: &str) -> bool {
    let Some(filter) = get_filter() else {
        return false;
    };
    filter(info, log) <= 0
}

/// `mars::comm::XloggerCategory`.
///
/// The C++ keeps a write callback here as well; what the port keeps is the
/// level and the appender the instance writes through — see the module note.
#[derive(Debug, Clone, Copy)]
pub struct XloggerCategory {
    level: LogLevel,
    /// The C++ gives every instance its own `XloggerAppender`. `None` is what
    /// [`XloggerCategory::default`] builds and what nothing ever registers: a
    /// handle in the table always carries one.
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
    /// The C++ has a second path beside this one — `xlogger_Write`, which its
    /// handle `0` reaches and which has no level filter at all. The port has
    /// no appender for handle `0` to write through, so this is the only path.
    ///
    /// `log` of `None` mirrors the C++ `NULL == _log`: the record is written
    /// anyway, promoted to `Fatal` with a fixed message.
    pub fn write(&self, info: Option<&XLoggerInfo>, log: Option<&str>) -> bool {
        let mut info = info.cloned();

        // The filter is asked first, the way `XLogger::~XLogger` asks it
        // before it writes: what it answers is about the record as the caller
        // made it, level included, and a level it raised is one the test
        // below then lets through.
        if let (Some(info), Some(log)) = (info.as_mut(), log) {
            if filtered_out(info, log) {
                return false;
            }
        }

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

/// What a write ends with: the `NULL == _log` promotion and the write itself.
///
/// `false` for a category that carries no appender, which the table never
/// holds one of — see [`XloggerCategory::appender`].
fn write_log(
    appender: Option<AppenderId>,
    info: &mut Option<XLoggerInfo>,
    log: Option<&str>,
) -> bool {
    let Some(appender) = appender else {
        return false;
    };
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
    /// `<dir>/<prefix>.mmap3` and the slots next to it: one-shot recovery reads
    /// and unlinks those files, so it has to stay away from an instance that
    /// owns one.
    mmap_paths: HashMap<XloggerHandle, PathBuf>,
    /// The prefixes whose appender is being opened right now.
    opening: HashSet<String>,
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
pub(crate) enum Target {
    /// The instance's own appender.
    Instance(AppenderId),
    /// Nothing at all: [`DEFAULT_HANDLE`], a handle whose instance was
    /// released, or one that was never handed out. Calls through it are a
    /// no-op — there is no process-wide appender to fall back to.
    Gone,
}

pub(crate) fn target(handle: XloggerHandle) -> Target {
    match lookup(handle).and_then(|category| category.appender) {
        Some(id) => Target::Instance(id),
        None => Target::Gone,
    }
}

/// Writes through the instance's own appender.
fn write_through(id: AppenderId, info: Option<&XLoggerInfo>, log: &str) -> bool {
    appender_write_instance(id, info, log)
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
    // The slot the instance got, not `<prefix>.mmap3` as such: a second process
    // using the same prefix holds slot 0 and this one has been given slot 1.
    if let Some(path) = appender.and_then(crate::instance_cache_path) {
        registry.mmap_paths.insert(handle, path);
    }
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
    release_locked(
        registry().lock().unwrap_or_else(|e| e.into_inner()),
        nameprefix,
    );
}

/// [`release_xlogger_instance`] for a caller that knows which handle it holds:
/// the prefix is released only while it still answers `handle`.
///
/// Releasing takes the prefix and not the handle, so asking the registry and
/// then releasing is two answers and not one: a third `Xlog` opening this
/// prefix in between is handed a new handle, and the release by prefix closes
/// that one — the appender the caller was asking about is already gone and the
/// one it just closed is not its own. This is the question and the release
/// under one lock, so nothing can land between them.
pub fn release_xlogger_instance_of(nameprefix: &str, handle: XloggerHandle) {
    let registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    if registry.by_prefix.get(nameprefix).copied() != Some(handle) {
        return;
    }
    release_locked(registry, nameprefix);
}

fn release_locked(mut registry: MutexGuard<'_, Registry>, nameprefix: &str) {
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
/// An unknown handle writes nothing: [`DEFAULT_HANDLE`] names no appender, and
/// neither does one whose instance was released. The module promises that a
/// stale handle is a no-op, and there is no default logger to fall back to.
pub fn xlogger_write(handle: XloggerHandle, info: Option<&XLoggerInfo>, log: Option<&str>) -> bool {
    match lookup(handle) {
        Some(category) => category.write(info, log),
        None => false,
    }
}

/// `mars::xlog::IsEnabledFor`.
///
/// `false` for an unknown handle — [`DEFAULT_HANDLE`] among them — so nothing
/// is written through it.
pub fn is_enabled_for(handle: XloggerHandle, level: LogLevel) -> bool {
    match lookup(handle) {
        Some(category) => category.is_enabled_for(level),
        None => false,
    }
}

/// `mars::xlog::GetLevel`.
///
/// `None` for an unknown handle.
pub fn get_level(handle: XloggerHandle) -> Option<LogLevel> {
    lookup(handle).map(|category| category.level())
}

/// `mars::xlog::SetLevel` — a no-op for an unknown handle.
pub fn set_level(handle: XloggerHandle, level: LogLevel) {
    with_category_mut(handle, |category| category.set_level(level));
}

/// `mars::xlog::SetAppenderMode` — applies to the instance's own appender.
///
/// A handle whose instance is gone changes nothing.
pub fn set_appender_mode(handle: XloggerHandle, mode: AppenderMode) {
    match target(handle) {
        Target::Instance(id) => appender_set_mode_instance(id, mode),
        Target::Gone => {}
    }
}

/// `mars::xlog::SetMaxFileSize` — per instance, like
/// `xlogger_interface.cc`'s `SetMaxFileSize`.
pub fn set_max_file_size(handle: XloggerHandle, bytes: u64) {
    match target(handle) {
        Target::Instance(id) => appender_set_max_file_size_instance(id, bytes),
        Target::Gone => {}
    }
}

/// `mars::xlog::SetMaxAliveTime` — per instance.
///
/// Whether `secs` was applied is what the answer says: the appender refuses a
/// value below one day, the way `open` refuses one, and a caller that reports
/// the value it asked for would be reporting a limit no file is held to.
pub fn set_max_alive_duration(handle: XloggerHandle, secs: u64) -> bool {
    match target(handle) {
        Target::Instance(id) => appender_set_max_alive_duration_instance(id, secs),
        Target::Gone => false,
    }
}

/// `mars::xlog::Flush` — asks the writer thread of the handle's own appender
/// to drain, and returns at once: the drain is the writer's, and nothing here
/// says when it is over. [`flush_now`] is the call that comes back with the
/// records on the disk, and [`flush`] is the one an async caller awaits.
///
/// A no-op for a handle whose instance was released — see `Target::Gone`.
pub fn request_flush(handle: XloggerHandle) {
    match target(handle) {
        Target::Instance(id) => appender_request_flush_instance(id),
        Target::Gone => {}
    }
}

/// [`request_flush`] drained on the calling thread: the records are on the disk
/// when this returns.
///
/// A no-op for a handle whose instance was released — see `Target::Gone`.
pub fn flush_now(handle: XloggerHandle) {
    match target(handle) {
        Target::Instance(id) => appender_flush_now_instance(id),
        Target::Gone => {}
    }
}

/// [`flush_now`] for a caller that can wait without holding a thread: the
/// [`Flush`] this hands back drains on a thread of its own and is Ready when
/// the records are on the disk.
///
/// Nothing drains until the future is polled, and dropping it does not stop a
/// drain that has started. A future that drains nothing for a handle whose
/// instance was released — see `Target::Gone`.
pub fn flush(handle: XloggerHandle) -> Flush {
    match target(handle) {
        Target::Instance(id) => appender_flush_instance(id),
        Target::Gone => Flush::noop(),
    }
}

/// `mars::xlog::SetConsoleLogOpen` — per instance.
pub fn set_console_log_open(handle: XloggerHandle, open: bool) {
    match target(handle) {
        Target::Instance(id) => appender_set_console_log_instance(id, open),
        Target::Gone => {}
    }
}

/// The directory the appender of `handle` writes its files to, or `None` when
/// there is none — an appender with no log dir was never opened.
///
/// The name is the C++'s, and so is what it answers:
/// the directory and not a file — the same question `Xlog::current_log_path`
/// asks of an object.
pub fn current_log_path(handle: XloggerHandle) -> Option<PathBuf> {
    match target(handle) {
        Target::Instance(id) => appender_get_current_log_path_instance(id),
        Target::Gone => None,
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
        flush_now(one);
        flush_now(two);

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

    /// Takes the filter away again when the test is over — including when it
    /// panicked: it is one process-wide static, and a filter left behind
    /// answers for the records of every test that runs after it.
    struct NoFilter;

    impl Drop for NoFilter {
        fn drop(&mut self) {
            set_filter(None);
        }
    }

    /// The same filter, on the path of a handle that names an **instance**:
    /// `XLogger::~XLogger` asks it before it writes there too.
    #[test]
    fn a_filter_is_asked_about_the_records_of_an_instance_too() {
        let _guard = serial();
        let _no_filter = NoFilter;
        let dir = tempfile::tempdir().unwrap();
        let handle = new_xlogger_instance(&config("filtered", dir.path()), LogLevel::Verbose);
        set_appender_mode(handle, AppenderMode::Sync);
        set_filter(Some(|_, log| i32::from(log.contains("KEPT"))));

        let info = XLoggerInfo {
            level: LogLevel::Verbose,
            ..XLoggerInfo::default()
        };
        assert!(!xlogger_write(
            handle,
            Some(&info),
            Some("INSTANCE-DROPPED")
        ));
        assert!(xlogger_write(handle, Some(&info), Some("INSTANCE-KEPT")));
        flush_now(handle);

        let text = log_text(dir.path());
        assert!(text.contains("INSTANCE-KEPT"), "{text}");
        assert!(!text.contains("INSTANCE-DROPPED"), "{text}");

        release_xlogger_instance("filtered");
        set_filter(None);
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

    /// Handle `0` is not a logger: it names no appender and no category, so
    /// the level it is given is dropped and every question about it answers
    /// "nothing there" — the same answers a released handle gives.
    #[test]
    fn the_default_handle_is_no_logger() {
        let _guard = serial();
        set_level(DEFAULT_HANDLE, LogLevel::Error);
        assert_eq!(get_level(DEFAULT_HANDLE), None);
        assert!(!is_enabled_for(DEFAULT_HANDLE, LogLevel::Fatal));
        assert!(!xlogger_write(DEFAULT_HANDLE, None, Some("dropped")));
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
}
