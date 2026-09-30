//! `mars/sdt/sdt_logic.cc` — the surface the app calls.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use marsrs_sdt::checkimpl::Ask;
use marsrs_sdt::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use marsrs_sdt::{
    Callback, CheckIPPort, CheckIPPorts, Mode, NetCheckStatus, NetCheckType, SdtLogic,
    NET_CHECK_BASIC, NET_CHECK_LONG, NET_CHECK_SHORT, UNUSE_TIMEOUT,
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

/// A mode of no checks at all is a plan of nothing: taking the request would
/// leave a diagnosis that runs nothing, reports nothing and still answers
/// `is_checking()` — a caller that asked for one would be told it has one.
/// Every seam has to say no to it alike, because a seam that says yes is a
/// diagnosis that is over before it began.
#[test]
fn a_mode_that_names_no_check_starts_nothing() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut logic = SdtLogic::new();
    assert!(
        !logic.start_active_check(&longlink, &shortlink, 0, 3000),
        "a mode of no bits is not a request"
    );
    assert!(!logic.is_checking());
    assert!(logic.plan().is_empty());

    // and it is not a check that was refused for being one too many: the core
    // is still empty, so the next request is taken
    assert!(logic.start_active_check(&longlink, &shortlink, NET_CHECK_LONG, 3000));
    assert!(logic.is_checking());

    // a bit that is not one of the three is no plan either
    logic.cancel_active_check();
    assert!(logic.run(record).is_empty());
    assert!(!logic.start_active_check(&longlink, &shortlink, 1 << 7, 3000));
    assert!(!logic.is_checking());
}

#[test]
fn the_status_of_a_run_says_whether_it_ended_or_was_cut_short() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut logic = SdtLogic::new();
    assert_eq!(logic.status(), NetCheckStatus::None);

    // a run that ran to its end
    assert!(logic.start_active_check(&longlink, &shortlink, NET_CHECK_BASIC, UNUSE_TIMEOUT));
    assert_eq!(logic.status(), NetCheckStatus::Checking);
    assert_eq!(logic.run(record).len(), 2);
    assert_eq!(logic.status(), NetCheckStatus::CheckEnd);
    assert!(!logic.is_cancelled(), "it ended on its own");

    // the same run, cancelled before the first check: the results are the ones
    // the checks before the cancel recorded, and that is all a listener has
    // to tell the two apart
    assert!(logic.start_active_check(&longlink, &shortlink, NET_CHECK_BASIC, UNUSE_TIMEOUT));
    logic.cancel_active_check();
    assert!(logic.run(record).is_empty());
    assert_eq!(logic.status(), NetCheckStatus::CheckEnd);
    assert!(logic.is_cancelled());
    assert!(!logic.is_checking());
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

/// Whoever the app set is told, and what it was told is kept.
fn listener() -> (Arc<Mutex<Vec<Vec<CheckResultProfile>>>>, impl Callback) {
    struct Forward(Arc<Mutex<Vec<Vec<CheckResultProfile>>>>);
    impl Callback for Forward {
        fn report_net_check_result(&self, check_results: &[CheckResultProfile]) {
            self.0.lock().unwrap().push(check_results.to_vec());
        }
    }

    let results = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&results);
    (results, Forward(sink))
}

#[test]
fn one_call_is_the_whole_diagnosis() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);
    let (reported, callback) = listener();

    let mut logic = SdtLogic::new();
    logic.set_http_netcheck_cgi("/cgi-bin/netcheck");
    logic.set_callback(callback);

    // what the C++ needs three calls and its `__RunOn` thread for
    let mut ask = Ask::default();
    let results = logic
        .diagnose(
            &longlink,
            &shortlink,
            Mode::BASIC | Mode::SHORT,
            UNUSE_TIMEOUT,
            &mut ask,
            2,
        )
        .expect("no check was in flight");

    assert!(!results.is_empty());
    assert!(!logic.is_checking(), "the run is over when the call is");
    assert_eq!(*reported.lock().unwrap(), vec![results.clone()]);

    // and the logic is one diagnosis richer: nothing is left in flight, so a
    // second one is taken
    let again = logic.diagnose(
        &longlink,
        &shortlink,
        Mode::LONG,
        UNUSE_TIMEOUT,
        &mut ask,
        2,
    );
    assert!(again.is_some(), "the first run is over");
}

#[test]
fn a_diagnosis_that_is_already_in_flight_is_not_taken() {
    let longlink = hosts(&["long.weixin.qq.com"]);
    let shortlink = hosts(&["short.weixin.qq.com"]);

    let mut logic = SdtLogic::new();
    assert!(logic.start_active_check(&longlink, &shortlink, NET_CHECK_LONG, 3000));

    // the `false` of the upstream call, which is what `None` is here
    let mut ask = Ask::default();
    assert!(logic
        .diagnose(&longlink, &shortlink, Mode::BASIC, 3000, &mut ask, 2)
        .is_none());
    assert!(
        logic.is_checking(),
        "the run that was in flight is still in flight"
    );
}

#[test]
fn a_mode_is_the_bits_the_cpp_counts() {
    assert_eq!(Mode::NONE.bits(), 0);
    assert_eq!(Mode::BASIC.bits(), NET_CHECK_BASIC);
    assert_eq!(Mode::LONG.bits(), NET_CHECK_LONG);
    assert_eq!(Mode::SHORT.bits(), NET_CHECK_SHORT);
    assert_eq!(
        (Mode::BASIC | Mode::LONG | Mode::SHORT).bits(),
        Mode::ALL.bits()
    );
    // and the other way round, for an app that still counts
    assert_eq!(
        Mode::of(NET_CHECK_BASIC | NET_CHECK_SHORT),
        Mode::BASIC | Mode::SHORT
    );
    assert_eq!(i32::from(Mode::ALL), Mode::ALL.bits());
}
