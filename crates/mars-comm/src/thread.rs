//! `mars/comm/thread/` — threads, spin locks and the "vector" lock, on top of
//! `std::thread` / `std::sync`.
//!
//! `mutex.h`, `lock.h` and `condition.h` are just platform switches around
//! pthreads, and `atomic_oper.h` is a set of helpers over the platform atomics:
//! `std::sync` and `std::sync::atomic` are those, so they are re-exported here
//! instead of being reimplemented. What is left — the named `Thread`, the
//! delayed/periodic starts, `ThreadUtil`, `SpinLock` and `MutexVector` — is
//! ported below.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub use std::sync::atomic;
/// `comm::Condition` — the condition variable of `std::sync`.
pub use std::sync::Condvar as Condition;
/// `comm::Mutex` — the non-reentrant mutex of `std::sync`.
pub use std::sync::Mutex;
/// `comm::ScopedLock` — the RAII guard of [`Mutex`].
pub type ScopedLock<'a, T> = std::sync::MutexGuard<'a, T>;

/// `detail::Runnable`: anything that can be run on a [`Thread`].
pub trait Runnable: Send {
    /// `Runnable::run()`.
    fn run(&mut self);
}

impl<F: FnMut() + Send> Runnable for F {
    fn run(&mut self) {
        self()
    }
}

/// `detail::transform(op)` — turn a closure into a boxed [`Runnable`].
pub fn runnable<F: FnMut() + Send + 'static>(op: F) -> Box<dyn Runnable> {
    Box::new(op)
}

/// `ThreadUtil`.
pub struct ThreadUtil;

impl ThreadUtil {
    /// `ThreadUtil::yield()` — give up the rest of the time slice.
    pub fn yield_now() {
        thread::yield_now()
    }

    /// `ThreadUtil::sleep(sec)`.
    pub fn sleep(sec: u64) {
        thread::sleep(Duration::from_secs(sec))
    }

    /// `ThreadUtil::usleep(usec)`.
    pub fn usleep(usec: u64) {
        thread::sleep(Duration::from_micros(usec))
    }

    /// `ThreadUtil::currentthreadid()` — a stable id of the calling thread.
    pub fn current_thread_id() -> thread::ThreadId {
        thread::current().id()
    }
}

/// A spin lock, the counterpart of `comm::SpinLock`.
///
/// The C++ reaches for `OSSpinLock`/`os_unfair_lock`/`pthread_spinlock_t`
/// depending on the platform; this one is an `AtomicBool` with an exponential
/// back-off, which is what those do short of the kernel path.
#[derive(Debug, Default)]
pub struct SpinLock {
    locked: AtomicBool,
}

impl SpinLock {
    /// An unlocked lock.
    pub const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
        }
    }

    /// `SpinLock::lock()`.
    pub fn lock(&self) -> SpinLockGuard<'_> {
        let mut spins = 0u32;
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            spins += 1;
            if spins < 16 {
                std::hint::spin_loop();
            } else {
                thread::yield_now();
            }
        }
        SpinLockGuard { lock: self }
    }

    /// `SpinLock::trylock()`.
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_>> {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| SpinLockGuard { lock: self })
    }

    /// Whether the lock is held right now.
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
}

/// `ScopedSpinLock` — releases the lock when it goes out of scope.
#[derive(Debug)]
pub struct SpinLockGuard<'a> {
    lock: &'a SpinLock,
}

impl Drop for SpinLockGuard<'_> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

/// `comm::MutexVector` — a lock that is shared by everyone who asks for the
/// same `vector` and exclusive between different vectors.
///
/// STN uses it to serialise "all connections of one network" without
/// serialising every network against each other.
#[derive(Debug)]
pub struct MutexVector {
    inner: Arc<MutexVectorInner>,
}

#[derive(Debug, Default)]
struct MutexVectorState {
    /// The vector currently holding the lock.
    vector: i32,
    /// How many holders it has.
    count: i32,
}

#[derive(Debug)]
struct MutexVectorInner {
    state: Mutex<MutexVectorState>,
    cond: Condition,
}

impl MutexVector {
    /// An unlocked vector lock; the C++ starts at vector `0` with no holder.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(MutexVectorInner {
                state: Mutex::new(MutexVectorState::default()),
                cond: Condition::new(),
            }),
        }
    }

    /// `ScopedMutexVector(mutex_vector, vector)` — blocks until `vector` can be
    /// held, then returns the RAII guard.
    pub fn lock(&self, vector: i32) -> MutexVectorGuard<'_> {
        let mut guard = MutexVectorGuard {
            inner: &self.inner,
            vector,
            locked: false,
        };
        guard.lock();
        guard
    }

    /// `ScopedMutexVector::TryLock()` — `None` instead of blocking.
    pub fn try_lock(&self, vector: i32) -> Option<MutexVectorGuard<'_>> {
        let mut guard = MutexVectorGuard {
            inner: &self.inner,
            vector,
            locked: false,
        };
        if guard.try_lock() {
            Some(guard)
        } else {
            None
        }
    }
}

impl Default for MutexVector {
    fn default() -> Self {
        Self::new()
    }
}

/// `ScopedMutexVector`.
#[derive(Debug)]
pub struct MutexVectorGuard<'a> {
    inner: &'a MutexVectorInner,
    vector: i32,
    locked: bool,
}

impl MutexVectorGuard<'_> {
    /// `ScopedMutexVector::IsLocked()`.
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// `ScopedMutexVector::Lock()`.
    pub fn lock(&mut self) {
        assert!(!self.locked, "ScopedMutexVector::Lock() while locked");
        let mut state = self.inner.state.lock().unwrap();
        if state.vector == self.vector {
            state.count += 1;
            self.locked = true;
            return;
        }
        while state.count > 0 && state.vector != self.vector {
            state = self.inner.cond.wait(state).unwrap();
        }
        state.vector = self.vector;
        state.count += 1;
        self.locked = true;
        drop(state);
        self.inner.cond.notify_all();
    }

    /// `ScopedMutexVector::TryLock()`.
    pub fn try_lock(&mut self) -> bool {
        if self.locked {
            return false;
        }
        let mut state = self.inner.state.lock().unwrap();
        if state.vector == self.vector {
            state.count += 1;
            self.locked = true;
            return true;
        }
        if state.count > 0 {
            return false;
        }
        state.vector = self.vector;
        state.count = 1;
        self.locked = true;
        drop(state);
        self.inner.cond.notify_all();
        true
    }

    /// `ScopedMutexVector::UnLock()`.
    pub fn unlock(&mut self) {
        if !self.locked {
            return;
        }
        let mut state = self.inner.state.lock().unwrap();
        state.count -= 1;
        self.locked = false;
        if state.count <= 0 {
            drop(state);
            self.inner.cond.notify_all();
        }
    }
}

impl Drop for MutexVectorGuard<'_> {
    fn drop(&mut self) {
        if self.locked {
            self.unlock();
        }
    }
}

/// `comm::Thread` — a named thread with an optional delayed or periodic start.
///
/// The C++ keeps a reference-counted `RunnableReference` so the target can be
/// replaced between runs; the port takes the closure at start time, which is
/// what every caller in mars actually does (`Thread(boost::bind(...), "name")`
/// or `thread.start(op)`).
/// Clears the `running` flag when the thread body ends, however it ends.
///
/// A panicking callback unwinds past everything after it in the body, so the
/// flag used to stay `true` forever: `is_running()` then refused every later
/// `start` until somebody called `join()`.
struct RunningGuard(Arc<AtomicBool>);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug)]
pub struct Thread {
    name: Option<String>,
    handle: Option<thread::JoinHandle<()>>,
    running: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

impl Thread {
    /// `Thread(name)` — a thread without a target yet.
    pub fn new(name: Option<&str>) -> Self {
        Self {
            name: name.map(str::to_owned),
            handle: None,
            running: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// `Thread::start()`, after the C++ `thread.start(op)`: `false` when the
    /// thread was already running and nothing was started.
    pub fn start<F>(&mut self, op: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        if self.is_running() {
            return false;
        }
        self.detach_previous();
        self.cancel.store(false, Ordering::SeqCst);
        self.spawn(|cancel, _running| {
            let _ = cancel;
            op();
        })
        .is_ok()
    }

    /// `Thread::start_after(after)` — run once, `after` milliseconds from now.
    /// [`Thread::cancel_after`] aborts the wait.
    pub fn start_after<F>(&mut self, after_ms: u64, op: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        if self.is_running() {
            return false;
        }
        self.detach_previous();
        self.cancel.store(false, Ordering::SeqCst);
        let cancel = Arc::clone(&self.cancel);
        self.spawn(move |_, _| {
            if !sleep_until_cancelled(Duration::from_millis(after_ms), &cancel) {
                return;
            }
            op();
        })
        .is_ok()
    }

    /// `Thread::start_periodic(after, periodic)` — wait `after` ms, then run
    /// every `periodic` ms until [`Thread::cancel_periodic`].
    pub fn start_periodic<F>(&mut self, after_ms: u64, period_ms: u64, op: F) -> bool
    where
        F: FnMut() + Send + 'static,
    {
        if self.is_running() {
            return false;
        }
        self.detach_previous();
        self.cancel.store(false, Ordering::SeqCst);
        let cancel = Arc::clone(&self.cancel);
        let mut op = op;
        self.spawn(move |_, _| {
            if !sleep_until_cancelled(Duration::from_millis(after_ms), &cancel) {
                return;
            }
            loop {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                op();
                if !sleep_until_cancelled(Duration::from_millis(period_ms), &cancel) {
                    break;
                }
            }
        })
        .is_ok()
    }

    /// `Thread::cancel_after()` — abort a pending delayed start.
    pub fn cancel_after(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// `Thread::cancel_periodic()` — stop after the current iteration.
    pub fn cancel_periodic(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// `Thread::isruning()`.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// `Thread::join()` — waits for the thread to finish.
    pub fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.running.store(false, Ordering::SeqCst);
    }

    /// The name the thread was created with.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn detach_previous(&mut self) {
        // Rust has no detach: dropping the handle is the equivalent, and the
        // C++ detaches the previous tid so it can create a new one.
        self.handle.take();
        self.running.store(false, Ordering::SeqCst);
    }

    fn spawn<F>(&mut self, body: F) -> std::io::Result<()>
    where
        F: FnOnce(Arc<AtomicBool>, Arc<AtomicBool>) + Send + 'static,
    {
        let running = Arc::clone(&self.running);
        let cancel = Arc::clone(&self.cancel);
        let thread_running = Arc::clone(&self.running);
        let mut builder = thread::Builder::new();
        if let Some(name) = &self.name {
            builder = builder.name(name.clone());
        }

        // `running` has to be true before `start` answers: a second `start`
        // that arrives before the child is scheduled would otherwise still
        // read `false`, detach this handle and launch a second callback.
        self.running.store(true, Ordering::SeqCst);
        match builder.spawn(move || {
            let _running = RunningGuard(thread_running);
            body(cancel, running);
        }) {
            Ok(handle) => {
                self.handle = Some(handle);
                Ok(())
            }
            Err(e) => {
                // Nothing was started, so nothing is running.
                self.running.store(false, Ordering::SeqCst);
                Err(e)
            }
        }
    }
}

/// Sleeps in slices so a cancellation is picked up; `false` when cancelled.
fn sleep_until_cancelled(duration: Duration, cancel: &AtomicBool) -> bool {
    const SLICE: Duration = Duration::from_millis(1);
    let mut left = duration;
    while !cancel.load(Ordering::SeqCst) {
        if left <= SLICE {
            thread::sleep(left);
            break;
        }
        thread::sleep(SLICE);
        left -= SLICE;
    }
    !cancel.load(Ordering::SeqCst)
}
