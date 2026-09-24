//! `mars/sdt/netchecker_profile.h` — the results and the request.

use mars_sdt::constants::{NET_CHECK_BASIC, NET_CHECK_LONG, NET_CHECK_SHORT, UNUSE_TIMEOUT};
use mars_sdt::{CheckRequestProfile, CheckResultProfile, CheckStatus, NetCheckType};

#[test]
fn a_result_profile_starts_out_empty() {
    let profile = CheckResultProfile::new();
    // `Reset()` sets the type to -1, which is no `NetCheckType` at all
    assert_eq!(profile.netcheck_type, -1);
    assert_eq!(profile.kind(), None);
    assert_eq!(profile.error_code, 0);
    assert_eq!(profile.network_type, 0);
    assert!(profile.ip.is_empty());
    assert_eq!(profile.port, 0);
    assert_eq!(profile.conntime, 0);
    assert_eq!(profile.rtt, 0);
    assert!(profile.rtt_str.is_empty());
    assert!(profile.url.is_empty());
    assert_eq!(profile.status_code, 0);
    assert_eq!(profile.checkcount, 0);
    assert!(profile.loss_rate.is_empty());
    assert!(profile.domain_name.is_empty());
    assert!(profile.local_dns.is_empty());
    assert!(profile.ip1.is_empty());
    assert!(profile.ip2.is_empty());
}

#[test]
fn reset_puts_a_filled_in_profile_back_to_empty() {
    let mut profile = CheckResultProfile::of(NetCheckType::TcpCheck);
    profile.ip = "1.2.3.4".to_owned();
    profile.port = 80;
    profile.rtt = 23;
    profile.error_code = 1;

    profile.reset();
    assert_eq!(profile, CheckResultProfile::new());
}

#[test]
fn the_kind_is_what_the_type_says_it_is() {
    let profile = CheckResultProfile::of(NetCheckType::HttpCheck);
    assert_eq!(profile.netcheck_type, NetCheckType::HttpCheck.as_i32());
    assert_eq!(profile.kind(), Some(NetCheckType::HttpCheck));
}

#[test]
fn the_dump_lines_read_like_the_cpp() {
    // `SdtCore::__DumpCheckResult()` logs one line per result, with the fields
    // that kind of check fills in; the wording is the C++'s.
    let mut tcp = CheckResultProfile::of(NetCheckType::TcpCheck);
    tcp.ip = "1.2.3.4".to_owned();
    tcp.port = 80;
    tcp.network_type = 1;
    tcp.rtt = 23;
    assert_eq!(
        tcp.to_string(),
        "tcp check result, error_code:0, ip:1.2.3.4, port:80, network_type:1, rtt:23"
    );

    let mut http = CheckResultProfile::of(NetCheckType::HttpCheck);
    http.url = "http://www.qq.com/".to_owned();
    http.ip = "1.2.3.4".to_owned();
    http.port = 80;
    http.status_code = 200;
    http.network_type = 1;
    http.rtt = 120;
    assert_eq!(
        http.to_string(),
        "http check result, status_code:200, url:http://www.qq.com/, ip:1.2.3.4, port:80, network_type:1, rtt:120"
    );

    let mut ping = CheckResultProfile::of(NetCheckType::PingCheck);
    ping.ip = "1.2.3.4".to_owned();
    ping.network_type = 2;
    ping.checkcount = 2;
    ping.loss_rate = "0%".to_owned();
    ping.rtt_str = "12ms".to_owned();
    assert_eq!(
        ping.to_string(),
        "ping check result, error_code:0, ip:1.2.3.4, network_type:2, loss_rate:0%, rtt:12ms"
    );

    let mut dns = CheckResultProfile::of(NetCheckType::DnsCheck);
    dns.domain_name = "www.qq.com".to_owned();
    dns.network_type = 1;
    dns.ip1 = "1.2.3.4".to_owned();
    dns.rtt = 15;
    assert_eq!(
        dns.to_string(),
        "dns check result, error_code:0, domain_name:www.qq.com, network_type:1, ip1:1.2.3.4, rtt:15"
    );

    // the C++ switch has no case for the other kinds, so nothing is dumped
    let traceroute = CheckResultProfile::of(NetCheckType::TracerouteCheck);
    assert_eq!(traceroute.to_string(), "");
    let unset = CheckResultProfile::new();
    assert_eq!(unset.to_string(), "");
}

#[test]
fn a_fresh_request_already_asks_for_the_basic_check() {
    // `CheckRequestProfile()` is `Reset()`, and `Reset()` sets `NET_CHECK_BASIC`
    let request = CheckRequestProfile::new();
    assert_eq!(request.mode, NET_CHECK_BASIC);
    assert_eq!(request.check_status, CheckStatus::CheckContinue);
    assert_eq!(request.total_timeout, 0);
    assert!(request.longlink_items.is_empty());
    assert!(request.shortlink_items.is_empty());
    assert!(request.checkresult_profiles.is_empty());
    assert!(!request.shortlink_is_checked());
}

#[test]
fn a_request_profile_is_reset_the_way_the_cpp_resets_it() {
    let mut request = CheckRequestProfile::new();
    request.mode = NET_CHECK_LONG;
    request.total_timeout = 1000;
    request.check_status = CheckStatus::CheckFinish;

    request.reset();
    assert_eq!(request.mode, NET_CHECK_BASIC);
    assert_eq!(request.check_status, CheckStatus::CheckContinue);
    assert_eq!(request.total_timeout, 0);
    assert!(request.longlink_items.is_empty());
    assert!(request.shortlink_items.is_empty());
    assert!(request.checkresult_profiles.is_empty());
}

#[test]
fn only_a_short_mode_checks_the_short_link_hosts() {
    let mut request = CheckRequestProfile::new();
    request.mode = NET_CHECK_LONG | NET_CHECK_SHORT;
    assert!(request.shortlink_is_checked());

    request.mode = NET_CHECK_BASIC | NET_CHECK_LONG;
    assert!(!request.shortlink_is_checked());
}

#[test]
fn a_zero_timeout_means_no_timeout() {
    let mut request = CheckRequestProfile::new();
    assert_eq!(request.timeout(), UNUSE_TIMEOUT);

    request.total_timeout = 3000;
    assert_eq!(request.timeout(), 3000);
}
