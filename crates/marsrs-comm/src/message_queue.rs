//! `mars/comm/messagequeue/message_queue.h` — the async message queue.
//!
//! This is the primitive the port posts work onto where the C++ does: the
//! alarm and the JNI glue both go through it. STN is not one of them — it
//! drives its own passes over its two task queues, so none of its
//! callbacks comes in here as a message. The port keeps the shapes of the
//! C++ (`MessageQueue_t`, `MessageHandler_t`, `MessagePost_t`,
//! `MessageTitle_t`, `Message`, `MessageTiming`, `RunLoop`) and the rules
//! that matter:
//!
//! * a handler with `seq == 0` is a **broadcast** handler and receives
//!   every message of its queue, including the ones addressed to another
//!   handler;
//! * `post_message` returns a `MessagePost` that can be cancelled — by
//!   post, by handler, or by handler + title;
//! * `after`/`period` messages only run once their time has come;
//! * `RunLoop` drains the queue of the calling thread until its breaker
//!   says stop.
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
    /// `title` — what a `CancelMessage` by title matches on, and what a
    /// `FasterMessage` or a `SingletonMessage` looks for.
    pub title: MessageTitle,
    /// `body1` — arbitrary payload, which a handler downcasts back to the
    /// type it was posted as.
    pub body1: Option<Box<dyn Any + Send>>,
    /// `body2` — the second payload, for a handler that needs two.
    pub body2: Option<Box<dyn Any + Send>>,
    /// The function an `AsyncInvoke` posted, if any. It is erased to a
    /// callable `FnMut` and not to [`Any`], because that is what it takes
    /// to run it: `Any` hands a value back only to a caller that names its
    /// concrete type, and a closure's type has no name. Upstream keeps it
    /// in `body1` — wrapped in a `shared_ptr` it can name — and any_casts
    /// it back out, a cast that comes back empty for any other payload. A
    /// field of its own needs no cast, and leaves `body1` and `body2` free
    /// for the payloads a handler downcasts.
    pub invoke: Option<Box<dyn FnMut() + Send>>,
    /// `msg_name`.
    pub name: String,
    /// `anr_timeout` — kept for parity, see the module note.
    pub anr_timeout: u64,
    /// `create_time` — when this message was built, in [`gettickcount`] ms.
    /// Nothing stamps it again on the way into the queue, so one built now
    /// and posted a minute from now is a minute old when it is queued: read
    /// it as the start of the message's life, and not of its wait in the
    /// queue. The C++ is no different — its `Message` constructors are the
    /// only writers — so posting is not where to move it.
    pub create_time: u64,
    /// Set when the loop picks the message up, in [`gettickcount`] ms.
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

    /// Carries `body` to the handler as [`Message::body1`], and hands `self`
    /// back so the calls chain. A handler reaches it by downcasting back to
    /// `T`, which is what the `Any` asks of it.
    pub fn with_body1<T: Any + Send>(mut self, body: T) -> Self {
        self.body1 = Some(Box::new(body));
        self
    }

    /// The same for [`Message::body2`].
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
    /// `Message::title`, copied here so that a cancel can match on it
    /// without locking the payload — a handler cancelling its own periodic
    /// message runs while the dispatcher holds that very lock.
    title: MessageTitle,
    /// When an `After`/`Period` message becomes due.
    due: Option<Instant>,
    /// Set for `Period`: the delay between two runs.
    period: Option<Duration>,
    /// Shared with the dispatcher so a periodic message can stay in the
    /// queue while it runs (its payload cannot be cloned).
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

/// `MessageQueue::MessageHandler`, boxed so it can be taken out of the
/// registry while it runs — a handler posts messages of its own.
type HandlerFn = dyn Fn(&mut Message) + Send + Sync;

struct HandlerEntry {
    /// The sequence number the handler was installed with, which is the one
    /// a `MessagePost` names it by.
    seq: u32,
    handler: Arc<HandlerFn>,
    recv_broadcast: bool,
}

struct QueueState {
    /// The handlers in the order they were installed.
    ///
    /// `std::list<HandlerWrapper*> lst_handler` of
    /// `comm/messagequeue/message_queue.cc`, which a dispatch walks from the
    /// front: with more than one handler for a message — a broadcast, or
    /// several alarms on one queue — the order they run in is the order they
    /// were installed in, and not whatever a map hands out.
    handlers: Vec<HandlerEntry>,
    messages: VecDeque<PostedMessage>,
    next_handler_seq: u32,
    next_post_seq: u32,
    /// The messages the queue is running, one entry per dispatch under way: a
    /// dispatch pushes its post before it calls the handlers and takes it out
    /// when they are done.
    ///
    /// `lst_runloop_info` of `comm/messagequeue/message_queue.cc` is a list
    /// with one `RunLoopInfo` — and so one `runing_message_id` — per run loop
    /// of the queue, which is to say per thread dispatching it. One post for
    /// the whole queue is the second of two concurrent dispatches overwriting
    /// the first, and whichever of the two handlers finished first reporting
    /// the other's message as gone while it was still running: `found_message`
    /// answered `false` for a message the queue was running, and a
    /// `wait_message` on it returned before its handler had finished.
    running_posts: Vec<MessagePost>,
}

impl QueueState {
    fn new() -> Self {
        Self {
            handlers: Vec::new(),
            messages: VecDeque::new(),
            next_handler_seq: 1,
            next_post_seq: 1,
            running_posts: Vec::new(),
        }
    }

    /// The sequence number of the next post, which is never 0 and never
    /// one that is still out.
    ///
    /// Wrapping, and not `+= 1`: the number is a `u32` the way the C++'s
    /// `MessagePost_t` is, so a queue that lives long enough runs it past
    /// `u32::MAX` — a panic in a debug build, and in a release one a post
    /// that is equal to an earlier post of the same handler, so
    /// `cancel_message` and `found_message` answer for a message the
    /// caller never asked about. 0 is skipped for the same reason: it is
    /// the `seq` of [`NULL_POST`] and of a broadcast handler, which is to
    /// say a post the queue never handed out.
    fn next_post_seq(&mut self) -> u32 {
        let seq = self.next_post_seq;
        // `max(1)` is the skip: a wrap to 0 moves on to 1 instead
        self.next_post_seq = seq.wrapping_add(1).max(1);
        seq
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

    /// The message is over: [`found_message`] stops reporting it and a
    /// [`wait_message`] on it can return, so everyone waiting on the queue is
    /// woken. Only `post` is taken out — a second dispatch of this queue is
    /// running a message of its own, and it is still running when this one is
    /// done.
    fn clear_running(&self, post: MessagePost) {
        let mut state = self.lock();
        state.running_posts.retain(|it| *it != post);
        drop(state);
        self.cond.notify_all();
    }
}

static QUEUES: Mutex<Option<HashMap<MessageQueueId, Arc<Queue>>>> = Mutex::new(None);
static NEXT_QUEUE_ID: Mutex<MessageQueueId> = Mutex::new(DEFAULT_QUEUE_ID + 1);

thread_local! {
    static CURRENT_QUEUE: std::cell::Cell<MessageQueueId> = const { std::cell::Cell::new(INVALID_QUEUE_ID) };
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
///
/// [`INVALID_QUEUE_ID`] when the calling thread owns no queue. The C++
/// (`comm/messagequeue/message_queue.cc`) derives the id from the thread id
/// and answers `KInvalidQueueID` for a thread that is not one of its queues,
/// so "am I the thread this queue runs on" is a question the answer names a
/// queue for. Starting the thread-local at [`DEFAULT_QUEUE_ID`] answered
/// "yes, the default one" on every thread instead, and a caller that posts
/// work to be run asynchronously then ran it inline — the wrong thread, and
/// not asynchronous.
pub fn current_thread_message_queue() -> MessageQueueId {
    CURRENT_QUEUE.with(|cell| cell.get())
}

/// Binds the calling thread to `id`, the way a `RunLoop` of that
/// queue does.
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
    state.handlers.push(HandlerEntry {
        seq,
        handler: Arc::new(handler),
        recv_broadcast,
    });
    MessageHandler { queue: id, seq }
}

/// `MessageQueue::UnInstallMessageHandler`.
pub fn uninstall_message_handler(handler: &MessageHandler) {
    if let Some(queue) = queue(handler.queue) {
        queue.lock().handlers.retain(|it| it.seq != handler.seq);
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
    post(handler, message, timing, false)
}

/// `MessageQueue::PostMessageAtFirst` — jumps the queue.
pub fn post_message_at_first(handler: &MessageHandler, message: Message) -> MessagePost {
    post(handler, message, MessageTiming::Immediate, true)
}

/// Both of the above: the insert and the notify are one step under the
/// queue's lock, which is what the C++'s `ScopedLock` around `push_front`
/// and `breaker->Notify` is.
///
/// Not two steps. `post_message_at_first` used to post and then move its
/// message to the head afterwards, and the notify of the post is between
/// the two: a dispatcher woken by it can take the lock in that window and
/// run the message that was already at the head, which is the one this
/// call exists to be run before.
fn post(
    handler: &MessageHandler,
    message: Message,
    timing: MessageTiming,
    at_front: bool,
) -> MessagePost {
    let Some(queue) = queue(handler.queue) else {
        return NULL_POST;
    };
    if handler.seq != 0 && !queue.lock().handlers.iter().any(|it| it.seq == handler.seq) {
        return NULL_POST;
    }
    let (due, period) = first_due(&timing);
    let mut state = queue.lock();
    let seq = state.next_post_seq();
    let entry = PostedMessage {
        post: MessagePost { reg: *handler, seq },
        title: message.title,
        due,
        period,
        message: Arc::new(Mutex::new(message)),
    };
    if at_front {
        state.messages.push_front(entry);
    } else {
        state.messages.push_back(entry);
    }
    drop(state);
    queue.cond.notify_all();
    MessagePost { reg: *handler, seq }
}

/// `MessageQueue::SingletonMessage(replace, handler, message)` — at most
/// one pending message with this title. `replace` swaps the payload of the
/// pending one; otherwise the pending one wins and its post is returned.
pub fn singleton_message(replace: bool, handler: &MessageHandler, message: Message) -> MessagePost {
    let title = message.title;
    let Some(queue) = queue(handler.queue) else {
        return NULL_POST;
    };
    let mut state = queue.lock();
    // An uninstalled handler has nothing left to deliver a message to, so a
    // message carrying its `reg` could never run: `post_message` answers
    // `NULL_POST` for the same reason, and matching a pending entry below
    // would hand back a post that no dispatch will ever pick up.
    if handler.seq != 0 && !state.handlers.iter().any(|it| it.seq == handler.seq) {
        return NULL_POST;
    }
    // Search and insertion are one step under the queue's lock, not two,
    // which is what `faster_message` says it does and why: two threads
    // asking for the same title when nothing is pending would each see an
    // empty queue and post a copy of its own, and the handler would run
    // twice — the one thing this call promises it does not do.
    let post = if let Some(index) = state
        .messages
        .iter()
        .position(|m| m.post.reg.seq == handler.seq && m.title == title)
    {
        if replace {
            // A new `Message` behind a new `Arc`, which is what the C++
            // does when it drops the pending wrapper and posts a fresh
            // one: whatever the replacement was asked for is what the
            // next dispatch hands to the handlers, and a dispatch that
            // is already under way keeps the copy it took.
            //
            // Writing through the lock instead needs `try_lock` for the
            // reason above, and a `try_lock` that fails drops the
            // payload on the floor: a handler that replaced its own
            // periodic message — the one case where the lock is always
            // held — silently kept logging the old one.
            //
            // The title is matched on the queue entry, and the payload is
            // never locked: a periodic message is still in the queue while
            // it runs — the dispatcher re-arms it before it calls the
            // handlers and holds the message's lock for as long as they
            // do — and a `Mutex` is not reentrant, so a handler asking for
            // its own message would stop the queue thread for good.
            state.messages[index].message = Arc::new(Mutex::new(message));
        }
        state.messages[index].post
    } else {
        let post = MessagePost {
            reg: *handler,
            seq: state.next_post_seq(),
        };
        state.messages.push_back(PostedMessage {
            post,
            title,
            due: None,
            period: None,
            message: Arc::new(Mutex::new(message)),
        });
        post
    };
    drop(state);
    // `content.breaker->Notify(lock)` of the C++, which it does on the way
    // out of every post: a thread sleeping on this queue until the next
    // message is due is waiting for this title too, and what it will be
    // handed is the payload a `replace` just swapped in.
    queue.cond.notify_all();
    post
}

/// `MessageQueue::BroadcastMessage(queue, message, timing)` — every
/// handler of the queue that accepts broadcasts (and `seq == 0` marks the
/// post as one).
pub fn broadcast_message(
    id: MessageQueueId,
    message: Message,
    timing: MessageTiming,
) -> MessagePost {
    post_message(&MessageHandler { queue: id, seq: 0 }, message, timing)
}

/// `MessageQueue::FasterMessage(handler, message)` — with the C++'s default
/// timing, which is "now".
///
/// It is addressed to `handler`, and not broadcast: upstream builds its
/// wrapper from the `_handlerid` it is handed
/// (`MessageWrapper(_handlerid, ...)` in
/// `comm/messagequeue/message_queue.cc`) and only `BroadcastMessage` posts
/// with `seq == 0`. Posting this one as a broadcast ran every handler that
/// had asked for broadcasts and *not* the one it was addressed to, so a
/// `FasterMessage` disappeared instead of being delivered.
///
/// What "faster" means is **not** jumping the queue — that is
/// [`post_message_at_first`]. Upstream looks for a message already pending
/// for the same handler with the same title (`Message::operator==` compares
/// the title and nothing else) and, since a message due now cannot be later
/// than one that is already waiting, replaces it: the pending payload is
/// dropped and its `post` is handed to the new one, so a post the caller is
/// holding keeps naming this message. Asking twice therefore runs the
/// handler once, with the payload of the second ask.
pub fn faster_message(handler: &MessageHandler, message: Message) -> MessagePost {
    let title = message.title;
    let Some(queue) = queue(handler.queue) else {
        return NULL_POST;
    };
    let mut state = queue.lock();
    // An uninstalled handler has nothing left to deliver a message to, so a
    // message carrying its `reg` could never run: `post_message` answers
    // `NULL_POST` for the same reason, and matching a pending entry below would
    // hand back a post that no dispatch will ever pick up.
    if handler.seq != 0 && !state.handlers.iter().any(|it| it.seq == handler.seq) {
        return NULL_POST;
    }
    // Search and insertion are one step under the queue's lock, not two:
    // two threads asking for the same message when nothing is pending would
    // each see an empty queue and post a copy of its own, and the handler
    // would run twice — which is the one thing this call promises it does
    // not do.
    if let Some(index) = state
        .messages
        .iter()
        .position(|m| m.post.reg == *handler && m.title == title)
    {
        let entry = state.messages.remove(index).expect("found above");
        // The old `post`, and the new payload: the pending copy of this
        // message was asked for and then superseded, so what runs is what
        // was asked for last, and not both.
        let replacement = PostedMessage {
            post: entry.post,
            title,
            due: None,
            period: None,
            message: Arc::new(Mutex::new(message)),
        };
        state.messages.push_back(replacement);
        drop(state);
        // `content.breaker->Notify(lock)` of the C++, which it does on the
        // way out of every post: the replacement is due now, and the
        // message it replaces may well have been due in a minute — which is
        // exactly what a thread waiting on this queue is sleeping until.
        // Left unnotified, the replacement sits there until that wait runs
        // out on its own.
        queue.cond.notify_all();
        return entry.post;
    }
    // `MessagePost` of the C++'s `post_message`, done here rather than by
    // calling it: the sequence number and the insertion have to happen
    // under the lock the search above took, and `MessageTiming::Immediate`
    // is what "faster" posts — a message that is due now.
    let seq = state.next_post_seq();
    state.messages.push_back(PostedMessage {
        post: MessagePost { reg: *handler, seq },
        title,
        due: None,
        period: None,
        message: Arc::new(Mutex::new(message)),
    });
    drop(state);
    queue.cond.notify_all();
    MessagePost { reg: *handler, seq }
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
    // `m.message.lock()`: a handler cancelling its own periodic
    // message runs while the dispatcher holds that lock, and a `Mutex`
    // is not reentrant.
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
///
/// `true` for a message that is pending **or** running: the C++ compares
/// `post` with the `runing_message_id` of every run loop of the queue
/// before it walks `lst_message`, so a caller that asks from inside the
/// message — the handler, or something the handler called — is told the
/// message is there. Looking only at the pending messages answered `false`
/// for the message the question was asked about.
pub fn found_message(post: &MessagePost) -> bool {
    queue(post.reg.queue)
        .map(|queue| {
            let state = queue.lock();
            state.running_posts.contains(post) || state.messages.iter().any(|m| m.post == *post)
        })
        .unwrap_or(false)
}

/// `MessageQueue::WaitMessage(post, timeout)` — `true` once the message has
/// been handled. A negative timeout waits forever.
///
/// What it waits for is `post`, and not for the queue to go quiet: a message
/// another thread is running on the same queue is not this one's business,
/// and the C++ waits on the condition of the one run loop that is running
/// `post` (`WaitForRunningLockEnd`).
pub fn wait_message(post: &MessagePost, timeout_ms: i64) -> bool {
    let Some(queue) = queue(post.reg.queue) else {
        return false;
    };
    let deadline =
        (timeout_ms >= 0).then(|| Instant::now() + Duration::from_millis(timeout_ms as u64));
    let mut state = queue.lock();
    loop {
        if !state.messages.iter().any(|m| m.post == *post) && !state.running_posts.contains(post) {
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
    /// before the queue is looked at and before the breaker is asked — the
    /// order of the C++ RunLoop, so it also runs on the turn that stops the
    /// loop.
    pub fn run(id: MessageQueueId, mut breaker: impl FnMut() -> bool, mut duty: impl FnMut()) {
        let Some(queue) = queue(id) else { return };
        set_current_thread_message_queue(id);
        loop {
            // The duty runs before the breaker is asked, which is the order of
            // `RunLoop::Run` in `comm/messagequeue/message_queue.cc`: the duty
            // therefore also runs on the turn that stops the loop. A caller
            // whose duty flushes or saves got one pass fewer than it asked for
            // — the last one, on the turn the breaker said stop — while the
            // breaker was asked first.
            duty();
            if breaker() {
                break;
            }
            Self::dispatch(&queue, Some(Duration::from_millis(1)));
        }
    }

    /// Handles at most one message, waiting up to `timeout` for one to be due.
    ///
    /// The calling thread is bound to `id` for as long as the dispatch lasts,
    /// the way [`RunLoop::run`] binds the thread it runs the loop on: a
    /// handler asking [`current_thread_message_queue`] from inside its message
    /// is answered the queue it is running on. Only `run` bound it before, so
    /// a queue driven this way — the default queue is, by the JNI glue —
    /// answered [`INVALID_QUEUE_ID`] from inside its own handlers, and a
    /// caller that asks whether it may run queue work inline
    /// (`CurrentThreadMessageQueue() == Handler2Queue(...)`, the macros of
    /// `comm/messagequeue/message_queue.h`) took the wrong way round.
    ///
    /// The binding the thread had is put back when the dispatch is over: a
    /// thread is a queue's thread for exactly as long as it is running that
    /// queue's work, so dispatching another queue from inside a message does
    /// not unbind the thread from the one it is running.
    pub fn dispatch_timeout(id: MessageQueueId, timeout: Duration) -> bool {
        let Some(queue) = queue(id) else {
            return false;
        };
        let previous = current_thread_message_queue();
        set_current_thread_message_queue(id);
        let ran = Self::dispatch(&queue, Some(timeout));
        set_current_thread_message_queue(previous);
        ran
    }

    fn dispatch(queue: &Arc<Queue>, timeout: Option<Duration>) -> bool {
        // Wait until a message is due, up to `timeout`. A wake-up is not
        // the end of the wait: only the deadline or a due message is,
        // otherwise a spurious wake-up reports "nothing to do" before an
        // `After` message is due.
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        // The post this dispatch is running, which is taken out of
        // `running_posts` once its handlers are done.
        let post;
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
                // The wait is capped by the earliest message that is still
                // to come, not only by the caller's timeout: an `After(40)`
                // with a 300 ms timeout has to run after 40 ms, and with no
                // timeout at all there is nothing else that would ever wake
                // this up.
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

            // Take the message out, re-arm it when it is periodic, and
            // collect the handlers to call — all while holding the same
            // lock the wait ended on, so that a handler can post or cancel
            // from inside a message and the index still names the message
            // it was computed for.
            let Some(mut entry) = state.messages.remove(index) else {
                return false;
            };
            // `seq == 0` is a broadcast: only handlers that asked for
            // it run.
            let addressed = entry.post.reg.seq;
            let is_broadcast = addressed == 0;
            if let Some(period) = entry.period {
                entry.due = Some(Instant::now() + period);
                state.messages.push_back(entry.clone_for_next_run());
            }
            let handlers: Vec<Arc<HandlerFn>> = state
                .handlers
                .iter()
                .filter(|it| it.seq == addressed || (is_broadcast && it.recv_broadcast))
                .map(|it| Arc::clone(&it.handler))
                .collect();
            // `runing_message_id` of one `RunLoopInfo` in the C++'s
            // `lst_runloop_info`, which `FoundMessage` answers `true` for:
            // from here until the handlers are done, this dispatch is running
            // this message and not merely holding it. It is pushed rather
            // than stored in one slot per queue, because two threads
            // dispatching the same queue run two messages at once and the
            // first of them is still running when the second is pushed.
            post = entry.post;
            state.running_posts.push(post);
            (handlers, Arc::clone(&entry.message))
        };

        {
            let mut message = message.lock().unwrap();
            message.execute_time = gettickcount();
            for handler in handlers {
                handler(&mut message);
            }
        }

        queue.clear_running(post);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A queue does not run out of sequence numbers at the top of a `u32`:
    /// the count wraps, and the wrap skips 0, which is the `seq` of
    /// [`NULL_POST`] and of a broadcast handler — a post the queue never
    /// hands out.
    ///
    /// It is tested here and not in `tests/`, because the counter is a
    /// field of [`QueueState`]: reaching `u32::MAX` from outside would take
    /// four thousand million posts.
    #[test]
    fn the_sequence_number_of_a_post_wraps_past_the_top_of_a_u32() {
        let mut state = QueueState::new();
        state.next_post_seq = u32::MAX - 1;
        assert_eq!(state.next_post_seq(), u32::MAX - 1);
        assert_eq!(state.next_post_seq(), u32::MAX);
        assert_eq!(
            state.next_post_seq(),
            1,
            "it wrapped to 0, which is no post"
        );
    }
}
