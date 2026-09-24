//! `mars/stn/src/longlink_metadata.cc`, through the public api.
//!
//! The samples are what the C++ answers for the same calls: a monitor the app
//! wired asks the link it shares for a run, a network change drops it before a
//! new one is asked for, a check that found a pair that answers takes a link on
//! a backup pair down and leaves one that is where dns put it alone, and the
//! keeper's buffers go out on the same link.
//!
//! The app's own side of the wiring — the active logic, the net source, which
//! pairs there are — is what a `NetCore` would hand the three helpers, and the
//! tests put it on them the way a host does.

use mars_stn::{
    ChannelFactory, DisconnectInternalCode, IpPortItem, IpSourceType, LongLink, LongLinkMetaData,
    LongLinkStatus, LonglinkConfig, Task, DEFAULT_PERIOD, NET_TYPE_WIFI,
};

/// The two readings the tests use: the link asked for its ips at `50_000`, and
/// the monitor is asked at `60_000` — ten seconds later, which is more than the
/// ten-second interval a foregrounded app waits.
const DNS_AT: u64 = 50 * 1000;
const NOW: u64 = 60 * 1000;

fn config() -> LonglinkConfig {
    LonglinkConfig::new("long.weixin.qq.com")
}

/// `NetCore::__CreateLongLink` — the link is the factory's, which is mars's
/// until the app replaces it.
fn a_metadata() -> LongLinkMetaData {
    let config = config();
    let mut factory = ChannelFactory::new();
    LongLinkMetaData::new(config.clone(), factory.create_longlink(&config))
}

/// What `NetCore` wires on the monitor that is not the link's: an active app
/// with an account, in the foreground on wi-fi, and no jitter.
fn wire_the_app(meta: &mut LongLinkMetaData) {
    let monitor = meta.monitor();
    monitor.set_is_active(|| true);
    monitor.set_is_foreground(|| true);
    monitor.set_last_foreground_change_time(|| 0);
    monitor.set_net_info(|| NET_TYPE_WIFI);
    monitor.set_has_account(|| true);
    monitor.set_random(|_bound| 0);
}

/// `NetSource::GetLongLinkItems` — one pair, and where it came from. A connect
/// with no socket operator fails, but the pair it was given is the one the
/// profile names.
fn put_the_link_on(meta: &LongLinkMetaData, ip: &str, source: IpSourceType) {
    let ip = ip.to_string();
    let mut link = meta.channel().lock().unwrap();
    link.set_longlink_items(move |_config| {
        vec![IpPortItem {
            ip: ip.clone(),
            port: 80,
            source_type: source,
            host: "long.weixin.qq.com".to_string(),
            transport_protocol: Task::TRANSPORT_PROTOCOL_TCP,
            from_source: 0,
        }]
    });
    let _ = link.connect_at(DNS_AT);
    assert_eq!(link.profile().ip_type, source, "the link is on the pair");
}

/// What the net source says there is to try, and whether one of them answers.
fn wire_the_net_source(meta: &mut LongLinkMetaData) {
    meta.checker().set_dns(|_host| vec!["2.2.2.2".to_string()]);
    meta.checker().set_longlink_ports(|| vec![80]);
    meta.checker().set_speed_test(|_ip, _port| true);
}

#[test]
fn the_link_the_metadata_keeps_is_the_one_the_factory_made() {
    let mut factory = ChannelFactory::new();
    factory.set_create_longlink(|config| {
        let mut config = config.clone();
        config.name = "the app's own".to_string();
        LongLink::new(config)
    });

    let config = config();
    let meta = LongLinkMetaData::new(config.clone(), factory.create_longlink(&config));
    assert_eq!(
        meta.channel().lock().unwrap().config().name,
        "the app's own",
        "not the link mars would have made"
    );
    // ... and the config it was made for is the one the metadata answers with
    assert_eq!(meta.config().name, "long.weixin.qq.com");
    assert!(!meta.is_connected());
    assert!(format!("{meta:?}").contains("long.weixin.qq.com"));
}

#[test]
fn the_monitor_the_app_wired_asks_the_link_it_shares_for_a_run() {
    let mut meta = a_metadata();
    wire_the_app(&mut meta);
    assert!(
        !meta.channel().lock().unwrap().is_running(),
        "no run was started yet"
    );

    // ten seconds is longer than the interval, so the link is asked for now
    assert!(!meta.monitor().make_sure_connected_at(NOW));
    assert!(
        meta.channel().lock().unwrap().is_running(),
        "the link the monitor shares was told to make itself connected"
    );

    // what the host says when the connect came back
    meta.channel()
        .lock()
        .unwrap()
        .set_status(LongLinkStatus::Connected);
    assert!(meta.is_connected());
    assert!(meta.monitor().make_sure_connected_at(NOW));
}

#[test]
fn a_network_change_drops_the_link_before_a_new_one_is_asked_for() {
    let mut meta = a_metadata();
    wire_the_app(&mut meta);
    assert!(!meta.monitor().make_sure_connected_at(NOW));

    assert!(
        meta.monitor().network_change_at(NOW),
        "a network change is a connect that is due at once"
    );
    assert_eq!(
        meta.channel().lock().unwrap().disconnect_code(),
        DisconnectInternalCode::NetworkChange,
        "the monitor dropped the link it shares first"
    );
}

#[test]
fn a_check_that_found_a_pair_takes_a_link_on_a_backup_one_down() {
    let mut meta = a_metadata();
    wire_the_app(&mut meta);
    assert!(!meta.monitor().make_sure_connected_at(NOW));
    put_the_link_on(&meta, "1.1.1.1", IpSourceType::Backup);
    wire_the_net_source(&mut meta);

    meta.checker().start_check_at(0);
    assert_eq!(meta.checker().check_at(NOW), Some(true));
    assert_eq!(
        meta.channel().lock().unwrap().disconnect_code(),
        DisconnectInternalCode::TimeCheckSucc,
        "the link was taken down for the pair the check found"
    );
}

#[test]
fn a_check_leaves_a_link_that_is_where_dns_put_it_alone() {
    let mut meta = a_metadata();
    wire_the_app(&mut meta);
    assert!(!meta.monitor().make_sure_connected_at(NOW));
    put_the_link_on(&meta, "1.1.1.1", IpSourceType::Dns);
    wire_the_net_source(&mut meta);

    meta.checker().start_check_at(0);
    assert_eq!(
        meta.checker().check_at(NOW),
        None,
        "a check of a link that is not on a backup pair makes no test at all"
    );
    assert_eq!(
        meta.channel().lock().unwrap().disconnect_code(),
        DisconnectInternalCode::None
    );
}

#[test]
fn the_keeper_holds_a_mapping_up_on_the_link_it_shares() {
    let mut meta = a_metadata();
    meta.channel()
        .lock()
        .unwrap()
        .set_status(LongLinkStatus::Connected);

    meta.keeper().keep_at(1_000);
    assert_eq!(meta.keeper().sent(), 1, "the first touch sends at once");
    assert!(meta.channel().lock().unwrap().has_data_to_send());

    // what the host's run does with it: the whole buffer goes out
    let len = meta.channel().lock().unwrap().queued()[0].buffer.len();
    meta.channel().lock().unwrap().wrote(len);
    assert!(!meta.channel().lock().unwrap().has_data_to_send());

    // ... and the next one is posted a period after the data that came back
    meta.keeper().on_network_data_changed_at(1_500);
    assert_eq!(meta.keeper().due_time(), Some(1_500 + DEFAULT_PERIOD));
    meta.keeper().on_timeout();
    assert_eq!(meta.keeper().sent(), 2);
    assert!(meta.channel().lock().unwrap().has_data_to_send());
}

#[test]
fn the_status_the_monitor_keeps_is_its_own_and_not_the_link_s() {
    let mut meta = a_metadata();
    wire_the_app(&mut meta);

    // what the C++ tells the monitor when the link's status changed: it writes
    // its own `status_`, and that is not the status of the link it shares
    meta.monitor()
        .on_longlink_status_changed_at(NOW, LongLinkStatus::Connected);
    assert_eq!(meta.monitor().status(), LongLinkStatus::Connected);

    // a link nothing has connected is idle, which is not up
    assert_eq!(
        meta.channel().lock().unwrap().connect_status(),
        LongLinkStatus::ConnectIdle
    );
    assert!(!meta.is_connected());
}
