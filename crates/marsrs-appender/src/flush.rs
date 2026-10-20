//! The [`Future`] an awaited drain is: `appender_flush` and
//! [`crate::appender_flush_instance`] hand one back instead of holding the
//! thread that asked for it, and what waits for the drain is a thread of
//! their own.
//!
//! Nothing here needs a runtime. The port carries no async dependency — it is
//! a port of a C++ library that has no notion of one — and a drain is a
//! blocking call that belongs on a thread, so what a caller awaits is a
//! `Future` written against [`std::thread::spawn`] and a [`Waker`]: it runs
//! on any executor, and asks nothing of the app but the `await`.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread;

/// What the drain and the future it answers to share: whether the drain is
/// over, and the one task that asked to be told when it is.
struct Shared {
    drained: bool,
    waker: Option<Waker>,
}

/// A drain that has been asked for and not waited on yet.
///
/// [`Future`], and not a blocking call: `appender_flush` hands this
/// back so that a caller in an async context can wait for the records to
/// reach the disk without parking whatever thread its executor gave it — the
/// drain runs on a thread of this one's, and the future is Ready once that
/// thread is done.
///
/// Two things it does not do. It does nothing until it is polled — a `Future`
/// that is dropped unpolled never drains, and a caller that wants the drain
/// whatever happens wants `Xlog::flush_now`. And it is not
/// cancelled with the task that asked for it: the drain is already running on
/// a thread that holds its own handle on the appender, so dropping this leaves
/// it to finish.
#[must_use = "a flush that is not awaited does not drain"]
pub struct Flush {
    /// What to run, taken out on the first poll and handed to the thread.
    /// `None` afterwards, which is what makes a second poll wait rather than
    /// start a second drain of the same appender.
    drain: Option<Box<dyn FnOnce() + Send>>,

    shared: Arc<Mutex<Shared>>,
}

impl Flush {
    /// A drain of nothing: what a handle with no appender behind it answers,
    /// so that the caller's `await` is an `await` and not a second shape to
    /// match on.
    pub(crate) fn noop() -> Self {
        Self::new(|| {})
    }

    /// What to run on the thread, as a closure and not as an appender: the
    /// one thing every caller has in common is "the drain, whenever it is
    /// polled".
    pub(crate) fn new(drain: impl FnOnce() + Send + 'static) -> Self {
        Self {
            drain: Some(Box::new(drain)),
            shared: Arc::new(Mutex::new(Shared {
                drained: false,
                waker: None,
            })),
        }
    }
}

impl Future for Flush {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // `get_mut`, and not a projection: nothing in `Flush` is pinned, so
        // the future is `Unpin` and the state is behind a `Mutex` because the
        // thread writes to it out of poll's sight.
        let this = self.get_mut();
        let mut shared = this
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if shared.drained {
            return Poll::Ready(());
        }

        shared.waker = Some(cx.waker().clone());
        let Some(drain) = this.drain.take() else {
            return Poll::Pending;
        };
        drop(shared);

        let shared = Arc::clone(&this.shared);
        // One thread per drain, and the drain is a short one: a pool would be
        // a pool the port has to own, expire and shut down, for work that is
        // one lock, one write and one `fflush`.
        thread::spawn(move || {
            // What makes the drain over is the drain ending, and not the drain
            // succeeding: a drain that panicked is one the caller is told
            // about too, or the `await` of it never comes back — the panic is
            // on its own thread, and this future is the only thing holding the
            // caller. So the end is written by a guard and not by the lines
            // after the call, which an unwind never reaches.
            let done = Drained::of(&shared);
            drain();
            drop(done);
        });

        Poll::Pending
    }
}

/// The end of the drain, written whether the drain came back or unwound: the
/// flag and the wake, which is all the future has left to do once the thread
/// is done with the closure.
struct Drained<'a> {
    shared: &'a Arc<Mutex<Shared>>,
}

impl<'a> Drained<'a> {
    fn of(shared: &'a Arc<Mutex<Shared>>) -> Self {
        Self { shared }
    }
}

impl Drop for Drained<'_> {
    fn drop(&mut self) {
        let mut shared = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        shared.drained = true;
        let waker = shared.waker.take();
        drop(shared);
        // Woken with the lock let go: `wake` can run the task on this thread,
        // and the task's next question is the lock's.
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::task::{Context, Waker};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::Flush;

    /// What the caller awaited is the drain ending, and not the drain
    /// succeeding: a drain that panicked is one the `await` of it has to come
    /// back from, or an app that asked for a flush before it exited waits for
    /// a thread that is never going to answer.
    #[test]
    fn a_drain_that_panicked_is_one_the_caller_is_told_about() {
        let ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        let flush = Flush::new(move || {
            flag.store(true, Ordering::SeqCst);
            panic!("a drain that did not come back");
        });

        let mut flush = pin!(flush);
        let mut context = Context::from_waker(Waker::noop());
        assert!(
            flush.as_mut().poll(&mut context).is_pending(),
            "the drain is on its own thread and is not over yet"
        );

        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until {
            if flush.as_mut().poll(&mut context).is_ready() {
                assert!(ran.load(Ordering::SeqCst), "and it did run");
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("an await of a drain that panicked never came back");
    }
}
