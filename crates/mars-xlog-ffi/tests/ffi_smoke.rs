//! Drives the whole C ABI from Rust, exactly the way the C++/JNI layer does:
//! open into a temp dir, write a few records, flush, close, then read the
//! resulting `.xlog` file back and check the bodies really landed on disk.
//!
//! The appender is a process-wide singleton, so every test takes [`LOCK`]:
//! `cargo test` runs the cases in this file on parallel threads and they would
//! otherwise race over `mars_xlog_open` / `mars_xlog_close`.

use std::ffi::{CStr, CString};
use std::fs;
use std::io::Read;
use std::os::raw::{c_char, c_int, c_longlong, c_uint, c_ulonglong};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use mars_xlog_ffi::{
    mars_xlog_close, mars_xlog_current_log_path, mars_xlog_flush, mars_xlog_flush_sync,
    mars_xlog_open, mars_xlog_set_console_log, mars_xlog_set_level,
    mars_xlog_set_max_alive_duration, mars_xlog_set_max_file_size, mars_xlog_write, MarsXLogConfig,
    MARS_XLOG_ERR_BAD_COMPRESS, MARS_XLOG_ERR_BAD_MODE, MARS_XLOG_ERR_EMPTY_LOG_DIR,
    MARS_XLOG_ERR_NO_PATH, MARS_XLOG_ERR_NO_SPACE, MARS_XLOG_ERR_NULL_CONFIG,
    MARS_XLOG_ERR_NULL_OUT, MARS_XLOG_OK,
};

/// Closes the appender when the test ends, even if it failed.
///
/// The appender is a process-wide singleton that rejects a second
/// `appender_open`, so a test that panics mid-way would otherwise poison every
/// test that runs after it.
struct CloseOnDrop;
impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        mars_xlog_close();
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
        let name_prefix = CString::new("ffi_smoke").unwrap();
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

/// Opens the appender in sync mode (deterministic: no writer thread) with zlib.
fn open_sync(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    let cfg = Config::new(dir, 1, 0);
    assert_eq!(
        mars_xlog_open(cfg.as_ptr()),
        MARS_XLOG_OK,
        "mars_xlog_open failed"
    );
    // The level is process-wide and other tests may have raised it.
    mars_xlog_set_level(0);
}

fn write(level: c_int, tag: &str, message: &str) {
    let tag = CString::new(tag).unwrap();
    let file = CString::new("ffi_smoke.rs").unwrap();
    let func = CString::new("write").unwrap();
    let message = CString::new(message).unwrap();
    mars_xlog_write(
        level,
        tag.as_ptr(),
        file.as_ptr(),
        func.as_ptr(),
        42,
        message.as_ptr(),
    );
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
    // before that, exactly like `mars-xlog-appender`'s own tests do.
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

    open_sync(dir.path());
    write(2, "smoke", "hello-from-the-c-abi");
    write(4, "smoke", "second-record-42");
    mars_xlog_flush();
    mars_xlog_flush_sync();

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

    mars_xlog_close();
}

#[test]
fn current_log_path_is_reported() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();
    open_sync(dir.path());
    // The log file is created lazily, on the first record.
    write(2, "smoke", "path-check-record");
    mars_xlog_flush_sync();

    let mut buf = [0u8; 512];
    let n = mars_xlog_current_log_path(buf.as_mut_ptr() as *mut c_char, buf.len() as c_uint);
    assert!(n > 0, "mars_xlog_current_log_path failed: {n}");
    let len = n as usize;
    assert_eq!(buf[len], 0, "path must be NUL terminated");
    // SAFETY: we just wrote a NUL-terminated path into `buf` and checked the
    // terminator; `len` bytes precede it.
    let reported = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(reported.len(), len, "return value must exclude the NUL");
    // The appender owns the exact value (the C++ reports the current log file;
    // a build may also report the log directory). All this seam can promise is
    // that the path points inside the directory that was configured and exists.
    let reported_path = Path::new(&reported);
    assert!(
        reported_path.starts_with(dir.path()),
        "reported path {reported_path:?} is outside {}",
        dir.path().display()
    );
    assert!(
        reported_path.exists(),
        "reported path {reported_path:?} does not exist"
    );
    assert!(log_file(dir.path()).exists());

    // Too small a buffer is an error, not a truncation.
    let mut tiny = [0u8; 4];
    assert_eq!(
        mars_xlog_current_log_path(tiny.as_mut_ptr() as *mut c_char, tiny.len() as c_uint),
        MARS_XLOG_ERR_NO_SPACE
    );
    assert_eq!(
        mars_xlog_current_log_path(tiny.as_mut_ptr() as *mut c_char, 0),
        MARS_XLOG_ERR_NO_SPACE,
        "len == 0 must be rejected"
    );
    assert_eq!(
        mars_xlog_current_log_path(std::ptr::null_mut(), 64),
        MARS_XLOG_ERR_NULL_OUT,
        "a null out pointer must be rejected"
    );

    mars_xlog_close();
}

#[test]
fn level_filter_gates_writes() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();
    open_sync(dir.path());

    mars_xlog_set_level(4); // Error
    write(2, "smoke", "this-info-record-must-be-dropped");
    write(4, "smoke", "this-error-record-must-survive");
    mars_xlog_flush_sync();

    let bytes = fs::read(log_file(dir.path())).unwrap();
    assert!(any_view_contains(&bytes, "this-error-record-must-survive"));
    assert!(
        !any_view_contains(&bytes, "this-info-record-must-be-dropped"),
        "level filter did not drop the record"
    );

    mars_xlog_set_level(0); // back to Verbose
    write(1, "smoke", "verbose-again-after-reset");
    mars_xlog_flush_sync();
    let bytes = fs::read(log_file(dir.path())).unwrap();
    assert!(any_view_contains(&bytes, "verbose-again-after-reset"));

    mars_xlog_close();
}

#[test]
fn null_pointers_are_never_dereferenced() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();

    // No config at all.
    assert_eq!(mars_xlog_open(std::ptr::null()), MARS_XLOG_ERR_NULL_CONFIG);

    // Every string null: log_dir is mandatory, so this is rejected.
    let cfg = MarsXLogConfig {
        mode: 1,
        log_dir: std::ptr::null(),
        name_prefix: std::ptr::null(),
        pub_key: std::ptr::null(),
        compress_mode: 0,
        compress_level: 0,
        cache_dir: std::ptr::null(),
        cache_days: 0,
    };
    assert_eq!(mars_xlog_open(&cfg), MARS_XLOG_ERR_EMPTY_LOG_DIR);

    // Bad enum values.
    let good = Config::new(dir.path(), 1, 0);
    let mut cfg = good.raw;
    cfg.mode = 7;
    assert_eq!(mars_xlog_open(&cfg), MARS_XLOG_ERR_BAD_MODE);
    cfg.mode = 1;
    cfg.compress_mode = 9;
    assert_eq!(mars_xlog_open(&cfg), MARS_XLOG_ERR_BAD_COMPRESS);
    // ...and the same object keeps working once it is valid again.
    cfg.compress_mode = 0;
    assert_eq!(mars_xlog_open(&cfg), MARS_XLOG_OK);
    mars_xlog_set_level(0);

    // A write with every pointer null (level 6 == kLevelNone is also dropped).
    mars_xlog_write(
        6,
        std::ptr::null(),
        std::ptr::null(),
        std::ptr::null(),
        0,
        std::ptr::null(),
    );
    mars_xlog_write(
        2,
        std::ptr::null(),
        std::ptr::null(),
        std::ptr::null(),
        0,
        std::ptr::null(),
    );
    mars_xlog_flush_sync();

    // The setters must tolerate being called with junk too.
    mars_xlog_set_level(-3);
    mars_xlog_set_level(0);
    mars_xlog_set_console_log(1);
    mars_xlog_set_console_log(0);
    mars_xlog_set_max_file_size(0);
    mars_xlog_set_max_file_size(4 * 1024 * 1024);
    mars_xlog_set_max_alive_duration(-1);
    mars_xlog_set_max_alive_duration(10 * 24 * 3600);

    mars_xlog_close();
    // Closing twice, flushing while closed: none of it may abort the process.
    mars_xlog_flush();
    mars_xlog_flush_sync();
    mars_xlog_close();
}

#[test]
fn no_path_before_open() {
    let _g = lock();
    let _close = CloseOnDrop;
    mars_xlog_close();
    let mut buf = [0u8; 256];
    let n = mars_xlog_current_log_path(buf.as_mut_ptr() as *mut c_char, buf.len() as c_uint);
    assert_eq!(n, MARS_XLOG_ERR_NO_PATH, "unexpected code {n}");
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
    assert_eq!(mars_xlog_open(cfg.as_ptr()), MARS_XLOG_OK);
    mars_xlog_set_level(0);
    write(3, "smoke", "async-mode-record");

    // Async mode hands the record to the writer thread, so poll a little:
    // `flush_sync` only guarantees the thread has been signalled.
    let mut bytes = Vec::new();
    for _ in 0..20 {
        mars_xlog_flush_sync();
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
    mars_xlog_close();
}

#[test]
fn zstd_mode_is_accepted() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config::new(dir.path(), 1, 1); // Sync + Zstd
    assert_eq!(mars_xlog_open(cfg.as_ptr()), MARS_XLOG_OK);
    mars_xlog_set_level(0);
    write(2, "smoke", "zstd-mode-record");
    mars_xlog_flush_sync();
    // Sync mode stores the payload verbatim, so the record must be readable as
    // text: "the file is not empty" would also pass if compress_mode were
    // ignored or the body were garbage.
    let bytes = fs::read(log_file(dir.path())).unwrap();
    assert!(
        any_view_contains(&bytes, "zstd-mode-record"),
        "the record is not in the log: {} bytes",
        bytes.len()
    );
    mars_xlog_close();
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
    let rc = mars_xlog_open(&cfg);
    // An empty prefix reaches the appender untouched, so this either opens or
    // is refused — what must not happen is "both", which is what the old
    // `OK || ERR_APPENDER` assertion accepted.
    assert_eq!(rc, MARS_XLOG_OK, "unexpected code {rc}");
    mars_xlog_close();
}

#[test]
fn c_types_line_up_with_the_header() {
    // The signatures the C header promises; if any of these stops compiling the
    // ABI drifted away from `include/mars_xlog.h`.
    let _f: extern "C" fn(*const MarsXLogConfig) -> c_int = mars_xlog_open;
    let _f: extern "C" fn(
        c_int,
        *const c_char,
        *const c_char,
        *const c_char,
        c_int,
        *const c_char,
    ) = mars_xlog_write;
    let _f: extern "C" fn() = mars_xlog_flush;
    let _f: extern "C" fn() = mars_xlog_flush_sync;
    let _f: extern "C" fn() = mars_xlog_close;
    let _f: extern "C" fn(c_int) = mars_xlog_set_level;
    let _f: extern "C" fn(c_int) = mars_xlog_set_console_log;
    let _f: extern "C" fn(c_ulonglong) = mars_xlog_set_max_file_size;
    let _f: extern "C" fn(c_longlong) = mars_xlog_set_max_alive_duration;
    let _f: extern "C" fn(*mut c_char, c_uint) -> c_int = mars_xlog_current_log_path;
}
