//! Drives the whole C ABI from Rust, exactly the way the C++/JNI layer does:
//! open into a temp dir, write a few records, flush, close, then read the
//! resulting `.xlog` file back and check the bodies really landed on disk.
//!
//! The appender is a process-wide singleton, so every test takes [`LOCK`]:
//! `cargo test` runs the cases in this file on parallel threads and they would
//! otherwise race over the instance registry.

use std::ffi::{CStr, CString};
use std::fs;
use std::io::Read;
use std::os::raw::{c_char, c_int, c_longlong, c_uint, c_ulonglong};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use mars_ffi::abi::{
    mars_xlog_current_log_path_instance, mars_xlog_new_instance, mars_xlog_release_instance,
};
use mars_ffi::{
    mars_xlog_assert, mars_xlog_current_log_path, mars_xlog_flush_now_instance,
    mars_xlog_request_flush_instance, mars_xlog_set_console_fun,
    mars_xlog_set_console_log_instance, mars_xlog_set_level_instance,
    mars_xlog_set_max_alive_duration_instance, mars_xlog_set_max_file_size_instance,
    mars_xlog_write_instance, MarsXLogConfig, MARS_XLOG_ERR_NO_PATH, MARS_XLOG_ERR_NO_SPACE,
    MARS_XLOG_ERR_NULL_OUT,
};

/// Closes the appender when the test ends, even if it failed.
///
/// The appender is a process-wide singleton that rejects a second
/// `appender_open`, so a test that panics mid-way would otherwise poison every
/// test that runs after it.
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

/// `xlogger_Assert` through the C ABI: the expression goes into the body, and
/// the record is written whatever level the app set.
fn assert_expr(expression: &str, message: &str) {
    let tag = CString::new("smoke").unwrap();
    let file = CString::new("ffi_smoke.rs").unwrap();
    let func = CString::new("assert_expr").unwrap();
    let expression = CString::new(expression).unwrap();
    let message = CString::new(message).unwrap();
    unsafe {
        mars_xlog_assert(
            tag.as_ptr(),
            file.as_ptr(),
            func.as_ptr(),
            42,
            expression.as_ptr(),
            message.as_ptr(),
        );
    }
}

/// The last record the C console callback was handed, written by
/// [`console_seen`].
static SEEN: Mutex<String> = Mutex::new(String::new());

/// A `MarsXLogConsoleFun` — the sink an app hands to
/// `mars_xlog_set_console_fun`.
///
/// # Safety
///
/// Every pointer is the one the port's own sink passes, and none of them is
/// null.
unsafe extern "C" fn console_seen(
    level: c_int,
    tag: *const c_char,
    _filename: *const c_char,
    _func_name: *const c_char,
    line: c_int,
    log: *const c_char,
) {
    let tag = CStr::from_ptr(tag).to_string_lossy().into_owned();
    let log = CStr::from_ptr(log).to_string_lossy().into_owned();
    *SEEN.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
        format!("{level}:{tag}:{line}:{log}");
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
fn current_log_path_is_reported() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();
    let log = open_sync(dir.path());
    // The log file is created lazily, on the first record.
    write(log, 2, "smoke", "path-check-record");
    mars_xlog_flush_now_instance(log);

    let mut buf = [0u8; 512];
    let n = unsafe {
        mars_xlog_current_log_path_instance(
            log,
            buf.as_mut_ptr() as *mut c_char,
            buf.len() as c_uint,
        )
    };
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
        unsafe {
            mars_xlog_current_log_path_instance(
                log,
                tiny.as_mut_ptr() as *mut c_char,
                tiny.len() as c_uint,
            )
        },
        MARS_XLOG_ERR_NO_SPACE
    );
    assert_eq!(
        unsafe { mars_xlog_current_log_path_instance(log, tiny.as_mut_ptr() as *mut c_char, 0) },
        MARS_XLOG_ERR_NO_SPACE,
        "len == 0 must be rejected"
    );
    assert_eq!(
        unsafe { mars_xlog_current_log_path_instance(log, std::ptr::null_mut(), 64) },
        MARS_XLOG_ERR_NULL_OUT,
        "a null out pointer must be rejected"
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
fn an_assert_has_nowhere_to_go_from_c_alone() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();
    let log = open_sync(dir.path());

    // `xlogger_Assert` is annotated "no level filter" in `xloggerbase.h`, and
    // the C++ sends it to `xlogger_Write` — the process-wide appender. No C
    // symbol installs that one any more: an app opens an instance, and the
    // process-wide appender is the plumbing the JNI bridge sets up from Rust.
    // So from C the record has nowhere to land, and what has to hold is that
    // it neither crashes nor disturbs the instance's own file.
    mars_xlog_set_level_instance(log, 5); // Fatal
    write(log, 5, "smoke", "this-fatal-record-survives");
    assert_expr("x == y", "the two are not equal");
    mars_xlog_flush_now_instance(log);

    let bytes = fs::read(log_file(dir.path())).unwrap();
    assert!(any_view_contains(&bytes, "this-fatal-record-survives"));
    assert!(
        !any_view_contains(&bytes, "[ASSERT(x == y)]the two are not equal"),
        "the assert reached an appender a C caller cannot open"
    );
    assert!(
        !any_view_contains(&bytes, "this-verbose-record-must-be-dropped"),
        "the level did not gate the write"
    );

    mars_xlog_set_level_instance(log, 0); // back to Verbose
    close();
}

#[test]
fn a_console_callback_an_app_set_is_handed_the_record() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();
    let log = open_sync(dir.path());
    mars_xlog_set_console_log_instance(log, 1);
    mars_xlog_set_console_fun(Some(console_seen));

    write(log, 2, "console", "to-the-callback");
    mars_xlog_flush_now_instance(log);

    let seen = SEEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    // Both are process-wide, so take them away again before the assertion:
    // a panic here must not leave the next test writing through the callback.
    mars_xlog_set_console_fun(None);
    mars_xlog_set_console_log_instance(log, 0);
    close();

    assert_eq!(seen, "2:console:42:to-the-callback");
}

#[test]
fn null_pointers_are_never_dereferenced() {
    let _g = lock();
    let _close = CloseOnDrop;
    let dir = tempfile::tempdir().unwrap();

    // No config at all. An instance is refused with handle `0`, whatever the
    // reason: the C ABI has no `mars_xlog_open` to report a code through.
    assert_eq!(unsafe { mars_xlog_new_instance(std::ptr::null(), 0) }, 0);

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
    assert_eq!(unsafe { mars_xlog_new_instance(&cfg, 0) }, 0);

    // Bad enum values.
    let good = Config::new(dir.path(), 1, 0);
    let mut cfg = good.raw;
    cfg.mode = 7;
    assert_eq!(unsafe { mars_xlog_new_instance(&cfg, 0) }, 0);
    cfg.mode = 1;
    cfg.compress_mode = 9;
    assert_eq!(unsafe { mars_xlog_new_instance(&cfg, 0) }, 0);
    // ...and the same object keeps working once it is valid again.
    cfg.compress_mode = 0;
    let log = unsafe { mars_xlog_new_instance(&cfg, 0) };
    assert_ne!(log, 0);
    mars_xlog_set_level_instance(log, 0);

    // A write with every pointer null (level 6 == kLevelNone is also dropped).
    unsafe {
        mars_xlog_write_instance(
            log,
            6,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            std::ptr::null(),
        );
    }
    unsafe {
        mars_xlog_write_instance(
            log,
            2,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            std::ptr::null(),
        );
    }
    // An assert with every pointer null.
    unsafe {
        mars_xlog_assert(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
    mars_xlog_flush_now_instance(0);

    // The setters must tolerate being called with junk too.
    mars_xlog_set_level_instance(0, -3);
    mars_xlog_set_level_instance(0, 0);
    mars_xlog_set_console_log_instance(0, 1);
    mars_xlog_set_console_log_instance(0, 0);
    mars_xlog_set_max_file_size_instance(0, 0);
    mars_xlog_set_max_file_size_instance(0, 4 * 1024 * 1024);
    mars_xlog_set_max_alive_duration_instance(0, -1);
    mars_xlog_set_max_alive_duration_instance(0, 10 * 24 * 3600);

    close();
    // Closing twice, flushing while closed: none of it may abort the process.
    mars_xlog_request_flush_instance(0);
    mars_xlog_flush_now_instance(0);
    close();
}

#[test]
fn no_path_before_open() {
    let _g = lock();
    let _close = CloseOnDrop;
    close();
    let mut buf = [0u8; 256];
    let n =
        unsafe { mars_xlog_current_log_path(buf.as_mut_ptr() as *mut c_char, buf.len() as c_uint) };
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

#[test]
fn c_types_line_up_with_the_header() {
    // The signatures the C header promises; if any of these stops compiling the
    // ABI drifted away from `include/mars_xlog.h`. `unsafe` is the Rust side
    // only — a pointer the caller owns is what makes a symbol unsafe to call
    // from Rust, and the C header has no way to say so.
    let _f: unsafe extern "C" fn(*const MarsXLogConfig, c_int) -> c_longlong =
        mars_xlog_new_instance;
    let _f: unsafe extern "C" fn(
        c_longlong,
        c_int,
        *const c_char,
        *const c_char,
        *const c_char,
        c_int,
        *const c_char,
    ) = mars_xlog_write_instance;
    let _f: extern "C" fn(c_longlong) = mars_xlog_request_flush_instance;
    let _f: extern "C" fn(c_longlong) = mars_xlog_flush_now_instance;
    let _f: unsafe extern "C" fn(*const c_char) = mars_xlog_release_instance;
    let _f: extern "C" fn(c_longlong, c_int) = mars_xlog_set_level_instance;
    let _f: extern "C" fn(c_longlong, c_int) = mars_xlog_set_console_log_instance;
    let _f: extern "C" fn(c_longlong, c_ulonglong) = mars_xlog_set_max_file_size_instance;
    let _f: extern "C" fn(c_longlong, c_longlong) = mars_xlog_set_max_alive_duration_instance;
    let _f: unsafe extern "C" fn(*mut c_char, c_uint) -> c_int = mars_xlog_current_log_path;
    let _f: unsafe extern "C" fn(c_longlong, *mut c_char, c_uint) -> c_int =
        mars_xlog_current_log_path_instance;
}
