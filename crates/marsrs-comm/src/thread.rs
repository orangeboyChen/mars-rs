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
use std::time::{Duration, Instant};

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
            // Saturation, and not `+= 1`: the count only ever decides
            // whether to spin or to yield, so a thread that has failed
            // `u32::MAX` times keeps yielding — where an overflow is a
            // panic in a debug build, in the middle of taking a lock.
            spins = spins.saturating_add(1);
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

/// `comm::Thread` — a named thread with an optional delayed or periodic start.
///
/// The C++ keeps a reference-counted `RunnableReference` so the target can be
/// replaced between runs; the port takes the closure at start time, which is
/// what every caller in mars actually does (`Thread(boost::bind(...), "name")`
/// or `thread.start(op)`).
#[derive(Debug)]
pub struct Thread {
    name: Option<String>,
    handle: Option<thread::JoinHandle<()>>,
    running: Arc<AtomicBool>,
    cancel: Arc<CancelSignal>,
}

impl Thread {
    /// `Thread(name)` — a thread without a target yet.
    pub fn new(name: Option<&str>) -> Self {
        Self {
            name: name.map(str::to_owned),
            handle: None,
            running: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(CancelSignal::new()),
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
        self.cancel.reset();
        self.spawn(|_cancel, _running| op()).is_ok()
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
        self.cancel.reset();
        let cancel = Arc::clone(&self.cancel);
        self.spawn(move |_, _| {
            if !cancel.wait(Duration::from_millis(after_ms)) {
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
        self.cancel.reset();
        let cancel = Arc::clone(&self.cancel);
        let mut op = op;
        self.spawn(move |_, _| {
            if !cancel.wait(Duration::from_millis(after_ms)) {
                return;
            }
            loop {
                if cancel.is_cancelled() {
                    break;
                }
                op();
                if !cancel.wait(Duration::from_millis(period_ms)) {
                    break;
                }
            }
        })
        .is_ok()
    }

    /// `Thread::cancel_after()` — abort a pending delayed start.
    pub fn cancel_after(&mut self) {
        self.cancel.cancel();
    }

    /// `Thread::cancel_periodic()` — stop after the current iteration.
    pub fn cancel_periodic(&mut self) {
        self.cancel.cancel();
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
        F: FnOnce(Arc<CancelSignal>, Arc<AtomicBool>) + Send + 'static,
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

/// The wait is one `wait_timeout` on a [`Condition`] and not a run of short
/// sleeps, which is what a start thirty seconds from now used to cost: a
/// wake-up every millisecond, thirty thousand of them, and a cancellation
/// that went unnoticed for up to a millisecond after it was asked for.
///
/// A thousand years: what [`CancelSignal::wait`] waits for when the wait it
/// was asked for does not fit in an [`Instant`].
const UNREACHABLE_WAIT: Duration = Duration::from_secs(60 * 60 * 24 * 365 * 1000);

/// What a delayed or periodic start waits on: the flag [`Thread::cancel_after`]
/// and [`Thread::cancel_periodic`] set, and the condition the waiter sleeps
/// on — the C++'s `condtime`.
#[derive(Debug)]
struct CancelSignal {
    cancelled: AtomicBool,
    lock: Mutex<()>,
    wake: Condition,
}

impl CancelSignal {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            lock: Mutex::new(()),
            wake: Condition::new(),
        }
    }

    /// `false` when the wait was cancelled, whether that happened before it
    /// began or while it was being waited out.
    fn wait(&self, duration: Duration) -> bool {
        // `Instant` has a ceiling, and `now + duration` past it panics rather
        // than overflowing: a wait that far out is a wait nothing outlives, so
        // it is put where the clock can hold it instead.
        let deadline = Instant::now()
            .checked_add(duration)
            .unwrap_or_else(|| Instant::now() + UNREACHABLE_WAIT);
        // The flag is read under the lock, and a canceller takes the lock
        // before it notifies: a cancel that lands between the read and the
        // wait is therefore one the wait knows about before it sleeps.
        let mut guard = self.lock.lock().unwrap();
        while !self.cancelled.load(Ordering::SeqCst) {
            let now = Instant::now();
            if deadline <= now {
                break;
            }
            // A wake-up that came early — spurious, or notified by a start
            // that has since been cancelled — is waited out again, so what
            // is left of the wait is what is left of it.
            let (next, _) = self.wake.wait_timeout(guard, deadline - now).unwrap();
            guard = next;
        }
        !self.cancelled.load(Ordering::SeqCst)
    }

    /// `Thread::cancel_after()` / `Thread::cancel_periodic()`.
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let _guard = self.lock.lock().unwrap();
        self.wake.notify_all();
    }

    /// What a new start does with it.
    fn reset(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }

    /// Whether a cancellation is pending, which a periodic start asks before
    /// it runs its next iteration.
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}
