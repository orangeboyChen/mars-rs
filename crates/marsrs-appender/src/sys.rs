//! The platform queries `mars/xlog/src/appender.cc` makes that `std` cannot
//! express.
//!
//! | C++                                                   | Rust                |
//! |-------------------------------------------------------|---------------------|
//! | `gettid()` / `pthread_threadid_np` / `GetCurrentThreadId` | [`thread_id`]   |
//! | `boost::filesystem::space(dir)`                          | [`space_info`]  |
//! | `boost::filesystem::space(dir).available`                | [`available_space`] |
//!
//! Both are `unsafe` underneath (raw libc / win32 calls), so they are fenced
//! off in this module — the rest of the crate stays `#![deny(unsafe_code)]`.
#![allow(unsafe_code)]

use std::fs::File;
use std::path::Path;
use std::sync::OnceLock;

use crate::file_util::private_file;

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

/// The OS thread id with no caching, so it is safe to call from inside the
/// allocator: one syscall (or one libsystem call) and no thread-local state.
#[cfg(test)]
pub(crate) fn raw_thread_id() -> i64 {
    os_thread_id()
}

/// The raw OS query behind [`thread_id`].
fn os_thread_id() -> i64 {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // The syscall and not `libc::gettid()`: that wrapper is glibc 2.30 and
        // later, and a Kotlin/Native klib is linked against its own sysroot,
        // which is glibc 2.19 — a strong `gettid` is an undefined symbol in
        // every Linux archive this crate is embedded in, which is what ended
        // the release's `linuxX64Test`. `std` takes the symbol weakly for the
        // same reason and falls back to this call. The syscall is Linux 2.4.11,
        // and it cannot fail, so there is no errno to read.
        let tid: libc::pid_t = unsafe { libc::syscall(libc::SYS_gettid) as libc::pid_t };
        tid as i64
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

/// `xlogger_maintid()` — the value a record's `maintid` is filled in with,
/// captured once so that a record written on a worker thread reports the same
/// one for the lifetime of the process (`XloggerAppender` marks records whose
/// `tid == maintid` with a `*`).
///
/// What that value *is* is `os_main_thread_id`'s answer: the real main
/// thread on Apple, and the process id everywhere else — which is what the
/// C++'s own `getpid()` gives too. It is a main *thread* id only on the one
/// platform that has an api for asking.
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

/// `boost::filesystem::space(dir)` — `(capacity, free, available)` in bytes.
///
/// `statvfs` on unix (`f_blocks` / `f_bfree` / `f_bavail`, each times
/// `f_frsize`) and `GetDiskFreeSpaceExW` on Windows. Returns `None` when the
/// query fails, which the caller treats as "unknown" rather than "no space".
///
/// `appender.cc` prints all three in its `cache dir space info` / `log dir
/// space info` records, and compares `available` against the 1 GiB threshold
/// of `__CacheLogs`.
pub fn space_info(path: &Path) -> Option<(u64, u64, u64)> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let path = CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        let block = stat.f_frsize as u64;
        Some((
            (stat.f_blocks as u64).saturating_mul(block),
            (stat.f_bfree as u64).saturating_mul(block),
            // `f_bavail` is the space available to unprivileged users, i.e.
            // what `boost::filesystem::space_info::available` reports.
            (stat.f_bavail as u64).saturating_mul(block),
        ))
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        // The names are spelled the way Rust spells them; the signature is the
        // one `GetDiskFreeSpaceExW` has, and a parameter's name is not part of
        // it.
        extern "system" {
            fn GetDiskFreeSpaceExW(
                directory_name: *const u16,
                free_bytes_available_to_caller: *mut u64,
                total_number_of_bytes: *mut u64,
                total_number_of_free_bytes: *mut u64,
            ) -> i32;
        }

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available: u64 = 0;
        let mut capacity: u64 = 0;
        let mut free: u64 = 0;
        let ok =
            unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, &mut capacity, &mut free) };
        (ok != 0).then_some((capacity, free, available))
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// Free space of the filesystem holding `path`, in bytes.
///
/// `boost::filesystem::space(dir).available` — the `available` third of
/// [`space_info`]. Returns `None` when the query fails, which the caller treats
/// as "unknown" rather than "no space".
pub fn available_space(path: &Path) -> Option<u64> {
    space_info(path).map(|(_capacity, _free, available)| available)
}

/// Takes the exclusive advisory lock on `file`, waiting for it.
///
/// `false` when this platform has no advisory locking at all, and — on
/// Windows, where the wait is a poll with a give-up — when a holder inside
/// this process did not let go in time. Either way the section the caller
/// wanted to lock runs unlocked, which is what the C++ does anyway.
///
/// The lock is released when `file` is dropped — and by the kernel when the
/// process dies, which is the property the appender relies on: a lock no
/// longer held is how a later start tells a cache file some *other* process
/// is still writing through from one a dead process left behind.
pub fn lock_exclusive(file: &File) -> bool {
    lock(file, false)
}

/// [`lock_exclusive`] without the waiting: `false` when somebody else holds the
/// lock, which the caller reads as "that file is still in use".
pub fn try_lock_exclusive(file: &File) -> bool {
    lock(file, true)
}

/// Releases the lock [`lock_exclusive`] / [`try_lock_exclusive`] took, keeping
/// the handle.
///
/// `flock(fd, LOCK_UN)` / `UnlockFileEx`. Separate from dropping the file
/// because a caller that locks once per section would otherwise pay an `open`
/// and a `close` on top of the `flock` pair, and on macOS those two measured
/// ~16 µs against ~0.5 µs for the lock itself — more than a whole record costs.
/// One handle, opened once, is locked and unlocked as often as needed.
pub fn unlock(file: &File) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;

        // SAFETY: `flock` needs only a valid descriptor, which `File` is.
        0 == unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) }
    }

    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;

        /// `OVERLAPPED`: only the offset and the event are read, and both are
        /// zero here, matching the range `lock` locked.
        #[repr(C)]
        struct Overlapped {
            internal: usize,
            internal_high: usize,
            offset: u32,
            offset_high: u32,
            event: usize,
        }

        extern "system" {
            fn UnlockFileEx(
                file: *mut core::ffi::c_void,
                reserved: u32,
                bytes_low: u32,
                bytes_high: u32,
                overlapped: *mut Overlapped,
            ) -> i32;
        }

        let mut overlapped = Overlapped {
            internal: 0,
            internal_high: 0,
            offset: 0,
            offset_high: 0,
            event: 0,
        };
        // SAFETY: `file` is a valid handle and `overlapped` is a live,
        // correctly sized `OVERLAPPED`.
        let ok = unsafe { UnlockFileEx(file.as_raw_handle(), 0, 1, 0, &mut overlapped) };
        ok != 0
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        false
    }
}

/// A file a test can hold whose `ftruncate` succeeds and whose every `write`
/// fails — the pair a cache file on a full disk presents, which is the one
/// `open_region` has to survive.
///
/// No ordinary file gives that pair: a filesystem that refuses the `write`
/// refuses the `ftruncate` in front of it as well, and one that takes the
/// `ftruncate` takes the write. A `memfd_create` file sealed with
/// `F_SEAL_WRITE` does: the seal denies writes to the buffer (`EPERM`) and
/// leaves `ftruncate` alone, which is exactly what a disk with no space left
/// looks like from the appender — a length recorded with no blocks behind it,
/// and the write that would reserve them failing.
///
/// `None` where the file cannot be made: not Linux, no `memfd_create`, or a
/// kernel that will not take the seal. A test that gets `None` passes rather
/// than fails — what it asserts is what the appender does with such a file,
/// and there is no way to ask without one.
#[cfg(all(test, target_os = "linux"))]
pub(crate) fn unwritable_file() -> Option<File> {
    use std::os::unix::io::FromRawFd;

    /// `MFD_ALLOW_SEALING`, which `libc` exports for android and not for the
    /// gnu and musl targets this crate is built for.
    const MFD_ALLOW_SEALING: libc::c_uint = 0x0002;
    /// `MFD_CLOEXEC`, for the same reason: the tests re-execute this binary to
    /// get a second process, and a descriptor that outlives the one that made
    /// it would be one the child cannot account for.
    const MFD_CLOEXEC: libc::c_uint = 0x0001;

    let name = b"marsrs-unwritable\0";
    // SAFETY: `memfd_create` returns a descriptor of its own or -1, and the
    // name is a live NUL-terminated buffer.
    let fd = unsafe {
        libc::memfd_create(
            name.as_ptr().cast::<libc::c_char>(),
            MFD_ALLOW_SEALING | MFD_CLOEXEC,
        )
    };
    if fd < 0 {
        return None;
    }
    // SAFETY: `fd` is the descriptor created above and nothing else owns it,
    // so the `File` — which closes it — is its only owner from here.
    let file = unsafe { File::from_raw_fd(fd) };
    // SAFETY: `fcntl` over a descriptor this function owns; the third argument
    // is a flag and not a pointer, so there is nothing for it to outlive.
    let sealed = unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, libc::F_SEAL_WRITE) };
    (sealed == 0).then_some(file)
}

/// Whether two handles of `path` opened independently really exclude each
/// other here.
///
/// `flock` and `LockFileEx` are advisory and per open file description, so the
/// same process can own the same file twice through two `open()`s — that is
/// what lets two copies of this crate in one process contend. Not every
/// filesystem implements them, though (some FUSE and FAT mounts answer "locked"
/// without ever denying anybody), and believing a lock that excludes nobody
/// would be worse than knowing: the caller falls back to the unprotected
/// behaviour the C++ has.
///
/// The answer is probed rather than assumed, by locking the path twice —
/// but **not** `path` itself, and that is the whole trick.
///
/// `path` is the lock every writer of this directory takes and releases
/// around the sections that move files — the log directory's when the appender
/// has no cache directory of its own, and the cache directory's when it has. Probing it directly reads "another
/// writer is in a locked section right now" as "locking excludes nobody on
/// this filesystem": the first `try_lock` is denied, the probe answers
/// `false`, and the caller falls back to the unprotected behaviour — which,
/// for the cache file, is the C++'s single fixed name that every writer then
/// shares and corrupts. Two processes opening one prefix at the same moment
/// hit it roughly one run in three.
///
/// So the probe is run on a sibling nobody else can have open: created with
/// `create_new`, so an existing file can only mean this is not the first
/// probe of that name, and removed again on the way out.
pub fn lock_excludes(path: &Path) -> bool {
    let probe = probe_path(path);
    // Created the way every other file of the port's is — [`private_file`] —
    // and not with the process default: a probe is a file in the directory the
    // logs are in, and `create_private_dir` only sets the mode of a directory
    // it makes. One an app made before this port was linked, or one it hands
    // over with a mode of its own, is a directory the probe would otherwise
    // come out readable by every uid on the device.
    let Ok(first) =
        private_file(File::options().read(true).write(true).create_new(true)).open(&probe)
    else {
        return false;
    };
    // The guard owns the handle as well as the name, so the file is closed
    // before it is unlinked: on Windows a `remove_file` of a file another
    // handle holds open is denied unless that handle asked for delete
    // sharing, which `File::options` does not — and a probe left behind in
    // the log directory is a file an app did not write.
    let remove = RemoveOnDrop {
        file: Some(first),
        path: &probe,
    };
    if !try_lock_exclusive(remove.file.as_ref().expect("the handle is there")) {
        return false;
    }
    let Ok(second) = File::options().read(true).write(true).open(&probe) else {
        return false;
    };
    // A second, independent handle must not be able to take the same lock.
    !try_lock_exclusive(&second)
}

/// A sibling of `path` no other writer of it will ever have open: same
/// directory and therefore same filesystem, because whether locking excludes
/// anybody is a property of the filesystem and not of the name.
fn probe_path(path: &Path) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("lock");
    let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    path.with_file_name(format!("{name}.{}.{unique}.probe", std::process::id()))
}

/// Closes `file` and unlinks `path`, on the way out whatever the answer was.
struct RemoveOnDrop<'a> {
    file: Option<File>,
    path: &'a Path,
}

impl Drop for RemoveOnDrop<'_> {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(self.path);
    }
}

/// `flock(fd, LOCK_EX[ | LOCK_NB])` / `LockFileEx(..., LOCKFILE_EXCLUSIVE_LOCK
/// [, LOCKFILE_FAIL_IMMEDIATELY])`.
fn lock(file: &File, non_blocking: bool) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;

        let operation = if non_blocking {
            libc::LOCK_EX | libc::LOCK_NB
        } else {
            libc::LOCK_EX
        };
        // SAFETY: `flock` needs only a valid descriptor, which `File` is.
        0 == unsafe { libc::flock(file.as_raw_fd(), operation) }
    }

    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;

        /// `LOCKFILE_FAIL_IMMEDIATELY` / `LOCKFILE_EXCLUSIVE_LOCK`.
        const FAIL_IMMEDIATELY: u32 = 0x1;
        const EXCLUSIVE: u32 = 0x2;

        /// `OVERLAPPED`: only the offset and the event are read, and both are
        /// zero here — the range locked is `[0, 1)`, which is enough to make
        /// two holders exclude each other without locking any real byte (the
        /// file is a lock, it has none).
        #[repr(C)]
        struct Overlapped {
            internal: usize,
            internal_high: usize,
            offset: u32,
            offset_high: u32,
            event: usize,
        }

        extern "system" {
            fn LockFileEx(
                file: *mut core::ffi::c_void,
                flags: u32,
                reserved: u32,
                bytes_low: u32,
                bytes_high: u32,
                overlapped: *mut Overlapped,
            ) -> i32;
        }

        let flags = if non_blocking {
            EXCLUSIVE | FAIL_IMMEDIATELY
        } else {
            EXCLUSIVE
        };
        let mut overlapped = Overlapped {
            internal: 0,
            internal_high: 0,
            offset: 0,
            offset_high: 0,
            event: 0,
        };

        // A blocking request is what `flock(fd, LOCK_EX)` does on unix, and it
        // is the one that takes a lock another *process* is holding: the moment
        // that one lets go, this one has it. What it cannot do is wait for this
        // process: a request that overlaps a lock this process already holds —
        // through another handle, which is what a second appender of one prefix
        // has — fails at once with ERROR_LOCK_VIOLATION, whether
        // LOCKFILE_FAIL_IMMEDIATELY is set or not. Two copies of this crate in
        // one process are a case the port supports, so *that* one is waited for
        // below, and a request that fails here for any other reason is retried
        // there too: what the failure costs is five seconds, and what the
        // alternative costs is a section that runs with no lock at all.
        if !non_blocking {
            // SAFETY: as below.
            let taken =
                unsafe { LockFileEx(file.as_raw_handle(), EXCLUSIVE, 0, 1, 0, &mut overlapped) };
            if taken != 0 {
                return true;
            }

            use std::time::{Duration, Instant};

            /// Five seconds: a cache-file move is milliseconds.
            const GIVE_UP_AFTER: Duration = Duration::from_secs(5);
            let deadline = Instant::now() + GIVE_UP_AFTER;
            loop {
                // SAFETY: as below.
                let taken = unsafe {
                    LockFileEx(
                        file.as_raw_handle(),
                        EXCLUSIVE | FAIL_IMMEDIATELY,
                        0,
                        1,
                        0,
                        &mut overlapped,
                    )
                };
                if taken != 0 {
                    return true;
                }
                // The give-up is what keeps a writer of this process that never
                // lets go from stalling it forever. Giving up is answered
                // `false`, and the caller then runs its section unprotected —
                // the way the C++ runs it, and the only answer left that does
                // not drop the records the section was going to write.
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        // SAFETY: `file` is a valid handle and `overlapped` is a live,
        // correctly sized `OVERLAPPED` (zeroed, which is what the call wants).
        let ok = unsafe { LockFileEx(file.as_raw_handle(), flags, 0, 1, 0, &mut overlapped) };
        ok != 0
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, non_blocking);
        false
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

    /// Two handles of one file, opened separately, must exclude each other:
    /// that is what lets two processes — and two copies of this crate in one
    /// process — each keep a cache file of their own.
    #[test]
    fn an_exclusive_lock_denies_a_second_independent_handle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Mars.lock");
        let first = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        assert!(try_lock_exclusive(&first), "the first taker must win");

        let second = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        assert!(!try_lock_exclusive(&second), "a second handle must lose");

        // … and it is released when the winner is dropped, so the next start
        // can claim the file a dead process left behind.
        drop(first);
        assert!(try_lock_exclusive(&second));
    }

    /// Unlocking hands the file to the next taker without dropping the handle,
    /// which is how one handle is reused for every locked section.
    #[test]
    fn unlocking_a_held_handle_lets_another_handle_take_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Mars.lock");
        let first = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        assert!(lock_exclusive(&first));

        let second = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        assert!(!try_lock_exclusive(&second), "the lock is still held");

        assert!(unlock(&first));
        assert!(try_lock_exclusive(&second), "the lock was released");
        assert!(unlock(&second));
    }

    #[test]
    fn lock_excludes_answers_about_the_file_it_is_given() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Mars.lock");
        assert!(
            lock_excludes(&path),
            "advisory locking is what the cache slots are built on"
        );
        // The probe must not leave the file behind in a locked state.
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        assert!(try_lock_exclusive(&file));
    }

    #[test]
    fn a_lock_on_a_missing_directory_cannot_be_taken() {
        assert!(!lock_excludes(Path::new(
            "/definitely/not/here/xlog/Mars.lock"
        )));
    }

    /// The one thing the probe is for, and the one it used to get backwards:
    /// `path` is the lock every writer of this log directory takes, so another
    /// writer holding it *right now* has to read as "locking works here" and
    /// not as "locking excludes nobody". Answering the second is what hands
    /// every writer the same cache file.
    #[test]
    fn a_probe_under_contention_still_answers_that_locking_excludes() {
        let dir = std::env::temp_dir().join(format!(
            "marsrs-sys-contended-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Mars.lock");

        let held = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        assert!(lock_exclusive(&held), "the lock could not be taken at all");

        assert!(
            lock_excludes(&path),
            "a peer holding the log lock was read as a filesystem that cannot lock"
        );

        drop(held);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The probe's own file must not outlive the question: a directory an app
    /// reads to find its logs has no `<prefix>.lock.*.probe` in it.
    #[test]
    fn a_probe_leaves_nothing_behind() {
        let dir = std::env::temp_dir().join(format!(
            "marsrs-sys-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Mars.lock");
        assert!(lock_excludes(&path));

        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, Vec::<String>::new(), "the probe left {left:?} behind");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
