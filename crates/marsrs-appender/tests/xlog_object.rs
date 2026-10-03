//! Integration tests for [`Xlog`]: the appender an app holds, which is the one
//! shape every platform of the port has — `Xlog.open(config)` in Kotlin, in Dart
//! and in TypeScript, `Xlog(config:)` in Swift, `Xlog::open(config)` here.
//!
//! What is under test is the object and not the appender: that a record reaches
//! the file, that the level gate is the object's, that two objects of one prefix
//! are one appender, and that a closed — or dropped — one writes nothing. The
//! bytes themselves are the appender's own tests' business.

use std::path::PathBuf;

use marsrs_appender::{
    get_xlogger_instance, AppenderError, AppenderMode, LogLevel, XLogConfig, Xlog, DEFAULT_HANDLE,
};
use marsrs_crypt::magic;

fn config(dir: &std::path::Path, nameprefix: &str) -> XLogConfig {
    config_cached(dir, nameprefix, None)
}

/// [`config`] with a cache directory: the one branch of a day's names the
/// other tests never take, and the one that used to be filtered by whether
/// the file existed.
fn config_cached(
    dir: &std::path::Path,
    nameprefix: &str,
    cachedir: Option<std::path::PathBuf>,
) -> XLogConfig {
    XLogConfig {
        mode: AppenderMode::Sync,
        logdir: dir.to_path_buf(),
        nameprefix: nameprefix.to_owned(),
        pub_key: String::new(),
        compress_mode: marsrs_buffer::CompressMode::Zlib,
        compress_level: 6,
        cachedir,
        cache_days: 0,
    }
}

/// The bytes of the file the appender wrote, as one whole `.xlog`: the magic
/// number is the first thing in it, so an empty file — or one that is only a
/// header — is a write that did not happen.
fn wrote(dir: &std::path::Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(files_first(dir)?).ok()?;
    (!bytes.is_empty() && magic::magic_start_is_valid(bytes[0])).then_some(bytes)
}

/// The one `.xlog` of the directory, which is the file the appender is writing
/// today.
fn files_first(dir: &std::path::Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "xlog"))
        .collect();
    files.sort();
    files.first().cloned()
}
#[test]
fn an_object_writes_and_drains_to_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let xlog = Xlog::open(config(dir.path(), "writes"), LogLevel::Info).unwrap();

    assert!(xlog.is_open());
    assert_eq!(xlog.name_prefix(), "writes");
    assert_eq!(xlog.level(), Some(LogLevel::Info));
    assert!(xlog.i("startup", "hello from mars"));

    // Sync mode hands every record straight to the file, so no drain is needed
    // for the record to be there — but the drain is what closes the file off.
    xlog.flush_now();
    let bytes = wrote(dir.path()).expect("the object wrote a .xlog");
    assert!(bytes.windows(6).any(|w| w == b"hello "));

    // The directory, which is what the C++'s `GetCurrentLogPath` answers:
    // a day's file is named for the day its records carry.
    assert_eq!(xlog.current_log_path(), Some(dir.path().to_path_buf()));

    // A day of files, asked of this object and out of its own prefix and
    // directory — and not of a directory the caller names.
    let today = files_first(dir.path()).expect("the day's file is there");
    assert_eq!(xlog.log_files(0), vec![today.clone()]);
    assert_eq!(xlog.log_file_names(0), vec![today]);
    assert!(
        xlog.log_files(1).is_empty(),
        "yesterday has no file: {:?}",
        xlog.log_files(1)
    );

    xlog.close();
    assert!(!xlog.is_open());
    assert!(!xlog.i("startup", "nothing after close"));

    // A closed object answers neither: a handle whose appender is gone has no
    // directory and no day, and an empty list is what a caller gets and not a
    // panic.
    assert_eq!(xlog.current_log_path(), None);
    assert!(xlog.log_files(0).is_empty());
    assert!(xlog.log_file_names(0).is_empty());
}

#[test]
fn the_level_gate_is_the_objects_own() {
    let dir = tempfile::tempdir().unwrap();
    let xlog = Xlog::open(config(dir.path(), "levels"), LogLevel::Warn).unwrap();

    assert!(!xlog.is_loggable(LogLevel::Debug));
    assert!(xlog.is_loggable(LogLevel::Error));
    assert!(!xlog.d("startup", "dropped"));
    assert!(xlog.e("startup", "kept"));
    assert!(xlog.log(LogLevel::Fatal, "startup", "kept too"));

    xlog.set_level(LogLevel::Verbose);
    assert_eq!(xlog.level(), Some(LogLevel::Verbose));
    assert!(xlog.v("startup", "kept now"));

    // Below `None`, which is the level that drops everything.
    xlog.set_level(LogLevel::None);
    assert!(!xlog.f("startup", "dropped"));
}

#[test]
fn the_four_the_appender_has_no_getter_for_answer_what_was_set() {
    let dir = tempfile::tempdir().unwrap();
    let xlog = Xlog::open(config(dir.path(), "mirrored"), LogLevel::Info).unwrap();

    assert_eq!(xlog.mode(), AppenderMode::Sync);
    xlog.set_mode(AppenderMode::Async);
    assert_eq!(xlog.mode(), AppenderMode::Async);

    assert!(!xlog.console_log_enabled());
    xlog.set_console_log_enabled(true);
    assert!(xlog.console_log_enabled());

    assert_eq!(xlog.max_file_size_bytes(), 0);
    xlog.set_max_file_size_bytes(1 << 20);
    assert_eq!(xlog.max_file_size_bytes(), 1 << 20);

    // Three days, and not an hour: a limit below one day is one the appender
    // refuses, and the getter answers what is in force — see the test below.
    assert_eq!(xlog.max_alive_time_seconds(), 0);
    xlog.set_max_alive_time_seconds(3 * 24 * 60 * 60);
    assert_eq!(xlog.max_alive_time_seconds(), 3 * 24 * 60 * 60);
}

/// The one of the four the appender can refuse: a value below one day.
///
/// The getter answers what the appender holds files to, and not what was
/// asked for, because the appender is what deletes them. An app that reads
/// back an hour it never got stops uploading the logs it believes are gone.
#[test]
fn an_alive_time_below_a_day_is_refused_and_the_getter_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let xlog = Xlog::open(config(dir.path(), "alive"), LogLevel::Info).unwrap();

    xlog.set_max_alive_time_seconds(3600);
    assert_eq!(
        xlog.max_alive_time_seconds(),
        0,
        "what this `Xlog` last set is what it answers, and the hour was refused"
    );

    // A value the appender does take is the one the getter answers.
    xlog.set_max_alive_time_seconds(3 * 24 * 60 * 60);
    assert_eq!(xlog.max_alive_time_seconds(), 3 * 24 * 60 * 60);
}

#[test]
fn two_objects_of_one_prefix_are_one_appender() {
    let dir = tempfile::tempdir().unwrap();
    let one = Xlog::open(config(dir.path(), "shared"), LogLevel::Info).unwrap();
    let two = Xlog::open(config(dir.path(), "shared"), LogLevel::Info).unwrap();

    // The four the appender has no getter for are this object's own answer and
    // not the appender's, which is what `Xlog`'s own doc says of them: `one`
    // set the alive time on the appender it shares with `two`, and `two` goes
    // on answering what `two` last set itself, which is nothing — `0`, the
    // ten days. An app that reads the value back out of a second `Xlog` of a
    // prefix is told its logs are kept ten days whatever the first one asked
    // for, and no platform of the port can do better: there is nothing to
    // read the value back from.
    one.set_max_alive_time_seconds(3 * 24 * 60 * 60);
    assert_eq!(one.max_alive_time_seconds(), 3 * 24 * 60 * 60);
    assert_eq!(
        two.max_alive_time_seconds(),
        0,
        "the mirror is this object's and not the appender's"
    );

    // The second is the appender the first opened, and not a second one.
    assert!(two.is_open());
    one.close();

    // Closing one closes what the other writes through: a prefix is one
    // appender, which is what every platform's `Xlog` says about itself.
    assert!(!two.i("startup", "written through a closed appender"));
    assert_eq!(get_xlogger_instance("shared"), DEFAULT_HANDLE);

    // … and the other one says so, which the handle it cached cannot: `isOpen`
    // is asked of the prefix and not of the handle, on every platform.
    assert!(!two.is_open(), "a twin's close left the other open");
    assert_eq!(two.level(), None);
    assert_eq!(two.current_log_path(), None);
}

#[test]
fn close_is_safe_twice_and_the_destructor_closes() {
    let dir = tempfile::tempdir().unwrap();
    let xlog = Xlog::open(config(dir.path(), "closed"), LogLevel::Info).unwrap();
    xlog.close();
    xlog.close();
    assert!(!xlog.is_open());

    // Dropped, not closed: the destructor is what releases the prefix, so a
    // prefix that was scoped to a block is one the next block can open again.
    drop(Xlog::open(config(dir.path(), "dropped"), LogLevel::Info).unwrap());
    assert_eq!(get_xlogger_instance("dropped"), DEFAULT_HANDLE);
}

/// The C++'s `NewXloggerInstance`, and what every platform of the port says about
/// itself: a prefix is one appender, so the second `Xlog::open` of one is the
/// first one — and the config it hands in is ignored, level included. What that
/// buys is that two modules of one app can each open the logger they write
/// through without either of them holding a handle for the other; what it costs
/// is that a config that differs is silently the first one's.
#[test]
fn a_second_open_of_one_prefix_answers_the_first_appender() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();

    let one = Xlog::open(config(first_dir.path(), "twice"), LogLevel::Warn).unwrap();
    // The same prefix, a different directory and a different level: both are
    // the first config's.
    let two = Xlog::open(config(second_dir.path(), "twice"), LogLevel::Verbose).unwrap();

    assert!(two.is_open());
    assert_eq!(two.level(), Some(LogLevel::Warn));
    assert_eq!(two.current_log_path(), Some(first_dir.path().to_path_buf()));

    assert!(one.w("startup", "through the first"));
    assert!(!two.d("startup", "dropped: the level is the first one's"));
    assert!(two.w("startup", "through the second"));
    one.flush_now();

    let bytes = wrote(first_dir.path()).expect("the appender wrote one file");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("through the first"), "{text}");
    assert!(text.contains("through the second"), "{text}");
    assert!(!text.contains("dropped"), "{text}");

    // Nothing was opened in the second config's directory: an appender is one
    // per prefix, and the prefix was already taken.
    assert!(files_first(second_dir.path()).is_none());
}

#[test]
fn a_config_the_appender_refuses_is_an_error() {
    let xlog = Xlog::open(
        XLogConfig {
            logdir: PathBuf::new(),
            ..config(std::path::Path::new("/tmp"), "empty")
        },
        LogLevel::Info,
    );
    assert!(matches!(xlog, Err(AppenderError(_))));

    let xlog = Xlog::open(
        XLogConfig {
            nameprefix: String::new(),
            ..config(std::path::Path::new("/tmp"), "empty")
        },
        LogLevel::Info,
    );
    assert!(matches!(xlog, Err(AppenderError(_))));
}

/// A day's *names* with a cache directory, which is the one branch the answer
/// is two paths and not one: the name the day is written under in the log
/// directory, and the twin the async cache file has in the cache directory.
#[test]
fn a_day_of_names_is_answered_before_the_files_are_there() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let xlog = Xlog::open(
        config_cached(dir.path(), "cached", Some(cache.path().to_path_buf())),
        LogLevel::Info,
    )
    .unwrap();

    // The name of the day is answered before anything is written to it, and
    // it is the log directory's: a *name* is not a file, and which files are
    // there is `log_files`' question.
    let names = xlog.log_file_names(0);
    assert_eq!(names[0].parent(), Some(dir.path()), "{names:?}");

    xlog.i("startup", "hello from mars");
    xlog.flush_now();

    // Now both are: the file in the log directory, and the cache-dir twin the
    // async cache had while the record sat in it.
    let files = xlog.log_files(0);
    assert!(!files.is_empty(), "the record reached a file");
    let names = xlog.log_file_names(0);
    assert!(
        names.contains(&files[0]),
        "{names:?} does not name {files:?}"
    );

    // A day that has not happened yet is not one, and a negative `daysAgo`
    // does not ask for it — it asks for the day that is here. Both answers
    // are taken before the comparison, so a run that straddles midnight
    // compares two names of the same day and not of two.
    let today = xlog.log_file_names(0);
    assert_eq!(
        xlog.log_file_names(-1),
        today,
        "a negative day asked for tomorrow"
    );
    assert_eq!(xlog.log_files(-1), xlog.log_files(0));
}
