//! Integration tests for the process-wide singleton API of
//! `marsrs-appender` (the `appender_*` free functions).
//!
//! The C++ keeps `sg_default_appender` in file scope, so there is exactly one
//! appender per process: the tests here serialise themselves with a mutex and
//! always `appender_close()` before the next one starts.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::task::{Context, Waker};

use marsrs_appender::{
    appender_close, appender_flush, appender_flush_now, appender_get_current_log_cache_path,
    appender_get_current_log_path, appender_getfilepath_from_timespan, appender_make_logfile_name,
    appender_oneshot_flush, appender_open, appender_set_console_log,
    appender_set_max_alive_duration, appender_set_max_file_size, appender_set_mode,
    appender_signal_flush, appender_write, AppenderMode, FileIoAction, LogLevel, XLogConfig,
    XLoggerInfo,
};
use marsrs_crypt::{magic, LogCrypt, HEADER_LEN, TAILER_LEN};

/// Serialises the tests that touch the process-wide singleton.
fn singleton() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn config(dir: &Path, mode: AppenderMode) -> XLogConfig {
    XLogConfig {
        mode,
        logdir: dir.to_path_buf(),
        nameprefix: "Mars".to_owned(),
        ..Default::default()
    }
}

fn info(level: LogLevel) -> XLoggerInfo<'static> {
    XLoggerInfo {
        level,
        tag: Some("test".into()),
        filename: Some("singleton.rs".into()),
        ..Default::default()
    }
}

/// Splits a `.xlog` file into `[header][payload][tailer]` records, asserting
/// the framing of every one of them.
fn records(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + HEADER_LEN + TAILER_LEN <= bytes.len() {
        assert!(
            magic::magic_start_is_valid(bytes[pos]),
            "bad magic {:#04x} at {pos}",
            bytes[pos]
        );
        let len = LogCrypt::get_log_len(&bytes[pos..]) as usize;
        let end = pos + HEADER_LEN + len;
        assert!(end + TAILER_LEN <= bytes.len(), "truncated record at {pos}");
        assert_eq!(bytes[end], magic::END, "bad tailer at {end}");
        out.push(&bytes[pos + HEADER_LEN..end]);
        pos = end + TAILER_LEN;
    }
    assert_eq!(pos, bytes.len(), "{} trailing bytes", bytes.len() - pos);
    out
}

/// Raw payloads plus, when a payload is a DEFLATE stream, its inflated form.
fn payload_text(bytes: &[u8]) -> String {
    let mut text = String::new();
    for body in records(bytes) {
        text.push_str(&String::from_utf8_lossy(body));
        use std::io::Read;
        let mut out = Vec::new();
        let mut decoder = flate2::bufread::DeflateDecoder::new(std::io::Cursor::new(body));
        // A `Z_SYNC_FLUSH` stream is never terminated, so `read_to_end`
        // reports `UnexpectedEof` after emitting everything it could.
        let _ = decoder.read_to_end(&mut out);
        if !out.is_empty() {
            text.push('\n');
            text.push_str(&String::from_utf8_lossy(&out));
        }
    }
    text
}

fn today_log_file(dir: &Path) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d");
    dir.join(format!("Mars_{stamp}.xlog"))
}

#[test]
fn open_write_flush_close_roundtrip() {
    let _guard = singleton();
    let tmp = tempfile::tempdir().unwrap();

    appender_open(config(tmp.path(), AppenderMode::Sync)).unwrap();
    assert_eq!(appender_get_current_log_path().as_deref(), Some(tmp.path()));
    assert!(appender_get_current_log_cache_path().is_none());

    assert!(appender_write(
        Some(&info(LogLevel::Info)),
        "singleton roundtrip"
    ));
    appender_signal_flush();
    appender_flush_now();
    appender_close();

    // After `close()` the appender is gone again.
    assert!(appender_get_current_log_path().is_none());
    assert!(!appender_write(Some(&info(LogLevel::Info)), "dropped"));

    let path = today_log_file(tmp.path());
    assert!(path.exists(), "{path:?} was not created");
    let bytes = std::fs::read(&path).unwrap();
    assert!(payload_text(&bytes).contains("singleton roundtrip"));
}

#[test]
fn async_mode_writes_through_the_writer_thread() {
    let _guard = singleton();
    let tmp = tempfile::tempdir().unwrap();

    appender_open(config(tmp.path(), AppenderMode::Async)).unwrap();
    for i in 0..32 {
        assert!(appender_write(
            Some(&info(LogLevel::Debug)),
            &format!("async line {i}")
        ));
    }
    appender_flush_now();
    appender_close();

    let bytes = std::fs::read(today_log_file(tmp.path())).unwrap();
    let text = payload_text(&bytes);
    assert!(text.contains("async line 0"), "{text}");
    assert!(text.contains("async line 31"), "{text}");
}

/// Polls `future` here until it answers, or gives up: the port carries no
/// runtime, so there is no `block_on` to borrow one from, and a test that
/// hangs is worse than one that fails.
///
/// Bounded on purpose — the drain behind this is one lock, one write and one
/// `fflush`, so a million yields is a generous answer and not a wait.
fn poll(future: impl Future<Output = ()>) {
    let mut future = std::pin::pin!(future);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    for _ in 0..1_000_000 {
        if future.as_mut().poll(&mut cx).is_ready() {
            return;
        }
        std::thread::yield_now();
    }
    panic!("the flush never answered");
}

#[test]
fn awaited_flush_drains_off_the_calling_thread() {
    let _guard = singleton();
    let tmp = tempfile::tempdir().unwrap();

    appender_open(config(tmp.path(), AppenderMode::Async)).unwrap();
    assert!(appender_write(Some(&info(LogLevel::Info)), "awaited flush"));

    // Pending at least once, which is the whole point of the call: the drain
    // is another thread's, and this one is not held while it runs.
    let mut future = std::pin::pin!(appender_flush());
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    assert!(future.as_mut().poll(&mut cx).is_pending());
    for _ in 0..1_000_000 {
        if future.as_mut().poll(&mut cx).is_ready() {
            break;
        }
        std::thread::yield_now();
    }

    // Dropped and never polled: the drain never runs, and nothing panics.
    drop(appender_flush());

    let bytes = std::fs::read(today_log_file(tmp.path())).unwrap();
    assert!(
        payload_text(&bytes).contains("awaited flush"),
        "{}",
        payload_text(&bytes)
    );

    // And the call the Rust API gives a caller with no executor: the same
    // drain, waited for on this thread.
    assert!(appender_write(Some(&info(LogLevel::Info)), "second record"));
    poll(appender_flush());
    appender_close();

    let bytes = std::fs::read(today_log_file(tmp.path())).unwrap();
    let text = payload_text(&bytes);
    assert!(text.contains("second record"), "{text}");
}

#[test]
fn mode_can_be_switched_on_a_live_appender() {
    let _guard = singleton();
    let tmp = tempfile::tempdir().unwrap();

    appender_open(config(tmp.path(), AppenderMode::Async)).unwrap();
    assert!(appender_write(
        Some(&info(LogLevel::Warn)),
        "written while async"
    ));
    appender_set_mode(AppenderMode::Sync);
    assert!(appender_write(
        Some(&info(LogLevel::Warn)),
        "written while sync"
    ));
    appender_close();

    let text = payload_text(&std::fs::read(today_log_file(tmp.path())).unwrap());
    assert!(text.contains("written while async"), "{text}");
    assert!(text.contains("written while sync"), "{text}");
}

#[test]
fn opening_twice_is_an_error() {
    let _guard = singleton();
    let tmp = tempfile::tempdir().unwrap();

    appender_open(config(tmp.path(), AppenderMode::Sync)).unwrap();
    let err = appender_open(config(tmp.path(), AppenderMode::Sync)).unwrap_err();
    assert!(err.to_string().contains("already been opened"), "{err}");
    appender_close();

    // ... and works again after `close()`.
    appender_open(config(tmp.path(), AppenderMode::Sync)).unwrap();
    appender_close();

    // An empty logdir is rejected instead of silently doing nothing.
    let mut bad = config(tmp.path(), AppenderMode::Sync);
    bad.logdir = PathBuf::new();
    assert!(appender_open(bad).is_err());
}

#[test]
fn setters_before_open_are_applied() {
    let _guard = singleton();
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");

    appender_set_max_file_size(0);
    appender_set_max_alive_duration(3 * 24 * 60 * 60);
    appender_set_console_log(false);

    let mut cfg = config(tmp.path(), AppenderMode::Sync);
    cfg.cachedir = Some(cache.clone());
    appender_open(cfg).unwrap();

    assert_eq!(
        appender_get_current_log_cache_path().as_deref(),
        Some(cache.as_path())
    );
    let paths = appender_make_logfile_name(0, "Mars", tmp.path());
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], today_log_file(tmp.path()));

    appender_close();
    appender_set_max_file_size(0);
    appender_set_max_alive_duration(0);
}

#[test]
fn getfilepath_from_timespan_finds_todays_file() {
    let _guard = singleton();
    let tmp = tempfile::tempdir().unwrap();

    appender_open(config(tmp.path(), AppenderMode::Sync)).unwrap();
    assert!(appender_write(
        Some(&info(LogLevel::Info)),
        "make the file exist"
    ));
    appender_close();

    let found = appender_getfilepath_from_timespan(0, "Mars", tmp.path());
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0], today_log_file(tmp.path()));
    assert!(appender_getfilepath_from_timespan(3, "Mars", tmp.path()).is_empty());
}

#[test]
fn oneshot_flush_drains_a_foreign_cache_file() {
    use marsrs_buffer::{CompressMode, LogBuffer};

    let _guard = singleton();
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path(), AppenderMode::Sync);

    // Nothing to flush yet.
    assert_eq!(appender_oneshot_flush(&cfg), FileIoAction::Unnecessary);

    // Produce a cache file the way a crashed process would have left it: a
    // mmap region with one record in it.
    let mut region = vec![0u8; 150 * 1024];
    let mut buffer = LogBuffer::new(true, None, CompressMode::Zlib, 6);
    buffer.attach(&mut region);
    assert!(buffer.write(&mut region, b"recovered from the cache file"));
    let mmap_path = tmp.path().join("Mars.mmap3");
    std::fs::write(&mmap_path, &region).unwrap();

    assert_eq!(appender_oneshot_flush(&cfg), FileIoAction::Success);
    assert!(!mmap_path.exists(), "the cache file must be removed");

    let bytes = std::fs::read(today_log_file(tmp.path())).unwrap();
    let text = payload_text(&bytes);
    assert!(text.contains("recovered from the cache file"), "{text}");
    assert!(text.contains("begin of mmap from other process"), "{text}");
}

/// A process that was killed leaves the records it wrote in the cache file it
/// mmap'd, and the next `appender_open` is what takes them to the log file —
/// the region is the kernel's, so nothing of a killed process is lost, it is
/// only not in a `.xlog` yet. That is the whole reason an app does not have to
/// flush on its way out.
/// Sync mode is the one mode a process that is killed loses something in: there
/// is no cache file behind its records, so the tail the process is still holding
/// dies with it. `close()` is what hands it to the file — which is why the docs
/// ask a synchronous app for a `close` or a `flush`, and an asynchronous one for
/// nothing at all.
#[test]
fn sync_mode_holds_the_tail_until_it_is_handed_over() {
    let _guard = singleton();
    let tmp = tempfile::tempdir().unwrap();

    appender_open(config(tmp.path(), AppenderMode::Sync)).unwrap();
    appender_write(None, "the tail of a session that was killed");

    // Nothing has been handed to the OS yet, and there is no cache file behind
    // it: a process that dies here takes the record with it.
    let held = std::fs::read(today_log_file(tmp.path())).unwrap_or_default();
    assert!(
        !payload_text(&held).contains("the tail of a session"),
        "{held:?}"
    );

    appender_close();

    let bytes = std::fs::read(today_log_file(tmp.path())).unwrap();
    let text = payload_text(&bytes);
    assert!(text.contains("the tail of a session"), "{text}");
}

#[test]
fn open_drains_the_cache_a_killed_process_left() {
    use marsrs_buffer::{CompressMode, LogBuffer};

    let _guard = singleton();
    let tmp = tempfile::tempdir().unwrap();

    // The cache file the way a killed process left it: a region with one record
    // in it, and no writer holding it.
    let mut region = vec![0u8; 150 * 1024];
    let mut buffer = LogBuffer::new(true, None, CompressMode::Zlib, 6);
    buffer.attach(&mut region);
    assert!(buffer.write(&mut region, b"the last record of a killed process"));
    std::fs::write(tmp.path().join("Mars.mmap3"), &region).unwrap();

    // A start, and nothing else: no `flush`, and no call that names the cache.
    appender_open(config(tmp.path(), AppenderMode::Async)).unwrap();
    appender_close();

    let bytes = std::fs::read(today_log_file(tmp.path())).unwrap();
    let text = payload_text(&bytes);
    assert!(
        text.contains("the last record of a killed process"),
        "{text}"
    );
    // The banner `Open` brackets the drained records with.
    assert!(text.contains("begin of mmap"), "{text}");
}

/// `xlogger_dump`: the dump that leaves a file in the log directory behind, so
/// the blob is still there after the process is gone.
#[test]
fn dump_writes_a_file_and_reports_it() {
    let _guard = singleton();
    let tmp = tempfile::tempdir().unwrap();
    appender_open(config(tmp.path(), AppenderMode::Sync)).unwrap();

    let report = marsrs_appender::xlogger_dump(b"payload of the dump");
    assert!(report.contains(".dump :\n"), "{report}");

    let path = report
        .split(" dump file to ")
        .nth(1)
        .unwrap()
        .split(" :\n")
        .next()
        .unwrap();
    let path = PathBuf::from(path);
    assert!(
        path.starts_with(tmp.path()),
        "the dump left the log directory: {path:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"payload of the dump");

    appender_close();

    // No appender: the C++ `sg_release_guard` case, an empty report.
    assert_eq!(marsrs_appender::xlogger_dump(b"nowhere to go"), "");
}
