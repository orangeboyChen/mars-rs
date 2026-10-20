//! `mars/sdt/constants.h` — the literals, checked against the header.

use marsrs_sdt::constants::*;

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

/// `constants.h` picks one literal per platform, so what the port has to get
/// right is that the one it built is the one that suits the platform it built
/// for: an Android build's user agent is an Android one, and so on.
#[test]
fn the_user_agent_is_picked_for_this_platform() {
    assert!(
        USER_AGENT.starts_with("Mozilla/5.0"),
        "a user agent of this shape: {USER_AGENT:?}"
    );
    let platform = if cfg!(target_os = "android") {
        "Android"
    } else if cfg!(target_vendor = "apple") {
        "iPhone"
    } else {
        "Windows NT"
    };
    assert!(
        USER_AGENT.contains(platform),
        "{platform} is the platform this was built for: {USER_AGENT:?}"
    );
    assert!(!USER_AGENT.is_empty());
}
