//! File-name and file-lifecycle helpers — port of the private
//! `XloggerAppender::__*File*` helpers in `mars/xlog/src/appender.cc`
//! (`__MakeLogFileNamePrefix`, `__GetFileNamesByPrefix`,
//! `__GetFilePathsFromTimeval`, `__GetNextFileIndex`, `__MakeLogFileName`,
//! `__DelTimeoutFile`, `__AppendFile`, `__MoveOldFiles`).
//!
//! They are `pub(crate)` because the contract exposes them only through the
//! appender: `Appender::make_logfile_name` and
//! `Appender::getfilepath_from_timespan` are what a day of files is asked of,
//! and everything else stays an implementation detail of the appender.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use chrono::{Local, TimeZone};
use marsrs_core::local_time;

/// `LOG_EXT` in `appender.cc`.
pub(crate) const LOG_EXT: &str = "xlog";
/// Extension of the mmap cache file. Must match `appender.cc` exactly so a
/// cache file left behind by the C++ implementation is still drained by the
/// Rust one (and vice versa).
pub(crate) const MMAP_EXT: &str = "mmap3";
/// One day in seconds — the unit of the `timespan` arguments.
pub(crate) const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

/// The mode every file this crate creates gets on unix: for its owner alone.
///
/// A log holds everything an app wrote since it was installed, and the cache
/// file holds the records of the block that is not in a log yet — which is the
/// one file a reader would have to go after to read records before they are
/// filed. `std` creates both with the process default, `0666 & umask`, which
/// under the usual `022` leaves them readable by every other uid on the
/// device; the private key `xlog keygen` writes was already opened this way.
/// A platform with no unix modes has nothing to set: there the directory the
/// app is sandboxed in is what keeps its files private.
#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;
/// [`PRIVATE_FILE_MODE`] for a directory: `0o700`, because a directory every
/// uid can traverse is one whose files every uid can open however narrow the
/// files' own mode is.
#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;

/// Sets the mode a file of this crate is created with: see
/// [`PRIVATE_FILE_MODE`]. Given to the `open` that creates the file, so there
/// is no window in which it is permissive.
#[cfg(unix)]
pub(crate) fn private_file(options: &mut OpenOptions) -> &mut OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(PRIVATE_FILE_MODE)
}

/// On a platform with no unix modes there is nothing to set — see
/// [`PRIVATE_FILE_MODE`].
#[cfg(not(unix))]
pub(crate) fn private_file(options: &mut OpenOptions) -> &mut OpenOptions {
    options
}

/// [`fs::create_dir_all`] over [`PRIVATE_DIR_MODE`].
#[cfg(unix)]
pub(crate) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    fs::DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(path)
}

/// On a platform with no unix modes there is nothing to set — see
/// [`PRIVATE_FILE_MODE`].
#[cfg(not(unix))]
pub(crate) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

/// `gettimeofday` — the pair `XLoggerInfo::timeval` carries.
pub(crate) fn now_timeval() -> (i64, i64) {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.as_secs() as i64, i64::from(d.subsec_micros())))
        .unwrap_or((0, 0))
}

/// Wall-clock seconds since the epoch (`gettimeofday` / `time(nullptr)`).
pub(crate) fn now_secs() -> i64 {
    now_timeval().0
}

/// Monotonic milliseconds since the first call — the port of
/// `mars::comm::gettickcount()` used for `last_tick_`.
pub(crate) fn monotonic_millis() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_millis() as u64
}

/// Local calendar date of `_tv_sec` as `(year, month, day)`.
///
/// Comes out of [`marsrs_core::local_time`], so the per-record day-rollover check
/// costs no `localtime` of its own when another part of the same record already
/// asked for that second.
fn local_date_parts(tv_sec: i64) -> (i32, u32, u32) {
    local_time(tv_sec).date
}

/// `XloggerAppender::__MakeLogFileNamePrefix` — `<prefix>_<YYYYMMDD>`.
pub(crate) fn make_log_file_name_prefix(tv_sec: i64, prefix: &str) -> String {
    let (year, month, day) = local_date_parts(tv_sec);
    format!("{prefix}_{year:04}{month:02}{day:02}")
}

/// Whether `name` is a log file this appender wrote:
/// `<nameprefix>_<YYYYMMDD>.xlog`, or the same with a `_<index>` for a day
/// that was split for size — and nothing else.
///
/// The C++ matches `starts_with(prefix) && ends_with(ext)`, which is the
/// right question for [`get_file_names_by_prefix`] and too wide a one for a
/// caller holding the bare prefix of a configuration: `Mars` is a prefix of
/// `Mars_push_20240101.xlog`, and `Mars_push` is a prefix another `Xlog` is
/// every bit as entitled to open in the same `logdir` — so the sweep deleted
/// another appender's logs on an expiry that was not theirs, which is the
/// data loss the prefix match was added to prevent. `ends_with(LOG_EXT)` is
/// the wider half of the same mistake: it takes any name ending in those four
/// letters, dot or no dot. What is matched is the shape
/// [`make_log_file_name`] writes, and a file of some other shape is left
/// alone even when a prefix of it is ours.
fn is_log_file_of(name: &str, nameprefix: &str) -> bool {
    let Some(rest) = name
        .strip_prefix(nameprefix)
        .and_then(|rest| rest.strip_prefix('_'))
    else {
        return false;
    };
    let Some(stem) = rest.strip_suffix(&format!(".{LOG_EXT}")) else {
        return false;
    };
    let (date, index) = match stem.split_once('_') {
        Some((date, index)) => (date, index),
        None => (stem, ""),
    };
    let is_date = date.len() == 8 && date.chars().all(|c| c.is_ascii_digit());
    let is_index = index.chars().all(|c| c.is_ascii_digit());
    is_date && is_index
}

/// `__GetFileNamesByPrefix` — the *names* (not paths) of the regular files in
/// `_logdir` that start with `_fileprefix` and end with `._fileext`.
///
/// The prefix it is handed is `<prefix>_<YYYYMMDD>` (see
/// [`get_file_paths_from_timeval`]), so `starts_with` is already as narrow as
/// one day of one appender's files; [`is_log_file_of`] is the same question
/// for a caller holding the bare prefix of a configuration.
pub(crate) fn get_file_names_by_prefix(dir: &Path, fileprefix: &str, fileext: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let suffix = format!(".{fileext}");
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(fileprefix) && name.ends_with(&suffix) {
            names.push(name);
        }
    }
    names
}

/// `XloggerAppender::__GetFilePathsFromTimeval`.
pub(crate) fn get_file_paths_from_timeval(
    tv_sec: i64,
    logdir: &Path,
    prefix: &str,
    fileext: &str,
) -> Vec<PathBuf> {
    let fileprefix = make_log_file_name_prefix(tv_sec, prefix);
    get_file_names_by_prefix(logdir, &fileprefix, fileext)
        .into_iter()
        .map(|name| logdir.join(name))
        .collect()
}

/// `XloggerAppender::__GetNextFileIndex`.
///
/// `_scan_dir` is the directory whose files decide the index (`config_.logdir_`
/// in C++); `_cachedir` files are counted as well.
pub(crate) fn get_next_file_index(
    scan_dir: &Path,
    cachedir: Option<&Path>,
    fileprefix: &str,
    fileext: &str,
    max_file_size: u64,
) -> i64 {
    let mut filename_vec = get_file_names_by_prefix(scan_dir, fileprefix, fileext);
    if let Some(dir) = cachedir {
        filename_vec.extend(get_file_names_by_prefix(dir, fileprefix, fileext));
    }

    if filename_vec.is_empty() {
        return 0;
    }

    // `__string_compare_greater`: longest first, then lexicographically greatest.
    filename_vec.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| b.cmp(a)));
    let last_filename = &filename_vec[0];

    let mut index = 0i64;
    let suffix = format!(".{fileext}");
    if let Some(ext_pos) = last_filename.rfind(&suffix) {
        if let Some(index_str) = last_filename.get(fileprefix.len()..ext_pos) {
            if !index_str.is_empty() {
                let index_str = index_str.strip_prefix('_').unwrap_or(index_str);
                // C++ `atol` — leading garbage yields 0.
                index = index_str.parse::<i64>().unwrap_or(0);
            }
        }
    }

    let mut filesize = 0u64;
    let logfilepath = scan_dir.join(last_filename);
    if let Ok(meta) = fs::metadata(&logfilepath) {
        filesize += meta.len();
    }
    if let Some(dir) = cachedir {
        if let Ok(meta) = fs::metadata(dir.join(last_filename)) {
            filesize += meta.len();
        }
    }

    if filesize > max_file_size {
        // i64::MAX + 1 must not wrap: in release that silently stops rotation
        // and in debug it panics — on the async writer thread, which has no
        // panic barrier.
        index.saturating_add(1)
    } else {
        index
    }
}

/// `XloggerAppender::__MakeLogFileName`.
///
/// `_out_dir` is where the file lives (`_log_dir` argument), `_scan_dir` is the
/// directory whose existing files decide the `_1`, `_2`, ... split index
/// (C++ always scans `config_.logdir_`).
pub(crate) fn make_log_file_name(
    tv_sec: i64,
    out_dir: &Path,
    scan_dir: &Path,
    prefix: &str,
    fileext: &str,
    max_file_size: u64,
    cachedir: Option<&Path>,
) -> PathBuf {
    let fileprefix = make_log_file_name_prefix(tv_sec, prefix);
    let index = if max_file_size > 0 {
        get_next_file_index(scan_dir, cachedir, &fileprefix, fileext, max_file_size)
    } else {
        0
    };

    let mut name = fileprefix;
    if index > 0 {
        name.push_str(&format!("_{index}"));
    }
    name.push('.');
    name.push_str(fileext);

    out_dir.join(name)
}

/// `strftime(tmp_time, sizeof(tmp_time), "%Y-%m-%d %z %H:%M:%S", &tm)` as
/// used by `XloggerAppender::__GetMarkInfo` and the "last log file" record.
pub(crate) fn format_local_timestamp(tv_sec: i64) -> String {
    match Local.timestamp_opt(tv_sec, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %z %H:%M:%S").to_string(),
        None => String::from("1970-01-01 +0000 00:00:00"),
    }
}

/// `__DATE__` / `__TIME__` of `appender.cc`'s open and close banners: **when
/// this crate was built**, not when the appender was opened.
///
/// The C++ splices the compiler's `__DATE__` / `__TIME__` into the banner, so a
/// log file says which build produced it. Rust has no such constant, so
/// `build.rs` captures the time of the build and this reads it back. A
/// translation unit keeps its stamp until it is recompiled, and so does this.
pub(crate) fn build_stamp() -> (String, String) {
    let built_at: i64 = env!("MARS_XLOG_BUILD_TIMESTAMP").parse().unwrap_or(0);
    match Local.timestamp_opt(built_at, 0).single() {
        Some(dt) => (
            dt.format("%Y-%m-%d").to_string(),
            dt.format("%H:%M:%S").to_string(),
        ),
        None => ("1970-01-01".to_owned(), "00:00:00".to_owned()),
    }
}

/// `__DelTimeoutFile`.
///
/// `<nameprefix>_<YYYYMMDD>[_<index>].xlog` files that have not been touched
/// for `_max_alive_time` seconds.
///
/// The files are matched by the whole name [`make_log_file_name`] writes, the
/// way [`move_old_files`] does — and not by `starts_with` of the
/// prefix alone: one `logdir` is what two `Xlog`s with two prefixes share — an
/// app and a module it links, say — and a sweep over every `.xlog` in it
/// deletes the other one's logs, on a schedule that is not its own. The C++
/// has one prefix per process, so it never had the question.
///
/// A directory is never removed, whatever it is named: see the comment at the
/// end of the loop.
pub(crate) fn del_timeout_file(dir: &Path, max_alive_time: i64, nameprefix: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        // `elapsed()` fails when the file was modified in the future, which is
        // exactly the `now_time > file_modify_time` guard in the C++.
        let Ok(age) = modified.elapsed() else {
            continue;
        };
        if age.as_secs() as i64 <= max_alive_time {
            continue;
        }

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if meta.is_file() && is_log_file_of(&name, nameprefix) {
            let _ = fs::remove_file(&path);
        }
        // The C++ deletes its `YYYYMMDD` dump directories here. This port
        // writes none — see `dump.rs`, whose only dump is the string
        // `xlogger_memory_dump` answers with — so every such directory in a
        // `logdir` is one something else made: an app's own dated folder, or
        // another component's. Removing it recursively, at every `open`, under
        // the log's lock, is the one thing in this file that can destroy data
        // it was never shown the ownership of, so the arm is not carried over.
    }
}

/// `XloggerAppender::__AppendFile` — appends `_src_file` to `_dst_file`,
/// rolling the destination back to its original length on a short write.
pub(crate) fn append_file(src_file: &Path, dst_file: &Path) -> bool {
    if src_file == dst_file {
        return false;
    }

    let Ok(src_meta) = fs::metadata(src_file) else {
        return false;
    };
    if src_meta.len() == 0 {
        return true;
    }

    let Ok(mut src) = File::open(src_file) else {
        return false;
    };
    // The destination is a log file this crate may be creating: see
    // [`private_file`].
    let Ok(mut dst) = private_file(OpenOptions::new().create(true).append(true)).open(dst_file)
    else {
        return false;
    };

    let src_len = src_meta.len();
    let dst_len = fs::metadata(dst_file).map(|m| m.len()).unwrap_or(0);

    let mut buffer = [0u8; 4096];
    loop {
        match src.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                if dst.write_all(&buffer[..n]).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let written = fs::metadata(dst_file).map(|m| m.len()).unwrap_or(0);
    if dst_len + src_len > written {
        // Partial write: undo it.
        let _ = dst.set_len(dst_len);
        return false;
    }

    true
}

/// `XloggerAppender::__MoveOldFiles` — drains `_src_path` (the cache dir) into
/// `_dest_path` (the log dir).
///
/// Matched the way [`del_timeout_file`] matches, and not by `starts_with` of
/// the prefix: a cache dir is what one `logdir`'s two `Xlog`s share as well,
/// and appending another prefix's cache file into a log of this one is
/// records filed under a name they were never written under.
pub(crate) fn move_old_files(src_path: &Path, dest_path: &Path, nameprefix: &str, cache_days: u32) {
    if src_path == dest_path {
        return;
    }
    let Ok(entries) = fs::read_dir(src_path) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_log_file_of(&name, nameprefix) {
            continue;
        }

        if cache_days > 0 {
            let skip = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|m| m.elapsed().ok())
                .map(|age| age.as_secs() < u64::from(cache_days) * SECONDS_PER_DAY as u64)
                .unwrap_or(false);
            if skip {
                continue;
            }
        }

        let path = entry.path();
        if !append_file(&path, &dest_path.join(&name)) {
            break;
        }
        let _ = fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn touch(path: &Path, bytes: &[u8]) {
        let mut f = File::create(path).unwrap();
        f.write_all(bytes).unwrap();
    }

    #[test]
    fn name_prefix_is_prefix_yyyymmdd() {
        // 2023-11-27 12:00:00 UTC -> local date may differ by a day, so only the
        // shape is asserted.
        let p = make_log_file_name_prefix(1_701_000_000, "Mars");
        assert!(p.starts_with("Mars_"), "{p}");
        assert_eq!(p.len(), "Mars_".len() + 8, "{p}");
        assert!(p[5..].chars().all(|c| c.is_ascii_digit()), "{p}");
    }

    #[test]
    fn make_log_file_name_without_split() {
        let dir = Path::new("/tmp/log");
        let p = make_log_file_name(1_701_000_000, dir, dir, "Mars", LOG_EXT, 0, None);
        let name = p.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("Mars_"), "{name}");
        assert!(name.ends_with(".xlog"), "{name}");
        assert_eq!(name.matches('_').count(), 1, "{name}");
    }

    #[test]
    fn next_file_index_counts_splits_and_size() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let prefix = make_log_file_name_prefix(now_secs(), "Mars");

        // No files at all -> index 0.
        assert_eq!(get_next_file_index(dir, None, &prefix, LOG_EXT, 10), 0);

        touch(&dir.join(format!("{prefix}.xlog")), &[0u8; 20]);
        // The only file is over the limit -> next index is 1.
        assert_eq!(get_next_file_index(dir, None, &prefix, LOG_EXT, 10), 1);
        // ... and stays 1 while the newest file is still small.
        touch(&dir.join(format!("{prefix}_1.xlog")), &[0u8; 2]);
        assert_eq!(get_next_file_index(dir, None, &prefix, LOG_EXT, 10), 1);

        // `_10` sorts before `_9` because it is longer (C++ `__string_compare_greater`).
        touch(&dir.join(format!("{prefix}_9.xlog")), &[0u8; 2]);
        assert_eq!(get_next_file_index(dir, None, &prefix, LOG_EXT, 10), 9);
        touch(&dir.join(format!("{prefix}_10.xlog")), &[0u8; 2]);
        assert_eq!(get_next_file_index(dir, None, &prefix, LOG_EXT, 10), 10);
    }

    #[test]
    fn make_log_file_name_appends_split_index() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let prefix = make_log_file_name_prefix(now_secs(), "Mars");
        touch(&dir.join(format!("{prefix}.xlog")), &[0u8; 4096]);

        let p = make_log_file_name(now_secs(), dir, dir, "Mars", LOG_EXT, 1024, None);
        let name = p.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, format!("{prefix}_1.xlog"));
    }

    #[test]
    fn get_file_paths_from_timeval_lists_existing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let tv = now_secs();
        let prefix = make_log_file_name_prefix(tv, "Mars");
        touch(&dir.join(format!("{prefix}.xlog")), b"a");
        touch(&dir.join(format!("{prefix}_1.xlog")), b"a");
        touch(&dir.join("other.xlog"), b"a");
        // A file from another day.
        touch(&dir.join("Mars_19700101.xlog"), b"a");

        let mut paths = get_file_paths_from_timeval(tv, dir, "Mars", LOG_EXT);
        paths.sort();
        assert_eq!(paths.len(), 2, "{paths:?}");
        assert!(paths.iter().all(|p| p
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(&prefix)));
    }

    #[test]
    fn append_file_rolls_back_on_partial_write() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.xlog");
        let dst = tmp.path().join("dst.xlog");
        touch(&src, &[b'a'; 100]);
        touch(&dst, &[b'b'; 10]);

        assert!(append_file(&src, &dst));
        let mut out = Vec::new();
        File::open(&dst).unwrap().read_to_end(&mut out).unwrap();
        assert_eq!(out.len(), 110);
        assert_eq!(&out[..10], &[b'b'; 10]);
        assert_eq!(&out[10..], &[b'a'; 100]);

        // Same file -> false; missing source -> false.
        assert!(!append_file(&src, &src));
        assert!(!append_file(&tmp.path().join("nope"), &dst));
    }

    #[test]
    fn del_timeout_file_only_removes_stale_xlog_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let fresh = dir.join("Mars_20240101.xlog");
        let stale = dir.join("Mars_20240102.xlog");
        let keep = dir.join("Mars_20240102.txt");
        let no_dot = dir.join("Mars_20240102xlog");
        touch(&fresh, b"a");
        touch(&stale, b"a");
        touch(&keep, b"a");
        touch(&no_dot, b"a");

        // Pretend `stale` was modified 2 days ago (and `keep` too).
        let old = SystemTime::now() - std::time::Duration::from_secs(2 * 24 * 3600);
        for p in [&stale, &keep, &no_dot] {
            let f = File::options().write(true).open(p).unwrap();
            f.set_modified(old).unwrap();
        }

        del_timeout_file(dir, 24 * 3600, "Mars");
        assert!(fresh.exists());
        assert!(!stale.exists());
        assert!(keep.exists());
        // `ends_with("xlog")` would have taken this one: a name that ends in
        // the extension's four letters without the dot is not a log file.
        assert!(no_dot.exists());
    }

    /// The prefix the sweep is given is a prefix of another prefix, and the
    /// two `Xlog`s share one `logdir`: `Mars` is a prefix of `Mars_push`, and
    /// `Mars_push_20240101.xlog` is a log of another appender on another
    /// schedule. Matching `starts_with(prefix)` deleted it here.
    #[test]
    fn the_expiry_sweep_leaves_the_logs_of_a_prefix_this_one_is_a_prefix_of_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mine = dir.join("Mars_20240101.xlog");
        let theirs = dir.join("Mars_push_20240101.xlog");
        touch(&mine, b"a");
        touch(&theirs, b"a");

        del_timeout_file(dir, -1, "Mars");

        assert!(!mine.exists());
        assert!(
            theirs.exists(),
            "the logs of a prefix this one is a prefix of are not this sweep's to delete"
        );
    }

    /// One `logdir` is what two `Xlog`s with two prefixes share, and a sweep
    /// that removed every `.xlog` in it deleted the other one's logs — on an
    /// expiry that is not its own. (`move_old_files` has always matched the
    /// prefix; this is the sweep catching up with it.)
    #[test]
    fn the_expiry_sweep_leaves_the_log_files_of_another_prefix_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mine = dir.join("Mars_20240101.xlog");
        let theirs = dir.join("Other_20240101.xlog");
        touch(&mine, b"a");
        touch(&theirs, b"a");

        // A negative alive time is "everything is expired": no file was
        // modified less than zero seconds ago.
        del_timeout_file(dir, -1, "Mars");

        assert!(!mine.exists());
        assert!(
            theirs.exists(),
            "the logs of another prefix are not this sweep's to delete"
        );
    }

    #[test]
    fn move_old_files_appends_and_removes() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let log = tmp.path().join("log");
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&log).unwrap();
        touch(&cache.join("Mars_20240101.xlog"), b"cached");
        touch(&cache.join("Other_20240101.xlog"), b"nope");
        touch(&cache.join("Mars_push_20240101.xlog"), b"nope");

        move_old_files(&cache, &log, "Mars", 0);

        let mut out = Vec::new();
        File::open(log.join("Mars_20240101.xlog"))
            .unwrap()
            .read_to_end(&mut out)
            .unwrap();
        assert_eq!(out, b"cached");
        assert!(!cache.join("Mars_20240101.xlog").exists());
        assert!(cache.join("Other_20240101.xlog").exists());
        // A prefix of which this one is a prefix: another appender's cached
        // records, and not one to file under a name they were not written
        // under.
        assert!(cache.join("Mars_push_20240101.xlog").exists());
        assert!(!log.join("Mars_push_20240101.xlog").exists());
    }
}
