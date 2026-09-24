//! `mars/comm/messagequeue/` — posting, cancelling and draining.
//!
//! Every test uses its own queue: the default one is process-wide and the
//! tests run in parallel.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use mars_comm::message_queue::{
    broadcast_message, cancel_message, cancel_message_by_handler, cancel_message_by_handler_title,
    create_message_queue, destroy_message_queue, found_message, get_def_message_queue,
    install_async_handler, install_message_handler, install_message_handler as install,
    pending_message_count, post_message, post_message_at_first, singleton_message, wait_message,
    KNullPost, Message, MessageHandler, MessageTiming, MessageTitle, RunLoop,
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

    broadcast_message(queue, Message::new(MessageTitle(1), "broadcast"));
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

    post_message(
        &handler,
        Message::new(MessageTitle(1), "later"),
        MessageTiming::After(40),
    );
    assert!(
        !RunLoop::dispatch_timeout(queue, Duration::from_millis(5)),
        "must not run yet"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 0);

    let start = Instant::now();
    assert!(RunLoop::dispatch_timeout(queue, Duration::from_millis(300)));
    assert!(start.elapsed() >= Duration::from_millis(30));
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
        KNullPost
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
