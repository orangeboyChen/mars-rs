//! The platform queries `mars/xlog/src/appender.cc` makes that `std` cannot
//! express.
//!
//! | C++                                                   | Rust                |
//! |-------------------------------------------------------|---------------------|
//! | `gettid()` / `pthread_threadid_np` / `GetCurrentThreadId` | [`thread_id`]   |
//! | `boost::filesystem::space(dir).available`             | [`available_space`] |
//!
//! Both are `unsafe` underneath (raw libc / win32 calls), so they are fenced
//! off in this module — the rest of the crate stays `#![deny(unsafe_code)]`.
#![allow(unsafe_code)]

use std::path::Path;
use std::sync::OnceLock;

/// The OS thread id of the calling thread.
///
/// The C++ stamps the real OS tid into every `XLoggerInfo`; the port used to
/// hand out a per-thread counter instead, which made logs written by Rust and
/// C++ in the same process impossible to correlate.
pub fn thread_id() -> i64 {
    // The OS tid cannot change for a thread, so it is looked up once and then
    // cached: this is called for every single log record. The pid is part of the
    // cache key because `fork()` invalidates the tid while `pid()` is re-read
    // every time — logging (child pid, parent tid) would be a pair that cannot
    // exist, and correlating with the C++ is the point of using the OS tid.
    thread_local! {
        static TID: std::cell::Cell<(u32, i64)> = const { std::cell::Cell::new((0, 0)) };
    }
    let pid = std::process::id();
    TID.with(|cell| {
        let (cached_pid, tid) = cell.get();
        if cached_pid != pid {
            let tid = os_thread_id();
            cell.set((pid, tid));
            tid
        } else {
            tid
        }
    })
}

/// The raw OS query behind [`thread_id`].
fn os_thread_id() -> i64 {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // `gettid` never fails and needs no errno handling.
        unsafe { libc::gettid() as i64 }
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos"
    ))]
    {
        extern "C" {
            fn pthread_threadid_np(thread: *mut libc::c_void, id: *mut u64) -> libc::c_int;
        }
        let mut id: u64 = 0;
        // `NULL` is accepted as "the calling thread".
        let ret = unsafe { pthread_threadid_np(std::ptr::null_mut(), &mut id) };
        if ret == 0 {
            id as i64
        } else {
            -1
        }
    }

    #[cfg(windows)]
    {
        extern "system" {
            fn GetCurrentThreadId() -> u32;
        }
        unsafe { GetCurrentThreadId() as i64 }
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
        windows
    )))]
    {
        // No OS tid available: fall back to the process id so the field is at
        // least stable and non-zero.
        std::process::id() as i64
    }
}

/// `xlogger_maintid()` — the id of the thread that first called this, i.e. the
/// process main thread for every realistic caller.
///
/// Captured once so that a record written on a worker thread still reports the
/// real main thread id (`XloggerAppender` marks records whose `tid == maintid`
/// with a `*`).
pub fn main_thread_id() -> i64 {
    static MAIN: OnceLock<i64> = OnceLock::new();
    *MAIN.get_or_init(os_main_thread_id)
}

/// `xlogger_maintid()`: `getpid()` on unix (`mars/comm/unix/xlogger_threadinfo.cc`),
/// and the real main thread on Apple, where the C++ captures `pthread_self()`
/// from a load-time constructor.
///
/// Rust has no portable load-time constructor, so on Apple the first caller is
/// used *only* when it really is the main thread (`pthread_main_np()`);
/// otherwise the pid is used rather than stamping a worker thread's tid onto
/// every record for the lifetime of the process.
fn os_main_thread_id() -> i64 {
    #[cfg(target_vendor = "apple")]
    {
        extern "C" {
            fn pthread_main_np() -> libc::c_int;
        }
        if unsafe { pthread_main_np() } != 0 {
            return thread_id();
        }
        std::process::id() as i64
    }

    #[cfg(not(target_vendor = "apple"))]
    {
        std::process::id() as i64
    }
}

/// Free space of the filesystem holding `path`, in bytes.
///
/// `boost::filesystem::space(dir).available` — `statvfs`' `f_bavail * f_frsize`
/// on unix, `GetDiskFreeSpaceExW`'s "available to caller" on Windows. Returns
/// `None` when the query fails, which the caller treats as "unknown" rather
/// than "no space".
pub fn available_space(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let path = CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        // `f_bavail` is the space available to unprivileged users, i.e. what
        // `boost::filesystem::space_info::available` reports.
        Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        #[allow(non_snake_case)]
        extern "system" {
            fn GetDiskFreeSpaceExW(
                lpDirectoryName: *const u16,
                lpFreeBytesAvailableToCaller: *mut u64,
                lpTotalNumberOfBytes: *mut u64,
                lpTotalNumberOfFreeBytes: *mut u64,
            ) -> i32;
        }

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available: u64 = 0;
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(available)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_id_is_positive_and_stable() {
        let first = thread_id();
        let second = thread_id();
        assert!(first > 0, "unexpected tid {first}");
        assert_eq!(first, second);
    }

    #[test]
    fn thread_id_differs_per_thread() {
        let main_tid = thread_id();
        let spawned = std::thread::spawn(thread_id).join().unwrap();
        assert_ne!(main_tid, spawned);
    }

    #[test]
    fn available_space_is_reported_for_a_real_directory() {
        let dir = std::env::temp_dir();
        let space = available_space(&dir).expect("statvfs/GetDiskFreeSpaceEx failed");
        assert!(space > 0, "no free space reported for {dir:?}");
    }

    #[test]
    fn available_space_is_none_for_a_missing_directory() {
        assert_eq!(
            available_space(Path::new("/definitely/not/here/xlog")),
            None
        );
    }
}
