//! `mars/sdt/sdt.h` — the vocabulary: hosts, kinds, error codes, the callback.

use std::collections::BTreeMap;

use mars_sdt::{
    Callback, CheckErrCode, CheckIPPort, CheckIPPorts, CheckResultProfile, CheckStatus,
    CollectingCallback, NetCheckStatus, NetCheckType, TcpErrCode,
};

#[test]
fn a_check_ip_port_defaults_to_empty_and_resets_to_empty() {
    let empty = CheckIPPort::default();
    assert!(empty.ip.is_empty());
    assert_eq!(empty.port, 0);

    let mut port = CheckIPPort::new("10.0.0.1", 8080);
    assert_eq!(port.ip, "10.0.0.1");
    assert_eq!(port.port, 8080);
    port.reset();
    assert_eq!(port, CheckIPPort::default());
}

#[test]
fn the_hosts_are_ordered_by_ip_and_then_by_port() {
    let mut ports = [
        CheckIPPort::new("10.0.0.2", 80),
        CheckIPPort::new("10.0.0.1", 8080),
        CheckIPPort::new("10.0.0.1", 80),
    ];
    ports.sort();
    let ordered: Vec<&str> = ports.iter().map(|p| p.ip.as_str()).collect();
    assert_eq!(ordered, vec!["10.0.0.1", "10.0.0.1", "10.0.0.2"]);
    assert_eq!(ports[0].port, 80);
    assert_eq!(ports[1].port, 8080);
}

#[test]
fn check_ip_ports_is_a_map_of_hosts_to_ports() {
    // the C++ is a `std::map`, so the keys come out sorted
    let mut items: CheckIPPorts = BTreeMap::new();
    items.insert(
        "short.weixin.qq.com".to_owned(),
        vec![CheckIPPort::new("1.2.3.4", 443)],
    );
    items.insert(
        "long.weixin.qq.com".to_owned(),
        vec![
            CheckIPPort::new("5.6.7.8", 80),
            CheckIPPort::new("5.6.7.8", 8080),
        ],
    );
    let hosts: Vec<&String> = items.keys().collect();
    assert_eq!(hosts, vec!["long.weixin.qq.com", "short.weixin.qq.com"]);
    assert_eq!(items["long.weixin.qq.com"].len(), 2);
}

#[test]
fn the_enums_carry_the_discriminants_of_the_header() {
    assert_eq!(NetCheckStatus::None as i32, 0);
    assert_eq!(NetCheckStatus::Checking as i32, 1);
    assert_eq!(NetCheckStatus::CheckEnd as i32, 2);
    assert_eq!(NetCheckStatus::default(), NetCheckStatus::None);

    assert_eq!(NetCheckType::PingCheck as i32, 0);
    assert_eq!(NetCheckType::DnsCheck as i32, 1);
    assert_eq!(NetCheckType::NewDnsCheck as i32, 2);
    assert_eq!(NetCheckType::TcpCheck as i32, 3);
    assert_eq!(NetCheckType::HttpCheck as i32, 4);
    assert_eq!(NetCheckType::TracerouteCheck as i32, 5);
    assert_eq!(NetCheckType::ReqBufCheck as i32, 6);

    assert_eq!(CheckErrCode::CheckOK as i32, 0);
    assert_eq!(CheckErrCode::Dns as i32, 1);
    assert_eq!(CheckErrCode::Socket as i32, 2);
    assert_eq!(CheckErrCode::IsRunning as i32, 3);
    assert_eq!(CheckErrCode::default(), CheckErrCode::CheckOK);

    assert_eq!(TcpErrCode::TcpSucc as i32, 0);
    assert_eq!(TcpErrCode::TcpNonErr as i32, 1);
    assert_eq!(TcpErrCode::SelectErr as i32, -1);
    assert_eq!(TcpErrCode::PipeIntr as i32, -2);
    assert_eq!(TcpErrCode::SndRcvErr as i32, -3);
    assert_eq!(TcpErrCode::AssertErr as i32, -4);
    assert_eq!(TcpErrCode::TimeoutErr as i32, -5);
    assert_eq!(TcpErrCode::SelectExpErr as i32, -6);
    assert_eq!(TcpErrCode::PipeExp as i32, -7);
    assert_eq!(TcpErrCode::ConnectErr as i32, -8);
    assert_eq!(TcpErrCode::TcpRespErr as i32, -9);
    assert_eq!(TcpErrCode::default(), TcpErrCode::TcpSucc);

    assert_eq!(CheckStatus::CheckContinue as i32, 0);
    assert_eq!(CheckStatus::CheckFinish as i32, 1);
    assert_eq!(CheckStatus::default(), CheckStatus::CheckContinue);
}

#[test]
fn a_netcheck_type_round_trips_through_its_integer() {
    for kind in [
        NetCheckType::PingCheck,
        NetCheckType::DnsCheck,
        NetCheckType::NewDnsCheck,
        NetCheckType::TcpCheck,
        NetCheckType::HttpCheck,
        NetCheckType::TracerouteCheck,
        NetCheckType::ReqBufCheck,
    ] {
        assert_eq!(NetCheckType::of(kind.as_i32()), Some(kind));
    }
    // -1 is what `CheckResultProfile::Reset()` leaves behind
    assert_eq!(NetCheckType::of(-1), None);
    assert_eq!(NetCheckType::of(7), None);
}

#[test]
fn the_callback_defaults_to_doing_nothing() {
    struct Silent;
    impl Callback for Silent {}

    // the C++ class has an empty body, so this has to compile and do nothing
    Silent.report_net_check_result(&[CheckResultProfile::new()]);
}

#[test]
fn a_collecting_callback_keeps_what_it_was_told() {
    let callback = CollectingCallback::new();
    assert!(callback.is_empty());

    let results = vec![
        CheckResultProfile::of(NetCheckType::PingCheck),
        CheckResultProfile::of(NetCheckType::DnsCheck),
    ];
    callback.report_net_check_result(&results);
    callback.report_net_check_result(&results[1..]);

    assert_eq!(callback.len(), 3);
    assert!(!callback.is_empty());
    assert_eq!(callback.results()[0].kind(), Some(NetCheckType::PingCheck));
    assert_eq!(callback.results()[2].kind(), Some(NetCheckType::DnsCheck));
}
