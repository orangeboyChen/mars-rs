//! `mars/stn/src/longlink_speed_test.cc` — which of a link's pairs answers
//! first.
//!
//! A long link does not ask dns which ip to use and then trust it: it races
//! them. The same noop goes out on every candidate pair at once, and the first
//! pair whose answer comes back is the pair the link is made on.
//!
//! What the C++ does with a socket, a `SocketSelect` and three fd sets is two
//! values here. One is the state of a single candidate ([`SpeedTestItem`]): it
//! knows the noop it has to write, how much of it has gone out, what has come
//! back, and whether that was an answer ([`SpeedTestState`]). The other is the
//! race itself ([`LongLinkSpeedTest`]), which steps every candidate with what
//! happened on its socket ([`SocketEvent`]) and asks the item what it wants
//! next ([`Need`]).
//!
//! The host owns the sockets, the select and the clock: it makes one
//! [`Socket`] per pair, answers what is ready on each of them, and is handed
//! the bytes to write and the bytes that came in. Nothing here blocks, and
//! nothing here knows what a file descriptor is — which is why
//! [`LongLinkSpeedTest::fastest`] hands back *which* candidate won rather than
//! the fd the C++ hands back.
//!
//! The noop the candidates race with is the one [`crate::longlink`] packs and
//! unpacks, and an answer is what that module calls a response to the noop
//! task; an out-of-band package is not an answer and the noop goes out again.

use crate::longlink::{longlink_pack, longlink_unpack, LongLinkEncoder, Unpacked};
use crate::simple_ipport_sort::IpPortItem;
use crate::task::Task;
use mars_comm::tickcount::gettickcount;

/// `kTimeout` — what `SocketSelect::Select` is given, in milliseconds.
pub const TIMEOUT: u64 = 10 * 1000;
/// `kCmdIdOutOfBand` — a package with this command id is a message from the
/// server, not an answer to the noop.
pub const OUT_OF_BAND_CMDID: u32 = 72;
/// How many `EINTR`s the C++'s loop sits through before it gives up (`tryCount
/// < 3`).
pub const MAX_RETRIES: usize = 3;

/// `ELongLinkSpeedTestState` of the C++ — where one candidate pair is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpeedTestState {
    /// `kLongLinkSpeedTestConnecting` — the connect is still in flight.
    #[default]
    Connecting,
    /// `kLongLinkSpeedTestReq` — the noop is going out.
    Req,
    /// `kLongLinkSpeedTestResp` — the answer is coming in.
    Resp,
    /// `kLongLinkSpeedTestOOB` — the server said something else first.
    OutOfBand,
    /// `kLongLinkSpeedTestSuc` — the noop was answered.
    Suc,
    /// `kLongLinkSpeedTestFail`.
    Fail,
}

impl SpeedTestState {
    /// Whether the race is over for this pair, which is what decides whether
    /// the loop goes on.
    pub fn is_over(self) -> bool {
        matches!(self, Self::Suc | Self::Fail)
    }
}

/// What the host's select says happened on one socket this round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketEvent {
    /// Nothing was ready: `HandleFDISSet`'s empty `else`.
    Nothing,
    /// `Read_FD_ISSET`.
    Readable,
    /// `Write_FD_ISSET`.
    Writable,
    /// `Exception_FD_ISSET`.
    Exception,
}

/// What the item wants done with its socket next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Nothing: the pair is done, or nothing was ready for it.
    Nothing,
    /// Write [`SpeedTestItem::pending`].
    Write,
    /// Read, and hand what came in to [`SpeedTestItem::on_received`].
    Read,
}

/// `HandleSetFD` — what the select has to watch a socket for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Watch {
    /// A pair that is done is watched for nothing.
    None,
    /// `kLongLinkSpeedTestResp`: only reading is left.
    Read,
    /// Connecting, or the noop going out: both.
    ReadWrite,
}

/// Why the C++'s loop gives up: `Select` answering `0`, answering less than
/// `0` without an `EINTR` it still wants to sit through, `IsException()`, or
/// `IsBreak()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// `Select` answered `0`.
    Timeout,
    /// `Select` answered less than `0` with `EINTR`.
    Interrupted,
    /// `IsException()`.
    Exception,
    /// `IsBreak()` — the breakerPipe was woken.
    Broken,
}

/// `send` on a socket: how many bytes went out. `0` or fewer is a fail, the way
/// the C++ reads `nwrite <= 0`.
pub type SocketSend = dyn FnMut(&[u8]) -> isize + Send;
/// `recv` on a socket: the bytes that came in. Nothing is a fail, the way the
/// C++ reads `nrecv <= 0`.
pub type SocketRecv = dyn FnMut() -> Vec<u8> + Send;

/// What the host does with one socket. The port has no sockets of its own: it
/// writes and reads through these, and dropping one is what closes it.
pub struct Socket {
    send: Box<SocketSend>,
    recv: Box<SocketRecv>,
}

impl Socket {
    /// A socket that writes and reads through two closures of the host's.
    pub fn new(
        send: impl FnMut(&[u8]) -> isize + Send + 'static,
        recv: impl FnMut() -> Vec<u8> + Send + 'static,
    ) -> Self {
        Self {
            send: Box::new(send),
            recv: Box::new(recv),
        }
    }
}

impl std::fmt::Debug for Socket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Socket")
    }
}

/// `socket()` + `connect()` of the C++'s item constructor: one socket per
/// candidate pair.
pub type Open = dyn FnMut(&str, u16) -> Socket + Send;
/// `SocketSelect::Select(kTimeout)` — one [`SocketEvent`] per candidate, in the
/// order they were handed in, or the reason the race has to stop.
pub type Select = dyn FnMut(&[SpeedTestItem]) -> Result<Vec<SocketEvent>, Stop> + Send;

/// `LongLinkSpeedTestItem` — one candidate pair: the noop it has to write, how
/// much of it has gone out, what has come back, and where that leaves it.
#[derive(Debug, Clone)]
pub struct SpeedTestItem {
    pair: IpPortItem,
    request: Vec<u8>,
    sent: usize,
    response: Vec<u8>,
    state: SpeedTestState,
    connect_started: u64,
    connected: Option<u64>,
}

impl SpeedTestItem {
    /// One candidate pair, with the reading of the clock the connect starts at.
    pub fn new(pair: IpPortItem) -> Self {
        Self::new_at(gettickcount(), pair)
    }

    /// The same, with the reading handed in. The noop is packed here, the way
    /// the C++'s constructor packs it before it connects.
    pub fn new_at(now: u64, pair: IpPortItem) -> Self {
        Self {
            pair,
            request: longlink_pack(
                LongLinkEncoder::default().noop_cmdid(),
                Task::NOOP_TASK_ID,
                &[],
            ),
            sent: 0,
            response: Vec::new(),
            state: SpeedTestState::default(),
            connect_started: now,
            connected: None,
        }
    }

    /// Where the pair is.
    pub fn state(&self) -> SpeedTestState {
        self.state
    }

    /// The pair this item is testing.
    pub fn pair(&self) -> &IpPortItem {
        &self.pair
    }

    /// How long the connect took, in milliseconds — `0` until the socket took
    /// its first write.
    pub fn connect_ms(&self) -> u64 {
        self.connected
            .unwrap_or(self.connect_started)
            .saturating_sub(self.connect_started)
    }

    /// `HandleSetFD` — what the select has to watch this socket for.
    pub fn watch(&self) -> Watch {
        match self.state {
            SpeedTestState::Connecting | SpeedTestState::Req | SpeedTestState::OutOfBand => {
                Watch::ReadWrite
            }
            SpeedTestState::Resp => Watch::Read,
            SpeedTestState::Suc | SpeedTestState::Fail => Watch::None,
        }
    }

    /// The bytes of the noop that have not gone out yet.
    pub fn pending(&self) -> &[u8] {
        &self.request[self.sent..]
    }

    /// `HandleFDISSet` — one thing happened on the socket, and this is what the
    /// item wants next. A pair that is already done is left alone.
    pub fn apply(&mut self, event: SocketEvent, now: u64) -> Need {
        match (self.state, event) {
            (state, _) if state.is_over() => Need::Nothing,
            (_, SocketEvent::Nothing) => Need::Nothing,
            (_, SocketEvent::Exception) => {
                self.state = SpeedTestState::Fail;
                Need::Nothing
            }
            (_, SocketEvent::Writable) => {
                // the first write the socket takes is the connect finishing
                self.connected.get_or_insert(now);
                self.state = SpeedTestState::Req;
                Need::Write
            }
            (_, SocketEvent::Readable) => {
                self.state = SpeedTestState::Resp;
                Need::Read
            }
        }
    }

    /// `send` handed back how many bytes went out, and this is what the item
    /// wants next.
    pub fn on_sent(&mut self, written: isize) -> Need {
        if written <= 0 {
            self.state = SpeedTestState::Fail;
            return Need::Nothing;
        }

        self.sent = (self.sent + written as usize).min(self.request.len());
        if self.sent == self.request.len() {
            self.state = SpeedTestState::Resp;
            Need::Read
        } else {
            self.state = SpeedTestState::Req;
            Need::Write
        }
    }

    /// `recv` handed back what came in, and this is what the item wants next.
    ///
    /// An out-of-band package is not an answer: it is thrown away and the noop
    /// goes out again, which is what it is for. The C++ instead sends the bytes
    /// it has left — none, the noop having gone out whole — and a send of
    /// nothing is a fail there.
    pub fn on_received(&mut self, bytes: &[u8]) -> Need {
        if bytes.is_empty() {
            self.state = SpeedTestState::Fail;
            return Need::Nothing;
        }
        self.response.extend_from_slice(bytes);

        match longlink_unpack(&self.response) {
            // not a whole package yet: read again
            Unpacked::Continue => {
                self.state = SpeedTestState::Resp;
                Need::Read
            }
            // not one of ours
            Unpacked::False => {
                self.state = SpeedTestState::Fail;
                Need::Nothing
            }
            Unpacked::Package { cmdid, .. } if cmdid == OUT_OF_BAND_CMDID => {
                self.response.clear();
                self.sent = 0;
                self.state = SpeedTestState::OutOfBand;
                Need::Write
            }
            // the answer to the noop: this pair wins
            Unpacked::Package { cmdid, .. }
                if LongLinkEncoder::default().noop_isresp(Task::NOOP_TASK_ID, cmdid) =>
            {
                self.state = SpeedTestState::Suc;
                Need::Nothing
            }
            Unpacked::Package { .. } => {
                self.state = SpeedTestState::Fail;
                Need::Nothing
            }
        }
    }
}

/// What `GetFastestSocket` hands back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fastest {
    /// The pair whose noop was answered.
    pub pair: IpPortItem,
    /// `_connectMillSec` — how long its connect took.
    pub connect_ms: u64,
    /// Which of the candidates it was. The port has no fds to hand out; this is
    /// the handle the host maps back to the socket it made.
    pub socket: usize,
}

/// `LongLinkSpeedTest` — the race. It holds one [`SpeedTestItem`] per candidate
/// pair and one [`Socket`] per item.
pub struct LongLinkSpeedTest {
    items: Vec<SpeedTestItem>,
    sockets: Vec<Socket>,
    open: Option<Box<Open>>,
    select: Option<Box<Select>>,
    retries: usize,
}

impl Default for LongLinkSpeedTest {
    /// A race with no candidates in it, which is one that has nothing to
    /// answer.
    fn default() -> Self {
        Self::new([])
    }
}

impl LongLinkSpeedTest {
    /// The candidates, with the reading of the clock the connects start at.
    pub fn new(pairs: impl IntoIterator<Item = IpPortItem>) -> Self {
        Self::new_at(gettickcount(), pairs)
    }

    /// The same, with the reading handed in.
    pub fn new_at(now: u64, pairs: impl IntoIterator<Item = IpPortItem>) -> Self {
        Self {
            items: pairs
                .into_iter()
                .map(|pair| SpeedTestItem::new_at(now, pair))
                .collect(),
            sockets: Vec::new(),
            open: None,
            select: None,
            retries: 0,
        }
    }

    /// `socket()` + `connect()` — how the host makes a socket for a pair.
    /// Unset, the candidates are stepped but nothing is written or read.
    pub fn set_open(&mut self, open: impl FnMut(&str, u16) -> Socket + Send + 'static) {
        self.open = Some(Box::new(open));
    }

    /// `SocketSelect::Select` — what is ready on each socket this round.
    /// Unset, every round answers [`Stop::Timeout`].
    pub fn set_select(
        &mut self,
        select: impl FnMut(&[SpeedTestItem]) -> Result<Vec<SocketEvent>, Stop> + Send + 'static,
    ) {
        self.select = Some(Box::new(select));
    }

    /// The candidates, in the order they were handed in.
    pub fn items(&self) -> &[SpeedTestItem] {
        &self.items
    }

    /// `ReportLongLinkSpeedTestResult` — what the host reports: every pair with
    /// where its race ended.
    pub fn results(&self) -> impl Iterator<Item = (&IpPortItem, SpeedTestState)> {
        self.items.iter().map(|item| (item.pair(), item.state()))
    }

    /// How many sockets are still open. The C++ closes the ones that lost, so
    /// after a race this is one at most.
    pub fn open_sockets(&self) -> usize {
        self.sockets.len()
    }

    /// `GetFastestSocket` — runs the race, reading the clock again after every
    /// `Select`: the connect of a pair is over when the host's select says its
    /// socket takes a write, which is a later reading than the one the race
    /// started with.
    pub fn fastest(&mut self) -> Option<Fastest> {
        self.race(gettickcount)
    }

    /// The same with one reading of the clock for every round: for a host that
    /// steps the clock itself, or a test that wants to say when each round
    /// happened. One round per `Select`, until a pair was answered, every pair
    /// has failed, or the select gave up.
    pub fn fastest_at(&mut self, now: u64) -> Option<Fastest> {
        self.race(|| now)
    }

    fn race(&mut self, mut clock: impl FnMut() -> u64) -> Option<Fastest> {
        while self.sockets.len() < self.items.len() {
            let Some(open) = self.open.as_mut() else {
                break;
            };
            let (ip, port) = {
                let item = &self.items[self.sockets.len()];
                (item.pair.ip.clone(), item.pair.port)
            };
            self.sockets.push(open(&ip, port));
        }

        loop {
            let events = match self.select() {
                Ok(events) => events,
                // `EINTR` is sat through, but not for ever
                Err(Stop::Interrupted) if self.retries < MAX_RETRIES => {
                    self.retries += 1;
                    continue;
                }
                Err(_) => break,
            };

            // the C++'s item reads the clock when its socket takes the first
            // write, which is here: after the select, before the round
            let now = clock();
            self.round(&events, now);

            if self
                .items
                .iter()
                .any(|item| item.state() == SpeedTestState::Suc)
            {
                break;
            }
            if self
                .items
                .iter()
                .all(|item| item.state() == SpeedTestState::Fail)
            {
                break;
            }
        }

        self.winner()
    }

    /// One round of the loop: every candidate is stepped with its event, and
    /// what it wants written or read is done on its socket.
    pub fn round(&mut self, events: &[SocketEvent], now: u64) -> Vec<Need> {
        let mut needs = Vec::with_capacity(events.len());
        for (index, event) in events.iter().copied().enumerate() {
            let Some(item) = self.items.get_mut(index) else {
                continue;
            };
            let need = item.apply(event, now);
            match need {
                Need::Nothing => {}
                Need::Write => {
                    let bytes = item.pending().to_vec();
                    let written = match self.sockets.get_mut(index) {
                        Some(socket) => (socket.send)(&bytes),
                        None => -1,
                    };
                    needs.push(item.on_sent(written));
                    continue;
                }
                Need::Read => {
                    let bytes = match self.sockets.get_mut(index) {
                        Some(socket) => (socket.recv)(),
                        None => Vec::new(),
                    };
                    needs.push(item.on_received(&bytes));
                    continue;
                }
            }
            needs.push(need);
        }
        needs
    }

    /// `SocketSelect::Select` of the host, `None` for one that has no host and
    /// so no race to run.
    fn select(&mut self) -> Result<Vec<SocketEvent>, Stop> {
        match self.select.as_mut() {
            Some(select) => select(&self.items),
            None => Err(Stop::Timeout),
        }
    }

    /// The pair that was answered, with the sockets of the ones that lost
    /// dropped — `CloseSocket` of the C++'s loop.
    fn winner(&mut self) -> Option<Fastest> {
        let socket = self
            .items
            .iter()
            .position(|item| item.state() == SpeedTestState::Suc);
        let mut sockets = std::mem::take(&mut self.sockets);
        self.sockets = socket
            .filter(|index| *index < sockets.len())
            .map(|index| sockets.remove(index))
            .into_iter()
            .collect();
        drop(sockets);

        socket.map(|index| Fastest {
            pair: self.items[index].pair().clone(),
            connect_ms: self.items[index].connect_ms(),
            socket: index,
        })
    }
}

impl std::fmt::Debug for LongLinkSpeedTest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LongLinkSpeedTest")
            .field("items", &self.items)
            .field("open_sockets", &self.sockets.len())
            .field("retries", &self.retries)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simple_ipport_sort::IpSourceType;
    use std::sync::{Arc, Mutex};

    /// A pair to race, with the source a caller would have set.
    fn pair(ip: &str, port: u16) -> IpPortItem {
        let mut pair = IpPortItem::new(ip, port);
        pair.source_type = IpSourceType::NewDns;
        pair.host = "long.example".to_string();
        pair
    }

    /// The answer to the noop, as the long link would have received it.
    fn noop_answer() -> Vec<u8> {
        longlink_pack(
            LongLinkEncoder::default().noop_cmdid(),
            Task::NOOP_TASK_ID,
            &[],
        )
    }

    /// A host that writes everything it is given, and whose sockets read the
    /// answer of the noop on the pairs `answered` says are up.
    fn host(
        answered: impl Fn(usize) -> bool + Send + Sync + 'static,
    ) -> impl FnMut(&str, u16) -> Socket + Send + 'static {
        let answered = Arc::new(answered);
        let next = Arc::new(Mutex::new(0usize));
        move |_ip, _port| {
            let index = {
                let mut next = next.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let index = *next;
                *next += 1;
                index
            };
            let answered = Arc::clone(&answered);
            Socket::new(
                |bytes| bytes.len() as isize,
                move || {
                    if answered(index) {
                        noop_answer()
                    } else {
                        Vec::new()
                    }
                },
            )
        }
    }

    /// A select that answers `events` round by round and then times out.
    fn select(
        events: Vec<Vec<SocketEvent>>,
    ) -> impl FnMut(&[SpeedTestItem]) -> Result<Vec<SocketEvent>, Stop> + Send + 'static {
        let mut rounds = events.into_iter();
        move |_items| rounds.next().ok_or(Stop::Timeout)
    }

    #[test]
    fn the_numbers_are_the_ones_the_c_plus_plus_writes_down() {
        assert_eq!(TIMEOUT, 10 * 1000);
        assert_eq!(OUT_OF_BAND_CMDID, 72);
        assert_eq!(MAX_RETRIES, 3);
    }

    #[test]
    fn a_noop_that_is_answered_wins() {
        let mut item = SpeedTestItem::new_at(1_000, pair("1.1.1.1", 80));
        assert_eq!(item.state(), SpeedTestState::Connecting);
        assert_eq!(item.watch(), Watch::ReadWrite);

        // the socket takes a write: the connect is over and the noop goes out
        assert_eq!(item.apply(SocketEvent::Writable, 1_500), Need::Write);
        assert_eq!(item.connect_ms(), 500);
        let whole = item.pending().len();
        assert_eq!(item.on_sent(whole as isize), Need::Read);
        assert_eq!(item.watch(), Watch::Read);

        // and the answer to it is what makes the pair win
        assert_eq!(item.on_received(&noop_answer()), Need::Nothing);
        assert_eq!(item.state(), SpeedTestState::Suc);
        assert_eq!(item.watch(), Watch::None);
    }

    #[test]
    fn a_noop_that_goes_out_in_two_writes_is_still_written_twice() {
        let mut item = SpeedTestItem::new_at(0, pair("1.1.1.1", 80));
        item.apply(SocketEvent::Writable, 10);
        let whole = item.pending().len();
        assert_eq!(item.on_sent(whole as isize - 1), Need::Write);
        assert_eq!(item.state(), SpeedTestState::Req);
        assert_eq!(item.pending().len(), 1);
        assert_eq!(item.on_sent(1), Need::Read);
    }

    #[test]
    fn a_write_or_a_read_that_does_not_happen_is_a_fail() {
        let mut item = SpeedTestItem::new_at(0, pair("1.1.1.1", 80));
        item.apply(SocketEvent::Writable, 0);
        assert_eq!(item.on_sent(0), Need::Nothing);
        assert!(item.state().is_over());

        let mut item = SpeedTestItem::new_at(0, pair("1.1.1.1", 80));
        item.apply(SocketEvent::Readable, 0);
        assert_eq!(item.on_received(&[]), Need::Nothing);
        assert_eq!(item.state(), SpeedTestState::Fail);
    }

    #[test]
    fn an_answer_that_is_not_whole_yet_is_read_again() {
        let mut item = SpeedTestItem::new_at(0, pair("1.1.1.1", 80));
        let answer = noop_answer();
        assert_eq!(item.on_received(&answer[..answer.len() - 1]), Need::Read);
        assert_eq!(item.state(), SpeedTestState::Resp);
        assert_eq!(item.on_received(&answer[answer.len() - 1..]), Need::Nothing);
        assert_eq!(item.state(), SpeedTestState::Suc);
    }

    #[test]
    fn a_package_that_is_not_an_answer_to_the_noop_is_a_fail() {
        // not one of ours at all
        let mut item = SpeedTestItem::new_at(0, pair("1.1.1.1", 80));
        assert_eq!(item.on_received(&[0u8; 32]), Need::Nothing);
        assert_eq!(item.state(), SpeedTestState::Fail);

        // one of ours, but a different command id
        let mut item = SpeedTestItem::new_at(0, pair("1.1.1.1", 80));
        let pushed = longlink_pack(1, Task::NOOP_TASK_ID, &[]);
        assert_eq!(item.on_received(&pushed), Need::Nothing);
        assert_eq!(item.state(), SpeedTestState::Fail);
    }

    #[test]
    fn an_out_of_band_package_is_not_an_answer_and_the_noop_goes_out_again() {
        let mut item = SpeedTestItem::new_at(0, pair("1.1.1.1", 80));
        item.apply(SocketEvent::Writable, 0);
        let whole = item.pending().len();
        item.on_sent(whole as isize);

        let out_of_band = longlink_pack(OUT_OF_BAND_CMDID, Task::NOOP_TASK_ID, &[]);
        assert_eq!(item.on_received(&out_of_band), Need::Write);
        assert_eq!(item.state(), SpeedTestState::OutOfBand);
        assert_eq!(item.pending().len(), whole, "the noop goes out again");
        assert_eq!(item.watch(), Watch::ReadWrite);
    }

    #[test]
    fn an_exception_on_the_socket_is_a_fail_and_a_pair_that_is_done_is_left_alone() {
        let mut item = SpeedTestItem::new_at(0, pair("1.1.1.1", 80));
        assert_eq!(item.apply(SocketEvent::Exception, 0), Need::Nothing);
        assert_eq!(item.state(), SpeedTestState::Fail);

        // and nothing moves it afterwards
        assert_eq!(item.apply(SocketEvent::Writable, 0), Need::Nothing);
        assert_eq!(item.apply(SocketEvent::Readable, 0), Need::Nothing);
        assert_eq!(item.state(), SpeedTestState::Fail);
    }

    #[test]
    fn the_race_is_won_by_the_first_pair_that_answers() {
        let mut test = LongLinkSpeedTest::new_at(0, [pair("1.1.1.1", 80), pair("2.2.2.2", 80)]);
        test.set_select(select(vec![
            vec![SocketEvent::Writable, SocketEvent::Writable],
            vec![SocketEvent::Nothing, SocketEvent::Readable],
        ]));
        test.set_open(host(|index| index == 1));

        let fastest = test.fastest_at(0).expect("the second pair answers");
        assert_eq!(fastest.pair.ip, "2.2.2.2");
        assert_eq!(fastest.socket, 1);
        assert_eq!(fastest.connect_ms, 0);
        assert_eq!(test.open_sockets(), 1, "the loser's socket is closed");
        assert_eq!(
            test.results().map(|(_, state)| state).collect::<Vec<_>>(),
            vec![SpeedTestState::Resp, SpeedTestState::Suc]
        );
    }

    #[test]
    fn a_race_nobody_wins_hands_nothing_back() {
        let mut test = LongLinkSpeedTest::new_at(0, [pair("1.1.1.1", 80)]);
        test.set_select(select(vec![
            vec![SocketEvent::Writable],
            vec![SocketEvent::Readable],
        ]));
        test.set_open(host(|_| false));

        assert_eq!(test.fastest_at(0), None);
        assert_eq!(test.open_sockets(), 0, "every socket is closed");
    }

    #[test]
    fn a_select_that_gives_up_ends_the_race() {
        // a timeout on the first round
        let mut test = LongLinkSpeedTest::new_at(0, [pair("1.1.1.1", 80)]);
        test.set_select(|_items| Err(Stop::Timeout));
        assert_eq!(test.fastest_at(0), None);

        // `EINTR` is sat through, but only `MAX_RETRIES` times
        let mut test = LongLinkSpeedTest::new_at(0, [pair("1.1.1.1", 80)]);
        let rounds = Arc::new(Mutex::new(0usize));
        let count = Arc::clone(&rounds);
        test.set_select(move |_items| {
            *count
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) += 1;
            Err(Stop::Interrupted)
        });
        assert_eq!(test.fastest_at(0), None);
        assert_eq!(
            *rounds
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            MAX_RETRIES + 1
        );

        // and an exception or a woken breaker ends it at once
        for stop in [Stop::Exception, Stop::Broken] {
            let mut test = LongLinkSpeedTest::new_at(0, [pair("1.1.1.1", 80)]);
            test.set_select(move |_items| Err(stop));
            assert_eq!(test.fastest_at(0), None);
        }
    }

    #[test]
    fn without_a_host_there_is_no_race() {
        let mut test = LongLinkSpeedTest::default();
        assert!(test.items().is_empty());
        assert_eq!(test.fastest_at(0), None);

        // candidates but no select: the race cannot be run
        let mut test = LongLinkSpeedTest::new([pair("1.1.1.1", 80)]);
        assert_eq!(test.fastest(), None);
        assert_eq!(test.round(&[SocketEvent::Writable], 0), vec![Need::Nothing]);
        assert!(format!("{test:?}").contains("LongLinkSpeedTest"));
    }
}
