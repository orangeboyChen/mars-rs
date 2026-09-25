//! `mars/stn/src/net_source.cc`, through the public api.
//!
//! The samples are what the C++ answers for the same calls: a long link made of
//! the hosts and ports the app set, resolved by the dns the app answers and cut
//! down to five pairs; a host with a debug ip that never reaches dns at all; a
//! short link whose cgi has a debug pair of its own; and a host list that is
//! shared out over its hosts while the app is in the background.

use mars_stn::{
    IpPortItem, IpSourceType, LonglinkConfig, NetSource, Task, DEFAULT_QUIC_RW_TIMEOUT_MS,
    DISABLE_QUIC_SECONDS, ITEM_DELIMITER, NUM_MAKE_COUNT,
};

/// A `NetSource` with two long-link hosts on two ports, a short link on `8080`,
/// a new dns that answers one ip per host, and an app in the foreground with a
/// network.
fn a_source() -> NetSource {
    let mut source = NetSource::new_at(0);
    source.set_longlink(
        vec!["long.example".to_string(), "long2.example".to_string()],
        vec![80, 443],
        "",
    );
    source.set_shortlink(8080, "");
    source.set_is_active(|| true);
    source.set_net_info(|| 1);
    source.set_net_label(|| Some("wifi-home".to_string()));
    source.set_new_dns(|host, _is_longlink, _extra| match host {
        "long.example" => vec!["1.1.1.1".to_string()],
        "long2.example" => vec!["2.2.2.2".to_string()],
        "short.example" => vec!["3.3.3.3".to_string()],
        _ => Vec::new(),
    });
    source.set_random(|_bound| 0);
    source
}

fn ips(items: &[IpPortItem]) -> Vec<String> {
    items.iter().map(|item| item.ip.clone()).collect()
}

#[test]
fn the_numbers_are_the_ones_the_c_plus_plus_writes_down() {
    assert_eq!(NUM_MAKE_COUNT, 5);
    assert_eq!(ITEM_DELIMITER, ":");
    assert_eq!(DISABLE_QUIC_SECONDS, 20 * 60);
    assert_eq!(DEFAULT_QUIC_RW_TIMEOUT_MS, 5_000);
}

#[test]
fn a_long_link_is_made_of_the_hosts_and_ports_the_app_set() {
    let mut source = a_source();
    let items = source.get_longlink_items(&LonglinkConfig::new("main"));
    assert_eq!(
        ips(&items),
        vec![
            "1.1.1.1".to_string(),
            "1.1.1.1".to_string(),
            "2.2.2.2".to_string(),
            "2.2.2.2".to_string()
        ],
        "one host after another, and every port of every ip"
    );
    // one host after another; inside one host the sort has the last word, which
    // is why what is pinned down here is which pairs there are, not their order
    let mut ports: Vec<u16> = items.iter().map(|item| item.port).collect();
    ports.sort_unstable();
    assert_eq!(ports, vec![80, 80, 443, 443]);
    assert!(items
        .iter()
        .all(|item| item.source_type == IpSourceType::NewDns));
    assert!(items.iter().all(|item| item.transport_protocol == 1));

    // and no host at all is nothing at all
    let mut source = NetSource::new_at(0);
    source.set_new_dns(|_, _, _| vec!["1.1.1.1".to_string()]);
    assert!(source
        .get_longlink_items(&LonglinkConfig::new("main"))
        .is_empty());
}

#[test]
fn a_host_with_a_debug_ip_never_reaches_dns() {
    let mut source = a_source();
    source.set_debug_ip("long.example", "9.9.9.9");
    let items = source.get_longlink_items(&LonglinkConfig::new("main"));
    assert_eq!(
        ips(&items),
        vec!["9.9.9.9".to_string(), "9.9.9.9".to_string()],
        "one item per long-link port"
    );
    assert!(items
        .iter()
        .all(|item| item.source_type == IpSourceType::Debug));
    assert_eq!(items[0].host, "long.example");

    // taking it away is what lets the dns answer again
    source.set_debug_ip("long.example", "");
    assert!(source
        .get_longlink_items(&LonglinkConfig::new("main"))
        .iter()
        .all(|item| item.ip != "9.9.9.9"));
}

#[test]
fn a_short_link_gets_the_port_the_app_set_and_one_pair_per_ip() {
    let mut source = a_source();
    let items = source.get_shortlink_items(&["short.example".to_string()], "");
    assert_eq!(ips(&items), vec!["3.3.3.3".to_string()]);
    assert_eq!(items[0].port, 8080);
    assert_eq!(items[0].host, "short.example");
    assert_eq!(items[0].source_type, IpSourceType::NewDns);

    // and a cgi with a debug pair of its own beats the dns and the host
    source.set_debug_ip("short.example", "6.6.6.6");
    source.set_cgi_debug_ip("/cgi-bin/mm", "4.4.4.4", 0);
    let items = source.get_shortlink_items(&["short.example".to_string()], "/cgi-bin/mm");
    assert_eq!(ips(&items), vec!["4.4.4.4".to_string()]);
    assert_eq!(items[0].port, 80, "a port of zero is written down as 80");

    // ... and an empty ip takes it away again
    source.set_cgi_debug_ip("/cgi-bin/mm", "", 0);
    let items = source.get_shortlink_items(&["short.example".to_string()], "/cgi-bin/mm");
    assert_eq!(ips(&items), vec!["6.6.6.6".to_string()]);
    assert_eq!(items[0].port, 8080);
}

#[test]
fn what_the_fallback_answered_is_kept_as_the_backup_ips_of_the_host() {
    let mut source = a_source();
    source.set_new_dns(|_, _, _| Vec::new());
    source.set_dns(|host| match host {
        "short.example" => vec!["5.5.5.5".to_string()],
        _ => Vec::new(),
    });
    assert!(source.backup_ips("short.example").is_empty());

    let items = source.get_shortlink_items(&["short.example".to_string()], "");
    assert!(!items.is_empty());
    assert_eq!(
        source.backup_ips("short.example"),
        vec!["5.5.5.5".to_string()],
        "the C++ writes what the fallback answered into the mapping"
    );
    assert!(items
        .iter()
        .any(|item| item.source_type == IpSourceType::Dns));
    assert!(items
        .iter()
        .any(|item| item.source_type == IpSourceType::Backup));
}

#[test]
fn in_the_background_the_pairs_are_shared_out_over_the_hosts() {
    let mut source = a_source();
    source.set_is_active(|| false);
    // two hosts: `4 / 2` for each of them, and the fifth is left for the
    // backup pass, which a dns that knows no backup ips cannot fill
    let items = source.get_longlink_items(&LonglinkConfig::new("main"));
    assert_eq!(
        ips(&items),
        vec![
            "1.1.1.1".to_string(),
            "1.1.1.1".to_string(),
            "2.2.2.2".to_string(),
            "2.2.2.2".to_string()
        ]
    );
}

#[test]
fn a_report_is_written_down_and_a_ban_takes_the_pair_out() {
    let mut source = a_source();

    // three failures on one pair is what bans it
    for index in 0..3 {
        source.report_long_ip_at(index * 20_000, 1_000 + index, false, "1.1.1.1", 80);
    }
    assert_eq!(source.ipport_strategy().records().len(), 1);
    assert_eq!(source.ipport_strategy().ban_list().len(), 1);

    // ... and a banned pair is not in the list the next time
    let items =
        source.get_longlink_items_at(60_000, &LonglinkConfig::new("main"), &Default::default());
    assert!(
        !items
            .iter()
            .any(|item| item.ip == "1.1.1.1" && item.port == 80),
        "{:?}",
        items
    );

    // a speed test that lifted the ban is what puts it back
    source.remove_long_ban_ip("1.1.1.1");
    assert!(source.ipport_strategy().ban_list().is_empty());
    let items =
        source.get_longlink_items_at(60_000, &LonglinkConfig::new("main"), &Default::default());
    assert!(items
        .iter()
        .any(|item| item.ip == "1.1.1.1" && item.port == 80));
}

#[test]
fn a_report_with_no_network_is_dropped() {
    let mut source = a_source();
    source.set_net_info(|| -1);
    source.report_long_ip_at(0, 1_000, false, "1.1.1.1", 80);
    source.report_short_ip_at(0, 1_000, false, "3.3.3.3", "short.example", 8080);
    assert!(source.ipport_strategy().records().is_empty());
}

#[test]
fn quic_comes_back_on_its_own_and_the_timeouts_say_where_they_came_from() {
    let mut source = a_source();
    // `quic_forbidden_` is what the C++ starts with
    assert!(!source.can_use_quic_at(0));
    source.forbid_quic(false);
    assert!(source.can_use_quic_at(0));

    source.disable_quic_at(1_000, DISABLE_QUIC_SECONDS);
    // twenty minutes, in milliseconds: `sg_quic_reopen_tick += seconds * 1000`
    assert!(!source.can_use_quic_at(1_000 + 1_000));
    assert!(source.can_use_quic_at(1_000 + DISABLE_QUIC_SECONDS as u64 * 1_000));

    assert_eq!(source.quic_rw_timeout_ms("/cgi-bin/mm").0, 5_000);
    source.set_default_quic_rw_timeout_ms(9_000);
    assert_eq!(
        source.quic_rw_timeout_ms("/cgi-bin/mm"),
        (9_000, mars_stn::TimeoutSource::ServerDefault)
    );
    source.set_quic_rw_timeout_ms("/cgi-bin/mm", 1_000);
    assert_eq!(
        source.quic_rw_timeout_ms("/cgi-bin/mm"),
        (1_000, mars_stn::TimeoutSource::CgiSpecial)
    );
}

#[test]
fn the_table_is_dumped_one_item_after_another() {
    let mut source = a_source();
    let items = source.get_longlink_items(&LonglinkConfig::new("main"));
    let table = NetSource::dump_table(&items);
    // four pairs, one line each, in whatever order the sort put them in
    let mut lines: Vec<&str> = table.split('|').collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        vec![
            "1.1.1.1:443:long.example:NewDNSIP",
            "1.1.1.1:80:long.example:NewDNSIP",
            "2.2.2.2:443:long2.example:NewDNSIP",
            "2.2.2.2:80:long2.example:NewDNSIP"
        ]
    );
    assert_eq!(table.split('|').count(), items.len());
    assert_eq!(NetSource::dump_table(&[]), "");
}

#[test]
fn without_anything_set_nothing_is_answered() {
    let mut source = NetSource::default();
    assert!(source.longlink_hosts().is_empty());
    assert!(source.longlink_ports().is_empty());
    assert_eq!(source.shortlink_port(), 0);
    assert!(source
        .get_longlink_items_at(0, &LonglinkConfig::new("main"), &Default::default())
        .is_empty());
    assert!(source
        .get_shortlink_items_at(0, &["short.example".to_string()], "", &Default::default())
        .is_empty());
    assert!(source.can_use_ipv6());
    assert_eq!(source.ip_connect_timeout(), (0, 0));
    source.set_ip_connect_timeout(3_000, 5_000);
    assert_eq!(source.ip_connect_timeout(), (3_000, 5_000));

    // a long link whose channel is the minor one, with nothing set: no debug ip,
    // so nothing
    let mut config = LonglinkConfig::new("minor");
    config.link_type = Task::CHANNEL_MINOR_LONG;
    config.host_list = vec!["minor.example".to_string()];
    assert!(source.get_longlink_items(&config).is_empty());
    assert!(format!("{source:?}").contains("NetSource"));
}
