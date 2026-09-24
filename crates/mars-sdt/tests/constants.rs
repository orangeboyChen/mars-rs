//! `mars/sdt/constants.h` — the literals, checked against the header.

use mars_sdt::constants::*;

#[test]
fn the_check_modes_are_bits() {
    assert_eq!(NET_CHECK_BASIC, 1);
    assert_eq!(NET_CHECK_LONG, 2);
    assert_eq!(NET_CHECK_SHORT, 4);
    assert_eq!(ERR_SEQ, -1);
}

#[test]
fn the_mode_macros_test_the_bits() {
    let all = NET_CHECK_BASIC | NET_CHECK_LONG | NET_CHECK_SHORT;
    assert!(mode_basic(all));
    assert!(mode_long(all));
    assert!(mode_short(all));

    assert!(mode_basic(NET_CHECK_BASIC));
    assert!(!mode_long(NET_CHECK_BASIC));
    assert!(!mode_short(NET_CHECK_BASIC));

    // a mode that is none of the three
    assert!(!mode_basic(0));
    assert!(!mode_long(0));
    assert!(!mode_short(0));
}

#[test]
fn the_default_hosts_are_the_qq_ones() {
    assert_eq!(DEFAULT_HTTP_HOST, "www.qq.com");
    assert_eq!(DEFAULT_PING_HOST, "www.qq.com");
    assert_eq!(DUMMY_HOST, "DUMMY HOST");
}

#[test]
fn the_report_literals_match_the_header() {
    assert_eq!(NET_CHECK_TAG, "NET_CHECK");
    assert_eq!(CHECK_SUC, "check success");
    assert_eq!(CHECK_FAIL, "check failed");

    assert_eq!(NO_NET_TYPE, "NoNet");
    assert_eq!(WIFI_TYPE, "Wifi Net");
    assert_eq!(MOBILE_TYPE, "Mobile Net");
    assert_eq!(OTHER_NET_TYPE, "Other Net");
}

#[test]
fn the_timeouts_match_the_header() {
    assert_eq!(HTTP_DEFAULT_TIMEOUT, 5000);
    assert_eq!(HTTP_DUMMY_RECV_DATA_SIZE, 1);

    assert_eq!(DEFAULT_PING_TIMEOUT, 4);
    assert_eq!(DEFAULT_PING_COUNT, 2);
    assert_eq!(DEFAULT_PING_INTERVAL, 1);

    assert_eq!(DEFAULT_TCP_CONN_TIMEOUT, 5000);
    assert_eq!(DEFAULT_TCP_RECV_TIMEOUT, 5000);

    assert_eq!(DEFAULT_DNS_TIMEOUT, 3000);
    // `INT_MAX`, i.e. "no timeout"
    assert_eq!(UNUSE_TIMEOUT, i32::MAX as u32);
}

#[test]
fn the_user_agent_is_picked_for_this_platform() {
    // `constants.h` picks one literal per platform; which one is a property of
    // the build, so all this can assert is that it is one of the three.
    let known = [
        "Mozilla/5.0  (Linux; Android 4.1.1; Nexus 7 Build/JRO03S) AppleWebKit/535.19 (KHTML,  like Gecko) Chrome/18.0.1025.166 Safari/535.19",
        "Mozilla/5.0  (iPhone; CPU iPhone OS 6_0 like Mac OS X) AppleWebKit/536.26 (KHTML,  like Gecko) Version/6.0 Mobile/10A403 Safari/8536.25",
        "Mozilla/5.0 (compatible; MSIE9.0 Windows NT 6.1; WOW64; Trident/5.0)",
    ];
    assert!(known.contains(&USER_AGENT));
}
