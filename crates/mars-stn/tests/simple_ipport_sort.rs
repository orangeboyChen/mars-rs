//! `mars/stn/src/simple_ipport_sort.cc`, through the public api.
//!
//! The samples are what the C++ answers for the same calls: three failures in
//! the history keep a pair out for `kBanTime`, the pairs with a history are
//! tried least-failed first, and what was learned comes back the next time the
//! app starts — here through the host's own storage instead of
//! `ipportrecords2.xml`.

use mars_stn::simple_ipport_sort::{
    IpPortItem, Record, RecordItem, SimpleIpPortSort, BAN_TIME, RECORD_TIMEOUT, SERVER_BAN_TIME,
};

/// A sort on `"wifi"`, seeded so the shuffle is the same every time.
fn a_sort() -> SimpleIpPortSort {
    let mut sort = SimpleIpPortSort::seeded(7);
    sort.set_net_label(|| Some("wifi".to_string()));
    sort
}

/// `Update(ip, port, false)`, `spaced` far enough from the one before it for
/// `kFailUpdateInterval` to let it through.
fn fail(sort: &mut SimpleIpPortSort, now: u64, ip: &str, port: u16) {
    sort.update_at(now, now / 1000, ip, port, false);
}

#[test]
fn three_failures_take_a_pair_out_of_the_candidates() {
    let mut sort = a_sort();
    for at in [0, 11_000, 22_000] {
        fail(&mut sort, at, "1.2.3.4", 80);
    }
    assert!(sort.is_banned_at(22_000, "1.2.3.4", 80));

    let items = vec![
        IpPortItem::new("1.2.3.4", 80),
        IpPortItem::new("1.2.3.4", 443),
        IpPortItem::new("5.6.7.8", 80),
    ];
    let items = sort.sort_and_filter_at(22_000, items, 3, false);
    assert_eq!(items.len(), 2, "the pair that failed three times is out");
    assert!(items
        .iter()
        .all(|item| !(item.ip == "1.2.3.4" && item.port == 80)));

    // `kBanTime` later it is a candidate again
    let items = vec![IpPortItem::new("1.2.3.4", 80)];
    let items = sort.sort_and_filter_at(22_000 + BAN_TIME, items, 3, false);
    assert_eq!(items.len(), 1);
}

#[test]
fn a_server_ban_takes_the_whole_ip_out() {
    let mut sort = a_sort();
    sort.add_server_ban_at(0, "1.2.3.4");

    let items = vec![
        IpPortItem::new("1.2.3.4", 80),
        IpPortItem::new("1.2.3.4", 443),
        IpPortItem::new("5.6.7.8", 80),
    ];
    let items = sort.sort_and_filter_at(0, items, 3, false);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].ip, "5.6.7.8");

    // `kServerBanTime` later the ip is back
    let items = vec![
        IpPortItem::new("1.2.3.4", 80),
        IpPortItem::new("5.6.7.8", 80),
    ];
    let items = sort.sort_and_filter_at(SERVER_BAN_TIME, items, 3, false);
    assert_eq!(items.len(), 2);
}

#[test]
fn the_least_failed_pair_is_tried_first_and_the_count_is_kept() {
    let mut sort = a_sort();
    // two failures on :80, one on :443 — both below `kBanFailCount`
    fail(&mut sort, 0, "1.2.3.4", 80);
    fail(&mut sort, 11_000, "1.2.3.4", 80);
    fail(&mut sort, 22_000, "5.6.7.8", 443);

    let items = vec![
        IpPortItem::new("1.2.3.4", 80),
        IpPortItem::new("5.6.7.8", 443),
    ];
    let items = sort.sort_and_filter_at(33_000, items, 2, false);
    let ports: Vec<u16> = items.iter().map(|item| item.port).collect();
    assert_eq!(ports, vec![443, 80], "one failure before two");

    // ... and `_needcount` keeps the first one only
    let items = vec![
        IpPortItem::new("1.2.3.4", 80),
        IpPortItem::new("5.6.7.8", 443),
    ];
    let items = sort.sort_and_filter_at(33_000, items, 1, false);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].port, 443);
}

#[test]
fn what_the_host_saved_is_what_the_next_run_reads() {
    let mut sort = a_sort();
    fail(&mut sort, 0, "1.2.3.4", 80);
    fail(&mut sort, 11_000, "1.2.3.4", 80);
    let saved = sort.save_records(0);

    // the next run: the same records, and the ban list they describe
    let mut next = a_sort();
    next.load_records(saved, 0);
    next.init_history_to_banned_list();
    assert_eq!(next.ban_list().len(), 1);
    assert_eq!(next.ban_list()[0].ip, "1.2.3.4");
    // the xml keeps one *bit* per attempt and `InitHistory2BannedList` reads
    // one *bit* per *byte* out of it, so the two failures that went in as
    // `0b11` come back as a single one, and the oldest of the eight
    assert_eq!(next.ban_list()[0].records, 0b1000_0000);

    // ... and the pair with a history goes in front of the one without one:
    // `rand()` deciding for the queue the sort put first every time
    next.set_random(|_bound| 0);
    let items = vec![
        IpPortItem::new("1.2.3.4", 80),
        IpPortItem::new("5.6.7.8", 80),
    ];
    let items = next.sort_and_filter_at(0, items, 2, false);
    assert_eq!(items.first().map(|item| item.ip.as_str()), Some("1.2.3.4"));

    // a record a day old does not survive the save
    assert!(next.save_records(RECORD_TIMEOUT).is_empty());
}

#[test]
fn a_history_the_host_hands_in_is_read_byte_by_byte() {
    let mut sort = a_sort();
    sort.load_records(
        vec![Record {
            net_info: "wifi".to_string(),
            time: Some(1_700_000_000),
            items: vec![RecordItem {
                ip: "1.2.3.4".to_string(),
                port: 80,
                // eight attempts, one byte each, the oldest in the low byte:
                // three in the first one and one in the next, and both of
                // those are failures
                history_result: 0x00_00_00_00_00_00_01_07,
            }],
        }],
        1_700_000_000,
    );
    sort.init_history_to_banned_list();

    assert_eq!(sort.ban_list().len(), 1);
    // two of the eight bytes are not `0`, so two of the eight attempts failed
    assert_eq!(sort.ban_list()[0].records, 0b1100_0000);
    // ... but a pair known only from history has no `last_fail_time`, so it is
    // never banned, only sorted last
    assert!(!sort.is_banned_at(1_000, "1.2.3.4", 80));
}

#[test]
fn removing_an_ip_forgets_every_port_of_it() {
    let mut sort = a_sort();
    fail(&mut sort, 0, "1.2.3.4", 80);
    fail(&mut sort, 0, "1.2.3.4", 443);
    assert_eq!(sort.ban_list().len(), 2);

    // `RemoveLongBanIP`, after a speed test showed the ip is fine again
    sort.remove_banned_list("1.2.3.4");
    assert!(sort.ban_list().is_empty());
}

#[test]
fn with_no_network_nothing_is_learned_and_nothing_is_banned() {
    let mut sort = SimpleIpPortSort::seeded(7);
    // `getCurrNetLabel` answering `kNoNet`, which is also the unset callback
    sort.update_at(0, 0, "1.2.3.4", 80, false);
    assert!(sort.records().is_empty());
    assert!(sort.ban_list().is_empty());

    // and the candidates come back in whatever order they went in
    let items: Vec<IpPortItem> = (0..4)
        .map(|port| IpPortItem::new("1.2.3.4", port))
        .collect();
    let items = sort.sort_and_filter_at(0, items, 4, false);
    assert_eq!(items.len(), 4);
    assert!(format!("{sort:?}").contains("SimpleIpPortSort"));
}

#[test]
fn the_two_families_come_in_turn_and_the_defaults_hold() {
    let mut sort = a_sort();
    let items = vec![
        IpPortItem::new("1.2.3.4", 80),
        IpPortItem::new("1.2.3.4", 443),
        IpPortItem::new("2001:db8::1", 80),
        IpPortItem::new("2001:db8::2", 80),
    ];
    let items = sort.sort_and_filter_at(0, items, 4, true);
    let v6: Vec<bool> = items.iter().map(|item| !item.ip.contains('.')).collect();
    assert_eq!(v6, vec![true, false, true, false]);

    // ... and an item carries the defaults the C++ gives it
    let item = IpPortItem::new("1.2.3.4", 80);
    assert_eq!(
        item.source_type,
        mars_stn::simple_ipport_sort::IpSourceType::Null
    );
    assert!(item.host.is_empty());
    assert_eq!(
        item.transport_protocol,
        mars_stn::Task::TRANSPORT_PROTOCOL_TCP
    );
    assert_eq!(item.from_source, 0);
}

#[test]
fn a_server_ban_is_what_the_ban_query_answers_too() {
    let mut sort = a_sort();
    sort.add_server_ban_at(0, "1.2.3.4");

    // no failure history at all, and still out
    assert!(sort.is_banned_at(0, "1.2.3.4", 80));
    assert!(sort.is_server_banned_at(0, "1.2.3.4"));
    assert!(!sort.is_banned_at(0, "5.6.7.8", 80));

    // `kServerBanTime` later it is a candidate again
    assert!(!sort.is_banned_at(SERVER_BAN_TIME, "1.2.3.4", 80));
    assert!(!sort.is_server_banned_at(SERVER_BAN_TIME, "1.2.3.4"));
}

#[test]
fn a_pair_known_only_from_history_is_neither_banned_nor_held_back() {
    let mut sort = a_sort();
    sort.load_records(
        vec![Record {
            net_info: "wifi".to_string(),
            time: Some(1_700_000_000),
            items: vec![RecordItem {
                ip: "1.2.3.4".to_string(),
                port: 80,
                // three bytes that are not `0`: `kBanFailCount` failures
                history_result: 0x00_00_00_00_00_01_01_01,
            }],
        }],
        1_700_000_000,
    );
    sort.init_history_to_banned_list();
    assert_eq!(sort.ban_list()[0].records, 0b1110_0000);

    // it never failed in this process, so there is no reading to measure
    // `kBanTime` from and no interval to wait out
    assert_eq!(sort.ban_list()[0].last_fail_time, None);
    assert!(!sort.is_banned_at(BAN_TIME - 1, "1.2.3.4", 80));
    assert!(sort
        .ban_list()
        .iter()
        .all(|item| item.last_suc_time.is_none()));

    // ... and the first failure on it is taken down, which is what stamps the
    // time the next one is held back by
    fail(&mut sort, 1, "1.2.3.4", 80);
    assert_eq!(sort.ban_list()[0].last_fail_time, Some(1));
    assert_eq!(sort.ban_list()[0].records, 0b1100_0001);
    fail(&mut sort, 2, "1.2.3.4", 80);
    assert_eq!(
        sort.ban_list()[0].records,
        0b1100_0001,
        "inside `kFailUpdateInterval`"
    );
}
