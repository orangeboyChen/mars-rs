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

use std::sync::{Arc, Mutex};

use crate::message_queue::{
    broadcast_message, cancel_message, install_message_handler, uninstall_message_handler, Message,
    MessagePost, MessageTiming, MessageTitle,
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

/// A one-shot timer.
pub struct Alarm {
    handler: Option<crate::message_queue::MessageHandler>,
    post: Option<MessagePost>,
    status: Status,
    after: u32,
    start_time: u64,
    end_time: u64,
}

impl Alarm {
    /// `Alarm(op, inthread)` — the alarm runs `target` when it fires.
    pub fn new<F>(queue: crate::message_queue::MessageQueueId, target: F) -> Self
    where
        F: FnMut() + Send + Sync + 'static,
    {
        let target = Arc::new(Mutex::new(target));
        let runner = Arc::clone(&target);
        let handler = install_message_handler(
            move |message: &mut Message| {
                if message.title != ALARM_MESSAGE_TITLE && message.title != ALARM_SYSTEM_TITLE {
                    return;
                }
                (runner.lock().unwrap())();
            },
            true,
            queue,
        );
        Self {
            handler: Some(handler),
            post: None,
            status: Status::Init,
            after: 0,
            start_time: 0,
            end_time: 0,
        }
    }

    /// `Alarm::Start(after)` — `false` when the alarm is already waiting.
    pub fn start(&mut self, after_ms: u32) -> bool {
        if let Some(post) = self.post {
            // already waiting: the C++ refuses to start a second time
            if crate::message_queue::found_message(&post) {
                return false;
            }
        }
        let queue = self
            .handler
            .map(|h| h.queue)
            .unwrap_or(crate::message_queue::KDefQueueID);
        let post = broadcast_message(
            queue,
            Message::new(ALARM_MESSAGE_TITLE, "Alarm.broadcast"),
            MessageTiming::After(after_ms as u64),
        );
        if post == crate::message_queue::KNullPost {
            return false;
        }
        self.post = Some(post);
        self.status = Status::Start;
        self.after = after_ms;
        self.start_time = gettickcount();
        self.end_time = 0;
        true
    }

    /// `Alarm::Cancel()`.
    pub fn cancel(&mut self) -> bool {
        if let Some(post) = self.post.take() {
            cancel_message(&post);
        }
        if self.status == Status::Init {
            return true;
        }
        self.status = Status::Cancel;
        self.end_time = gettickcount();
        true
    }

    /// `Alarm::IsWaiting()`.
    pub fn is_waiting(&self) -> bool {
        self.status == Status::Start
    }

    /// `Alarm::Status()`.
    pub fn status(&self) -> Status {
        self.status
    }

    /// `Alarm::After()`.
    pub fn after(&self) -> u32 {
        self.after
    }

    /// `Alarm::ElapseTime()` — 0 while the alarm has not finished.
    pub fn elapse_time(&self) -> u64 {
        if self.end_time == 0 {
            0
        } else {
            self.end_time.saturating_sub(self.start_time)
        }
    }

    /// Called by the message queue when the alarm fires: moves the status on.
    ///
    /// The port's handler runs the target directly, so this only has to be
    /// called by a caller that drives the alarm itself.
    pub fn mark_fired(&mut self) {
        self.status = Status::OnAlarm;
        self.end_time = gettickcount();
        self.post = None;
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

        assert!(alarm.start(20));
        assert!(alarm.is_waiting());
        assert_eq!(alarm.status(), Status::Start);
        assert!(!alarm.start(20), "a waiting alarm must not start again");

        assert!(
            !RunLoop::dispatch_timeout(queue, Duration::from_millis(5)),
            "not due yet"
        );
        assert_eq!(fired.load(Ordering::SeqCst), 0);
        assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(300)));
        alarm.mark_fired();
        assert_eq!(fired.load(Ordering::SeqCst), 1);
        assert_eq!(alarm.status(), Status::OnAlarm);
        assert!(alarm.elapse_time() >= 20);
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
