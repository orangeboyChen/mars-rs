//! `mars/sdt/src/sdt_core.cc` — what the mode turns into, and the run loop.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use marsrs_sdt::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use marsrs_sdt::sdt_core::{CancelHandle, SdtCore};
use marsrs_sdt::{
    CheckIPPort, CheckIPPorts, CheckStatus, NetCheckStatus, NetCheckType, NET_CHECK_BASIC,
    NET_CHECK_LONG, NET_CHECK_SHORT, UNUSE_TIMEOUT,
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
    // the cancellation outlives the run, so the caller can still see it
    assert!(core.is_cancelled());

    // and the next request is a new one: the core must not stay cancelled
    // forever, or one `CancelCheck()` would retire it for good
    core.start_check(&longlink, &shortlink, NET_CHECK_BASIC, UNUSE_TIMEOUT);
    assert!(!core.is_cancelled());
    assert_eq!(core.run_on(record).len(), 2);
}

#[test]
fn a_running_check_can_be_cancelled_from_the_outside() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut core = SdtCore::new();
    core.start_check(&longlink, &shortlink, NET_CHECK_BASIC, UNUSE_TIMEOUT);

    // `run_on` borrows the core for as long as the checks take, so the only
    // way to cancel one that is already running is a flag taken beforehand
    // and passed to whoever is blocked — here the checker, after its first
    // (pretend) socket read.
    let cancel = core.cancel_handle();
    let cancel_in_check = cancel.clone();
    let ran = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&ran);
    let results = core.run_on(move |kind, request| {
        sink.lock().unwrap().push(kind);
        cancel_in_check.cancel();
        record(kind, request);
    });

    assert!(core.is_cancelled());
    assert_eq!(*ran.lock().unwrap(), vec![NetCheckType::PingCheck]);
    assert_eq!(results.len(), 1, "the checks after the cancel must not run");

    // the handle and the core see the same flag, from either side
    assert!(cancel.is_cancelled());
    let cancel = core.cancel_handle();
    cancel.cancel();
    assert!(core.is_cancelled());
}

#[test]
fn a_probe_that_panics_does_not_leave_the_core_checking() {
    let longlink = hosts(&["long.weixin.qq.com"]);

    let mut core = SdtCore::new();
    core.start_check(
        &longlink,
        &CheckIPPorts::new(),
        NET_CHECK_BASIC,
        UNUSE_TIMEOUT,
    );

    // The panic is the host's — it came out of the probe the host handed in —
    // and `__RunOn` does not catch it, so there is nothing to assert about the
    // run but that the core was given back on the way out.
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        core.run_on(|_, _| panic!("a probe that fell over"));
    }));
    assert!(panicked.is_err(), "the panic is not swallowed");

    // `checking_` is cleared by the guard and not by the end of `__RunOn`,
    // which a panic unwinds past: a core that kept it would answer `false` to
    // every `start_check` after this one
    assert!(!core.is_checking());
    assert_eq!(core.status(), NetCheckStatus::CheckEnd);
    assert!(core.start_check(
        &longlink,
        &CheckIPPorts::new(),
        NET_CHECK_BASIC,
        UNUSE_TIMEOUT
    ));
    assert_eq!(core.run_on(record).len(), 2);
}

/// The rest of what a run leaves behind: not just `checking_`, but the
/// request it was running. `SdtLogic::run` takes no request of its own, so
/// what a run that unwinds leaves in the core is what the next one starts
/// from — a request already [`CheckStatus::CheckFinish`] stops it before its
/// first check, and the results it did not record are the ones it answers
/// with.
#[test]
fn a_run_that_panicked_leaves_no_request_behind() {
    let longlink = hosts(&["long.weixin.qq.com"]);

    let mut core = SdtCore::new();
    core.start_check(
        &longlink,
        &CheckIPPorts::new(),
        NET_CHECK_BASIC,
        UNUSE_TIMEOUT,
    );

    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        core.run_on(|_, request| {
            request
                .checkresult_profiles
                .push(CheckResultProfile::of(NetCheckType::PingCheck));
            request.check_status = CheckStatus::CheckFinish;
            panic!("a probe that fell over after finishing");
        })
    }));
    assert!(panicked.is_err(), "the panic is not swallowed");

    assert!(
        core.request().checkresult_profiles.is_empty(),
        "the results of the run that panicked are still there"
    );
    assert_ne!(
        core.request().check_status,
        CheckStatus::CheckFinish,
        "the request the next run would start from is already finished"
    );
}

#[test]
fn a_cancel_handle_stays_usable_after_a_run() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut core = SdtCore::new();
    let cancel = core.cancel_handle();
    core.start_check(&longlink, &shortlink, NET_CHECK_BASIC, UNUSE_TIMEOUT);
    cancel.cancel();
    assert!(core.run_on(record).is_empty());

    // a second request through the same handle
    core.start_check(&longlink, &shortlink, NET_CHECK_BASIC, UNUSE_TIMEOUT);
    assert!(!cancel.is_cancelled(), "the new request is not cancelled");
    assert_eq!(core.run_on(record).len(), 2);
    cancel.cancel();
    assert!(core.is_cancelled());
}

#[test]
fn a_default_core_is_a_fresh_one() {
    let core = SdtCore::default();
    assert_eq!(core.status(), NetCheckStatus::None);
    assert!(!core.is_checking());
    assert!(!core.is_cancelled());
    assert!(core.plan().is_empty());
    assert!(core.http_netcheck_cgi().is_empty());
    assert_eq!(core.request().mode, NET_CHECK_BASIC);
    // a handle of its own is one nothing has cancelled
    assert!(!CancelHandle::default().is_cancelled());
}

#[test]
fn the_http_check_cgi_is_kept_for_the_checker() {
    let mut core = SdtCore::new();
    assert!(core.http_netcheck_cgi().is_empty());
    core.set_http_netcheck_cgi("http://www.qq.com/netcheck");
    assert_eq!(core.http_netcheck_cgi(), "http://www.qq.com/netcheck");
}
