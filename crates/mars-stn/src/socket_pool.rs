//! `mars/stn/src/socket_pool.h` — the sockets a short link leaves open for the
//! next task.
//!
//! A task that answered does not have to pay for a new connect: the socket it
//! used goes into the pool, and the next task that wants the same ip, port and
//! host takes it back out — unless it has been in there too long, or the peer
//! closed it, or a socket taken from the pool turned out to be no good, which
//! bans the pool for [`BAN_INTERVAL`].
//!
//! The pool keeps no socket of its own: a socket is an opaque [`SocketFd`], and
//! what the C++ reaches for through three function pointers it carries in every
//! item is three callbacks of the host's here (a fourth says whether a socket
//! is still open, which the C++ asks with a `recv` of one byte it peeks at).
//!
//! A quic socket is not given back the way a tcp one is: what the caller gets
//! is a *stream* on it, and the socket stays in the pool for the task after
//! that one.

use std::collections::VecDeque;

use crate::simple_ipport_sort::IpPortItem;
use crate::socket_operator::SocketFd;
use crate::task::Task;
use mars_comm::tickcount::gettickcount;

/// `BAN_INTERVAL` — how long a socket that was taken from the pool and turned
/// out to be no good keeps the pool from handing one out, in milliseconds.
pub const BAN_INTERVAL: u64 = 5 * 60 * 1000;
/// `DEFAULT_MAX_KEEPALIVE_TIME` — what the C++ keeps a cached socket for. No
/// caller of `SocketPool` uses it: the timeout is the one the caller writes in
/// the [`CachedSocket`] it puts in, which is why it lives here and not in the
/// pool.
pub const DEFAULT_MAX_KEEPALIVE_TIME: u32 = 5 * 1000;

/// `closefunc` — `CacheSocketItem::CloseSocket()`.
pub type CloseSocket = dyn FnMut(SocketFd) + Send;
/// `createstream_func` — a quic stream on a socket that is already open.
pub type CreateStream = dyn FnMut(SocketFd) -> SocketFd + Send;
/// `issubstream_func` — whether a socket is a quic stream rather than the
/// socket it was made from.
pub type IsSubStream = dyn FnMut(SocketFd) -> bool + Send;
/// `SocketPool::_IsSocketClosed` — the C++ peeks at the socket with a `recv` of
/// one byte; the host says whether it is gone. Unset answers *no*, which is a
/// socket the pool believes is still open.
pub type IsClosed = dyn FnMut(SocketFd) -> bool + Send;

/// `CacheSocketItem` — one socket in the pool, and what it was made for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedSocket {
    /// `address_info` — what the socket was made for. The pool only hands it
    /// back to a task that wants the same thing.
    pub address: IpPortItem,
    /// `start_tick` — when it went in, which is when its timeout starts.
    pub start: u64,
    /// `socket_fd`
    pub socket: SocketFd,
    /// `timeout` — how long it may stay in the pool, **in seconds**, which is
    /// what the C++ multiplies by 1000.
    pub timeout: u32,
}

impl CachedSocket {
    /// One socket, with the reading of the clock it goes in at.
    pub fn new(address: IpPortItem, socket: SocketFd, timeout: u32) -> Self {
        Self::new_at(gettickcount(), address, socket, timeout)
    }

    /// The same, with the reading handed in.
    pub fn new_at(now: u64, address: IpPortItem, socket: SocketFd, timeout: u32) -> Self {
        Self {
            address,
            start: now,
            socket,
            timeout,
        }
    }

    /// `IsSame(_item)` — the same transport, ip, port and host: a socket is for
    /// one of those and not for the others.
    pub fn is_same(&self, item: &IpPortItem) -> bool {
        self.address.transport_protocol == item.transport_protocol
            && self.address.ip == item.ip
            && self.address.port == item.port
            && self.address.host == item.host
    }

    /// `HasTimeout()` at `now` — in the pool for longer than its timeout.
    pub fn has_timeout_at(&self, now: u64) -> bool {
        now.saturating_sub(self.start) >= u64::from(self.timeout).saturating_mul(1000)
    }

    /// `ResetTimeout()` at `now` — what a quic socket gets when a stream is
    /// taken from it, because the socket itself is still doing work.
    pub fn reset_timeout_at(&mut self, now: u64) {
        self.start = now;
    }
}

/// `SocketPool`.
#[derive(Default)]
pub struct SocketPool {
    /// `use_cache_`
    use_cache: bool,
    /// `ban_start_tick_` — [`None`] for a pool that is not banned, which is the
    /// C++'s `is_baned_` and an invalid tick in one.
    ban_start: Option<u64>,
    /// `socket_pool_` — most recently put in first, the way the C++'s
    /// `push_front` has it.
    pool: VecDeque<CachedSocket>,

    close: Option<Box<CloseSocket>>,
    create_stream: Option<Box<CreateStream>>,
    is_sub_stream: Option<Box<IsSubStream>>,
    is_closed: Option<Box<IsClosed>>,
}

impl SocketPool {
    /// `SocketPool()` — one that caches, the way the C++'s constructor has it.
    pub fn new() -> Self {
        Self {
            use_cache: true,
            ..Self::default()
        }
    }

    /// `closefunc` — how a socket the pool drops is closed.
    pub fn set_close(&mut self, close: impl FnMut(SocketFd) + Send + 'static) {
        self.close = Some(Box::new(close));
    }

    /// `createstream_func` — how a quic stream is made on a socket that is
    /// already open.
    pub fn set_create_stream(
        &mut self,
        create_stream: impl FnMut(SocketFd) -> SocketFd + Send + 'static,
    ) {
        self.create_stream = Some(Box::new(create_stream));
    }

    /// `issubstream_func` — whether a socket is a stream rather than the socket
    /// it was made from.
    pub fn set_is_sub_stream(
        &mut self,
        is_sub_stream: impl FnMut(SocketFd) -> bool + Send + 'static,
    ) {
        self.is_sub_stream = Some(Box::new(is_sub_stream));
    }

    /// `SocketPool::_IsSocketClosed` — whether a socket that is still in the
    /// pool has been closed by the peer.
    pub fn set_is_closed(&mut self, is_closed: impl FnMut(SocketFd) -> bool + Send + 'static) {
        self.is_closed = Some(Box::new(is_closed));
    }

    /// `use_cache_` — whether the pool hands sockets out at all.
    pub fn set_use_cache(&mut self, use_cache: bool) {
        self.use_cache = use_cache;
    }

    /// Whether the pool hands sockets out at all.
    pub fn use_cache(&self) -> bool {
        self.use_cache
    }

    /// How many sockets are in the pool.
    pub fn len(&self) -> usize {
        self.pool.len()
    }

    /// Whether the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.pool.is_empty()
    }

    /// `SocketPool::_isBaned()` at `now` — whether a socket that was taken from
    /// the pool and turned out to be no good is still keeping it shut.
    pub fn is_banned_at(&self, now: u64) -> bool {
        self.ban_start
            .is_some_and(|start| now.saturating_sub(start) <= BAN_INTERVAL)
    }

    /// `GetSocket(_item)` — the socket the next task on this ip, port and host
    /// can use, if there is one.
    pub fn get_socket(&mut self, item: &IpPortItem) -> Option<SocketFd> {
        self.get_socket_at(gettickcount(), item)
    }

    /// The same, with the reading handed in. The most recently put in is the
    /// first one looked at, and a socket that is out of time or whose peer
    /// closed it is dropped along the way rather than handed out.
    pub fn get_socket_at(&mut self, now: u64, item: &IpPortItem) -> Option<SocketFd> {
        if !self.use_cache || self.is_banned_at(now) || self.pool.is_empty() {
            return None;
        }

        let mut index = 0;
        while let Some(cached) = self.pool.get(index) {
            if !cached.is_same(item) {
                index += 1;
                continue;
            }

            let socket = cached.socket;
            let timed_out = cached.has_timeout_at(now);
            // only a tcp socket is asked: a quic one is there for its streams
            let closed =
                item.transport_protocol == Task::TRANSPORT_PROTOCOL_TCP && self.is_closed(socket);
            if timed_out || closed {
                self.close(socket);
                self.pool.remove(index);
                continue;
            }

            if item.transport_protocol == Task::TRANSPORT_PROTOCOL_TCP || self.is_sub_stream(socket)
            {
                self.pool.remove(index);
                return Some(socket);
            }

            // a quic socket stays in the pool: what the caller gets is a stream
            // on it, and the socket keeps its own timeout from here on
            let stream = self.create_stream(socket);
            if !stream.is_valid() {
                self.pool.remove(index);
                return None;
            }
            if let Some(cached) = self.pool.get_mut(index) {
                cached.reset_timeout_at(now);
            }
            return Some(stream);
        }

        None
    }

    /// `AddCache(item)` — the socket a task is done with, put in most recently
    /// used first.
    pub fn add_cache(&mut self, socket: CachedSocket) {
        self.pool.push_front(socket);
    }

    /// `CleanTimeout()` — drop the sockets whose time in the pool ran out.
    pub fn clean_timeout(&mut self) {
        self.clean_timeout_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn clean_timeout_at(&mut self, now: u64) {
        let mut index = 0;
        while let Some(cached) = self.pool.get(index) {
            if cached.has_timeout_at(now) {
                let socket = cached.socket;
                self.close(socket);
                self.pool.remove(index);
            } else {
                index += 1;
            }
        }
    }

    /// `Clear()` — close every socket in the pool and forget them.
    pub fn clear(&mut self) {
        for cached in std::mem::take(&mut self.pool) {
            if cached.socket.is_valid() {
                self.close(cached.socket);
            }
        }
    }

    /// `Report(_is_reused, _has_received, _is_decode_ok)` — what a task says
    /// about the socket it was given. One that came from the pool and did not
    /// answer, or answered something that could not be read, bans the pool for
    /// [`BAN_INTERVAL`]; one that did is what unbans it.
    pub fn report(&mut self, is_reused: bool, has_received: bool, is_decode_ok: bool) {
        self.report_at(gettickcount(), is_reused, has_received, is_decode_ok)
    }

    /// The same, with the reading handed in.
    pub fn report_at(&mut self, now: u64, is_reused: bool, has_received: bool, is_decode_ok: bool) {
        if !is_reused {
            return;
        }
        if has_received && is_decode_ok {
            self.ban_start = None;
        } else {
            self.ban_start = Some(now);
        }
    }

    fn close(&mut self, socket: SocketFd) {
        if let Some(close) = self.close.as_mut() {
            close(socket);
        }
    }

    fn create_stream(&mut self, socket: SocketFd) -> SocketFd {
        match self.create_stream.as_mut() {
            Some(create_stream) => create_stream(socket),
            None => SocketFd::INVALID,
        }
    }

    fn is_sub_stream(&mut self, socket: SocketFd) -> bool {
        match self.is_sub_stream.as_mut() {
            Some(is_sub_stream) => is_sub_stream(socket),
            None => false,
        }
    }

    fn is_closed(&mut self, socket: SocketFd) -> bool {
        match self.is_closed.as_mut() {
            Some(is_closed) => is_closed(socket),
            None => false,
        }
    }
}

impl std::fmt::Debug for SocketPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SocketPool")
            .field("use_cache", &self.use_cache)
            .field("ban_start", &self.ban_start)
            .field("pool", &self.pool)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(ip: &str, port: u16, host: &str) -> IpPortItem {
        IpPortItem {
            ip: ip.to_string(),
            port,
            host: host.to_string(),
            ..IpPortItem::new(ip, port)
        }
    }

    /// A pool that remembers what it closed.
    fn pool() -> (SocketPool, std::sync::Arc<std::sync::Mutex<Vec<SocketFd>>>) {
        let closed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut pool = SocketPool::new();
        let seen = closed.clone();
        pool.set_close(move |socket| seen.lock().unwrap().push(socket));
        (pool, closed)
    }

    #[test]
    fn a_new_pool_caches_and_is_empty() {
        let (pool, _) = pool();
        assert!(pool.use_cache());
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);
        assert!(!pool.is_banned_at(1_000));
    }

    #[test]
    fn a_socket_comes_back_for_the_pair_it_was_made_for() {
        let (mut pool, _) = pool();
        let address = item("1.1.1.1", 80, "long.example");
        pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 5));

        assert_eq!(pool.get_socket_at(1_000, &address), Some(SocketFd(3)));
        // taken out, not given away twice
        assert_eq!(pool.get_socket_at(1_000, &address), None);
        assert!(pool.is_empty());
    }

    #[test]
    fn a_socket_does_not_come_back_for_another_pair() {
        let (mut pool, _) = pool();
        pool.add_cache(CachedSocket::new_at(
            0,
            item("1.1.1.1", 80, "long.example"),
            SocketFd(3),
            5,
        ));

        assert_eq!(
            pool.get_socket_at(1_000, &item("1.1.1.1", 443, "long.example")),
            None
        );
        assert_eq!(
            pool.get_socket_at(1_000, &item("2.2.2.2", 80, "long.example")),
            None
        );
        assert_eq!(
            pool.get_socket_at(1_000, &item("1.1.1.1", 80, "other.example")),
            None
        );
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn a_socket_whose_time_ran_out_is_closed_and_not_handed_out() {
        let (mut pool, closed) = pool();
        let address = item("1.1.1.1", 80, "long.example");
        // five seconds, in seconds, is 5000 ms
        pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 5));

        assert_eq!(pool.get_socket_at(4_999, &address), Some(SocketFd(3)));
        pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(4), 5));
        assert_eq!(pool.get_socket_at(5_000, &address), None);
        assert_eq!(*closed.lock().unwrap(), vec![SocketFd(4)]);
    }

    #[test]
    fn a_socket_the_peer_closed_is_dropped() {
        let (mut pool, closed) = pool();
        let address = item("1.1.1.1", 80, "long.example");
        pool.set_is_closed(|socket| socket == SocketFd(3));
        pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 5));
        // most recently put in is in front, so this is the one handed out first
        pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(4), 5));

        assert_eq!(pool.get_socket_at(1_000, &address), Some(SocketFd(4)));
        // and the one behind it, which the peer closed, is closed and dropped on
        // the way rather than handed out
        assert_eq!(pool.get_socket_at(1_000, &address), None);
        assert_eq!(*closed.lock().unwrap(), vec![SocketFd(3)]);
        assert!(pool.is_empty());
    }

    #[test]
    fn a_quic_socket_gives_a_stream_and_stays_in_the_pool() {
        let (mut pool, _) = pool();
        let mut address = item("1.1.1.1", 80, "long.example");
        address.transport_protocol = Task::TRANSPORT_PROTOCOL_QUIC;
        pool.set_create_stream(|socket| SocketFd(socket.0 + 100));
        pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 5));

        assert_eq!(pool.get_socket_at(1_000, &address), Some(SocketFd(103)));
        // the socket itself is still there, and its timeout started over
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.get_socket_at(1_000, &address), Some(SocketFd(103)));
        assert_eq!(pool.get_socket_at(6_000, &address), None);
    }

    #[test]
    fn a_quic_socket_that_cannot_make_a_stream_is_dropped() {
        let (mut pool, closed) = pool();
        let mut address = item("1.1.1.1", 80, "long.example");
        address.transport_protocol = Task::TRANSPORT_PROTOCOL_QUIC;
        // no `create_stream` at all is what the C++'s null function pointer is
        pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 5));

        assert_eq!(pool.get_socket_at(1_000, &address), None);
        assert!(pool.is_empty());
        // the socket itself is not closed: it was never given away
        assert!(closed.lock().unwrap().is_empty());
    }

    #[test]
    fn a_socket_that_is_a_stream_is_handed_out_like_a_tcp_one() {
        let (mut pool, _) = pool();
        let mut address = item("1.1.1.1", 80, "long.example");
        address.transport_protocol = Task::TRANSPORT_PROTOCOL_QUIC;
        pool.set_is_sub_stream(|_| true);
        pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 5));

        assert_eq!(pool.get_socket_at(1_000, &address), Some(SocketFd(3)));
        assert!(pool.is_empty());
    }

    #[test]
    fn a_pool_that_is_not_caching_hands_nothing_out() {
        let (mut pool, _) = pool();
        let address = item("1.1.1.1", 80, "long.example");
        pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 5));
        pool.set_use_cache(false);

        assert_eq!(pool.get_socket_at(1_000, &address), None);
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn a_bad_reused_socket_bans_the_pool_and_the_ban_runs_out() {
        let (mut pool, _) = pool();
        let address = item("1.1.1.1", 80, "long.example");
        // a socket that stays in the pool for longer than the ban does
        pool.add_cache(CachedSocket::new_at(0, address.clone(), SocketFd(3), 400));

        // a socket that did not come from the pool says nothing
        pool.report_at(0, false, false, false);
        assert!(!pool.is_banned_at(1_000));

        // one that did, and that answered nothing readable
        pool.report_at(0, true, false, false);
        assert!(pool.is_banned_at(1_000));
        assert_eq!(pool.get_socket_at(1_000, &address), None);

        // the ban is `<= BAN_INTERVAL`, so it is still on at the interval
        // itself and off one tick later
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
        pool.report_at(0, true, false, false);
        assert!(pool.is_banned_at(1_000));

        pool.report_at(2_000, true, true, true);
        assert!(!pool.is_banned_at(2_000));
    }

    #[test]
    fn cleaning_drops_only_the_sockets_whose_time_ran_out() {
        let (mut pool, closed) = pool();
        pool.add_cache(CachedSocket::new_at(
            0,
            item("1.1.1.1", 80, "h"),
            SocketFd(3),
            5,
        ));
        pool.add_cache(CachedSocket::new_at(
            // put in a second later, so its five seconds are not up at 6_000
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
        // most recently put in is in front, so the old one is closed
        assert_eq!(*closed.lock().unwrap(), vec![SocketFd(3)]);
    }

    #[test]
    fn clearing_closes_every_socket_in_the_pool() {
        let (mut pool, closed) = pool();
        pool.add_cache(CachedSocket::new_at(
            0,
            item("1.1.1.1", 80, "h"),
            SocketFd(3),
            5,
        ));
        pool.add_cache(CachedSocket::new_at(
            0,
            item("2.2.2.2", 80, "h"),
            SocketFd(4),
            5,
        ));

        pool.clear();
        assert!(pool.is_empty());
        assert_eq!(closed.lock().unwrap().len(), 2);
    }

    #[test]
    fn the_pool_says_what_it_holds() {
        let (mut pool, _) = pool();
        pool.add_cache(CachedSocket::new_at(
            0,
            item("1.1.1.1", 80, "h"),
            SocketFd(3),
            5,
        ));
        let text = format!("{pool:?}");
        assert!(text.contains("SocketPool"), "{text}");
    }
}
