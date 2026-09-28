//! `mars/comm/messagequeue/` — posting, cancelling and draining.
//!
//! Every test uses its own queue: the default one is process-wide and the
//! tests run in parallel.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use marsrs_comm::message_queue::{
    broadcast_message, cancel_message, cancel_message_by_handler, cancel_message_by_handler_title,
    create_message_queue, current_thread_message_queue, destroy_message_queue, faster_message,
    found_message, get_def_message_queue, install_async_handler, install_message_handler,
    install_message_handler as install, pending_message_count, post_message, post_message_at_first,
    singleton_message, uninstall_message_handler, wait_message, Message, MessageHandler,
    MessagePost, MessageTiming, MessageTitle, RunLoop, INVALID_QUEUE_ID, NULL_POST,
};

#[test]
fn a_posted_message_reaches_its_handler() {
    let queue = create_message_queue();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let handler = install_message_handler(
        move |message: &mut Message| sink.lock().unwrap().push(message.title.0),
        false,
        queue,
    );

    let post = post_message(
        &handler,
        Message::new(MessageTitle(42), "test"),
        MessageTiming::Immediate,
    );
    assert!(found_message(&post));
    assert_eq!(pending_message_count(queue), 1);

    assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(200)));
    assert_eq!(*seen.lock().unwrap(), vec![42]);
    assert_eq!(pending_message_count(queue), 0);
    destroy_message_queue(queue);
}

#[test]
fn an_async_handler_runs_the_invoked_closure() {
    let queue = create_message_queue();
    let handler = install_async_handler(queue);
    let ran = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&ran);
    post_message(
        &handler,
        Message::with_invoke(MessageTitle(0), "async", move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
        MessageTiming::Immediate,
    );
    assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(200)));
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    destroy_message_queue(queue);
}

#[test]
fn a_broadcast_only_runs_the_handlers_that_asked_for_it() {
    let queue = create_message_queue();
    let broadcast_hits = Arc::new(AtomicUsize::new(0));
    let private_hits = Arc::new(AtomicUsize::new(0));
    let b = Arc::clone(&broadcast_hits);
    let p = Arc::clone(&private_hits);

    install_message_handler(
        move |_| {
            b.fetch_add(1, Ordering::SeqCst);
        },
        true,
        queue,
    );
    let private = install_message_handler(
        move |_| {
            p.fetch_add(1, Ordering::SeqCst);
        },
        false,
        queue,
    );

    broadcast_message(
        queue,
        Message::new(MessageTitle(1), "broadcast"),
        MessageTiming::Immediate,
    );
    assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(200)));
    assert_eq!(broadcast_hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        private_hits.load(Ordering::SeqCst),
        0,
        "a private handler must not see broadcasts"
    );

    // and a message addressed to it still runs
    post_message(
        &private,
        Message::new(MessageTitle(2), "direct"),
        MessageTiming::Immediate,
    );
    assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(200)));
    assert_eq!(private_hits.load(Ordering::SeqCst), 1);
    destroy_message_queue(queue);
}

#[test]
fn a_post_can_be_cancelled() {
    let queue = create_message_queue();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let handler = install(
        move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        },
        false,
        queue,
    );

    let post = post_message(
        &handler,
        Message::new(MessageTitle(1), "cancel me"),
        MessageTiming::Immediate,
    );
    assert!(cancel_message(&post));
    assert!(!found_message(&post));
    assert!(!cancel_message(&post), "cancelling twice is a no-op");

    assert!(!RunLoop::dispatch_timeout(queue, Duration::from_millis(20)));
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    destroy_message_queue(queue);
}

#[test]
fn messages_can_be_cancelled_by_handler_and_by_title() {
    let queue = create_message_queue();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let handler = install(
        move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        },
        false,
        queue,
    );

    post_message(
        &handler,
        Message::new(MessageTitle(1), "a"),
        MessageTiming::Immediate,
    );
    post_message(
        &handler,
        Message::new(MessageTitle(2), "b"),
        MessageTiming::Immediate,
    );
    assert_eq!(pending_message_count(queue), 2);

    cancel_message_by_handler_title(&handler, MessageTitle(1));
    assert_eq!(pending_message_count(queue), 1);

    cancel_message_by_handler(&handler);
    assert_eq!(pending_message_count(queue), 0);
    destroy_message_queue(queue);
}

#[test]
fn a_singleton_keeps_one_pending_message_per_title() {
    let queue = create_message_queue();
    let handler = install(|_| {}, false, queue);

    let first = singleton_message(false, &handler, Message::new(MessageTitle(7), "first"));
    let second = singleton_message(false, &handler, Message::new(MessageTitle(7), "second"));
    assert_eq!(first, second, "the pending post is returned");
    assert_eq!(pending_message_count(queue), 1);

    // with replace the payload of the pending message is swapped
    let replaced = singleton_message(
        true,
        &handler,
        Message::new(MessageTitle(7), "third").with_body1(String::from("payload")),
    );
    assert_eq!(replaced, first);
    assert_eq!(pending_message_count(queue), 1);
    destroy_message_queue(queue);
}

#[test]
fn an_after_message_waits_for_its_due_time() {
    let queue = create_message_queue();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let handler = install(
        move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        },
        false,
        queue,
    );

    // nothing is due on a queue that has nothing on it, however long the
    // wait it was handed turns out to be
    assert!(
        !RunLoop::dispatch_timeout(queue, Duration::from_millis(5)),
        "an empty queue has nothing to dispatch"
    );

    let start = Instant::now();
    post_message(
        &handler,
        Message::new(MessageTitle(1), "later"),
        MessageTiming::After(40),
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "a post does not run its handler"
    );

    // … and it runs once its 40 ms are up, and not before. What is asserted
    // is the time it ran at and not that a shorter dispatch did not run it:
    // a `wait_timeout` is only ever a lower bound, so on a loaded runner a
    // dispatch that asked for 5 ms may well sleep the whole 40 ms — and then
    // the message *is* due and running it is the right answer, which would
    // make an assertion that it did not run a lie.
    assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(300)));
    assert!(
        start.elapsed() >= Duration::from_millis(40),
        "ran {:?} after the post",
        start.elapsed()
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    destroy_message_queue(queue);
}

#[test]
fn a_periodic_message_keeps_running() {
    let queue = create_message_queue();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let handler = install(
        move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        },
        false,
        queue,
    );

    post_message(
        &handler,
        Message::new(MessageTitle(1), "ticker"),
        MessageTiming::Period {
            after: 0,
            period: 5,
        },
    );
    for _ in 0..3 {
        assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(200)));
        std::thread::sleep(Duration::from_millis(7));
    }
    assert!(
        hits.load(Ordering::SeqCst) >= 3,
        "expected at least three runs"
    );
    // unlike a one-shot it is never drained
    assert_eq!(pending_message_count(queue), 1);
    cancel_message_by_handler(&handler);
    destroy_message_queue(queue);
}

#[test]
fn a_message_can_be_jumped_to_the_front() {
    let queue = create_message_queue();
    let order = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&order);
    let handler = install(
        move |message: &mut Message| sink.lock().unwrap().push(message.title.0),
        false,
        queue,
    );

    post_message(
        &handler,
        Message::new(MessageTitle(1), "first"),
        MessageTiming::Immediate,
    );
    post_message(
        &handler,
        Message::new(MessageTitle(2), "second"),
        MessageTiming::Immediate,
    );
    post_message_at_first(&handler, Message::new(MessageTitle(3), "jumped"));

    RunLoop::dispatch_timeout(queue, Duration::from_millis(50));
    RunLoop::dispatch_timeout(queue, Duration::from_millis(50));
    RunLoop::dispatch_timeout(queue, Duration::from_millis(50));
    assert_eq!(*order.lock().unwrap(), vec![3, 1, 2]);
    destroy_message_queue(queue);
}

#[test]
fn post_to_an_unknown_handler_is_a_null_post() {
    let queue = create_message_queue();
    let unknown = MessageHandler { queue, seq: 999 };
    assert_eq!(
        post_message(
            &unknown,
            Message::new(MessageTitle(1), "nowhere"),
            MessageTiming::Immediate
        ),
        NULL_POST
    );
    destroy_message_queue(queue);
}

#[test]
fn wait_message_returns_when_the_message_has_been_handled() {
    let queue = create_message_queue();
    let handler = install(|_| {}, false, queue);
    let post = post_message(
        &handler,
        Message::new(MessageTitle(1), "waited"),
        MessageTiming::Immediate,
    );

    let worker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(10));
        RunLoop::dispatch_timeout(queue, Duration::from_millis(200))
    });
    assert!(
        wait_message(&post, 1_000),
        "the post should have been handled"
    );
    assert!(worker.join().unwrap());

    // a post that never existed times out instead of hanging
    let gone = post_message(
        &handler,
        Message::new(MessageTitle(2), "gone"),
        MessageTiming::Immediate,
    );
    cancel_message(&gone);
    assert!(wait_message(&gone, 1));
    destroy_message_queue(queue);
}

#[test]
fn the_run_loop_runs_until_its_breaker_says_stop() {
    let queue = create_message_queue();
    let handled = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&handled);
    let handler = install(
        move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        },
        false,
        queue,
    );
    for _ in 0..3 {
        post_message(
            &handler,
            Message::new(MessageTitle(1), "loop"),
            MessageTiming::Immediate,
        );
    }

    let duties = Arc::new(AtomicUsize::new(0));
    let duty_counter = Arc::clone(&duties);
    // stop once every message has been handled
    let stop = Arc::clone(&handled);
    RunLoop::run(
        queue,
        move || stop.load(Ordering::SeqCst) >= 3,
        move || {
            duty_counter.fetch_add(1, Ordering::SeqCst);
        },
    );
    // the breaker runs at the top of every turn, so there is exactly one duty
    // per dispatched message and none on the turn that stops the loop
    assert_eq!(handled.load(Ordering::SeqCst), 3);
    assert_eq!(duties.load(Ordering::SeqCst), 3);
    destroy_message_queue(queue);
}

#[test]
fn a_broadcast_runs_the_handlers_in_the_order_they_were_installed() {
    let queue = create_message_queue();
    let order = Arc::new(Mutex::new(Vec::new()));
    for id in 1_u32..=3 {
        let sink = Arc::clone(&order);
        install_message_handler(
            move |_| {
                sink.lock().unwrap().push(id);
            },
            true,
            queue,
        );
    }

    broadcast_message(
        queue,
        Message::new(MessageTitle(1), "order"),
        MessageTiming::Immediate,
    );
    assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(200)));

    // `lst_handler` of the C++ is a `std::list` walked from the front, so
    // which of several handlers for one message runs first is the order they
    // were installed in and not the order a `HashMap` happens to yield.
    assert_eq!(*order.lock().unwrap(), vec![1, 2, 3]);
    destroy_message_queue(queue);
}

#[test]
fn a_message_is_found_while_it_is_running() {
    let queue = create_message_queue();
    // The handler has to ask about the post, but the post is only known once
    // it has been posted — and a post names the handler it is addressed to,
    // so the handler has to be installed first.
    let slot = Arc::new(Mutex::new(NULL_POST));
    let found = Arc::new(AtomicUsize::new(0));
    let handler_slot = Arc::clone(&slot);
    let counter = Arc::clone(&found);
    let handler = install_message_handler(
        move |_| {
            let post = *handler_slot.lock().unwrap();
            if found_message(&post) {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        },
        false,
        queue,
    );

    let post = post_message(
        &handler,
        Message::new(MessageTitle(1), "running"),
        MessageTiming::Immediate,
    );
    *slot.lock().unwrap() = post;
    assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(200)));
    assert_eq!(
        found.load(Ordering::SeqCst),
        1,
        "the message it is running was not found"
    );
    // … and it is gone once it has been handled
    assert!(!found_message(&post));
    destroy_message_queue(queue);
}

#[test]
fn a_thread_with_no_queue_of_its_own_answers_the_invalid_id() {
    let queue = create_message_queue();
    let (sender, receiver) = mpsc::channel();
    // Both readings are taken on the worker and not on the thread this test
    // runs on: a thread keeps the queue a `RunLoop` bound it to, so a test
    // thread that has already run one would not answer the invalid id.
    let worker = thread::spawn(move || {
        sender.send(current_thread_message_queue()).unwrap();
        let mut turns = 0;
        RunLoop::run(
            queue,
            move || {
                turns += 1;
                turns > 1
            },
            move || {
                sender.send(current_thread_message_queue()).unwrap();
            },
        );
    });

    assert_eq!(receiver.recv().unwrap(), INVALID_QUEUE_ID);
    assert_eq!(receiver.recv().unwrap(), queue);
    worker.join().unwrap();
    destroy_message_queue(queue);
}

#[test]
fn the_default_queue_survives_another_queue_being_created_first() {
    // `create_message_queue` used to build the registry itself, so a process
    // whose first queue call was `create_message_queue` had no default queue
    // for good: every later post to `get_def_message_queue()` returned
    // `KNullPost` and vanished.
    let queue = create_message_queue();
    let handler = install_message_handler(|_| {}, false, get_def_message_queue());
    let post = post_message(
        &handler,
        Message::new(MessageTitle(1), "default"),
        MessageTiming::Immediate,
    );
    assert!(found_message(&post), "the default queue is missing");
    assert!(cancel_message(&post));
    destroy_message_queue(queue);
}

#[test]
fn cancelling_a_message_wakes_the_thread_waiting_for_it() {
    let queue = create_message_queue();
    let handler = install_message_handler(|_| {}, false, queue);
    let post = post_message(
        &handler,
        Message::new(MessageTitle(1), "never"),
        MessageTiming::After(60_000),
    );

    let waited = post;
    let started = Instant::now();
    let waiter = thread::spawn(move || wait_message(&waited, 30_000));
    // let the waiter reach the condition variable
    thread::sleep(Duration::from_millis(50));

    assert!(cancel_message(&post));
    let waited_out = waiter.join().unwrap();
    assert!(waited_out, "the cancellation did not wake the waiter");
    assert!(
        started.elapsed() < Duration::from_millis(2_000),
        "the waiter slept out its 30 s timeout"
    );
    destroy_message_queue(queue);
}

#[test]
fn a_handler_can_cancel_the_periodic_message_it_is_running() {
    let queue = create_message_queue();
    let handler = install_async_handler(queue);
    let self_cancelling = handler;
    post_message(
        &handler,
        Message::with_invoke(MessageTitle(7), "self-cancel", move || {
            cancel_message_by_handler_title(&self_cancelling, MessageTitle(7));
        }),
        MessageTiming::Period {
            after: 0,
            period: 10,
        },
    );

    // The dispatcher holds this message's lock while the handler runs, so
    // matching the title through that lock deadlocks the queue thread.
    let (tx, rx) = mpsc::channel();
    let runner = thread::spawn(move || {
        let ran = RunLoop::dispatch_timeout(queue, Duration::from_millis(200));
        let _ = tx.send(ran);
    });
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(ran) => assert!(ran, "the message did not run"),
        Err(_) => panic!("the dispatcher deadlocked on the message it was running"),
    }
    let _ = runner.join();
    assert_eq!(
        pending_message_count(queue),
        0,
        "the periodic message was not cancelled"
    );
    destroy_message_queue(queue);
}

#[test]
fn dispatch_waits_for_the_due_time_not_for_the_whole_timeout() {
    let queue = create_message_queue();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let handler = install(
        move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        },
        false,
        queue,
    );
    post_message(
        &handler,
        Message::new(MessageTitle(1), "soon"),
        MessageTiming::After(80),
    );

    let started = Instant::now();
    assert!(RunLoop::dispatch_timeout(
        queue,
        Duration::from_millis(5_000)
    ));
    let elapsed = started.elapsed();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(
        elapsed < Duration::from_millis(2_000),
        "an 80 ms message waited {elapsed:?} of the 5 s timeout"
    );
    destroy_message_queue(queue);
}

#[test]
fn a_periodic_message_can_ask_for_itself_while_it_runs() {
    // A periodic message stays in the queue while it runs — the C++ hands the
    // very same `Message` to the handlers — so asking for it from inside the
    // handler has to be answered without locking the payload the dispatcher
    // is holding. It used to deadlock: the queue thread waited for a lock it
    // held itself.
    let queue = create_message_queue();
    let handler_slot = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&handler_slot);
    let (done, waited) = mpsc::channel();
    let said = Mutex::new(Some(done));
    let handler = install(
        move |_| {
            let handler = slot.lock().unwrap().unwrap();
            let asked = singleton_message(false, &handler, Message::new(MessageTitle(7), "tick"));
            let replaced = singleton_message(
                true,
                &handler,
                Message::new(MessageTitle(7), "tick").with_body1(String::from("payload")),
            );
            if let Some(done) = said.lock().unwrap().take() {
                done.send((asked, replaced)).unwrap();
            }
        },
        false,
        queue,
    );
    *handler_slot.lock().unwrap() = Some(handler);

    let post = post_message(
        &handler,
        Message::new(MessageTitle(7), "tick"),
        MessageTiming::Period {
            after: 0,
            period: 10,
        },
    );

    // on another thread, so that a queue that never answers is a test that
    // fails instead of one that never finishes
    let runner = thread::spawn(move || RunLoop::dispatch_timeout(queue, Duration::from_secs(1)));
    let (asked, replaced) = waited
        .recv_timeout(Duration::from_secs(5))
        .expect("the queue thread is stuck asking for the message it is running");
    assert_eq!(
        asked, post,
        "the pending message is the one that is running"
    );
    assert_eq!(replaced, post);
    runner.join().unwrap();
    destroy_message_queue(queue);
}

/// `replace` has to reach the *next* run even when it is asked for while the
/// message is being dispatched — which is the only moment an app has to replace
/// its own periodic message. Writing through the payload's lock instead needs
/// `try_lock` (a `Mutex` is not reentrant and the dispatcher holds it), and a
/// `try_lock` that fails drops the replacement on the floor: the handler kept
/// being handed the payload it had just replaced, forever.
#[test]
fn replacing_a_periodic_message_while_it_runs_reaches_the_next_run() {
    let queue = create_message_queue();
    let handler_slot = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&handler_slot);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let handler = install(
        move |message: &mut Message| {
            let payload = message
                .body1
                .as_ref()
                .and_then(|body| body.downcast_ref::<String>())
                .cloned()
                .unwrap_or_default();
            sink.lock().unwrap().push(payload.clone());
            if payload == "first" {
                let handler = slot.lock().unwrap().unwrap();
                singleton_message(
                    true,
                    &handler,
                    Message::new(MessageTitle(7), "tick").with_body1(String::from("second")),
                );
            }
        },
        false,
        queue,
    );
    *handler_slot.lock().unwrap() = Some(handler);

    post_message(
        &handler,
        Message::new(MessageTitle(7), "tick").with_body1(String::from("first")),
        MessageTiming::Period {
            after: 0,
            period: 10,
        },
    );

    // Several turns rather than one: what is being checked is the run that
    // comes after the replacement, and a periodic message is due every 10 ms.
    for _ in 0..20 {
        RunLoop::dispatch_timeout(queue, Duration::from_millis(50));
        if seen.lock().unwrap().len() >= 2 {
            break;
        }
    }

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.first().map(String::as_str), Some("first"), "{seen:?}");
    assert!(
        seen.iter().any(|payload| payload == "second"),
        "the replacement never reached a run: {seen:?}"
    );
    cancel_message_by_handler(&handler);
    destroy_message_queue(queue);
}

/// `FasterMessage` is handed a handler, and upstream posts it to that handler
/// (`MessageWrapper(_handlerid, ...)`, `comm/messagequeue/message_queue.cc`).
/// Posted as a broadcast it ran every handler that had asked for broadcasts
/// and not the one it was addressed to, which is how one disappeared.
#[test]
fn a_faster_message_runs_the_handler_it_was_addressed_to() {
    let queue = create_message_queue();
    let addressed = Arc::new(AtomicUsize::new(0));
    let broadcasts = Arc::new(AtomicUsize::new(0));
    let addressed_counter = Arc::clone(&addressed);
    let broadcast_counter = Arc::clone(&broadcasts);

    let target = install(
        move |_| {
            addressed_counter.fetch_add(1, Ordering::SeqCst);
        },
        false,
        queue,
    );
    // a bystander that takes broadcasts, and would run if this were one
    let _bystander = install(
        move |_| {
            broadcast_counter.fetch_add(1, Ordering::SeqCst);
        },
        true,
        queue,
    );

    let post = faster_message(&target, Message::new(MessageTitle(11), "faster"));
    assert!(found_message(&post));
    assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(200)));

    assert_eq!(
        addressed.load(Ordering::SeqCst),
        1,
        "the handler it was addressed to never ran"
    );
    assert_eq!(
        broadcasts.load(Ordering::SeqCst),
        0,
        "it was posted as a broadcast"
    );
    destroy_message_queue(queue);
}

/// `FasterMessage` asked twice for the same handler and title is **one** message
/// with the payload of the second ask, and the post the first ask returned keeps
/// naming it (`messagewrapper->postid = (*it)->postid`).
///
/// Posing it twice as two pending messages is what the queue did before: the
/// handler then ran once per ask, with a payload the caller had already
/// superseded.
#[test]
fn a_faster_message_asked_twice_runs_once_with_the_later_payload() {
    let queue = create_message_queue();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let handler = install(
        move |message: &mut Message| sink.lock().unwrap().push(message.name.clone()),
        false,
        queue,
    );

    let first = faster_message(&handler, Message::new(MessageTitle(11), "first"));
    let second = faster_message(&handler, Message::new(MessageTitle(11), "second"));

    assert_eq!(first, second, "the pending message was given a new post");
    assert_eq!(
        pending_message_count(queue),
        1,
        "both asks are pending: the handler would run twice"
    );

    assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(200)));
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["second".to_owned()],
        "the superseded payload ran"
    );
    destroy_message_queue(queue);
}

/// The coalescing is on the title: another title for the same handler is another
/// message, and the same title for another handler is too.
#[test]
fn a_faster_message_only_replaces_its_own_title() {
    let queue = create_message_queue();
    let other = Arc::new(AtomicUsize::new(0));
    let other_counter = Arc::clone(&other);
    let second = install(
        move |_| {
            other_counter.fetch_add(1, Ordering::SeqCst);
        },
        false,
        queue,
    );
    let handler = install(|_| {}, false, queue);

    faster_message(&handler, Message::new(MessageTitle(11), "one"));
    faster_message(&handler, Message::new(MessageTitle(12), "two"));
    // Same title, different handler: not the message `handler` is waiting for.
    faster_message(&second, Message::new(MessageTitle(11), "three"));

    assert_eq!(pending_message_count(queue), 3);

    for _ in 0..3 {
        RunLoop::dispatch_timeout(queue, Duration::from_millis(50));
    }
    assert_eq!(
        other.load(Ordering::SeqCst),
        1,
        "the other handler's message was not coalesced away"
    );
    destroy_message_queue(queue);
}

/// The replacement is due now, and the message it replaces may have been due in
/// a minute — which is what a thread already waiting on this queue is sleeping
/// until. The C++ notifies after it has pushed the replacement
/// (`content.breaker->Notify(lock)`); without that the replacement sits in the
/// queue until the wait it was meant to cut short runs out.
#[test]
fn a_faster_message_wakes_the_thread_waiting_on_the_queue() {
    let queue = create_message_queue();
    let handler = install(|_| {}, false, queue);
    post_message(
        &handler,
        Message::new(MessageTitle(11), "later"),
        MessageTiming::After(60_000),
    );

    let (tx, rx) = mpsc::channel();
    let waiter = thread::spawn(move || {
        let ran = RunLoop::dispatch_timeout(queue, Duration::from_secs(30));
        let _ = tx.send(ran);
    });
    // let the waiter reach the condition variable
    thread::sleep(Duration::from_millis(50));

    let started = Instant::now();
    let _post = faster_message(&handler, Message::new(MessageTitle(11), "now"));

    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(ran) => assert!(ran, "the replacement did not run"),
        Err(_) => panic!("the replacement never woke the thread waiting for it"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the waiter slept out its 30 s timeout"
    );
    let _ = waiter.join();
    // Read once the waiter is done, so that the dispatch cannot be what makes
    // up the number: one message ran and none is left, which is the ask having
    // replaced the message it was waiting for rather than joined it.
    assert_eq!(
        pending_message_count(queue),
        0,
        "the replacement joined the message it was meant to replace"
    );
    destroy_message_queue(queue);
}

/// The search and the insertion are one step under the queue's lock: two threads
/// asking for the same message when nothing is pending must not each post a copy
/// of their own.
#[test]
fn two_asks_at_once_still_run_the_handler_once() {
    let queue = create_message_queue();
    let runs = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&runs);
    let handler = install(
        move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        },
        false,
        queue,
    );

    let barrier = Arc::new(Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                faster_message(&handler, Message::new(MessageTitle(11), "asked twice"))
            })
        })
        .collect();
    let posts: Vec<MessagePost> = threads
        .into_iter()
        .map(|thread| thread.join().expect("the asking thread panicked"))
        .collect();
    assert_eq!(
        posts[0], posts[1],
        "the two asks were coalesced into one message"
    );

    assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(200)));
    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "the handler ran once per ask instead of once"
    );
    destroy_message_queue(queue);
}

/// A handler that has been uninstalled has nothing left to deliver to, and a
/// message it still has pending does not make it installed again: the ask is
/// answered the way `post_message` answers it.
#[test]
fn an_ask_for_an_uninstalled_handler_is_no_post() {
    let queue = create_message_queue();
    let handler = install(|_| {}, false, queue);
    faster_message(&handler, Message::new(MessageTitle(11), "pending"));
    uninstall_message_handler(&handler);

    assert_eq!(
        faster_message(&handler, Message::new(MessageTitle(11), "asked anyway")),
        NULL_POST,
        "a message that can never be delivered was posted"
    );
    assert_eq!(
        pending_message_count(queue),
        1,
        "the uninstalled handler's pending message was replaced instead of refused"
    );
    destroy_message_queue(queue);
}
