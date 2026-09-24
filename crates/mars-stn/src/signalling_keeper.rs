//! `mars/stn/src/signalling_keeper.cc` — the signalling that keeps a mapping
//! open while the app is waiting for something.
//!
//! `Keep()` starts it, `OnNetWorkDataChanged()` keeps it going for `keepTime`
//! after the last touch and posts the next buffer `period` later, and `Stop()`
//! ends it. What the port leaves out is the UDP client: the C++ sends the
//! signalling buffer over a `comm::UdpClient` bound to the long link's ip and
//! port, and only falls back to `fun_send_signalling_buffer_` when it was built
//! with `use_UDP = false`. A socket is not part of this repository, so what is
//! here is the `fun_send_signalling_buffer_` path, and the call the C++ posts
//! to its message queue is a due time the host can compare its own clock
//! against.
//!
//! `g_period` and `g_keepTime` are `static` in the C++ — one strategy for the
//! whole process — so [`set_strategy`] is a free function, not a method.

use std::sync::{Mutex, OnceLock};

use mars_comm::tickcount::gettickcount;

use crate::longlink::SIGNALKEEP_CMDID;

/// `g_period` — the default period between two signalling buffers, in
/// milliseconds.
pub const DEFAULT_PERIOD: u64 = 5_000;
/// `g_keepTime` — the default time the signalling is kept up, in milliseconds.
pub const DEFAULT_KEEP_TIME: u64 = 20_000;

/// `g_period` / `g_keepTime`, which the C++ keeps in two `static unsigned int`s.
fn strategy() -> &'static Mutex<(u64, u64)> {
    static STRATEGY: OnceLock<Mutex<(u64, u64)>> = OnceLock::new();
    STRATEGY.get_or_init(|| Mutex::new((DEFAULT_PERIOD, DEFAULT_KEEP_TIME)))
}

/// `SignallingKeeper::SetStrategy(period, keep_time)` — in milliseconds, for
/// every keeper in the process.
///
/// A `0` is refused: the C++ asserts on it and answers with `xerror2` without
/// moving either value.
pub fn set_strategy(period: u64, keep_time: u64) {
    if period == 0 || keep_time == 0 {
        return;
    }
    let mut strategy = strategy()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *strategy = (period, keep_time);
}

/// `g_period` — what [`set_strategy`] set, or [`DEFAULT_PERIOD`].
pub fn period() -> u64 {
    strategy()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .0
}

/// `g_keepTime` — what [`set_strategy`] set, or [`DEFAULT_KEEP_TIME`].
pub fn keep_time() -> u64 {
    strategy()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .1
}

/// `fun_send_signalling_buffer_`.
///
/// The C++ calls it with `(KNullAtuoBuffer, KNullAtuoBuffer, cmdid)`: both
/// buffers are the empty `KNullAtuoBuffer` every time, so the port hands over
/// the cmdid alone. The `unsigned int` it answers with is what
/// `LongLink::SendWhenNoData` returns, and `__SendSignallingBuffer` ignores it.
pub type SendSignalling = dyn FnMut(u32) -> u32 + Send;

/// `SignallingKeeper`.
pub struct SignallingKeeper {
    send: Option<Box<SendSignalling>>,
    /// `last_touch_time_` — `None` before the first [`SignallingKeeper::keep`].
    last_touch_time: Option<u64>,
    /// `keeping_`
    keeping: bool,
    /// `postid_` — `MessageQueue::AsyncInvokeAfter(g_period, …)`: the reading
    /// the posted call is due at. `__OnTimeOut` does not cancel it, so it stays
    /// until [`SignallingKeeper::stop`] or the next
    /// [`SignallingKeeper::on_network_data_changed_at`] does.
    next_due: Option<u64>,
    /// How many signalling buffers went out. The C++ counts nothing, but the
    /// UDP path sends without [`SendSignalling`] being called at all, so the
    /// count is the port's.
    sent: u64,
}

impl SignallingKeeper {
    pub fn new() -> Self {
        Self {
            send: None,
            last_touch_time: None,
            keeping: false,
            next_due: None,
            sent: 0,
        }
    }

    /// `fun_send_signalling_buffer_ = …`.
    pub fn set_send(&mut self, send: impl FnMut(u32) -> u32 + Send + 'static) {
        self.send = Some(Box::new(send));
    }

    /// `fun_send_signalling_buffer_ = NULL` — nothing goes out, but the keeper
    /// still keeps time.
    pub fn clear_send(&mut self) {
        self.send = None;
    }

    /// `keeping_`
    pub fn is_keeping(&self) -> bool {
        self.keeping
    }

    /// `last_touch_time_`
    pub fn last_touch_time(&self) -> Option<u64> {
        self.last_touch_time
    }

    /// The reading the posted call is due at, `None` when there is none.
    pub fn due_time(&self) -> Option<u64> {
        self.next_due
    }

    /// How many signalling buffers went out.
    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// `Keep()` — `last_touch_time_ = ::gettickcount()`.
    pub fn keep(&mut self) {
        self.keep_at(gettickcount());
    }

    /// The same, with the reading handed in.
    ///
    /// The first touch sends a buffer at once; a touch while the keeper is
    /// already keeping only moves the time the `keepTime` is measured from.
    pub fn keep_at(&mut self, now: u64) {
        self.last_touch_time = Some(now);
        if !self.keeping {
            self.send_signalling_buffer();
            self.keeping = true;
        }
    }

    /// `Stop()`.
    ///
    /// Only a keeper that is keeping *and* has a post outstanding stops — that
    /// is what the C++'s `if (keeping_ && postid_ != KNullPost)` does, and it
    /// means a [`SignallingKeeper::keep`] that never saw network data is still
    /// keeping afterwards.
    pub fn stop(&mut self) {
        if self.keeping && self.next_due.is_some() {
            self.keeping = false;
            self.next_due = None;
        }
    }

    /// `OnNetWorkDataChanged(_, _, _)`.
    pub fn on_network_data_changed(&mut self) {
        self.on_network_data_changed_at(gettickcount());
    }

    /// The same, with the reading handed in: while the keeper is keeping and
    /// its `keepTime` has not run out, the next buffer is posted `period`
    /// later; once it has, the signalling stops.
    pub fn on_network_data_changed_at(&mut self, now: u64) {
        if !self.keeping {
            return;
        }
        // `xassert2(now >= last_touch_time_)`, and the C++ treats a clock that
        // went backwards the same way it treats a `keepTime` that ran out.
        let last = match self.last_touch_time {
            Some(last) => last,
            None => {
                self.keeping = false;
                return;
            }
        };
        if now < last || now.saturating_sub(last) > keep_time() {
            self.keeping = false;
            return;
        }
        // `CancelMessage(postid_)` + `AsyncInvokeAfter(g_period, …)`
        self.next_due = Some(now.saturating_add(period()));
    }

    /// `__OnTimeOut` — what the posted call does: send another buffer.
    pub fn on_timeout(&mut self) {
        self.send_signalling_buffer();
    }

    /// `__SendSignallingBuffer` — `gDefaultLongLinkEncoder.signal_keep_cmdid()`,
    /// over `fun_send_signalling_buffer_`.
    fn send_signalling_buffer(&mut self) {
        if let Some(send) = self.send.as_mut() {
            let _ = send(SIGNALKEEP_CMDID);
        }
        self.sent += 1;
    }
}

impl Default for SignallingKeeper {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SignallingKeeper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignallingKeeper")
            .field("last_touch_time", &self.last_touch_time)
            .field("keeping", &self.keeping)
            .field("next_due", &self.next_due)
            .field("sent", &self.sent)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn the_first_touch_sends_a_buffer_at_once() {
        let mut keeper = SignallingKeeper::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recording = Arc::clone(&seen);
        keeper.set_send(move |cmdid| {
            recording
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(cmdid);
            0
        });
        assert!(!keeper.is_keeping());

        keeper.keep_at(1_000);
        assert!(keeper.is_keeping());
        assert_eq!(keeper.last_touch_time(), Some(1_000));
        assert_eq!(
            *seen.lock().unwrap_or_else(|e| e.into_inner()),
            vec![SIGNALKEEP_CMDID]
        );
        assert_eq!(keeper.sent(), 1);

        // keeping again only moves the time, it does not send
        keeper.keep_at(2_000);
        assert_eq!(keeper.sent(), 1);
        assert_eq!(keeper.last_touch_time(), Some(2_000));
    }

    #[test]
    fn a_zero_period_or_keep_time_is_refused() {
        let guard = crate::test_lock();
        set_strategy(0, 30_000);
        assert_eq!((period(), keep_time()), (DEFAULT_PERIOD, DEFAULT_KEEP_TIME));
        set_strategy(1_000, 0);
        assert_eq!((period(), keep_time()), (DEFAULT_PERIOD, DEFAULT_KEEP_TIME));

        set_strategy(1_000, 3_000);
        assert_eq!((period(), keep_time()), (1_000, 3_000));
        set_strategy(DEFAULT_PERIOD, DEFAULT_KEEP_TIME);
        drop(guard);
    }

    #[test]
    fn network_data_posts_the_next_buffer_a_period_later() {
        let guard = crate::test_lock();
        set_strategy(1_000, 3_000);
        let mut keeper = SignallingKeeper::new();
        keeper.keep_at(1_000);

        keeper.on_network_data_changed_at(1_500);
        assert_eq!(keeper.due_time(), Some(2_500));
        assert!(keeper.is_keeping());

        // the timeout sends, and does not cancel the post
        keeper.on_timeout();
        assert_eq!(keeper.sent(), 2);
        assert_eq!(keeper.due_time(), Some(2_500));

        // and the next data moves it
        keeper.on_network_data_changed_at(2_000);
        assert_eq!(keeper.due_time(), Some(3_000));

        set_strategy(DEFAULT_PERIOD, DEFAULT_KEEP_TIME);
        drop(guard);
    }

    #[test]
    fn a_keep_time_that_ran_out_stops_the_signalling() {
        let guard = crate::test_lock();
        set_strategy(1_000, 3_000);
        let mut keeper = SignallingKeeper::new();
        keeper.keep_at(1_000);

        // exactly keepTime after the touch is still inside it
        keeper.on_network_data_changed_at(1_000 + 3_000);
        assert!(keeper.is_keeping());
        assert_eq!(keeper.due_time(), Some(4_000 + 1_000));

        // one millisecond more is not
        keeper.on_network_data_changed_at(1_000 + 3_000 + 1);
        assert!(!keeper.is_keeping());

        // and nothing is posted while it is not keeping
        keeper.on_network_data_changed_at(5_000);
        assert_eq!(keeper.due_time(), Some(4_000 + 1_000));

        // a clock that went backwards is treated the same way
        let mut keeper = SignallingKeeper::new();
        keeper.keep_at(5_000);
        keeper.on_network_data_changed_at(4_000);
        assert!(!keeper.is_keeping());

        set_strategy(DEFAULT_PERIOD, DEFAULT_KEEP_TIME);
        drop(guard);
    }

    #[test]
    fn stop_only_ends_a_keeper_that_has_a_post_outstanding() {
        let mut keeper = SignallingKeeper::new();
        keeper.keep_at(1_000);
        keeper.stop();
        assert!(keeper.is_keeping(), "nothing was posted yet");

        keeper.on_network_data_changed_at(1_000);
        keeper.stop();
        assert!(!keeper.is_keeping());
        assert_eq!(keeper.due_time(), None);
    }

    #[test]
    fn a_cleared_send_keeps_the_time_but_sends_nothing() {
        let mut keeper = SignallingKeeper::new();
        keeper.set_send(|_| 7);
        keeper.clear_send();
        keeper.keep_at(1_000);
        assert!(keeper.is_keeping());
        assert_eq!(keeper.sent(), 1, "the buffer still went out");
    }
}
