//! `mars/comm/thread/` — threads, the spin lock and the vector lock.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use mars_comm::thread::{runnable, MutexVector, SpinLock, Thread, ThreadUtil};

#[test]
fn thread_util_exposes_the_current_thread() {
    // the id is stable within a thread and differs between threads
    let here = ThreadUtil::current_thread_id();
    let there = thread::spawn(ThreadUtil::current_thread_id).join().unwrap();
    assert_eq!(here, ThreadUtil::current_thread_id());
    assert_ne!(here, there);
    ThreadUtil::yield_now();
    ThreadUtil::usleep(1);
}

#[test]
fn runnable_wraps_a_closure() {
    let mut runnable = runnable(|| {});
    runnable.run();
}

#[test]
fn a_thread_runs_its_body_and_can_be_joined() {
    let mut thread = Thread::new(Some("mars-test"));
    assert!(!thread.is_running());
    assert!(thread.start(|| {}));
    thread.join();
    assert!(!thread.is_running());
    assert_eq!(thread.name(), Some("mars-test"));
}

#[test]
fn starting_a_running_thread_is_a_no_op() {
    let barrier = Arc::new(Barrier::new(2));
    let worker = Arc::clone(&barrier);
    let mut thread = Thread::new(None);
    assert!(thread.start(move || {
        worker.wait();
        worker.wait();
    }));
    barrier.wait(); // the thread is inside its body
    assert!(!thread.start(|| unreachable!("must not start twice")));
    barrier.wait();
    thread.join();
}

#[test]
fn a_thread_is_running_as_soon_as_start_returns() {
    let barrier = Arc::new(Barrier::new(2));
    let worker = Arc::clone(&barrier);
    let mut thread = Thread::new(None);
    assert!(thread.start(move || {
        worker.wait();
    }));
    // `running` used to become true only once the child was scheduled, so a
    // second `start` arriving here saw `false`, detached the first handle and
    // launched a second callback.
    assert!(thread.is_running());
    assert!(!thread.start(|| unreachable!("must not start twice")));
    barrier.wait();
    thread.join();
}

#[test]
fn a_panicking_body_leaves_the_thread_startable() {
    let mut thread = Thread::new(None);
    assert!(thread.start(|| panic!("the callback panics")));
    thread.join();
    // Unwinding used to skip the `running.store(false)`, so every later start
    // was refused while `is_running()` stayed true.
    assert!(!thread.is_running());
    assert!(thread.start(|| {}));
    thread.join();
}

#[test]
fn a_delayed_start_can_be_cancelled() {
    let ran = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&ran);
    let mut thread = Thread::new(None);
    assert!(thread.start_after(5_000, move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    thread.cancel_after();
    thread.join();
    assert_eq!(ran.load(Ordering::SeqCst), 0);
}

#[test]
fn a_delayed_start_actually_runs() {
    let started = Instant::now();
    let ran = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&ran);
    let mut thread = Thread::new(None);
    assert!(thread.start_after(20, move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    thread.join();
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    assert!(started.elapsed() >= Duration::from_millis(20));
}

#[test]
fn a_periodic_thread_stops_when_cancelled() {
    let ticks = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&ticks);
    let mut thread = Thread::new(None);
    assert!(thread.start_periodic(0, 2, move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    thread::sleep(Duration::from_millis(40));
    thread.cancel_periodic();
    thread.join();

    let seen = ticks.load(Ordering::SeqCst);
    assert!(seen >= 2, "expected at least two ticks, got {seen}");
    thread::sleep(Duration::from_millis(20));
    // the closure owns the other handle to the same cell: it must not move
    // any more once cancel_periodic() returned.
    assert_eq!(
        seen,
        ticks.load(Ordering::SeqCst),
        "the thread kept running after cancel_periodic"
    );
}

#[test]
fn the_spin_lock_is_exclusive_and_scoped() {
    let lock = SpinLock::new();
    assert!(!lock.is_locked());
    {
        let guard = lock.lock();
        assert!(lock.is_locked());
        assert!(lock.try_lock().is_none());
        drop(guard);
    }
    assert!(!lock.is_locked());
    assert!(lock.try_lock().is_some());
}

#[test]
fn the_spin_lock_serialises_threads() {
    let lock = Arc::new(SpinLock::new());
    let shared = Arc::new(AtomicUsize::new(0));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let lock = Arc::clone(&lock);
            let shared = Arc::clone(&shared);
            thread::spawn(move || {
                for _ in 0..1000 {
                    let _guard = lock.lock();
                    let value = shared.load(Ordering::SeqCst);
                    shared.store(value + 1, Ordering::SeqCst);
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(shared.load(Ordering::SeqCst), 4000);
}

#[test]
fn the_same_vector_shares_the_lock() {
    let vector = MutexVector::new();
    let first = vector.lock(7);
    let second = vector.lock(7);
    assert!(first.is_locked());
    assert!(
        second.is_locked(),
        "the same vector must not exclude itself"
    );
    drop(first);
    drop(second);
    assert!(vector.try_lock(7).is_some());
}

#[test]
fn a_different_vector_has_to_wait() {
    let vector = Arc::new(MutexVector::new());
    let holder = vector.lock(1);
    let other = Arc::clone(&vector);
    let handle = thread::spawn(move || {
        let start = Instant::now();
        let _guard = other.lock(2);
        start.elapsed()
    });

    // vector 2 cannot enter while vector 1 is held
    thread::sleep(Duration::from_millis(20));
    drop(holder);
    let waited = handle.join().unwrap();
    assert!(
        waited >= Duration::from_millis(10),
        "vector 2 waited {waited:?}"
    );
}

#[test]
fn try_lock_refuses_another_vector_while_held() {
    let vector = MutexVector::new();
    let _holder = vector.lock(1);
    assert!(vector.try_lock(1).is_some(), "same vector is fine");
    assert!(vector.try_lock(2).is_none(), "another vector has to wait");
}

#[test]
fn the_vector_lock_is_released_by_drop() {
    let vector = MutexVector::new();
    {
        let guard = vector.lock(3);
        assert!(guard.is_locked());
    }
    assert!(vector.try_lock(4).is_some(), "the lock must be free again");
}
