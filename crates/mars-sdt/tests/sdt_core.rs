//! `mars/sdt/src/sdt_core.cc` — what the mode turns into, and the run loop.

use std::collections::BTreeMap;

use mars_sdt::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use mars_sdt::sdt_core::SdtCore;
use mars_sdt::{
    CheckIPPort, CheckIPPorts, CheckStatus, NetCheckType, NET_CHECK_BASIC, NET_CHECK_LONG,
    NET_CHECK_SHORT, UNUSE_TIMEOUT,
};

fn hosts(names: &[&str]) -> CheckIPPorts {
    names
        .iter()
        .map(|name| (name.to_string(), vec![CheckIPPort::new("1.2.3.4", 80)]))
        .collect::<BTreeMap<_, _>>()
}

/// A checker that records the kind it was asked to run, like the C++ checkers
/// push into `check_request_.checkresult_profiles`.
fn record(kind: NetCheckType, request: &mut CheckRequestProfile) {
    request
        .checkresult_profiles
        .push(CheckResultProfile::of(kind));
}

#[test]
fn the_mode_decides_which_checks_run() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut basic = SdtCore::new();
    basic.start_check(&longlink, &shortlink, NET_CHECK_BASIC, UNUSE_TIMEOUT);
    assert_eq!(
        basic.plan(),
        &[NetCheckType::PingCheck, NetCheckType::DnsCheck]
    );
    // a basic check does not look at the short-link hosts
    assert!(basic.request().shortlink_items.is_empty());

    let mut long = SdtCore::new();
    long.start_check(&longlink, &shortlink, NET_CHECK_LONG, 0);
    assert_eq!(long.plan(), &[NetCheckType::TcpCheck]);

    let mut short = SdtCore::new();
    short.start_check(&longlink, &shortlink, NET_CHECK_SHORT, 1000);
    assert_eq!(short.plan(), &[NetCheckType::HttpCheck]);
    assert_eq!(short.request().shortlink_items, shortlink);

    let mut all = SdtCore::new();
    all.start_check(
        &longlink,
        &shortlink,
        NET_CHECK_BASIC | NET_CHECK_SHORT | NET_CHECK_LONG,
        1000,
    );
    assert_eq!(
        all.plan(),
        &[
            NetCheckType::PingCheck,
            NetCheckType::DnsCheck,
            NetCheckType::HttpCheck,
            NetCheckType::TcpCheck
        ]
    );

    // a mode with no bits at all plans nothing
    let mut none = SdtCore::new();
    none.start_check(&longlink, &shortlink, 0, 1000);
    assert!(none.plan().is_empty());
}

#[test]
fn the_request_carries_the_hosts_the_mode_and_the_timeout() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut core = SdtCore::new();
    assert!(core.start_check(&longlink, &shortlink, NET_CHECK_SHORT, 2500));
    assert!(core.is_checking());

    let request = core.request();
    assert_eq!(request.longlink_items, longlink);
    assert_eq!(request.shortlink_items, shortlink);
    assert_eq!(request.mode, NET_CHECK_SHORT);
    assert_eq!(request.total_timeout, 2500);
    assert_eq!(request.timeout(), 2500);
}

#[test]
fn a_second_check_while_one_is_in_flight_is_ignored() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut core = SdtCore::new();
    assert!(core.start_check(&longlink, &shortlink, NET_CHECK_BASIC, 1000));
    assert!(!core.start_check(&longlink, &shortlink, NET_CHECK_LONG, 2000));

    // the request of the first check is untouched
    assert_eq!(core.request().mode, NET_CHECK_BASIC);
    assert_eq!(core.request().total_timeout, 1000);
    assert_eq!(core.plan().len(), 2);
}

#[test]
fn the_run_walks_the_plan_in_order_and_hands_back_the_results() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut core = SdtCore::new();
    core.start_check(
        &longlink,
        &shortlink,
        NET_CHECK_BASIC | NET_CHECK_LONG,
        UNUSE_TIMEOUT,
    );

    let results = core.run_on(record);
    let kinds: Vec<NetCheckType> = results
        .iter()
        .filter_map(CheckResultProfile::kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            NetCheckType::PingCheck,
            NetCheckType::DnsCheck,
            NetCheckType::TcpCheck
        ]
    );

    // `__Reset()` runs at the end of `__RunOn`
    assert!(core.plan().is_empty());
    assert!(!core.is_checking());
}

#[test]
fn a_check_that_finishes_the_request_stops_the_run() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut core = SdtCore::new();
    core.start_check(
        &longlink,
        &shortlink,
        NET_CHECK_BASIC | NET_CHECK_LONG,
        UNUSE_TIMEOUT,
    );

    let results = core.run_on(|kind, request| {
        record(kind, request);
        // the ping answered, so there is nothing left to ask
        if kind == NetCheckType::PingCheck {
            request.check_status = CheckStatus::CheckFinish;
        }
    });
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].kind(), Some(NetCheckType::PingCheck));
}

#[test]
fn a_cancelled_check_runs_nothing() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut core = SdtCore::new();
    core.start_check(&longlink, &shortlink, NET_CHECK_BASIC, UNUSE_TIMEOUT);
    assert!(!core.is_cancelled());

    core.cancel_check();
    assert!(core.is_cancelled());
    assert!(core.run_on(record).is_empty());

    // the C++ never clears `cancel_`, so the core stays cancelled
    core.start_check(&longlink, &shortlink, NET_CHECK_BASIC, UNUSE_TIMEOUT);
    assert!(core.is_cancelled());
    assert!(core.run_on(record).is_empty());
}

#[test]
fn a_default_core_is_a_fresh_one() {
    let core = SdtCore::default();
    assert!(!core.is_checking());
    assert!(!core.is_cancelled());
    assert!(core.plan().is_empty());
    assert!(core.http_netcheck_cgi().is_empty());
    assert_eq!(core.request().mode, NET_CHECK_BASIC);
}

#[test]
fn the_http_check_cgi_is_kept_for_the_checker() {
    let mut core = SdtCore::new();
    assert!(core.http_netcheck_cgi().is_empty());
    core.set_http_netcheck_cgi("http://www.qq.com/netcheck");
    assert_eq!(core.http_netcheck_cgi(), "http://www.qq.com/netcheck");
}
