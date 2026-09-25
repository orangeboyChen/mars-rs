//! `mars/stn/src/longlink_speed_test.cc`, through the public api.
//!
//! The samples are what the C++ answers for the same calls: the noop of the
//! long link going out on every candidate pair at once, the first pair whose
//! answer comes back winning the race, an out-of-band package being no answer
//! at all, and the sockets of the pairs that lost being closed.

use std::sync::{Arc, Mutex};

use mars_stn::longlink::{longlink_pack, LongLinkEncoder};
use mars_stn::longlink_speed_test::{
    LongLinkSpeedTest, Need, Socket, SocketEvent, SpeedTestItem, SpeedTestState, Stop, Watch,
    MAX_RETRIES, OUT_OF_BAND_CMDID, TIMEOUT,
};
use mars_stn::{IpPortItem, IpSourceType, Task};

/// A candidate pair, the way [`mars_stn::net_source`] hands them out.
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

/// A host that writes everything it is given, and whose sockets read the answer
/// of the noop on the pairs `answered` says are up.
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
    assert_eq!(TIMEOUT, 10 * 1000, "what `Select` is given");
    assert_eq!(OUT_OF_BAND_CMDID, 72);
    assert_eq!(MAX_RETRIES, 3, "`tryCount < 3`");
}

#[test]
fn the_noop_that_goes_out_is_the_one_the_long_link_packs() {
    let item = SpeedTestItem::new_at(0, pair("1.1.1.1", 80));
    assert_eq!(
        item.pending(),
        longlink_pack(
            LongLinkEncoder::default().noop_cmdid(),
            Task::NOOP_TASK_ID,
            &[]
        )
    );
}

#[test]
fn a_pair_that_answers_the_noop_wins_the_race() {
    let mut test = LongLinkSpeedTest::new_at(
        0,
        [
            pair("1.1.1.1", 80),
            pair("2.2.2.2", 80),
            pair("3.3.3.3", 443),
        ],
    );
    test.set_select(select(vec![
        // every socket takes its write: the noop goes out on all three
        vec![
            SocketEvent::Writable,
            SocketEvent::Writable,
            SocketEvent::Writable,
        ],
        // and the second one is the first whose answer comes back
        vec![
            SocketEvent::Nothing,
            SocketEvent::Readable,
            SocketEvent::Nothing,
        ],
    ]));
    test.set_open(host(|index| index == 1));

    let fastest = test.fastest_at(0).expect("the second pair answers");
    assert_eq!(fastest.pair.ip, "2.2.2.2");
    assert_eq!(fastest.pair.port, 80);
    assert_eq!(fastest.socket, 1);
    assert_eq!(test.open_sockets(), 1, "the losers' sockets are closed");
    assert_eq!(
        test.results().map(|(_, state)| state).collect::<Vec<_>>(),
        vec![
            SpeedTestState::Resp,
            SpeedTestState::Suc,
            SpeedTestState::Resp
        ]
    );
}

#[test]
fn a_race_that_nobody_wins_hands_nothing_back() {
    let mut test = LongLinkSpeedTest::new_at(0, [pair("1.1.1.1", 80), pair("2.2.2.2", 80)]);
    test.set_select(select(vec![
        vec![SocketEvent::Writable, SocketEvent::Writable],
        vec![SocketEvent::Readable, SocketEvent::Readable],
    ]));
    test.set_open(host(|_| false));

    assert_eq!(test.fastest_at(0), None);
    assert_eq!(test.open_sockets(), 0, "every socket is closed");
    assert!(test
        .results()
        .all(|(_, state)| state == SpeedTestState::Fail));
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
        let rounds = Arc::new(Mutex::new(0usize));
        let count = Arc::clone(&rounds);
        test.set_select(move |_items| {
            *count
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) += 1;
            Err(stop)
        });
        assert_eq!(test.fastest_at(0), None);
        assert_eq!(
            *rounds
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            1
        );
    }
}

#[test]
fn an_out_of_band_package_is_no_answer_and_the_noop_goes_out_again() {
    let mut item = SpeedTestItem::new_at(0, pair("1.1.1.1", 80));
    assert_eq!(item.apply(SocketEvent::Writable, 0), Need::Write);
    let whole = item.pending().len();
    assert_eq!(item.on_sent(whole as isize), Need::Read);

    let out_of_band = longlink_pack(OUT_OF_BAND_CMDID, Task::NOOP_TASK_ID, &[]);
    assert_eq!(item.on_received(&out_of_band), Need::Write);
    assert_eq!(item.state(), SpeedTestState::OutOfBand);
    assert_eq!(item.pending().len(), whole);

    // and the answer to the noop that went out again is what wins
    assert_eq!(item.on_sent(whole as isize), Need::Read);
    assert_eq!(item.on_received(&noop_answer()), Need::Nothing);
    assert_eq!(item.state(), SpeedTestState::Suc);
}

#[test]
fn what_the_select_has_to_watch_a_socket_for() {
    let mut item = SpeedTestItem::new_at(0, pair("1.1.1.1", 80));
    // a pair that is still connecting is watched for both
    assert_eq!(item.watch(), Watch::ReadWrite);

    item.apply(SocketEvent::Writable, 0);
    item.on_sent(item.pending().len() as isize);
    assert_eq!(item.state(), SpeedTestState::Resp);
    assert_eq!(item.watch(), Watch::Read, "only reading is left");

    item.on_received(&[]);
    assert_eq!(
        item.watch(),
        Watch::None,
        "and a pair that is done, for nothing"
    );
}

#[test]
fn a_connect_is_timed_from_the_first_write_the_socket_takes() {
    let mut item = SpeedTestItem::new_at(1_000, pair("1.1.1.1", 80));
    assert_eq!(item.connect_ms(), 0, "nothing has been written yet");
    item.apply(SocketEvent::Writable, 1_250);
    assert_eq!(item.connect_ms(), 250);
    // and a second write does not move it
    item.apply(SocketEvent::Writable, 9_000);
    assert_eq!(item.connect_ms(), 250);
}

#[test]
fn without_a_host_there_is_no_race() {
    let mut test = LongLinkSpeedTest::default();
    assert!(test.items().is_empty());
    assert_eq!(test.fastest(), None);
    assert!(test.results().next().is_none());
    assert!(format!("{test:?}").contains("LongLinkSpeedTest"));

    // candidates but no select: the race cannot be run
    let mut test = LongLinkSpeedTest::new([pair("1.1.1.1", 80)]);
    assert_eq!(test.fastest(), None);
    assert_eq!(test.round(&[SocketEvent::Writable], 0), vec![Need::Nothing]);
}
