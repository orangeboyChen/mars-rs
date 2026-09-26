//! `mars/comm/messagequeue/message_queue.h` — the async message queue.
//!
//! This is the primitive every higher layer of mars posts work onto: STN's
//! network callbacks, the alarm, and the JNI glue all go through it. The port
//! keeps the shapes of the C++ (`MessageQueue_t`, `MessageHandler_t`,
//! `MessagePost_t`, `MessageTitle_t`, `Message`, `MessageTiming`, `RunLoop`)
//! and the rules that matter:
//!
//! * a handler with `seq == 0` is a **broadcast** handler and receives every
//!   message of its queue, including the ones addressed to another handler;
//! * `post_message` returns a `MessagePost` that can be cancelled — by post,
//!   by handler, or by handler + title;
//! * `after`/`period` messages only run once their time has come;
//! * `RunLoop` drains the queue of the calling thread until its breaker says
//!   stop.
//!
//! What is deliberately left out: ANR reporting (the C++ logs and samples a
//! stack when a message runs for `anr_timeout` ms) and
//! `WaitForRunningLockEnd`, which only exists to make that sampling safe.

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::tickcount::gettickcount;

/// `MessageQueue::MessageQueue_t`.
pub type MessageQueueId = u64;
/// `MessageQueue::KInvalidQueueID`, spelled the way Rust spells a constant.
pub const INVALID_QUEUE_ID: MessageQueueId = 0;

/// `MessageQueue::KDefQueueID` — the queue messages are posted to when no
/// queue is named.
pub const DEFAULT_QUEUE_ID: MessageQueueId = 1;

/// `MessageQueue::MessageHandler_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MessageHandler {
    /// The owning queue.
    pub queue: MessageQueueId,
    /// `0` marks a broadcast handler.
    pub seq: u32,
}

impl MessageHandler {
    /// `MessageHandler_t::isbroadcast()`.
    pub fn is_broadcast(&self) -> bool {
        self.seq == 0
    }
}

/// `MessageQueue::MessagePost_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MessagePost {
    /// The handler the message was addressed to.
    pub reg: MessageHandler,
    /// The sequence number of this post.
    pub seq: u32,
}

/// `MessageQueue::KNullPost` — what a failed post returns.
pub const NULL_POST: MessagePost = MessagePost {
    reg: MessageHandler {
        queue: INVALID_QUEUE_ID,
        seq: 0,
    },
    seq: 0,
};

/// `MessageQueue::MessageTitle_t` — a small tag used to cancel a family of
/// messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MessageTitle(pub u64);

/// `MessageQueue::MessageTiming`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageTiming {
    /// Run as soon as the loop reaches it.
    #[default]
    Immediate,
    /// Run once, `after` ms from now.
    After(u64),
    /// Run every `period` ms, starting `after` ms from now.
    Period { after: u64, period: u64 },
}

/// `MessageQueue::Message`.
pub struct Message {
    /// `title`.
    pub title: MessageTitle,
    /// `body1` — arbitrary payload, or the async function.
    pub body1: Option<Box<dyn Any + Send>>,
    /// `body2`.
    pub body2: Option<Box<dyn Any + Send>>,
    /// The function an `AsyncInvoke` posted, if any.
    pub invoke: Option<Box<dyn FnMut() + Send>>,
    /// `msg_name`.
    pub name: String,
    /// `anr_timeout` — kept for parity, see the module note.
    pub anr_timeout: u64,
    /// `create_time`.
    pub create_time: u64,
    /// Set when the loop picks the message up.
    pub execute_time: u64,
}

impl Message {
    /// `Message(title, body1, body2, name)`.
    pub fn new(title: MessageTitle, name: impl Into<String>) -> Self {
        Self {
            title,
            body1: None,
            body2: None,
            invoke: None,
            name: name.into(),
            anr_timeout: 10 * 60 * 1000,
            create_time: gettickcount(),
            execute_time: 0,
        }
    }

    /// `Message(title, func, name)` — the `AsyncInvoke` form.
    pub fn with_invoke<F: FnMut() + Send + 'static>(
        title: MessageTitle,
        name: impl Into<String>,
        f: F,
    ) -> Self {
        let mut message = Self::new(title, name);
        message.invoke = Some(Box::new(f));
        message
    }

    /// Attaches `body1`.
    pub fn with_body1<T: Any + Send>(mut self, body: T) -> Self {
        self.body1 = Some(Box::new(body));
        self
    }

    /// Attaches `body2`.
    pub fn with_body2<T: Any + Send>(mut self, body: T) -> Self {
        self.body2 = Some(Box::new(body));
        self
    }
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Message")
            .field("title", &self.title)
            .field("name", &self.name)
            .field("create_time", &self.create_time)
            .finish_non_exhaustive()
    }
}

/// What the loop actually holds.
struct PostedMessage {
    post: MessagePost,
    /// `Message::title`, copied here so that a cancel can match on it without
    /// locking the payload — a handler cancelling its own periodic message
    /// runs while the dispatcher holds that very lock.
    title: MessageTitle,
    /// When an `After`/`Period` message becomes due.
    due: Option<Instant>,
    /// Set for `Period`: the delay between two runs.
    period: Option<Duration>,
    /// Shared with the dispatcher so a periodic message can stay in the queue
    /// while it runs (its payload cannot be cloned).
    message: Arc<Mutex<Message>>,
}

impl PostedMessage {
    /// The same post one period later, sharing the payload.
    fn clone_for_next_run(&self) -> Self {
        Self {
            post: self.post,
            title: self.title,
            due: self.due,
            period: self.period,
            message: Arc::clone(&self.message),
        }
    }
}

/// `MessageQueue::MessageHandler`, boxed so it can be taken out of the registry
/// while it runs — a handler posts messages of its own.
type HandlerFn = dyn Fn(&mut Message) + Send + Sync;

struct HandlerEntry {
    handler: Arc<HandlerFn>,
    recv_broadcast: bool,
}

struct QueueState {
    handlers: HashMap<u32, HandlerEntry>,
    messages: VecDeque<PostedMessage>,
    next_handler_seq: u32,
    next_post_seq: u32,
    running: bool,
}

impl QueueState {
    fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            messages: VecDeque::new(),
            next_handler_seq: 1,
            next_post_seq: 1,
            running: false,
        }
    }
}

struct Queue {
    state: Mutex<QueueState>,
    cond: Condvar,
}

impl Queue {
    fn new() -> Self {
        Self {
            state: Mutex::new(QueueState::new()),
            cond: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, QueueState> {
        self.state.lock().unwrap()
    }

    fn set_running(&self, running: bool) {
        self.lock().running = running;
        self.cond.notify_all();
    }
}

static QUEUES: Mutex<Option<HashMap<MessageQueueId, Arc<Queue>>>> = Mutex::new(None);
static NEXT_QUEUE_ID: Mutex<MessageQueueId> = Mutex::new(DEFAULT_QUEUE_ID + 1);

thread_local! {
    static CURRENT_QUEUE: std::cell::Cell<MessageQueueId> = const { std::cell::Cell::new(DEFAULT_QUEUE_ID) };
}

fn queues() -> &'static Mutex<Option<HashMap<MessageQueueId, Arc<Queue>>>> {
    &QUEUES
}

/// The registry, created on first use with the default queue already in it.
///
/// Every entry point has to go through here: `create_message_queue` used to
/// build the map itself, so when it happened to be the first queue call in
/// the process the default queue was missing for good and every later
/// `get_def_message_queue` post silently failed.
fn registry(
    guard: &mut Option<HashMap<MessageQueueId, Arc<Queue>>>,
) -> &mut HashMap<MessageQueueId, Arc<Queue>> {
    guard.get_or_insert_with(|| {
        let mut map = HashMap::new();
        map.insert(DEFAULT_QUEUE_ID, Arc::new(Queue::new()));
        map
    })
}

fn queue(id: MessageQueueId) -> Option<Arc<Queue>> {
    let mut guard = queues().lock().unwrap();
    registry(&mut guard).get(&id).cloned()
}

/// `MessageQueue::CurrentThreadMessageQueue()`.
pub fn current_thread_message_queue() -> MessageQueueId {
    CURRENT_QUEUE.with(|cell| cell.get())
}

/// Binds the calling thread to `id`, the way a `RunLoop` of that queue does.
pub fn set_current_thread_message_queue(id: MessageQueueId) {
    CURRENT_QUEUE.with(|cell| cell.set(id));
}

/// `MessageQueue::GetDefMessageQueue()`.
pub fn get_def_message_queue() -> MessageQueueId {
    DEFAULT_QUEUE_ID
}

/// Creates a new queue and returns its id.
pub fn create_message_queue() -> MessageQueueId {
    let id = {
        let mut next = NEXT_QUEUE_ID.lock().unwrap();
        *next += 1;
        *next
    };
    let mut guard = queues().lock().unwrap();
    registry(&mut guard).insert(id, Arc::new(Queue::new()));
    id
}

/// Drops a queue created by [`create_message_queue`].
pub fn destroy_message_queue(id: MessageQueueId) {
    if let Some(guard) = queues().lock().unwrap().as_mut() {
        guard.remove(&id);
    }
}

/// `MessageQueue::InstallMessageHandler(handler, recv_broadcast, id)`.
pub fn install_message_handler<F>(
    handler: F,
    recv_broadcast: bool,
    id: MessageQueueId,
) -> MessageHandler
where
    F: Fn(&mut Message) + Send + Sync + 'static,
{
    let Some(queue) = queue(id) else {
        return MessageHandler::default();
    };
    let mut state = queue.lock();
    let seq = state.next_handler_seq;
    state.next_handler_seq += 1;
    state.handlers.insert(
        seq,
        HandlerEntry {
            handler: Arc::new(handler),
            recv_broadcast,
        },
    );
    MessageHandler { queue: id, seq }
}

/// `MessageQueue::UnInstallMessageHandler`.
pub fn uninstall_message_handler(handler: &MessageHandler) {
    if let Some(queue) = queue(handler.queue) {
        queue.lock().handlers.remove(&handler.seq);
    }
}

/// `MessageQueue::InstallAsyncHandler(id)` — runs the `invoke` of every
/// message posted to it, which is what `AsyncInvoke` uses.
pub fn install_async_handler(id: MessageQueueId) -> MessageHandler {
    install_message_handler(
        |message: &mut Message| {
            if let Some(invoke) = message.invoke.as_mut() {
                invoke();
            }
        },
        false,
        id,
    )
}

/// `MessageQueue::DefAsyncInvokeHandler(id)`.
pub fn def_async_invoke_handler(id: MessageQueueId) -> MessageHandler {
    install_async_handler(id)
}

/// `MessageQueue::PostMessage(handler, message, timing)`.
pub fn post_message(
    handler: &MessageHandler,
    message: Message,
    timing: MessageTiming,
) -> MessagePost {
    let Some(queue) = queue(handler.queue) else {
        return NULL_POST;
    };
    if handler.seq != 0 && !queue.lock().handlers.contains_key(&handler.seq) {
        return NULL_POST;
    }
    let (due, period) = first_due(&timing);
    let mut state = queue.lock();
    let seq = state.next_post_seq;
    state.next_post_seq += 1;
    state.messages.push_back(PostedMessage {
        post: MessagePost { reg: *handler, seq },
        title: message.title,
        due,
        period,
        message: Arc::new(Mutex::new(message)),
    });
    drop(state);
    queue.cond.notify_all();
    MessagePost { reg: *handler, seq }
}

/// `MessageQueue::PostMessageAtFirst` — jumps the queue.
pub fn post_message_at_first(handler: &MessageHandler, message: Message) -> MessagePost {
    let post = post_message(handler, message, MessageTiming::Immediate);
    if let Some(queue) = queue(handler.queue) {
        let mut state = queue.lock();
        if let Some(index) = state.messages.iter().position(|m| m.post.seq == post.seq) {
            if index > 0 {
                let entry = state.messages.remove(index).unwrap();
                state.messages.push_front(entry);
            }
        }
    }
    post
}

/// `MessageQueue::SingletonMessage(replace, handler, message)` — at most one
/// pending message with this title. `replace` swaps the payload of the pending
/// one; otherwise the pending one wins and its post is returned.
pub fn singleton_message(replace: bool, handler: &MessageHandler, message: Message) -> MessagePost {
    let title = message.title;
    if let Some(queue) = queue(handler.queue) {
        let state = queue.lock();
        // The title is matched on the queue entry and the payload is only
        // locked to replace it, never to look at it: a periodic message is
        // still in the queue while it runs — the C++ hands the very same
        // `Message` to the handlers — and the dispatcher holds its lock. A
        // `Mutex` is not reentrant, so a handler asking for its own message
        // would stop the queue thread for good.
        if let Some(index) = state
            .messages
            .iter()
            .position(|m| m.post.reg.seq == handler.seq && m.title == title)
        {
            if replace {
                let pending = Arc::clone(&state.messages[index].message);
                // `try_lock`, for the same reason: a message that is being
                // dispatched right now is left alone rather than waited for.
                let locked = pending.try_lock();
                if let Ok(mut pending) = locked {
                    pending.body1 = message.body1;
                    pending.body2 = message.body2;
                    pending.invoke = message.invoke;
                }
            }
            return state.messages[index].post;
        }
    }
    post_message(handler, message, MessageTiming::Immediate)
}

/// `MessageQueue::BroadcastMessage(queue, message, timing)` — every handler of
/// the queue that accepts broadcasts (and `seq == 0` marks the post as one).
pub fn broadcast_message(
    id: MessageQueueId,
    message: Message,
    timing: MessageTiming,
) -> MessagePost {
    post_message(&MessageHandler { queue: id, seq: 0 }, message, timing)
}

/// `MessageQueue::FasterMessage` — a broadcast message that jumps the queue.
pub fn faster_message(handler: &MessageHandler, message: Message) -> MessagePost {
    post_message_at_first(
        &MessageHandler {
            queue: handler.queue,
            seq: 0,
        },
        message,
    )
}

/// `MessageQueue::CancelMessage(post)`.
pub fn cancel_message(post: &MessagePost) -> bool {
    let Some(queue) = queue(post.reg.queue) else {
        return false;
    };
    let mut state = queue.lock();
    let before = state.messages.len();
    state.messages.retain(|m| m.post != *post);
    let cancelled = state.messages.len() != before;
    drop(state);
    if cancelled {
        // A `wait_message` on this post is waiting for exactly this: its
        // completion condition just became true.
        queue.cond.notify_all();
    }
    cancelled
}

/// `MessageQueue::CancelMessage(handler)`.
pub fn cancel_message_by_handler(handler: &MessageHandler) {
    if let Some(queue) = queue(handler.queue) {
        let before = {
            let mut state = queue.lock();
            let before = state.messages.len();
            state.messages.retain(|m| m.post.reg.seq != handler.seq);
            before
        };
        if queue.lock().messages.len() != before {
            queue.cond.notify_all();
        }
    }
}

/// `MessageQueue::CancelMessage(handler, title)`.
pub fn cancel_message_by_handler_title(handler: &MessageHandler, title: MessageTitle) {
    let Some(queue) = queue(handler.queue) else {
        return;
    };
    // The title is matched on the queue entry, never through
    // `m.message.lock()`: a handler cancelling its own periodic message runs
    // while the dispatcher holds that lock, and a `Mutex` is not reentrant.
    let before = {
        let mut state = queue.lock();
        let before = state.messages.len();
        state
            .messages
            .retain(|m| !(m.post.reg.seq == handler.seq && m.title == title));
        before
    };
    if queue.lock().messages.len() != before {
        queue.cond.notify_all();
    }
}

/// `MessageQueue::FoundMessage(post)`.
pub fn found_message(post: &MessagePost) -> bool {
    queue(post.reg.queue)
        .map(|queue| queue.lock().messages.iter().any(|m| m.post == *post))
        .unwrap_or(false)
}

/// `MessageQueue::WaitMessage(post, timeout)` — `true` once the message has
/// been handled. A negative timeout waits forever.
pub fn wait_message(post: &MessagePost, timeout_ms: i64) -> bool {
    let Some(queue) = queue(post.reg.queue) else {
        return false;
    };
    let deadline =
        (timeout_ms >= 0).then(|| Instant::now() + Duration::from_millis(timeout_ms as u64));
    let mut state = queue.lock();
    loop {
        if !state.messages.iter().any(|m| m.post == *post) && !state.running {
            return true;
        }
        state = match deadline {
            Some(deadline) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return false;
                }
                let (guard, _) = queue.cond.wait_timeout(state, left).unwrap();
                guard
            }
            None => queue.cond.wait(state).unwrap(),
        };
    }
}

/// How many messages are pending on `id`.
pub fn pending_message_count(id: MessageQueueId) -> usize {
    queue(id)
        .map(|queue| queue.lock().messages.len())
        .unwrap_or(0)
}

/// `MessageQueue::RunLoop` — drains the queue of the calling thread.
pub struct RunLoop;

impl RunLoop {
    /// Runs until `breaker` returns `true`. `duty` is called once per turn,
    /// before the queue is looked at, exactly like the C++ RunLoop.
    pub fn run(id: MessageQueueId, mut breaker: impl FnMut() -> bool, mut duty: impl FnMut()) {
        let Some(queue) = queue(id) else { return };
        set_current_thread_message_queue(id);
        loop {
            if breaker() {
                break;
            }
            duty();
            Self::dispatch(&queue, Some(Duration::from_millis(1)));
        }
    }

    /// Handles at most one message, waiting up to `timeout` for one to be due.
    pub fn dispatch_timeout(id: MessageQueueId, timeout: Duration) -> bool {
        queue(id)
            .map(|queue| Self::dispatch(&queue, Some(timeout)))
            .unwrap_or(false)
    }

    fn dispatch(queue: &Arc<Queue>, timeout: Option<Duration>) -> bool {
        // Wait until a message is due, up to `timeout`. A wake-up is not the
        // end of the wait: only the deadline or a due message is, otherwise a
        // spurious wake-up reports "nothing to do" before an `After` message
        // is due.
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let (handlers, message) = {
            let mut state = queue.lock();
            let index = loop {
                let now = Instant::now();
                if let Some(index) = state
                    .messages
                    .iter()
                    .position(|m| m.due.is_none_or(|due| due <= now))
                {
                    break Some(index);
                }
                // The wait is capped by the earliest message that is still to
                // come, not only by the caller's timeout: an `After(40)` with
                // a 300 ms timeout has to run after 40 ms, and with no timeout
                // at all there is nothing else that would ever wake this up.
                let wait = match (state.messages.iter().filter_map(|m| m.due).min(), deadline) {
                    (Some(due), Some(deadline)) => due.min(deadline).saturating_duration_since(now),
                    (Some(due), None) => due.saturating_duration_since(now),
                    (None, Some(deadline)) => deadline.saturating_duration_since(now),
                    (None, None) => Duration::MAX,
                };
                if wait.is_zero() {
                    break None;
                }
                let (guard, _) = queue.cond.wait_timeout(state, wait).unwrap();
                state = guard;
            };
            let Some(index) = index else { return false };

            // Take the message out, re-arm it when it is periodic, and collect the
            // handlers to call — all while holding the same lock the wait ended
            // on, so that a handler can post or cancel from inside a message and
            // the index still names the message it was computed for.
            let Some(mut entry) = state.messages.remove(index) else {
                return false;
            };
            // `seq == 0` is a broadcast: only handlers that asked for it run.
            let addressed = entry.post.reg.seq;
            let is_broadcast = addressed == 0;
            if let Some(period) = entry.period {
                entry.due = Some(Instant::now() + period);
                state.messages.push_back(entry.clone_for_next_run());
            }
            let handlers: Vec<Arc<HandlerFn>> = state
                .handlers
                .iter()
                .filter(|(seq, entry)| **seq == addressed || (is_broadcast && entry.recv_broadcast))
                .map(|(_, entry)| Arc::clone(&entry.handler))
                .collect();
            state.running = true;
            (handlers, Arc::clone(&entry.message))
        };

        {
            let mut message = message.lock().unwrap();
            message.execute_time = gettickcount();
            for handler in handlers {
                handler(&mut message);
            }
        }

        queue.set_running(false);
        true
    }
}

fn first_due(timing: &MessageTiming) -> (Option<Instant>, Option<Duration>) {
    match *timing {
        MessageTiming::Immediate => (None, None),
        MessageTiming::After(after) => (Some(Instant::now() + Duration::from_millis(after)), None),
        MessageTiming::Period { after, period } => (
            Some(Instant::now() + Duration::from_millis(after)),
            Some(Duration::from_millis(period)),
        ),
    }
}
