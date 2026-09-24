//! What the C++'s `SocketPool` answers for the same calls, through the public
//! API: a task puts the socket it is done with in, and the next task that wants
//! the same ip, port and host gets it back — for a while.

use std::sync::{Arc, Mutex};

use mars_stn::{
    CachedSocket, IpPortItem, SocketFd, SocketPool, BAN_INTERVAL, DEFAULT_MAX_KEEPALIVE_TIME,
};

/// The pool plus what the host saw it close.
fn pool() -> (SocketPool, Arc<Mutex<Vec<SocketFd>>>) {
    let closed = Arc::new(Mutex::new(Vec::new()));
    let mut pool = SocketPool::new();
    let seen = closed.clone();
    pool.set_close(move |socket| seen.lock().unwrap().push(socket));
    (pool, closed)
}

fn item(ip: &str, port: u16, host: &str) -> IpPortItem {
    IpPortItem {
        ip: ip.to_string(),
        port,
        host: host.to_string(),
        ..IpPortItem::new(ip, port)
    }
}

#[test]
fn a_new_pool_caches_and_holds_nothing() {
    let (pool, _) = pool();
    assert!(pool.use_cache());
    assert!(pool.is_empty());
    assert_eq!(pool.len(), 0);
    assert!(!pool.is_banned_at(0));
}

#[test]
fn the_socket_a_task_is_done_with_comes_back_for_the_next_one() {
    let (mut pool, _) = pool();
    let address = item("1.1.1.1", 80, "short.example");
    pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 5));

    assert_eq!(pool.get_socket_at(1_000, &address), Some(SocketFd(3)));
    // and it is gone: one socket serves one task
    assert_eq!(pool.get_socket_at(1_000, &address), None);
}

#[test]
fn a_socket_is_only_for_the_pair_it_was_made_for() {
    let (mut pool, _) = pool();
    pool.add_cache(CachedSocket::new_at(
        0,
        item("1.1.1.1", 80, "short.example"),
        SocketFd(3),
        5,
    ));

    for other in [
        item("1.1.1.1", 443, "short.example"),
        item("2.2.2.2", 80, "short.example"),
        item("1.1.1.1", 80, "other.example"),
    ] {
        assert_eq!(pool.get_socket_at(1_000, &other), None);
    }
    assert_eq!(pool.len(), 1);
}

#[test]
fn keepalive_is_seconds_and_a_socket_that_outlived_it_is_closed() {
    // `DEFAULT_MAX_KEEPALIVE_TIME` is what the C++ keeps one for, and the item
    // counts it in seconds — which is what `HasTimeout` multiplies by 1000
    assert_eq!(DEFAULT_MAX_KEEPALIVE_TIME, 5 * 1000);

    let (mut pool, closed) = pool();
    let address = item("1.1.1.1", 80, "short.example");
    pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 5));

    // one tick before the five seconds are up
    assert_eq!(pool.get_socket_at(4_999, &address), Some(SocketFd(3)));

    pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(4), 5));
    assert_eq!(pool.get_socket_at(5_000, &address), None);
    assert_eq!(*closed.lock().unwrap(), vec![SocketFd(4)]);
}

#[test]
fn a_socket_the_peer_closed_is_closed_and_dropped() {
    let (mut pool, closed) = pool();
    let address = item("1.1.1.1", 80, "short.example");
    pool.set_is_closed(|socket| socket == SocketFd(3));
    pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 5));

    assert_eq!(pool.get_socket_at(1_000, &address), None);
    assert_eq!(*closed.lock().unwrap(), vec![SocketFd(3)]);
    assert!(pool.is_empty());
}

#[test]
fn a_quic_socket_hands_out_a_stream_and_stays_in_the_pool() {
    let (mut pool, _) = pool();
    let mut address = item("1.1.1.1", 80, "short.example");
    address.transport_protocol = mars_stn::Task::TRANSPORT_PROTOCOL_QUIC;
    pool.set_create_stream(|socket| SocketFd(socket.0 + 100));
    pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 5));

    assert_eq!(pool.get_socket_at(1_000, &address), Some(SocketFd(103)));
    // the socket itself is still there, and its keepalive started over
    assert_eq!(pool.len(), 1);
    // one tick before the keepalive it started over at 1_000 is up
    assert_eq!(pool.get_socket_at(5_999, &address), Some(SocketFd(103)));
    // five seconds after the last stream was taken from it, it is out of time
    assert_eq!(pool.get_socket_at(10_999, &address), None);
    assert!(pool.is_empty());
}

#[test]
fn a_socket_taken_from_the_pool_that_answered_nothing_bans_it() {
    let (mut pool, _) = pool();
    let address = item("1.1.1.1", 80, "short.example");
    pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 400));

    // a socket that was not reused says nothing about the pool
    pool.report_at(0, false, false, false);
    assert!(!pool.is_banned_at(1_000));

    pool.report_at(0, true, false, false);
    assert!(pool.is_banned_at(1_000));
    assert_eq!(pool.get_socket_at(1_000, &address), None);
    // `<= BAN_INTERVAL`, so the ban is still on at the interval itself
    assert!(pool.is_banned_at(BAN_INTERVAL));
    assert!(!pool.is_banned_at(BAN_INTERVAL + 1));
    assert_eq!(
        pool.get_socket_at(BAN_INTERVAL + 1, &address),
        Some(SocketFd(3))
    );
}

#[test]
fn a_reused_socket_that_answered_unbans_the_pool() {
    let (mut pool, _) = pool();
    pool.report_at(0, true, true, false);
    assert!(pool.is_banned_at(1_000));

    pool.report_at(2_000, true, true, true);
    assert!(!pool.is_banned_at(2_000));
}

#[test]
fn cleaning_drops_the_old_sockets_and_clearing_drops_them_all() {
    let (mut pool, closed) = pool();
    pool.add_cache(CachedSocket::new_at(
        0,
        item("1.1.1.1", 80, "h"),
        SocketFd(3),
        5,
    ));
    pool.add_cache(CachedSocket::new_at(
        2_000,
        item("2.2.2.2", 80, "h"),
        SocketFd(4),
        5,
    ));

    pool.clean_timeout_at(6_000);
    assert_eq!(pool.len(), 1);
    assert_eq!(
        pool.get_socket_at(6_000, &item("2.2.2.2", 80, "h")),
        Some(SocketFd(4))
    );

    pool.add_cache(CachedSocket::new_at(
        6_000,
        item("3.3.3.3", 80, "h"),
        SocketFd(5),
        5,
    ));
    pool.clear();
    assert!(pool.is_empty());
    // the one dropped for being old, and the one dropped by the clear
    assert_eq!(*closed.lock().unwrap(), vec![SocketFd(3), SocketFd(5)]);
}

#[test]
fn a_pool_that_is_not_caching_holds_the_sockets_but_hands_none_out() {
    let (mut pool, _) = pool();
    let address = item("1.1.1.1", 80, "short.example");
    pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 5));
    pool.set_use_cache(false);

    assert_eq!(pool.get_socket_at(1_000, &address), None);
    assert_eq!(pool.len(), 1);
}

#[test]
fn the_clock_the_pool_asks_for_is_the_hosts() {
    // `get_socket`, `clean_timeout` and `report` are the same calls without a
    // reading: what they do is what the `_at` ones do with a clock of their own
    let (mut pool, _) = pool();
    let address = item("1.1.1.1", 80, "short.example");
    pool.add_cache(CachedSocket::new(address.clone(), SocketFd(3), 400));
    assert_eq!(pool.get_socket(&address), Some(SocketFd(3)));

    pool.add_cache(CachedSocket::new(address.clone(), SocketFd(4), 0));
    pool.clean_timeout();
    assert!(pool.is_empty());

    pool.report(true, false, false);
    // banned from here on, so a socket put in afterwards is not handed out
    pool.add_cache(CachedSocket::new(address.clone(), SocketFd(5), 400));
    assert_eq!(pool.get_socket(&address), None);
}
