//! `mars/comm/alarm.h` — a one-shot timer on top of the message queue.
//!
//! `Alarm::Start(after)` posts a timed broadcast message; when the queue
//! dispatches it, the alarm runs its target. `Cancel()` cancels the pending
//! message, and `Status()` reports where the alarm is.
//!
//! Two C++ details are deliberately different:
//!
//! * the C++ runs the target on its own `Thread` by default (`inthread`), the
//!   port runs it on whichever thread drains the queue — which is what every
//!   caller that passes `_inthread = false` gets today, and what makes the
//!   alarm testable without a thread of its own;
//! * the Android wakelock bookkeeping (`startAlarm`/`stopAlarm`,
//!   `WakeUpLock`) is a platform feature of the C++ and has no counterpart
//!   here.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::message_queue::{
    broadcast_message, cancel_message, install_message_handler, uninstall_message_handler, Message,
    MessagePost, MessageQueueId, MessageTiming, MessageTitle,
};
use crate::tickcount::gettickcount;

/// `Alarm::KALARM_MESSAGETITLE`.
const ALARM_MESSAGE_TITLE: MessageTitle = MessageTitle(0x1F1FF);
/// `Alarm::KALARM_SYSTEMTITLE`.
const ALARM_SYSTEM_TITLE: MessageTitle = MessageTitle(0x1F1F1E);

/// The four states of `Alarm::Status()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// `kInit` — never started.
    Init,
    /// `kStart` — waiting for its due time.
    Start,
    /// `kCancel` — cancelled before it fired.
    Cancel,
    /// `kOnAlarm` — it fired.
    OnAlarm,
}

/// `Alarm::INVAILD_SEQ` — `seq_ == 0` means "not waiting".
const INVAILD_SEQ: u64 = 0;

/// One id per started alarm: the message that comes back carries it, and the
/// handler only reacts to its own.
static NEXT_SEQ: AtomicU64 = AtomicU64::new(1);

/// What the alarm and the handler it installed both have to move on.
///
/// The handler runs on whichever thread drains the queue, so it cannot borrow
/// the [`Alarm`]: the two share this instead.
struct AlarmState {
    status: Status,
    after: u32,
    start_time: u64,
    end_time: u64,
    /// `seq_` of the pending message; [`INVAILD_SEQ`] when nothing is waiting.
    seq: u64,
    post: Option<MessagePost>,
}

impl Default for AlarmState {
    fn default() -> Self {
        Self {
            status: Status::Init,
            after: 0,
            start_time: 0,
            end_time: 0,
            seq: INVAILD_SEQ,
            post: None,
        }
    }
}

/// A one-shot timer.
pub struct Alarm {
    handler: Option<crate::message_queue::MessageHandler>,
    state: Arc<Mutex<AlarmState>>,
}

impl Alarm {
    /// `Alarm(op, inthread)` — the alarm runs `target` when it fires.
    pub fn new<F>(queue: crate::message_queue::MessageQueueId, target: F) -> Self
    where
        F: FnMut() + Send + Sync + 'static,
    {
        let target = Arc::new(Mutex::new(target));
        let state = Arc::new(Mutex::new(AlarmState::default()));
        let runner = Arc::clone(&target);
        let shared = Arc::clone(&state);
        let handler = install_message_handler(
            move |message: &mut Message| {
                if message.title != ALARM_MESSAGE_TITLE && message.title != ALARM_SYSTEM_TITLE {
                    return;
                }
                // `seq_ != any_cast<int64_t>(body1)`: an alarm is a broadcast
                // message on a shared queue, so without this every alarm of
                // the queue would run for every alarm that comes due.
                let Some(seq) = message.body1.as_ref().and_then(|b| b.downcast_ref::<u64>()) else {
                    return;
                };
                let Some(from) = message
                    .body2
                    .as_ref()
                    .and_then(|b| b.downcast_ref::<MessageQueueId>())
                else {
                    return;
                };
                if *from != queue {
                    return;
                }
                {
                    let mut alarm = shared.lock().unwrap();
                    if alarm.seq != *seq {
                        return;
                    }
                    // `Alarm::OnAlarm` before it runs the target.
                    alarm.status = Status::OnAlarm;
                    alarm.end_time = gettickcount();
                    alarm.seq = INVAILD_SEQ;
                    alarm.post = None;
                }
                (runner.lock().unwrap())();
            },
            true,
            queue,
        );
        Self {
            handler: Some(handler),
            state,
        }
    }

    /// `Alarm::Start(after)` — `false` when the alarm is already waiting.
    pub fn start(&mut self, after_ms: u32) -> bool {
        let queue = self
            .handler
            .map(|h| h.queue)
            .unwrap_or(crate::message_queue::KDefQueueID);
        let mut alarm = self.state.lock().unwrap();
        // `INVAILD_SEQ != seq_`: already waiting, and the C++ refuses to
        // start a second time.
        if alarm.seq != INVAILD_SEQ {
            return false;
        }
        let seq = NEXT_SEQ.fetch_add(1, Ordering::SeqCst);
        let start_time = gettickcount();
        let post = broadcast_message(
            queue,
            Message::new(ALARM_MESSAGE_TITLE, "Alarm.broadcast")
                .with_body1(seq)
                .with_body2(queue),
            MessageTiming::After(after_ms as u64),
        );
        if post == crate::message_queue::KNullPost {
            return false;
        }
        alarm.post = Some(post);
        alarm.seq = seq;
        alarm.status = Status::Start;
        alarm.after = after_ms;
        alarm.start_time = start_time;
        alarm.end_time = 0;
        true
    }

    /// `Alarm::Cancel()`.
    pub fn cancel(&mut self) -> bool {
        let mut alarm = self.state.lock().unwrap();
        if let Some(post) = alarm.post.take() {
            cancel_message(&post);
        }
        // `INVAILD_SEQ == seq_`: nothing was waiting, so the status stays
        // where it is — a fired alarm is not turned into a cancelled one.
        if alarm.seq == INVAILD_SEQ {
            return true;
        }
        alarm.status = Status::Cancel;
        alarm.end_time = gettickcount();
        alarm.seq = INVAILD_SEQ;
        true
    }

    /// `Alarm::IsWaiting()`.
    pub fn is_waiting(&self) -> bool {
        self.status() == Status::Start
    }

    /// `Alarm::Status()`.
    pub fn status(&self) -> Status {
        self.state.lock().unwrap().status
    }

    /// `Alarm::After()`.
    pub fn after(&self) -> u32 {
        self.state.lock().unwrap().after
    }

    /// `Alarm::ElapseTime()` — 0 while the alarm has not finished.
    pub fn elapse_time(&self) -> u64 {
        let alarm = self.state.lock().unwrap();
        if alarm.end_time == 0 {
            0
        } else {
            alarm.end_time.saturating_sub(alarm.start_time)
        }
    }

    /// What `Alarm::OnAlarm` does before it runs the target.
    ///
    /// The handler installed by [`Alarm::new`] already does this when the queue
    /// dispatches the alarm; a caller that drives the alarm itself has to say
    /// so explicitly.
    pub fn mark_fired(&mut self) {
        let mut alarm = self.state.lock().unwrap();
        alarm.status = Status::OnAlarm;
        alarm.end_time = gettickcount();
        alarm.seq = INVAILD_SEQ;
        alarm.post = None;
    }
}

impl Drop for Alarm {
    fn drop(&mut self) {
        let _ = self.cancel();
        if let Some(handler) = self.handler.take() {
            uninstall_message_handler(&handler);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_queue::{create_message_queue, destroy_message_queue, RunLoop};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn an_alarm_fires_after_its_delay() {
        let queue = create_message_queue();
        let fired = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&fired);
        let mut alarm = Alarm::new(queue, move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        // 200 ms, not 20: the short dispatch below asks for 5 ms but a
        // loaded machine hands it more than that, and what it spends there
        // is part of the delay this alarm is waiting out.
        assert!(alarm.start(200));
        assert!(alarm.is_waiting());
        assert_eq!(alarm.status(), Status::Start);
        assert!(!alarm.start(200), "a waiting alarm must not start again");

        assert!(
            !RunLoop::dispatch_timeout(queue, Duration::from_millis(5)),
            "not due yet"
        );
        assert_eq!(fired.load(Ordering::SeqCst), 0, "nothing ran yet");
        assert!(RunLoop::dispatch_timeout(
            queue,
            Duration::from_millis(2_000)
        ));
        assert_eq!(fired.load(Ordering::SeqCst), 1);
        // Dispatching the alarm has to move its state on, not just run the
        // target: it used to stay kStart, waiting, with no elapsed time.
        assert_eq!(alarm.status(), Status::OnAlarm);
        assert!(!alarm.is_waiting());
        // the delay is 200 ms, but the two clocks (the message's due time and
        // `gettickcount`) need not round the same way
        assert!(alarm.elapse_time() >= 190);
        // A cancel after it fired leaves it fired, not cancelled.
        assert!(alarm.cancel());
        assert_eq!(alarm.status(), Status::OnAlarm);
        destroy_message_queue(queue);
    }

    #[test]
    fn two_alarms_on_one_queue_fire_on_their_own() {
        let queue = create_message_queue();
        let soon = Arc::new(AtomicUsize::new(0));
        let later = Arc::new(AtomicUsize::new(0));
        let soon_counter = Arc::clone(&soon);
        let later_counter = Arc::clone(&later);

        let mut first = Alarm::new(queue, move || {
            soon_counter.fetch_add(1, Ordering::SeqCst);
        });
        let mut second = Alarm::new(queue, move || {
            later_counter.fetch_add(1, Ordering::SeqCst);
        });

        assert!(first.start(10));
        assert!(second.start(60_000));

        // Both alarms are broadcast messages on the same queue, so without the
        // id check the first one to come due used to run both targets.
        assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(300)));
        assert_eq!(soon.load(Ordering::SeqCst), 1);
        assert_eq!(later.load(Ordering::SeqCst), 0, "the wrong alarm fired");
        assert!(
            second.is_waiting(),
            "the second alarm must still be waiting"
        );
        assert_eq!(second.status(), Status::Start);

        assert!(first.cancel());
        assert!(second.cancel());
        destroy_message_queue(queue);
    }

    #[test]
    fn a_manually_driven_alarm_can_be_marked_fired() {
        let queue = create_message_queue();
        let mut alarm = Alarm::new(queue, || {});
        assert!(alarm.start(60_000));
        alarm.mark_fired();
        assert_eq!(alarm.status(), Status::OnAlarm);
        assert!(!alarm.is_waiting());
        // It is startable again: `seq_` went back to INVAILD_SEQ.
        assert!(alarm.start(10));
        alarm.cancel();
        destroy_message_queue(queue);
    }

    #[test]
    fn a_far_away_alarm_is_not_dispatched_by_a_short_wait() {
        let queue = create_message_queue();
        let fired = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&fired);
        let mut alarm = Alarm::new(queue, move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        // 2 s versus a 5 ms window: a loaded runner cannot make this flaky
        assert!(alarm.start(2_000));
        assert!(
            !RunLoop::dispatch_timeout(queue, Duration::from_millis(5)),
            "not due yet"
        );
        assert_eq!(fired.load(Ordering::SeqCst), 0);
        alarm.cancel();
        destroy_message_queue(queue);
    }

    #[test]
    fn a_cancelled_alarm_never_fires() {
        let queue = create_message_queue();
        let fired = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&fired);
        let mut alarm = Alarm::new(queue, move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        assert!(alarm.start(10));
        assert!(alarm.cancel());
        assert!(!alarm.is_waiting());
        assert_eq!(alarm.status(), Status::Cancel);

        assert!(!RunLoop::dispatch_timeout(queue, Duration::from_millis(50)));
        assert_eq!(fired.load(Ordering::SeqCst), 0);
        destroy_message_queue(queue);
    }

    #[test]
    fn an_alarm_that_never_started_is_not_waiting() {
        let queue = create_message_queue();
        let alarm = Alarm::new(queue, || {});
        assert_eq!(alarm.status(), Status::Init);
        assert!(!alarm.is_waiting());
        assert_eq!(alarm.after(), 0);
        assert_eq!(alarm.elapse_time(), 0);
        destroy_message_queue(queue);
    }
}
