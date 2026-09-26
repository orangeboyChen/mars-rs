//! What a record costs the allocator on the C ABI boundary.
//!
//! `mars_xlog_write` used to build three `String`s before it could hand a
//! record to the appender — one each for `tag`, `filename` and `func_name` —
//! while `Java2C_Xlog.cc` hands the caller's `const char*` straight to
//! `xlogger_Write`. `XLoggerInfo` borrows those fields now, so this pins the
//! boundary at the C++'s zero.
//!
//! The counter below is deliberately global and this file deliberately holds
//! one test: `cargo test` runs this binary's cases on parallel threads, and
//! naming a thread from inside the allocator (a thread-local, or
//! `ThreadId::as_u64`) is not something an allocator may do.

use std::alloc::{GlobalAlloc, Layout, System};
use std::ffi::{CStr, CString};
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use mars_ffi::{
    mars_xlog_close, mars_xlog_open, mars_xlog_set_level, mars_xlog_write, MarsXLogConfig,
    MARS_XLOG_OK,
};

/// Whether [`COUNT`] is open.
static ARMED: AtomicBool = AtomicBool::new(false);
/// What has been allocated while [`ARMED`].
static COUNT: AtomicUsize = AtomicUsize::new(0);

fn counted<T>(alloc: impl FnOnce() -> T) -> T {
    if ARMED.load(Ordering::SeqCst) {
        COUNT.fetch_add(1, Ordering::SeqCst);
    }
    alloc()
}

/// Counts every allocation made while it is armed.
struct Counter;

unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        counted(|| unsafe { System.alloc(layout) })
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        counted(|| unsafe { System.realloc(ptr, layout, new_size) })
    }
}

#[global_allocator]
static ALLOC: Counter = Counter;

/// Counts the allocations from here on.
fn watch() {
    COUNT.store(0, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
}

/// Stops counting and answers what was counted since [`watch`].
fn stop() -> usize {
    ARMED.store(false, Ordering::SeqCst);
    COUNT.load(Ordering::SeqCst)
}

/// The appender is a process-wide singleton: one test at a time.
fn lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Opens the appender in sync mode — no writer thread, so nothing allocates on
/// behalf of the counting thread while it is being counted.
fn open(dir: &std::path::Path) {
    fs::create_dir_all(dir).unwrap();
    let log_dir = CString::new(dir.to_str().unwrap()).unwrap();
    let prefix = CString::new("write_alloc").unwrap();
    let empty = CString::new("").unwrap();
    let cfg = MarsXLogConfig {
        mode: 1,
        log_dir: log_dir.as_ptr(),
        name_prefix: prefix.as_ptr(),
        pub_key: std::ptr::null(),
        compress_mode: 0,
        compress_level: 0,
        cache_dir: empty.as_ptr(),
        cache_days: 0,
    };
    // SAFETY: `cfg` borrows the `CString`s above, which are alive across the
    // call, and a null `cfg` is answered inside rather than dereferenced.
    let opened = unsafe { mars_xlog_open(&cfg) };
    assert_eq!(opened, MARS_XLOG_OK, "mars_xlog_open failed");
    // Another test may have raised the process-wide level.
    mars_xlog_set_level(0);
}

/// Writes one record the way a C caller does: with pointers it owns.
///
/// The `CString`s are built by the caller and live outside the counting
/// window — the cost being measured is the one inside `mars_xlog_write`, not
/// the cost of a host building its arguments.
fn write(tag: &CStr, file: &CStr, func: &CStr, message: &CStr) {
    // SAFETY: the four pointers are borrowed from `CStr`s the caller keeps
    // alive across the call, so each is null-terminated and readable for as
    // long as the callee holds it.
    unsafe {
        mars_xlog_write(
            2,
            tag.as_ptr(),
            file.as_ptr(),
            func.as_ptr(),
            42,
            message.as_ptr(),
        );
    }
}

#[test]
fn a_record_written_through_the_c_abi_costs_no_allocation() {
    let _guard = lock();
    let dir = std::env::temp_dir().join("mars-ffi-write-alloc");
    let _ = fs::remove_dir_all(&dir);
    open(&dir);

    let tag = CString::new("write_alloc").unwrap();
    let file = CString::new("write_alloc.rs").unwrap();
    let func = CString::new("void Foo::bar(int)").unwrap();
    let warm_up = CString::new("warm up").unwrap();
    let body = CString::new("a record, long enough to be a record").unwrap();

    // Warm-up: opens the log file and grows the buffers a real caller grows in
    // its first record.
    write(&tag, &file, &func, &warm_up);

    watch();
    for _ in 0..16 {
        write(&tag, &file, &func, &body);
    }
    let count = stop();
    mars_xlog_close();

    // The records have to have reached the log for the count to mean
    // anything: a level that filtered them out would also cost nothing.
    let written: String = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| fs::read(entry.unwrap().path()).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .collect();
    assert!(
        written.contains("a record, long enough to be a record"),
        "the records did not reach the log: {written}"
    );

    assert_eq!(count, 0, "16 records through the C ABI allocated {count}x");
    let _ = fs::remove_dir_all(&dir);
}
