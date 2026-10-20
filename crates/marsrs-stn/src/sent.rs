//! `StartTask` as a value: the task an app awaits instead of listening for.
//!
//! `stn_logic.h` gives the app `StartTask` and nothing to hold on to: the
//! answer of the task comes back as one of the eighteen questions —
//! `OnTaskEnd` — so a caller that wants it keeps the id it started the task
//! with and matches it against the id it is handed. That is a correlation
//! every caller writes by hand, and in Rust it was the only way to see a task
//! end at all.
//!
//! [`Sent`] is the other shape: one task, one value. [`StnLogic::send`]
//! starts the task and hands back a future whose output is the answer of the
//! server or the failure of the run, so what an app writes is
//!
//! ```no_run
//! # use marsrs_stn::{gen_task_id, StnLogic, Task};
//! # async fn example(stn: &std::sync::Arc<std::sync::Mutex<StnLogic>>) {
//! let mut task = Task::new(gen_task_id(), 100);
//! task.cgi = "/cgi-bin/hello".to_owned();
//! task.shortlink_host_list = vec!["example.com".to_owned()];
//!
//! // the lock is dropped at the semicolon: a `Sent` borrows nothing
//! let sent = stn.lock().unwrap().send(task, b"hello".to_vec());
//! let answer = sent.await.expect("the task came back");
//! # }
//! ```
//!
//! and nothing else — no `App` for the two questions a request/response task
//! is asked, and no id to match. The body it is given is what `Req2Buf` would
//! have been asked for, and the body of the answer is what `Buf2Resp` would
//! have been handed; a task that needs either question for something else
//! still implements [`crate::App`], which is what the questions fall through
//! to.
//!
//! What is still the caller's is the draining: a task leaves its queue only
//! when somebody calls [`StnLogic::run_pending`], which is the C++'s
//! message-queue thread and not a thread this port starts on its own.
//! [`Driver::spawn`] is one call that does it on a thread of this crate's; a
//! host with a loop of its own keeps it, and the two work together, because a
//! pass that ends a task wakes whoever awaited it. A [`Sent`] that nothing
//! drains stays [`Pending`](std::task::Poll::Pending) — the same way a task
//! that is started and never drained stays in its queue. A task the app
//! stopped, or one a core that was destroyed or cleared threw away, is not
//! waiting for a pass at all: no queue holds it any more, so it is answered
//! as cancelled on the spot.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use crate::stn_callback_bridge::CgiProfile;
use crate::task_profile::ErrCmdType;
use crate::StnLogic;

/// What a task came back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The bytes the server answered with, which is the body `Buf2Resp` would
    /// have been handed.
    pub body: Vec<u8>,
    /// The timings of the connect the task ran on.
    pub profile: CgiProfile,
}

/// The error code a task the app broke off is given, which is none of its own:
/// a cancellation is a failure, and it is the one [`Failure::Ended::err_code`]
/// answers `0` for where everything else gets a negative code.
const CANCELLED: i32 = 0;

/// Why a task has no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// There is no net core yet: [`StnLogic::create`] has not been through.
    NotCreated,
    /// No queue took the task, and nothing reported an end for it: a core that
    /// was released starts nothing and reports nothing, which is the `false`
    /// the C++ answers without a word to the app. A task the gates refuse the
    /// usual way is reported, and so it ends as [`Failure::Ended`].
    Refused,
    /// The task has no answer: where it failed, and the code it failed with.
    /// This is a task that ran and came back with nothing, and one the core
    /// refused on its way in and reported the way it reports a failed run.
    Ended {
        /// Where it failed.
        err_type: ErrCmdType,
        /// What went wrong: negative, and `0` when it was cancelled.
        err_code: i32,
        /// The timings of the connect, as far as it got. Boxed because the
        /// other two ends carry nothing at all: an enum is as large as its
        /// largest arm, and this one would make every failure a hundred bytes
        /// of profile it does not have.
        profile: Box<CgiProfile>,
    },
}

/// A task that has been started, and the value it ends with.
///
/// Made by [`StnLogic::send`]. It borrows nothing, so it can be awaited
/// wherever the app's own executor puts it — and it is answered by whoever
/// drains the queues, which is a [`Driver`] or the host's own
/// [`StnLogic::run_pending`] loop.
pub struct Sent {
    ends: Option<Arc<Mutex<Ends>>>,
    taskid: u32,
    /// What the task ended with before the first poll came, which is a task no
    /// queue took, and one whose end the call that started it already brought.
    ended: Option<Result<Answer, Failure>>,
}

impl std::fmt::Debug for Sent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sent")
            .field("taskid", &self.taskid)
            .field("is_awaiting", &self.ends.is_some())
            .field("is_answered", &self.ended.is_some())
            .finish()
    }
}

impl Future for Sent {
    type Output = Result<Answer, Failure>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(outcome) = this.ended.take() {
            return Poll::Ready(outcome);
        }
        let ends = match &this.ends {
            Some(ends) => Arc::clone(ends),
            None => return Poll::Ready(Err(Failure::Refused)),
        };
        let mut ends = locked(&ends);
        match ends.take(this.taskid) {
            Some(ended) => {
                this.ends = None;
                Poll::Ready(ended.into())
            }
            None => {
                ends.park(this.taskid, cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

impl Drop for Sent {
    fn drop(&mut self) {
        // a `Sent` that is dropped is a task nobody is awaiting: what it ends
        // with is nobody's to take, so it is not kept — the C++ keeps no record
        // of a task either, once the app that started it has let go
        if let Some(ends) = self.ends.take() {
            locked(&ends).forget(self.taskid);
        }
    }
}

impl Sent {
    /// A task a queue took, and an end nobody has taken yet.
    pub(crate) fn waiting(ends: Arc<Mutex<Ends>>, taskid: u32) -> Self {
        Self {
            ends: Some(ends),
            taskid,
            ended: None,
        }
    }

    /// A task that was over before there was a poll to answer.
    pub(crate) fn answered(outcome: Result<Answer, Failure>) -> Self {
        Self {
            ends: None,
            taskid: 0,
            ended: Some(outcome),
        }
    }
}

/// One end of a task, as it was handed to the app's `OnTaskEnd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ended {
    err_type: ErrCmdType,
    err_code: i32,
    profile: CgiProfile,
    body: Vec<u8>,
}

impl From<Ended> for Result<Answer, Failure> {
    fn from(ended: Ended) -> Self {
        if ended.err_type == ErrCmdType::Ok {
            Ok(Answer {
                body: ended.body,
                profile: ended.profile,
            })
        } else {
            Err(Failure::Ended {
                err_type: ended.err_type,
                err_code: ended.err_code,
                profile: Box::new(ended.profile),
            })
        }
    }
}

/// The tasks an [`StnLogic`] was asked to await, and what is known about each.
///
/// One of these is owned by the logic and shared with every [`Sent`] it made,
/// which is what lets a task end on the thread that drained the queue and be
/// answered on the thread that awaited it. A task is in here from
/// [`StnLogic::send`] until its end has been taken by the poll that was
/// waiting for it.
#[derive(Default)]
pub(crate) struct Ends {
    /// What each task is to send, which is what `Req2Buf` is answered with.
    requests: HashMap<u32, Vec<u8>>,
    /// What each task was answered with, which is what `Buf2Resp` recorded.
    responses: HashMap<u32, Vec<u8>>,
    /// The ends that have not been taken yet.
    ended: HashMap<u32, Ended>,
    /// Who is waiting on each.
    wakers: HashMap<u32, Waker>,
    /// The wakers a pass has made ready, which [`StnLogic`] wakes once the
    /// pass is over: a wake has to happen after the queue is let go, because
    /// the poll it schedules asks for the logic's own lock.
    wakes: Vec<Waker>,
}

impl std::fmt::Debug for Ends {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ends")
            .field("awaiting", &self.requests.len())
            .field("ended", &self.ended.len())
            .finish()
    }
}

impl Ends {
    /// A task that is being awaited, and the body it is to send.
    pub(crate) fn start(&mut self, taskid: u32, body: Vec<u8>) {
        self.requests.insert(taskid, body);
    }

    /// `Req2Buf` of a task that is being awaited: the body it was sent with,
    /// and [`None`] for a task nobody is waiting on, which is one the
    /// [`crate::App`] is still asked about.
    pub(crate) fn request(&mut self, taskid: u32) -> Option<Vec<u8>> {
        self.requests.get(&taskid).cloned()
    }

    /// `Buf2Resp` of a task that is being awaited: the answer is kept for
    /// whoever awaits it, and `true` is a task that is ours.
    pub(crate) fn answer(&mut self, taskid: u32, body: &[u8]) -> bool {
        if !self.requests.contains_key(&taskid) {
            return false;
        }
        self.responses.insert(taskid, body.to_vec());
        true
    }

    /// `OnTaskEnd` of any task: one that is being awaited is finished, and one
    /// that is not is nothing to us.
    pub(crate) fn finish(
        &mut self,
        taskid: u32,
        err_type: ErrCmdType,
        err_code: i32,
        profile: CgiProfile,
    ) {
        if !self.requests.contains_key(&taskid) {
            return;
        }
        let body = self.responses.remove(&taskid).unwrap_or_default();
        self.requests.remove(&taskid);
        self.ended.insert(
            taskid,
            Ended {
                err_type,
                err_code,
                profile,
                body,
            },
        );
        if let Some(waker) = self.wakers.remove(&taskid) {
            self.wakes.push(waker);
        }
    }

    /// The end of a task that was dropped rather than run out, which is what
    /// [`StnLogic::stop_task`] and a core that throws its queues away leave
    /// behind: no queue holds the task any more, so no pass is ever going to
    /// report it, and an await that was not answered here would stay
    /// [`Pending`](std::task::Poll::Pending) for the life of the process. The
    /// app is not asked about it, which is the C++'s own answer to `StopTask`.
    ///
    /// A task nobody is awaiting is left alone, the way [`Ends::finish`] leaves
    /// one alone.
    pub(crate) fn cancel(&mut self, taskid: u32) {
        self.finish(
            taskid,
            ErrCmdType::Canceld,
            CANCELLED,
            CgiProfile::default(),
        );
    }

    /// [`Ends::cancel`] for every task still being awaited.
    pub(crate) fn cancel_all(&mut self) {
        for taskid in self.pending() {
            self.cancel(taskid);
        }
    }

    /// The ids an end has not come for yet, which is every task an app may
    /// still be awaiting.
    fn pending(&self) -> Vec<u32> {
        self.requests.keys().copied().collect()
    }

    /// The end of a task, which is taken once and is the one thing that stops
    /// it being awaited.
    pub(crate) fn take(&mut self, taskid: u32) -> Option<Ended> {
        self.ended.remove(&taskid)
    }

    /// Who to wake when this task ends.
    pub(crate) fn park(&mut self, taskid: u32, waker: Waker) {
        self.wakers.insert(taskid, waker);
    }

    /// A task nobody is awaiting any more, which is one no queue took or one
    /// whose [`Sent`] was dropped: nothing of it is kept, whatever it ended
    /// with.
    pub(crate) fn forget(&mut self, taskid: u32) {
        self.requests.remove(&taskid);
        self.responses.remove(&taskid);
        self.ended.remove(&taskid);
        self.wakers.remove(&taskid);
    }

    /// The wakers the ends of the last pass made ready.
    pub(crate) fn wakes(&mut self) -> Vec<Waker> {
        std::mem::take(&mut self.wakes)
    }
}

/// The queues of an [`StnLogic`], drained on a thread of this crate's.
///
/// This is the C++'s message-queue thread, which this port does not start on
/// its own: what the thread did there is a call the host makes here, and a
/// host that makes it in a loop of its own needs no [`Driver`] at all. One is
/// for an app that awaits a task and has no loop — it is started by
/// [`Driver::spawn`] and stops when the [`Driver`] is dropped.
///
/// The thread drains the queues at the end of every slice, and not at the
/// delay [`StnLogic::due_delay`] answers, which is the number a *host*
/// schedules a run loop of its own with: how long it may wait is what that
/// delay says, and a host that is woken before it is up is a host that comes
/// back early. Nothing wakes this thread, so see `drain`.
#[derive(Debug)]
pub struct Driver {
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// The logic the thread is draining, which [`Driver::shutdown`] asks
    /// whether it can join by.
    logic: Arc<Mutex<StnLogic>>,
}

impl Driver {
    /// Drains `logic`'s queues on a thread of this crate's until the
    /// [`Driver`] is dropped.
    ///
    /// The logic is shared and not moved: a driver only ever asks it for a
    /// pass, so an app keeps its own [`Arc`] and goes on starting tasks
    /// through it.
    ///
    /// A thread that could not be started is a [`Driver`] that drains
    /// nothing, which is the one case in which an app that awaits a task
    /// hears nothing back: the queues are still the host's to drain.
    pub fn spawn(logic: Arc<Mutex<StnLogic>>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = Arc::clone(&stop);
            let logic = Arc::clone(&logic);
            std::thread::Builder::new()
                .name("marsrs-stn-driver".to_owned())
                .spawn(move || drain(&logic, &stop))
                .ok()
        };
        Self {
            stop,
            thread,
            logic,
        }
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let Some(thread) = self.thread.take() else {
            return;
        };
        // The thread is joined rather than left to the process: a task that
        // was mid-pass when the driver was dropped is one whose end nobody is
        // waiting for any more, and it is not one the host should find still
        // running after the drop came back.
        //
        // Unless the join cannot come back, which is the case in which this
        // call is holding the logic the thread is waiting for: an app that
        // drops its driver with the lock taken — and a callback of the pass
        // the thread is running now, which is a pass holding it — would wait
        // for a thread waiting for a lock this call holds, and neither would
        // end. So the lock is asked first, and a thread that is not one this
        // call can join is left to the stop flag, which is what ends it
        // either way.
        match self.logic.try_lock() {
            // the lock is free, so the thread is not in a pass it cannot come
            // back from: it is sleeping, or about to take the lock itself
            Ok(free) => drop(free),
            // a panic poisoned it, which is not a lock anybody holds
            Err(TryLockError::Poisoned(poisoned)) => drop(poisoned.into_inner()),
            // somebody holds it — this call, or another thread of the app —
            // and the pass the thread is waiting for is the one holding it
            Err(TryLockError::WouldBlock) => return,
        }
        let _joined = thread.join();
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// How long a driver sleeps before it looks at the queues again, and the
/// longest it goes without noticing that a [`Driver`] was dropped.
const SLICE_MS: u64 = 20;

fn drain(logic: &Arc<Mutex<StnLogic>>, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        {
            let mut logic = locked(logic);
            logic.run_pending();
        }
        // A pass at the end of every slice, and not at the delay
        // [`StnLogic::due_delay`] answers. That delay is the earliest alarm
        // the queues have armed — a first-package timeout, a retry, the next
        // heartbeat — and an alarm is not when there is work to do: an
        // answer a socket has already read, and a task another thread has
        // already started, arm no alarm at all, because this crate starts
        // no thread of a socket's own and a queue is only ever drained by a
        // pass. Sleeping out the delay is a ceiling on how late both are
        // seen, and on a quiet link it is a ceiling of tens of seconds,
        // which is a task answered with a timeout an answer had beaten.
        //
        // A host may sleep the delay, and the bridges hand it across for
        // that: a host is woken by its own sockets and by whatever else it
        // waits on, and a thread of this crate's is woken by nothing.
        std::thread::sleep(Duration::from_millis(SLICE_MS));
    }
}

/// The lock, without letting a panic in one thread take every thread with it.
fn locked<T: ?Sized>(lock: &Arc<Mutex<T>>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Waker};
    use std::time::{Duration, Instant};

    use super::{Answer, Driver, Ends, Failure, Sent, StnLogic, SLICE_MS};
    use crate::stn_callback_bridge::CgiProfile;
    use crate::task_profile::{ConnectProfile, ErrCmdType};

    /// One poll, with a waker that wakes nobody: what a [`Sent`] parks is
    /// collected by [`Ends::wakes`] and made by [`StnLogic`] after the pass.
    fn poll(sent: &mut Sent) -> Poll<Result<Answer, Failure>> {
        let mut context = Context::from_waker(Waker::noop());
        Future::poll(Pin::new(sent), &mut context)
    }

    /// A task that is being awaited, and the end it has not come to yet.
    fn ends() -> Arc<Mutex<Ends>> {
        let ends = Arc::new(Mutex::new(Ends::default()));
        ends.lock().unwrap().start(7, b"ask".to_vec());
        ends
    }

    #[test]
    fn an_end_is_kept_until_the_task_that_awaited_it_takes_it() {
        let ends = ends();
        let mut sent = Sent::waiting(Arc::clone(&ends), 7);
        assert!(poll(&mut sent).is_pending(), "the task is out");

        // the body the task was sent with is what `Req2Buf` is answered with
        assert_eq!(ends.lock().unwrap().request(7), Some(b"ask".to_vec()));
        assert!(ends.lock().unwrap().answer(7, b"came back"));
        assert!(!ends.lock().unwrap().answer(8, b"not ours"));

        let profile = CgiProfile::of(&ConnectProfile::new());
        ends.lock().unwrap().finish(7, ErrCmdType::Ok, 0, profile);
        // the wake is not made inside the pass: the poll it schedules asks for
        // the lock the pass is still holding
        assert_eq!(ends.lock().unwrap().wakes().len(), 1);

        let mut sent = Sent::waiting(Arc::clone(&ends), 7);
        match poll(&mut sent) {
            Poll::Ready(Ok(answer)) => assert_eq!(answer.body, b"came back".to_vec()),
            other => panic!("the task ended: {other:?}"),
        }
        // and one end is taken once: nothing is left for a second taker
        assert!(ends.lock().unwrap().take(7).is_none());
    }

    #[test]
    fn a_task_that_failed_ends_with_where_and_how() {
        let ends = ends();
        ends.lock().unwrap().finish(
            7,
            ErrCmdType::Server,
            -500,
            CgiProfile::of(&ConnectProfile::new()),
        );
        let mut sent = Sent::waiting(ends, 7);
        match poll(&mut sent) {
            Poll::Ready(Err(Failure::Ended {
                err_type, err_code, ..
            })) => {
                assert_eq!(err_type, ErrCmdType::Server);
                assert_eq!(err_code, -500);
            }
            other => panic!("the task failed: {other:?}"),
        }
    }

    #[test]
    fn a_task_nobody_awaits_any_more_is_forgotten() {
        let ends = ends();
        let mut sent = Sent::waiting(Arc::clone(&ends), 7);
        assert!(poll(&mut sent).is_pending());
        drop(sent);
        assert_eq!(ends.lock().unwrap().request(7), None);
    }

    /// A task no queue holds any more — one the app stopped, or one a core that
    /// was cleared or destroyed threw away — is answered here or not at all:
    /// nothing is ever going to report it, so an end that was not made now
    /// would be a `Sent` that stays `Pending` for the life of the process.
    #[test]
    fn a_task_that_was_dropped_ends_as_cancelled_and_wakes_whoever_awaited_it() {
        let ends = ends();
        let mut sent = Sent::waiting(Arc::clone(&ends), 7);
        assert!(poll(&mut sent).is_pending());

        ends.lock().unwrap().cancel(7);
        assert_eq!(ends.lock().unwrap().wakes().len(), 1);

        let mut sent = Sent::waiting(Arc::clone(&ends), 7);
        match poll(&mut sent) {
            Poll::Ready(Err(Failure::Ended {
                err_type, err_code, ..
            })) => {
                assert_eq!(err_type, ErrCmdType::Canceld);
                assert_eq!(err_code, 0, "a cancellation carries no code of its own");
            }
            other => panic!("the task was cancelled: {other:?}"),
        }

        // and a task nobody is awaiting is not one there is an end to make
        let mut ends = Ends::default();
        ends.cancel(9);
        assert!(ends.take(9).is_none());
    }

    #[test]
    fn every_task_that_was_dropped_ends_as_cancelled() {
        let ends = Arc::new(Mutex::new(Ends::default()));
        ends.lock().unwrap().start(7, b"ask".to_vec());
        ends.lock().unwrap().start(8, b"ask".to_vec());
        ends.lock().unwrap().cancel_all();

        for taskid in [7, 8] {
            let mut sent = Sent::waiting(Arc::clone(&ends), taskid);
            match poll(&mut sent) {
                Poll::Ready(Err(Failure::Ended { err_type, .. })) => {
                    assert_eq!(err_type, ErrCmdType::Canceld);
                }
                other => panic!("task {taskid} was cancelled: {other:?}"),
            }
        }
    }

    #[test]
    fn an_end_nobody_awaited_is_kept_for_the_poll_that_comes_for_it() {
        // a task the two gates refused, which is over before there is a poll
        let mut sent = Sent::answered(Err(Failure::NotCreated));
        match poll(&mut sent) {
            Poll::Ready(Err(failure)) => assert_eq!(failure, Failure::NotCreated),
            other => panic!("the task never started: {other:?}"),
        }
    }

    #[test]
    fn a_driver_is_done_when_it_is_dropped() {
        let logic = Arc::new(Mutex::new(StnLogic::new()));
        let driver = Driver::spawn(Arc::clone(&logic));
        drop(driver);
        // the drop joins the thread, so the logic is this thread's again
        assert!(!logic.lock().unwrap().is_created());
    }

    /// The other drop: one made while the logic is held, by an app that is
    /// holding it or by a callback of the pass the thread is running — which
    /// is a pass holding it. The thread's next pass waits for that lock and
    /// the join waits for the thread, so a drop that joined would not come
    /// back at all; what ends the thread instead is the stop flag.
    #[test]
    fn a_driver_is_dropped_without_waiting_for_a_pass_that_cannot_end() {
        let logic = Arc::new(Mutex::new(StnLogic::new()));
        let driver = Driver::spawn(Arc::clone(&logic));
        let held = Arc::clone(&logic);
        let dropped = Arc::new(AtomicBool::new(false));
        let back = Arc::clone(&dropped);

        let holding = std::thread::spawn(move || {
            let _pass = held.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            // a slice, and then some: the thread is only ever waiting for the
            // lock between two passes, and it is the waiting for it that a
            // drop that joins cannot come back from
            std::thread::sleep(Duration::from_millis(SLICE_MS * 4));
            drop(driver);
            back.store(true, Ordering::SeqCst);
        });

        // A hang is not a failure any test reports, so the wait is bounded:
        // what is asked about is a drop that does not come back.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dropped.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "the drop of a driver whose logic is held is waiting for the thread, and the \
                 thread is waiting for the lock the drop is holding"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        holding.join().expect("the drop came back");
    }
}
