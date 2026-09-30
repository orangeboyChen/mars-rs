//! Drives the whole C ABI from Rust, exactly the way the C++/JNI layer does:
//! open into a temp dir, write a few records, flush, close, then read the
//! resulting `.xlog` file back and check the bodies really landed on disk.
//!
//! The instance registry is process-wide, so every test takes [`LOCK`]:
//! `cargo test` runs the cases in this file on parallel threads and they would
//! otherwise race over the one prefix they share.

use std::ffi::CString;
use std::fs;
use std::io::Read;
use std::os::raw::{c_int, c_longlong};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use mars_ffi::abi::{mars_xlog_new_instance, mars_xlog_release_instance};
use mars_ffi::{
    mars_xlog_flush_now_instance, mars_xlog_request_flush_instance, mars_xlog_set_level_instance,
    mars_xlog_write_instance, MarsXLogConfig,
};

/// Closes the appender when the test ends, even if it failed.
///
/// The instance registry is process-wide and a prefix is one appender to it,
/// so a test that panics mid-way would otherwise poison every test that runs
/// after it.
struct CloseOnDrop;
impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        close();
    }
}

/// The prefix every `Config` below is built with, and the one `close` releases.
const PREFIX: &str = "ffi_smoke";

/// Releases the instance the tests open. It takes the prefix and not the
/// handle, because a prefix is one appender: two opens of one prefix share it,
/// and releasing either closes it.
fn close() {
    let prefix = CString::new(PREFIX).unwrap();
    unsafe {
        mars_xlog_release_instance(prefix.as_ptr());
    }
}

/// Serialises the singleton-mutating tests inside this binary.
fn lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    // A poisoned mutex only means an earlier assertion failed; the appender
    // state is still usable, so recover instead of cascading the panic.
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Owns the `CString`s a `MarsXLogConfig` borrows, so callers cannot get the
/// lifetime wrong.
struct Config {
    log_dir: CString,
    name_prefix: CString,
    cache_dir: CString,
    raw: MarsXLogConfig,
}

impl Config {
    fn new(dir: &Path, mode: c_int, compress_mode: c_int) -> Self {
        let log_dir = CString::new(dir.to_str().unwrap()).unwrap();
        let name_prefix = CString::new(PREFIX).unwrap();
        let cache_dir = CString::new("").unwrap();
        let raw = MarsXLogConfig {
            mode,
            log_dir: log_dir.as_ptr(),
            name_prefix: name_prefix.as_ptr(),
            pub_key: std::ptr::null(),
            compress_mode,
            compress_level: 0,
            cache_dir: cache_dir.as_ptr(),
            cache_days: 0,
        };
        Self {
            log_dir,
            name_prefix,
            cache_dir,
            raw,
        }
    }

    /// Reads the owned strings. They are only ever reached through the raw
    /// pointers in `raw`, so this exists to keep them provably alive (and to
    /// document why the fields are stored at all).
    fn keep_alive(&self) -> usize {
        self.log_dir.as_bytes().len()
            + self.name_prefix.as_bytes().len()
            + self.cache_dir.as_bytes().len()
    }

    fn as_ptr(&self) -> *const MarsXLogConfig {
        debug_assert!(self.keep_alive() > 0);
        &self.raw as *const MarsXLogConfig
    }
}

/// Opens an instance in sync mode (deterministic: no writer thread) with zlib,
/// and answers its handle.
fn open_sync(dir: &Path) -> c_longlong {
    fs::create_dir_all(dir).unwrap();
    let cfg = Config::new(dir, 1, 0);
    let handle = unsafe { mars_xlog_new_instance(cfg.as_ptr(), 0) };
    assert_ne!(handle, 0, "mars_xlog_new_instance failed");
    // A prefix is one appender, and its level outlives the test that set it.
    mars_xlog_set_level_instance(handle, 0);
    handle
}

fn write(handle: c_longlong, level: c_int, tag: &str, message: &str) {
    let tag = CString::new(tag).unwrap();
    let file = CString::new("ffi_smoke.rs").unwrap();
    let func = CString::new("write").unwrap();
    let message = CString::new(message).unwrap();
    unsafe {
        mars_xlog_write_instance(
            handle,
            level,
            tag.as_ptr(),
            file.as_ptr(),
            func.as_ptr(),
            42,
            message.as_ptr(),
        );
    }
}

/// The `.xlog` file the appender produced in `dir` (the exact day-stamped name
/// is an implementation detail of the appender).
fn log_file(dir: &Path) -> PathBuf {
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "xlog"))
        .collect();
    found.sort();
    assert!(
        !found.is_empty(),
        "no .xlog file was created in {}",
        dir.display()
    );
    found.pop().unwrap()
}

/// Every plausible plain-text view of an `.xlog` file: the raw bytes (if a
/// build wrote uncompressed) plus the inflated body of each framed record.
///
/// Mars always compresses (`appender.cc` constructs `LogZlibBuffer` with
/// `is_compress = true`), so the interesting view is the inflated one.
fn plaintext_views(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut views = vec![bytes.to_vec()];
    // Whole-file raw-deflate decode.
    if let Some(d) = inflate_raw(bytes) {
        views.push(d);
    }
    // Per-record walk: 73-byte header (len at offset 5) + body + 1-byte tailer.
    let mut pos = 0usize;
    let mut inflated: Vec<u8> = Vec::new();
    while pos + 73 <= bytes.len() {
        let len = u32::from_le_bytes([
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
            bytes[pos + 8],
        ]) as usize;
        let body = &bytes[pos + 73..(pos + 73 + len).min(bytes.len())];
        if let Some(d) = inflate_raw(body) {
            inflated.extend_from_slice(&d);
        } else {
            break;
        }
        pos += 73 + len + 1;
    }
    if !inflated.is_empty() {
        views.push(inflated);
    }
    views
}

/// Raw DEFLATE (no zlib header), which is what `deflateInit2(..., -MAX_WBITS)`
/// produces.
fn inflate_raw(data: &[u8]) -> Option<Vec<u8>> {
    // A `Z_SYNC_FLUSH` stream is deliberately *not* terminated, so `read_to_end`
    // always finishes on an "unexpected eof" error. Keep whatever was decoded
    // before that, exactly like `marsrs-appender`'s own tests do.
    let mut out = Vec::new();
    let _ = flate2::bufread::DeflateDecoder::new(std::io::Cursor::new(data)).read_to_end(&mut out);
    (!out.is_empty()).then_some(out)
}

fn any_view_contains(bytes: &[u8], needle: &str) -> bool {
    plaintext_views(bytes)
        .iter()
        .any(|v| contains(v, needle.as_bytes()))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// ---------------------------------------------------------------------------

#[test]
fn open_write_flush_close_lands_on_disk() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();

    let log = open_sync(dir.path());
    write(log, 2, "smoke", "hello-from-the-c-abi");
    write(log, 4, "smoke", "second-record-42");
    mars_xlog_request_flush_instance(log);
    mars_xlog_flush_now_instance(log);

    let path = log_file(dir.path());
    let bytes = fs::read(&path).unwrap();
    assert!(!bytes.is_empty(), "log file is empty");
    assert!(
        any_view_contains(&bytes, "hello-from-the-c-abi"),
        "first record missing from {path:?}"
    );
    assert!(
        any_view_contains(&bytes, "second-record-42"),
        "second record missing from {path:?}"
    );
    // The formatted record carries the level and the source location, like
    // `mars::xlog::log_formater` does.
    assert!(
        any_view_contains(&bytes, "ffi_smoke.rs:42"),
        "source location missing from {path:?}"
    );

    close();
}

#[test]
fn the_level_one_function_sets_is_the_one_every_write_sees() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();
    let log = open_sync(dir.path());

    // One store, whatever the question is: the level a setter on the handle
    // writes is the one `get_level` and `is_enabled_for` read back.
    mars_xlog_set_level_instance(log, 4); // Error
    assert_eq!(mars_ffi::abi::mars_xlog_get_level(log), 4);

    // `XloggerWrite(0, …)` — the C++'s `xlogger_Write`, which `xloggerbase.h`
    // annotates "no level filter" — is not reachable from C any more: it is
    // the process-wide appender's write, and no C symbol opens that one. What
    // is left of the property is the half an app sees, which is the next
    // assertion: the level is the store `is_enabled_for` answers from.

    // `MARS_LEVEL_NONE` through the instance path, which used to be ignored.
    mars_ffi::abi::mars_xlog_set_level_instance(log, 6);
    write(log, 5, "smoke", "a-level-none-filter-drops-everything");
    mars_xlog_flush_now_instance(log);
    let bytes = fs::read(log_file(dir.path())).unwrap();
    assert!(!any_view_contains(
        &bytes,
        "a-level-none-filter-drops-everything"
    ));

    mars_xlog_set_level_instance(log, 0); // back to Verbose
}

#[test]
fn level_filter_gates_writes() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();
    let log = open_sync(dir.path());

    mars_xlog_set_level_instance(log, 4); // Error
    write(log, 2, "smoke", "this-info-record-must-be-dropped");
    write(log, 4, "smoke", "this-error-record-must-survive");
    mars_xlog_flush_now_instance(log);

    let bytes = fs::read(log_file(dir.path())).unwrap();
    assert!(any_view_contains(&bytes, "this-error-record-must-survive"));
    assert!(
        !any_view_contains(&bytes, "this-info-record-must-be-dropped"),
        "level filter did not drop the record"
    );

    mars_xlog_set_level_instance(log, 0); // back to Verbose
    write(log, 1, "smoke", "verbose-again-after-reset");
    mars_xlog_flush_now_instance(log);
    let bytes = fs::read(log_file(dir.path())).unwrap();
    assert!(any_view_contains(&bytes, "verbose-again-after-reset"));

    close();
}

#[test]
fn async_mode_also_writes() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();
    // Pre-create the directory: the async writer thread must not race the
    // appender's own `create_dir_all`.
    fs::create_dir_all(dir.path()).unwrap();
    let cfg = Config::new(dir.path(), 0, 0); // Async + Zlib
    let log = unsafe { mars_xlog_new_instance(cfg.as_ptr(), 0) };
    assert_ne!(log, 0);
    mars_xlog_set_level_instance(log, 0);
    write(log, 3, "smoke", "async-mode-record");

    // Async mode hands the record to the writer thread, so poll a little:
    // `flush_now` only guarantees the thread has been signalled.
    let mut bytes = Vec::new();
    for _ in 0..20 {
        mars_xlog_flush_now_instance(log);
        bytes = fs::read(log_file(dir.path())).unwrap_or_default();
        if any_view_contains(&bytes, "async-mode-record") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        any_view_contains(&bytes, "async-mode-record"),
        "the async record never reached {}",
        dir.path().display()
    );
    close();
}

#[test]
fn zstd_mode_is_accepted() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config::new(dir.path(), 1, 1); // Sync + Zstd
    let log = unsafe { mars_xlog_new_instance(cfg.as_ptr(), 0) };
    assert_ne!(log, 0);
    mars_xlog_set_level_instance(log, 0);
    write(log, 2, "smoke", "zstd-mode-record");
    mars_xlog_flush_now_instance(log);
    // Sync mode stores the payload verbatim, so the record must be readable as
    // text: "the file is not empty" would also pass if compress_mode were
    // ignored or the body were garbage.
    let bytes = fs::read(log_file(dir.path())).unwrap();
    assert!(
        any_view_contains(&bytes, "zstd-mode-record"),
        "the record is not in the log: {} bytes",
        bytes.len()
    );
    close();
}

#[test]
fn appender_error_is_reported_not_panicked() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();
    // A prefix that cannot be turned into a file name must surface as a
    // negative code; with a permissive appender it simply succeeds.
    let bad = CString::new("").unwrap();
    let dir_c = CString::new(dir.path().to_str().unwrap()).unwrap();
    let cfg = MarsXLogConfig {
        mode: 1,
        log_dir: dir_c.as_ptr(),
        name_prefix: bad.as_ptr(),
        pub_key: std::ptr::null(),
        compress_mode: 0,
        compress_level: 0,
        cache_dir: std::ptr::null(),
        cache_days: 0,
    };
    // An instance is registered *under* its prefix, so an empty one has no
    // name to be registered under and gets no handle at all. The appender
    // itself is never asked, which is what the old `OK || ERR_APPENDER`
    // assertion used to leave open.
    assert_eq!(unsafe { mars_xlog_new_instance(&cfg, 0) }, 0);
    close();
}
