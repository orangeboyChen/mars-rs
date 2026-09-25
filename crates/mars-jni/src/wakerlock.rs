//! `io.github.marsrs.comm.WakerLock` — the port of
//! `com/tencent/mars/comm/WakerLock.java`.
//!
//! The C++ reaches the Java class through `platform_comm.cc`
//! (`wakeupLock_new`, `wakeupLock_lock`, `wakeupLock_unlock`,
//! `wakeupLock_isLocking`), so there is no `native` method on the Java side:
//! this module is the state that class holds, kept in Rust.
//!
//! Two details of the Java are kept:
//!
//! * the `PowerManager.WakeLock` is **not** reference counted
//!   (`setReferenceCounted(false)`), so a second `lock()` does not need a
//!   second `unLock()`, and `unLock()` on a lock that is not held does
//!   nothing;
//! * `lock(timeInMills)` posts a delayed release, and both `lock()` and
//!   `unLock()` cancel a release that is still pending — so locking again
//!   keeps the lock until the *new* deadline.
//!
//! Everything the JVM touches lives in [`crate::jni_bridge`]; what is here is
//! plain Rust and is covered by `cargo test`.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use mars_comm::tickcount::gettickcount;

/// The handle `wakeupLock_new` hands back; `0` is the "no lock" answer, which
/// is what Java gets when `context` is null.
pub type WakerLockHandle = u64;

/// One lock: whether it is held, and when a pending release is due.
#[derive(Debug, Clone, Copy)]
struct Lock {
    held: bool,
    /// Tick count at which the delayed release fires; `None` when no release
    /// was posted (`lock()` without a time).
    release_at: Option<u64>,
}

impl Lock {
    fn new() -> Self {
        Self {
            held: false,
            release_at: None,
        }
    }
}

#[derive(Debug, Default)]
struct Locks {
    locks: BTreeMap<WakerLockHandle, Lock>,
    next: WakerLockHandle,
}

fn state() -> &'static Mutex<Locks> {
    static STATE: OnceLock<Mutex<Locks>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(Locks::default()))
}

fn with_state<R>(f: impl FnOnce(&mut Locks) -> R) -> R {
    let mut state = state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut state)
}

fn with_lock(handle: WakerLockHandle, f: impl FnOnce(&mut Lock)) {
    with_state(|state| {
        if let Some(lock) = state.locks.get_mut(&handle) {
            f(lock);
        }
    });
}

/// `wakeupLock_new` — the handle, never `0`.
pub fn wakerlock_new_impl() -> WakerLockHandle {
    with_state(|state| {
        state.next += 1;
        let handle = state.next;
        state.locks.insert(handle, Lock::new());
        handle
    })
}

/// `wakeupLock_lock` — cancels a pending release first, like the Java.
pub fn wakerlock_lock_impl(handle: WakerLockHandle) {
    with_lock(handle, |lock| {
        lock.held = true;
        lock.release_at = None;
    })
}

/// `lock(timeInMills)` — locks and posts the delayed release.
pub fn wakerlock_lock_for_impl(handle: WakerLockHandle, millis: u64, now: u64) {
    with_lock(handle, |lock| {
        lock.held = true;
        lock.release_at = Some(now + millis);
    })
}

/// `wakeupLock_unlock` — does nothing when the lock is not held.
pub fn wakerlock_unlock_impl(handle: WakerLockHandle) {
    with_lock(handle, |lock| {
        lock.held = false;
        lock.release_at = None;
    })
}

/// `wakeupLock_isLocking` at `now`: a release that is due has already run, so
/// the lock reads as released from then on.
pub fn wakerlock_is_locking_at_impl(handle: WakerLockHandle, now: u64) -> bool {
    with_state(|state| match state.locks.get_mut(&handle) {
        Some(lock) => {
            if let Some(release_at) = lock.release_at {
                if now >= release_at {
                    lock.held = false;
                    lock.release_at = None;
                }
            }
            lock.held
        }
        None => false,
    })
}

/// `wakeupLock_isLocking`.
pub fn wakerlock_is_locking_impl(handle: WakerLockHandle) -> bool {
    wakerlock_is_locking_at_impl(handle, gettickcount())
}

/// Drops the lock — `finalize()` unlocks before the object goes away.
pub fn wakerlock_delete_impl(handle: WakerLockHandle) -> bool {
    with_state(|state| state.locks.remove(&handle).is_some())
}

/// How many locks exist, for tests.
pub fn wakerlock_count_impl() -> usize {
    with_state(|state| state.locks.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated<R>(f: impl FnOnce() -> R) -> R {
        let guard = crate::test_lock();
        let result = f();
        drop(guard);
        result
    }

    #[test]
    fn a_lock_is_acquired_and_released() {
        isolated(|| {
            let handle = wakerlock_new_impl();
            assert_ne!(handle, 0);
            assert!(!wakerlock_is_locking_at_impl(handle, 0));

            wakerlock_lock_impl(handle);
            assert!(wakerlock_is_locking_at_impl(handle, 0));

            wakerlock_unlock_impl(handle);
            assert!(!wakerlock_is_locking_at_impl(handle, 0));

            assert!(wakerlock_delete_impl(handle));
            assert!(!wakerlock_delete_impl(handle), "already dropped");
        })
    }

    #[test]
    fn the_lock_is_not_reference_counted() {
        isolated(|| {
            let handle = wakerlock_new_impl();
            // `setReferenceCounted(false)`: two acquires, one release
            wakerlock_lock_impl(handle);
            wakerlock_lock_impl(handle);
            wakerlock_unlock_impl(handle);
            assert!(!wakerlock_is_locking_at_impl(handle, 0));

            // and unlocking a lock that is not held is harmless
            wakerlock_unlock_impl(handle);
            assert!(!wakerlock_is_locking_at_impl(handle, 0));
        })
    }

    #[test]
    fn a_timed_lock_releases_itself_when_it_is_due() {
        isolated(|| {
            let handle = wakerlock_new_impl();
            wakerlock_lock_for_impl(handle, 200, 1_000);

            assert!(wakerlock_is_locking_at_impl(handle, 1_000));
            assert!(wakerlock_is_locking_at_impl(handle, 1_199));
            assert!(!wakerlock_is_locking_at_impl(handle, 1_200), "released");
            assert!(!wakerlock_is_locking_at_impl(handle, 5_000));

            wakerlock_delete_impl(handle);
        })
    }

    #[test]
    fn locking_again_cancels_the_pending_release() {
        isolated(|| {
            let handle = wakerlock_new_impl();
            wakerlock_lock_for_impl(handle, 200, 1_000);
            // `lock()` removes the pending releaser before acquiring
            wakerlock_lock_impl(handle);
            assert!(wakerlock_is_locking_at_impl(handle, 5_000), "kept forever");

            wakerlock_delete_impl(handle);
        })
    }

    #[test]
    fn an_unknown_handle_is_not_locking() {
        isolated(|| {
            assert!(!wakerlock_is_locking_at_impl(0, 0));
            wakerlock_lock_impl(0);
            wakerlock_unlock_impl(0);
            assert!(!wakerlock_is_locking_at_impl(0, 0));
        })
    }

    #[test]
    fn every_handle_is_a_different_lock() {
        isolated(|| {
            let before = wakerlock_count_impl();
            let first = wakerlock_new_impl();
            let second = wakerlock_new_impl();
            assert_ne!(first, second);

            wakerlock_lock_impl(first);
            assert!(wakerlock_is_locking_at_impl(first, 0));
            assert!(!wakerlock_is_locking_at_impl(second, 0));

            wakerlock_delete_impl(first);
            wakerlock_delete_impl(second);
            assert_eq!(wakerlock_count_impl(), before);
        })
    }

    #[test]
    fn the_clock_the_app_sees_drives_the_release() {
        isolated(|| {
            let handle = wakerlock_new_impl();
            // a lock for 0 ms is due the moment it is taken, so even a clock
            // that has not moved on releases it
            wakerlock_lock_for_impl(handle, 0, gettickcount());
            assert!(!wakerlock_is_locking_impl(handle));

            // and one with no deadline at all is still held
            wakerlock_lock_impl(handle);
            assert!(wakerlock_is_locking_impl(handle));
            wakerlock_delete_impl(handle);
        })
    }
}
