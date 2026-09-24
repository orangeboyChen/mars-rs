//! `mars/sdt/sdt_logic.cc` — the surface the app calls.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mars_sdt::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use mars_sdt::{
    Callback, CheckIPPort, CheckIPPorts, NetCheckType, SdtLogic, NET_CHECK_BASIC, NET_CHECK_LONG,
    NET_CHECK_SHORT, UNUSE_TIMEOUT,
};

fn hosts(names: &[&str]) -> CheckIPPorts {
    names
        .iter()
        .map(|name| (name.to_string(), vec![CheckIPPort::new("1.2.3.4", 80)]))
        .collect::<BTreeMap<_, _>>()
}

fn record(kind: NetCheckType, request: &mut CheckRequestProfile) {
    request
        .checkresult_profiles
        .push(CheckResultProfile::of(kind));
}

#[test]
fn a_fresh_logic_is_not_checking_and_has_nobody_to_tell() {
    let logic = SdtLogic::new();
    assert!(!logic.is_checking());
    assert!(!logic.has_callback());
    assert!(logic.plan().is_empty());
    assert!(logic.http_netcheck_cgi().is_empty());
}

#[test]
fn start_and_cancel_drive_the_core() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut logic = SdtLogic::new();
    assert!(logic.start_active_check(
        &longlink,
        &shortlink,
        NET_CHECK_BASIC | NET_CHECK_SHORT,
        3000
    ));
    assert!(logic.is_checking());
    assert_eq!(
        logic.plan(),
        &[
            NetCheckType::PingCheck,
            NetCheckType::DnsCheck,
            NetCheckType::HttpCheck
        ]
    );
    assert_eq!(logic.request().total_timeout, 3000);

    // a second one is refused while the first is in flight
    assert!(!logic.start_active_check(&longlink, &shortlink, NET_CHECK_LONG, 3000));

    logic.cancel_active_check();
    assert!(logic.run(record).is_empty());
}

#[test]
fn a_check_that_is_already_running_can_be_cancelled() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut logic = SdtLogic::new();
    logic.start_active_check(
        &longlink,
        &shortlink,
        NET_CHECK_BASIC | NET_CHECK_SHORT,
        3000,
    );

    // `run` holds the logic exclusively until the last check is done, so
    // `cancel_active_check()` cannot be called from outside while it runs:
    // the handle is taken first and cancelled by the checker, which is what
    // would be blocked on a socket in the real thing.
    let cancel = logic.cancel_handle();
    let cancel_in_check = cancel.clone();
    let ran = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&ran);
    let results = logic.run(move |kind, request| {
        sink.lock().unwrap().push(kind);
        cancel_in_check.cancel();
        record(kind, request);
    });

    assert_eq!(*ran.lock().unwrap(), vec![NetCheckType::PingCheck]);
    assert_eq!(results.len(), 1, "the DNS and HTTP checks must not run");
    assert!(cancel.is_cancelled());

    // and the logic is usable again once it is given a new request
    assert!(logic.start_active_check(&longlink, &shortlink, NET_CHECK_BASIC, 3000));
    assert_eq!(logic.run(record).len(), 2);
}

#[test]
fn what_the_checks_record_is_handed_to_the_callback() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut logic = SdtLogic::new();
    let results = Arc::new(Mutex::new(Vec::new()));
    let sink = results.clone();

    struct Forward(Arc<Mutex<Vec<Vec<CheckResultProfile>>>>);
    impl Callback for Forward {
        fn report_net_check_result(&self, check_results: &[CheckResultProfile]) {
            self.0.lock().unwrap().push(check_results.to_vec());
        }
    }

    logic.set_callback(Forward(sink));
    assert!(logic.has_callback());

    logic.start_active_check(&longlink, &shortlink, NET_CHECK_BASIC, UNUSE_TIMEOUT);
    let reported = logic.run(record);

    assert_eq!(reported.len(), 2);
    let seen = results.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0], reported);
}

#[test]
fn reporting_with_nobody_listening_is_a_no_op() {
    let mut logic = SdtLogic::new();
    logic.start_active_check(&hosts(&["long"]), &hosts(&["short"]), NET_CHECK_LONG, 1000);

    // no callback was ever set: `ReportNetCheckResult` still has to work
    let results = logic.run(record);
    assert_eq!(results.len(), 1);
    assert!(!logic.has_callback());
}

#[test]
fn the_callback_is_replaced_not_appended() {
    let mut logic = SdtLogic::new();

    struct Counting(Arc<Mutex<usize>>);
    impl Callback for Counting {
        fn report_net_check_result(&self, check_results: &[CheckResultProfile]) {
            *self.0.lock().unwrap() += check_results.len();
        }
    }

    let first = Arc::new(Mutex::new(0));
    let second = Arc::new(Mutex::new(0));

    logic.set_callback(Counting(first.clone()));
    logic.report(&[
        CheckResultProfile::of(NetCheckType::PingCheck),
        CheckResultProfile::of(NetCheckType::DnsCheck),
    ]);

    // `SetCallBack` keeps one listener, so the first one is not told any more
    logic.set_callback(Counting(second.clone()));
    logic.report(&[CheckResultProfile::of(NetCheckType::TcpCheck)]);

    assert_eq!(*first.lock().unwrap(), 2);
    assert_eq!(*second.lock().unwrap(), 1);

    logic.clear_callback();
    assert!(!logic.has_callback());
    logic.report(&[CheckResultProfile::of(NetCheckType::HttpCheck)]);
    assert_eq!(*second.lock().unwrap(), 1);
}

#[test]
fn the_debug_output_says_whether_somebody_is_listening() {
    struct Silent;
    impl Callback for Silent {}

    let mut logic = SdtLogic::new();
    assert!(format!("{logic:?}").contains("has_callback: false"));

    logic.set_callback(Silent);
    assert!(format!("{logic:?}").contains("has_callback: true"));
}

#[test]
fn the_cgi_reaches_the_core() {
    let mut logic = SdtLogic::default();
    logic.set_http_netcheck_cgi("/cgi-bin/netcheck");
    assert_eq!(logic.http_netcheck_cgi(), "/cgi-bin/netcheck");
}
