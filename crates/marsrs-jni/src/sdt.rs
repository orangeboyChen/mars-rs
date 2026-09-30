//! `io.github.orangeboychen.marsrs.sdt.SdtLogic` — the network diagnosis.
//!
//! `com/tencent/mars/sdt/SdtLogic.java` declares two `native` methods,
//! `setHttpNetcheckCGI` and `getLoadLibraries`, and the C++
//! (`mars/sdt/jni/com_tencent_mars_sdt_SdtLogic_Java2C.cc`) forwards the first
//! one to `mars::sdt::SetHttpNetcheckCGI`. This module is that call, on top of
//! [`marsrs_sdt::SdtLogic`], plus the two things the CGI is for: running a check
//! and turning what it found into the JSON
//! `SdtLogic.reportSignalDetectResults(String)` hands to the app.
//!
//! That hand-over goes the other way round from every other call in this
//! crate: `reportSignalDetectResults` is a static **Java** method the native
//! side calls when a diagnosis ends
//! (`com_tencent_mars_sdt_SdtLogic_C2Java.cc`), not a `native` method Java
//! calls. It needs a live `Env`, so it lives in
//! [`crate::jni_bridge::report_signal_detect_results`] and is left out of the
//! tests; everything up to it — the JSON, and the record of what was handed
//! over — is here and is covered by `cargo test`.
//!
//! The two the C++'s Java declares are two because the C++ needs no more: a
//! diagnosis there is started from inside the C++, by threads that are its own
//! and sockets that are its own. The port has neither, so Java starts the
//! diagnosis ([`sdt::start_active_check_impl`]) and answers the four probes of
//! it ([`sdt::run_checks_java_impl`]) — and asks the two questions that go with
//! running one by hand: whether a check is in flight, and what it is going to
//! do. Those are the `external`s the port's `SdtLogic` declares beside the
//! C++'s two.
//!
//! Everything else the JVM touches lives in [`crate::jni_bridge`]; what is here
//! is plain Rust and is covered by `cargo test`.
//!
//! The checks themselves are [`marsrs_sdt`]'s: [`sdt::run_active_check_impl`]
//! runs the planned checks with the four checkers the C++ creates in
//! `__InitCheckReq`, which ask the network — DNS, TCP, HTTP, ping — through the
//! [`marsrs_sdt::checkimpl::Ask`] the host hands over, the way the C++ opens a
//! socket for each of them.

use std::sync::{Arc, Mutex, OnceLock};

use marsrs_sdt::checkimpl::Ask;
use marsrs_sdt::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use marsrs_sdt::sdt_core::CancelHandle;
use marsrs_sdt::{Callback, CheckIPPorts, NetCheckType, SdtLogic};

/// What `getLoadLibraries` reports: the C++ lists the modules the process
/// loaded, which in this port is this one library.
pub const LOAD_LIBRARIES: &[&str] = &["marsrsxlog"];

/// Keeps the results `SdtLogic` reported, so the host can pick them up.
///
/// What it does *not* do is hand them to Java: the callback runs inside
/// [`run_checks_impl`], which holds the process-wide diagnosis for the whole
/// run, and handing a report over is a call into the JVM — from which an app's
/// `onSignalDetectResults` is free to ask `isChecking()`, `plan()` or
/// `startActiveCheck()`, all of which come back into [`with_state`] on the
/// thread that is already holding it. `std::sync::Mutex` is not reentrant, so
/// that is a hang, and on the app's main thread it is an ANR. The run hands
/// its own results over once it has let the diagnosis go; see
/// [`run_checks_impl`].
struct Sink(Arc<Mutex<Vec<CheckResultProfile>>>);

impl Callback for Sink {
    fn report_net_check_result(&self, check_results: &[CheckResultProfile]) {
        // The poisoned lock is taken anyway: a panic that got out of a run
        // leaves the flag set, and a report that is dropped for it is one no
        // later take can hand over either — the results of a diagnosis lost
        // because something else in the process panicked.
        let mut reported = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reported.extend_from_slice(check_results);
    }
}

/// The JSON reports handed to Java since the last call.
///
/// Not a field of [`SdtState`]: asking for one must not build a diagnosis to
/// answer it, and reaching the state does — [`state`] makes the state the
/// first time anything asks for it, and [`new_state`] is a whole core with a
/// callback wired into it. A mutex of its own is also one a delivery does not
/// share with the run it came out of, so a host that asks for what it has
/// already been given never waits on a run, and a run never waits on it.
fn delivered() -> &'static Mutex<Vec<String>> {
    static DELIVERED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    DELIVERED.get_or_init(|| Mutex::new(Vec::new()))
}

struct SdtState {
    logic: SdtLogic,
    reported: Arc<Mutex<Vec<CheckResultProfile>>>,
}

/// A diagnosis nobody has touched yet, with the callback that records what it
/// finds already installed.
fn new_state() -> SdtState {
    let reported = Arc::new(Mutex::new(Vec::new()));
    let mut logic = SdtLogic::new();
    logic.set_callback(Sink(Arc::clone(&reported)));
    // A new diagnosis is a new core, and a new cancellation flag with it: this
    // is the one [`cancel_active_check_impl`] sets, and it has to be the one
    // the logic the state holds answers to.
    *cancel()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = logic.cancel_handle();
    SdtState { logic, reported }
}

/// The cancellation flag of the request in flight, kept *outside* [`state()`].
///
/// That is the whole point: [`run_active_check_impl`] holds the process-wide
/// diagnosis for as long as the checks take — which is exactly when a caller
/// wants to cancel — so a flag that had to be reached through that lock could
/// only ever be set before a run started or after it had already finished and
/// reset itself. [`marsrs_sdt::CancelHandle`] is shared for the same reason
/// inside the diagnosis, and this is the copy the boundary keeps of it.
fn cancel() -> &'static Mutex<CancelHandle> {
    static CANCEL: OnceLock<Mutex<CancelHandle>> = OnceLock::new();
    // A flag of its own, and not [`new_state`]'s: that one reaches for this,
    // so asking it here would be asking the question the answer is made of.
    // Every state that follows overwrites it with the handle of the core it
    // holds, which is the one a run of that core reads.
    CANCEL.get_or_init(|| Mutex::new(CancelHandle::new()))
}

fn state() -> &'static Mutex<SdtState> {
    static STATE: OnceLock<Mutex<SdtState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(new_state()))
}

fn with_state<R>(f: impl FnOnce(&mut SdtState) -> R) -> R {
    let mut state = state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut state)
}

/// Drops the diagnosis state and starts over: the waiting checks, the
/// callback and everything that was reported go away together.
pub fn reset_impl() {
    // The lock comes first: `new_state` reaches for [`cancel`], so building
    // the new state on the right of an assignment that has not locked yet
    // — the operands are evaluated right to left — would let the state's
    // own first build overwrite the handle the new one had just stored, and
    // a cancel would then set a flag no run reads.
    with_state(|state| *state = new_state());
    if let Ok(mut delivered) = delivered().lock() {
        delivered.clear();
    }
}

/// `SdtLogic.setHttpNetcheckCGI`.
pub fn set_http_netcheck_cgi_impl(cgi: &str) {
    with_state(|state| state.logic.set_http_netcheck_cgi(cgi))
}

/// The URL the HTTP check goes to.
pub fn http_netcheck_cgi_impl() -> String {
    with_state(|state| state.logic.http_netcheck_cgi().to_owned())
}

/// `StartActiveCheck` — `false` when no check was started: one that is
/// already in flight, or a `mode` with no check in it.
///
/// The C++'s Java declares no such call: there the diagnosis is started from
/// inside the C++, which has the sockets and the threads a run needs. The port
/// has neither, so starting one is the app's call and not the port's.
pub fn start_active_check_impl(
    longlink_items: &CheckIPPorts,
    shortlink_items: &CheckIPPorts,
    mode: i32,
    timeout: u32,
) -> bool {
    with_state(|state| {
        state
            .logic
            .start_active_check(longlink_items, shortlink_items, mode, timeout)
    })
}

/// `CancelActiveCheck`.
///
/// It stops a run that is *in* flight, not one that has not started: the flag
/// this sets is the one the run's own read, and it is reached without the lock
/// that run holds, so a caller may cancel from another thread while the probes
/// are still being asked — which is the only moment cancelling means anything.
pub fn cancel_active_check_impl() {
    cancel()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .cancel();
}

/// Whether a check is in flight.
pub fn is_checking_impl() -> bool {
    with_state(|state| state.logic.is_checking())
}

/// The checks the running request is going to make, in order.
pub fn plan_impl() -> Vec<NetCheckType> {
    with_state(|state| state.logic.plan().to_vec())
}

/// Runs the planned checks, one `do_check` per check, and reports what they
/// recorded. This is the `__RunOn` thread of the C++, driven by the host: the
/// port has no sockets of its own, so the caller supplies the checkers.
///
/// The report is handed over *after* `with_state` has let the diagnosis go,
/// which is the whole point: delivering it is a call into the JVM, and an
/// app's `onSignalDetectResults` asks the diagnosis questions of its own from
/// inside it. Holding the lock across the delivery would make that a hang —
/// an ANR on the main thread — because `std::sync::Mutex` is not reentrant.
pub fn run_checks_impl(
    do_check: impl FnMut(NetCheckType, &mut CheckRequestProfile),
) -> Vec<CheckResultProfile> {
    let results = with_state(|state| state.logic.run(do_check));
    deliver_report_impl(&results);
    results
}

/// [`run_checks_impl`] with the port's own checkers: the four classes the C++
/// creates in `__InitCheckReq`, asked through `ask` — the host's network is the
/// socket the C++ would have opened for every one of these probes.
///
/// `network_type` is the `comm::getNetInfo()` every checker writes into its
/// profiles. The platform is the host's here, which is why it comes with the
/// run: [`run_active_check_with_net_info_impl`] is this call with the one
/// [`crate::platform_comm::net_info_impl`] answered.
///
/// The probes are asked while the process-wide diagnosis is held, so a probe
/// must not ask the diagnosis about itself — `isChecking()`, `plan()`,
/// `startActiveCheck()` — from inside its answer. The port cannot move the ask
/// off that lock without moving the run off it, which is `marsrs-sdt`'s and not
/// this seam's; what the C ABI says about its own probe (`mars_sdt.h`) is the
/// same constraint, and it is written down there too.
pub fn run_active_check_impl(ask: &mut Ask, network_type: i32) -> Vec<CheckResultProfile> {
    let results = with_state(|state| state.logic.run_checks(ask, network_type));
    deliver_report_impl(&results);
    results
}

/// [`run_active_check_impl`] with the network type of the platform: the
/// `comm::getNetInfo()` the C++ asks from inside every check is
/// [`crate::platform_comm::net_info_impl`] here, which is the JVM's answer
/// (`PlatformComm.getNetInfo`) when nobody has set one.
pub fn run_active_check_with_net_info_impl(ask: &mut Ask) -> Vec<CheckResultProfile> {
    run_active_check_impl(ask, crate::platform_comm::net_info_impl().as_i32())
}

/// [`run_active_check_impl`] with the probes asked of Java: the four checks
/// want a socket, and the port opens none, so the app's `SdtLogic.IProbe` is
/// what they reach — [`crate::jni_bridge`] carries a query of the check to
/// Java and comes back with what that probe made of it, through the [`Ask`]
/// this call builds over it.
///
/// This is the `__RunOn` thread of the C++, run on the thread that called it:
/// it holds the process-wide diagnosis until every probe has answered, so a
/// run is one at a time and it does not come back until it is over. A probe
/// that asks the diagnosis about itself from inside its answer waits for that
/// lock on the thread holding it; see [`run_active_check_impl`].
///
/// `false` when nothing was in flight, and when the one that was got cancelled
/// before its first check: a run that answered nothing is a run that reported
/// nothing, which is what `MARS_SDT_ERR_NO_CHECK` is in the C ABI.
pub fn run_checks_java_impl(network_type: i32) -> bool {
    let mut ask = Ask::new(crate::jni_bridge::ask_probe);
    let results = run_active_check_impl(&mut ask, network_type);
    !results.is_empty()
}

/// Takes everything the checks have reported since the last call.
pub fn take_reported_impl() -> Vec<CheckResultProfile> {
    with_state(|state| {
        std::mem::take(
            &mut *state
                .reported
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    })
}

/// `SdtManagerJniCallback::ReportNetCheckResult()` — hands a finished diagnosis
/// over: the JSON [`report_json_impl`] builds goes to Java, and a copy stays
/// here for the host ([`take_delivered_impl`]) and for the tests, which have no
/// JVM to hand it to.
///
/// This is the call the C++ makes from inside `ReportNetCheckResult`, so a
/// diagnosis that ends is never just buffered: it reaches the app's
/// `SdtLogic.ICallBack` (or is recorded, when there is no JVM yet).
///
/// It is reached with the process-wide diagnosis released, and never from
/// inside a run: what it calls is Java, and Java calls back.
pub fn deliver_report_impl(check_results: &[CheckResultProfile]) -> String {
    let json = report_json_impl(check_results);
    if let Ok(mut delivered) = delivered().lock() {
        delivered.push(json.clone());
    }
    crate::jni_bridge::report_signal_detect_results(json.clone());
    json
}

/// The JSON reports handed to Java since the last call.
pub fn take_delivered_impl() -> Vec<String> {
    delivered()
        .lock()
        .map(|mut delivered| std::mem::take(&mut *delivered))
        .unwrap_or_default()
}

/// `SdtManagerJniCallback::ReportNetCheckResult()` — the document a finished
/// diagnosis is handed over as: the JSON [`marsrs_sdt::report_json`] builds.
///
/// The document itself is [`marsrs_sdt`]'s: the fields are
/// [`CheckResultProfile`]'s, and the C ABI hands the same one over, so both
/// seams spell a report the same way.
pub fn report_json_impl(check_results: &[CheckResultProfile]) -> String {
    marsrs_sdt::report_json(check_results)
}

/// `SdtLogic.getLoadLibraries`.
pub fn get_load_libraries_impl() -> Vec<String> {
    LOAD_LIBRARIES
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_comm::{self, NetInfo};
    use marsrs_sdt::checkimpl::{Answer, PingStatus, Query};
    use marsrs_sdt::{
        CheckIPPort, NetCheckType as Kind, NET_CHECK_BASIC, NET_CHECK_LONG, NET_CHECK_SHORT,
        UNUSE_TIMEOUT,
    };
    use std::collections::BTreeMap;

    fn isolated<R>(f: impl FnOnce() -> R) -> R {
        let guard = crate::test_lock();
        reset_impl();
        let result = f();
        reset_impl();
        drop(guard);
        result
    }

    fn hosts(name: &str) -> CheckIPPorts {
        let mut hosts = BTreeMap::new();
        hosts.insert(name.to_owned(), vec![CheckIPPort::new("1.2.3.4", 80)]);
        hosts
    }

    fn record(kind: Kind, request: &mut CheckRequestProfile) {
        request
            .checkresult_profiles
            .push(CheckResultProfile::of(kind));
    }

    /// A network that answers every probe the way a healthy one would: what the
    /// C++ gets from the sockets it opens for these four.
    fn network() -> Ask {
        Ask::new(|query| match query {
            Query::Dns { .. } => Answer::Dns {
                error_code: 0,
                rtt: 10,
                local_dns: String::new(),
                ips: vec!["1.2.3.4".to_owned()],
            },
            Query::Tcp { .. } => Answer::Tcp {
                sent: 0,
                received: 0,
                is_noop_resp: true,
                conntime: 0,
                rtt: 10,
            },
            Query::Http { .. } => Answer::Http {
                error_code: 0,
                status_code: 200,
                rtt: 10,
            },
            Query::Ping { .. } => Answer::Ping {
                error_code: 0,
                rtt: 10,
                status: Some(PingStatus::new(0.0, 12.5)),
            },
        })
    }

    #[test]
    fn the_cgi_is_set_and_read_back() {
        isolated(|| {
            assert!(http_netcheck_cgi_impl().is_empty());
            set_http_netcheck_cgi_impl("/cgi-bin/netcheck");
            assert_eq!(http_netcheck_cgi_impl(), "/cgi-bin/netcheck");
        })
    }

    #[test]
    fn a_check_runs_and_what_it_found_is_collected() {
        isolated(|| {
            assert!(start_active_check_impl(
                &hosts("long.weixin.qq.com"),
                &hosts("short.weixin.qq.com"),
                NET_CHECK_BASIC,
                UNUSE_TIMEOUT
            ));
            assert!(is_checking_impl());
            // a second one is refused while the first is in flight
            assert!(!start_active_check_impl(
                &hosts("long.weixin.qq.com"),
                &hosts("short.weixin.qq.com"),
                NET_CHECK_BASIC,
                UNUSE_TIMEOUT
            ));

            assert_eq!(plan_impl(), vec![Kind::PingCheck, Kind::DnsCheck]);
            let results = run_checks_impl(record);
            assert_eq!(results.len(), 2);
            assert!(!is_checking_impl(), "__RunOn resets the core when it ends");

            // what ran is what was reported
            assert_eq!(take_reported_impl().len(), 2);
            assert!(take_reported_impl().is_empty(), "taken only once");
        })
    }

    #[test]
    fn a_cancelled_check_runs_nothing() {
        isolated(|| {
            assert!(start_active_check_impl(
                &hosts("long"),
                &hosts("short"),
                NET_CHECK_BASIC,
                UNUSE_TIMEOUT
            ));
            cancel_active_check_impl();
            assert!(run_checks_impl(record).is_empty());
        })
    }

    /// A cancel that has to wait for the run it means to stop is no cancel at
    /// all — and the run holds the diagnosis for as long as its probes take,
    /// which is exactly when the app's UI thread presses "stop". So the flag is
    /// reached without that lock: the cancel below lands on another thread
    /// while the first check is still being made, and the run stops at the
    /// second. (Were it reached through the state lock, `join` would never
    /// return.)
    #[test]
    fn a_cancel_from_another_thread_stops_the_run_that_is_in_flight() {
        isolated(|| {
            assert!(start_active_check_impl(
                &hosts("long"),
                &hosts("short"),
                NET_CHECK_BASIC,
                UNUSE_TIMEOUT
            ));
            assert_eq!(plan_impl().len(), 2);

            let mut first = true;
            let results = run_checks_impl(|kind, request| {
                record(kind, request);
                if std::mem::take(&mut first) {
                    // The thread stands in for the app's: it takes no part of
                    // the run with it, which is the point.
                    std::thread::spawn(cancel_active_check_impl).join().unwrap();
                }
            });
            assert_eq!(results.len(), 1, "the second check never ran");
            assert_eq!(results[0].kind(), Some(Kind::PingCheck));
        })
    }

    /// A reset throws the cancellation away with the request it cancelled: the
    /// flag a caller sets afterwards belongs to the core the next run reads,
    /// not to the one that is gone.
    #[test]
    fn a_reset_takes_the_cancellation_with_it() {
        isolated(|| {
            assert!(start_active_check_impl(
                &hosts("long"),
                &hosts("short"),
                NET_CHECK_BASIC,
                UNUSE_TIMEOUT
            ));
            cancel_active_check_impl();
            reset_impl();
            assert!(start_active_check_impl(
                &hosts("long"),
                &hosts("short"),
                NET_CHECK_BASIC,
                UNUSE_TIMEOUT
            ));
            assert_eq!(run_checks_impl(record).len(), 2, "the new request ran");
        })
    }

    #[test]
    fn a_diagnosis_runs_with_the_checkers_of_the_port() {
        isolated(|| {
            set_http_netcheck_cgi_impl("/cgi-bin/netcheck");
            assert!(start_active_check_impl(
                &hosts("long.weixin.qq.com"),
                &hosts("short.weixin.qq.com"),
                NET_CHECK_BASIC | NET_CHECK_SHORT | NET_CHECK_LONG,
                UNUSE_TIMEOUT
            ));

            let results = run_active_check_impl(&mut network(), NetInfo::Wifi.as_i32());
            let kinds: Vec<Kind> = results
                .iter()
                .filter_map(CheckResultProfile::kind)
                .collect();
            assert_eq!(
                kinds,
                vec![
                    Kind::PingCheck,
                    Kind::PingCheck,
                    Kind::DnsCheck,
                    Kind::DnsCheck,
                    Kind::HttpCheck,
                    Kind::TcpCheck
                ]
            );

            // what the host answered is what the profiles say: the status of
            // the ping, the address the resolve came back with, and the URL the
            // HTTP check went to — the CGI on the short-link host
            assert_eq!(results[0].loss_rate, "0.000000");
            assert_eq!(results[0].rtt_str, "12.500000");
            assert_eq!(results[0].network_type, NetInfo::Wifi.as_i32());
            assert_eq!(results[2].domain_name, "long.weixin.qq.com");
            assert_eq!(results[2].ip1, "1.2.3.4");
            assert_eq!(
                results[4].url,
                "http://short.weixin.qq.com/cgi-bin/netcheck"
            );
            assert_eq!(results[4].status_code, 200);
            // the noop the TCP check sent: the long-link ip, and the round trip
            // the host measured
            assert_eq!(results[5].ip, "1.2.3.4");
            assert_eq!(results[5].port, 80);
            assert_eq!(results[5].error_code, 0);
            assert_eq!(results[5].rtt, 10);

            // a run with the port's own checkers is a diagnosis like any
            // other: what ran is what was reported, and the app is told
            assert_eq!(take_reported_impl().len(), 6);
            let delivered = take_delivered_impl();
            assert_eq!(delivered.len(), 1);
            assert!(
                delivered[0].contains("\"networkType\":1"),
                "{}",
                delivered[0]
            );
        })
    }

    #[test]
    fn the_network_type_of_a_run_is_the_one_the_platform_answered() {
        isolated(|| {
            // `comm::getNetInfo()` is `PlatformComm.getNetInfo` here, and a
            // host that answered "mobile" is what the checkers write in
            platform_comm::set_net_info_impl(NetInfo::Mobile.as_i32());
            assert!(start_active_check_impl(
                &hosts("long"),
                &hosts("short"),
                NET_CHECK_BASIC,
                UNUSE_TIMEOUT
            ));
            let results = run_active_check_with_net_info_impl(&mut network());
            assert_eq!(results.len(), 2, "a ping and a resolve for the long link");
            assert_eq!(results[0].network_type, NetInfo::Mobile.as_i32());

            // a platform that answered nothing is offline, which is what the
            // port says about a JVM that has not answered either — and the
            // platform is asked again for every run, not once
            platform_comm::set_net_info_impl(NetInfo::NoNet.as_i32());
            assert!(start_active_check_impl(
                &hosts("long"),
                &hosts("short"),
                NET_CHECK_BASIC,
                UNUSE_TIMEOUT
            ));
            let results = run_active_check_with_net_info_impl(&mut network());
            assert_eq!(results[0].network_type, NetInfo::NoNet.as_i32());

            // the platform's own tests reset what was set here
        })
    }

    /// Java is asked once per check when a run is driven from Java, and there
    /// is no JVM to ask in a unit test — so [`run_checks_java_impl`] is the run
    /// a host with no network at all would make.
    #[test]
    fn a_run_that_asks_java_with_no_jvm_to_ask_reports_one_failed_check() {
        isolated(|| {
            assert!(start_active_check_impl(
                &hosts("long.weixin.qq.com"),
                &hosts("short.weixin.qq.com"),
                NET_CHECK_BASIC,
                UNUSE_TIMEOUT
            ));

            assert!(
                run_checks_java_impl(NetInfo::Wifi.as_i32()),
                "the check ran, which is all a run reports"
            );
            let results = take_reported_impl();
            // One, and not the two of the plan: a host with nothing to answer
            // with is a platform the C++ skips the ping on, so the check that
            // reports is the resolve behind it — and a resolve nobody answered
            // is `kCheckFinish`, which ends the run there.
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].kind(), Some(Kind::DnsCheck));
            assert_ne!(
                results[0].error_code, 0,
                "a probe nobody answered is one that failed"
            );
        })
    }

    #[test]
    fn the_report_json_reads_like_the_cpp() {
        let mut ping = CheckResultProfile::of(Kind::PingCheck);
        ping.ip = "1.2.3.4".to_owned();
        ping.network_type = 1;
        ping.checkcount = 3;
        ping.loss_rate = "0%".to_owned();
        ping.rtt_str = "12ms".to_owned();

        let json = report_json_impl(&[ping]);
        assert!(json.starts_with("{\"details\":["), "{json}");
        assert!(json.ends_with("]}"), "{json}");
        assert!(json.contains("\"detectType\":0"), "{json}");
        assert!(json.contains("\"detectIP\":\"1.2.3.4\""), "{json}");
        assert!(json.contains("\"pingCheckCount\":3"), "{json}");
        assert!(json.contains("\"pingLossRate\":\"0%\""), "{json}");
        assert!(json.contains("\"rttStr\":\"12ms\""), "{json}");
        assert!(json.contains("\"httpStatusCode\":0"), "{json}");
    }

    #[test]
    fn a_finished_check_reaches_java() {
        isolated(|| {
            assert!(start_active_check_impl(
                &hosts("long.weixin.qq.com"),
                &hosts("short.weixin.qq.com"),
                NET_CHECK_BASIC,
                UNUSE_TIMEOUT
            ));
            let results = run_checks_impl(record);
            assert_eq!(results.len(), 2);

            // the report is not just buffered: it is handed over (here there
            // is no JVM, so what is recorded is what the host would get)
            let delivered = take_delivered_impl();
            assert_eq!(delivered.len(), 1, "the diagnosis was never delivered");
            assert_eq!(delivered[0], report_json_impl(&results));
            assert!(
                delivered[0].contains("\"detectType\":0"),
                "{}",
                delivered[0]
            );
            assert!(take_delivered_impl().is_empty(), "delivered only once");

            // a cancelled check runs nothing, and still says so: `SdtCore::
            // __RunOn` hands the (empty) profiles to `ReportNetCheckResult`
            // whatever the reason the run ended, so the app hears that the
            // diagnosis finished even when there is nothing in it
            assert!(start_active_check_impl(
                &hosts("long"),
                &hosts("short"),
                NET_CHECK_BASIC,
                UNUSE_TIMEOUT
            ));
            cancel_active_check_impl();
            assert!(run_checks_impl(record).is_empty());
            assert_eq!(take_delivered_impl(), vec!["{\"details\":[]}".to_owned()]);
        })
    }

    #[test]
    fn the_string_fields_are_json_escaped() {
        // a domain name comes from the caller, and the C++ writes it into the
        // document verbatim, so one quote used to break the whole report
        let mut dns = CheckResultProfile::of(Kind::DnsCheck);
        dns.domain_name = "a\"b\\c\nd\re\tf".to_owned();
        dns.local_dns = String::from("x\u{1}y");
        dns.ip1 = "1.2.3.4".to_owned();

        let json = report_json_impl(&[dns]);
        assert!(json.contains("\\\""), "the quote is escaped: {json}");
        assert!(json.contains("\\\\"), "the backslash is escaped: {json}");
        assert!(json.contains("\\n"), "the newline is escaped: {json}");
        assert!(
            json.contains("\\r"),
            "the carriage return is escaped: {json}"
        );
        assert!(json.contains("\\t"), "the tab is escaped: {json}");
        assert!(
            json.contains("\\u0001"),
            "a control char is escaped: {json}"
        );
        assert!(
            !json.contains('\n'),
            "the document has no raw newline: {json}"
        );

        // and the document is one object with one detail, not the two or three
        // a raw newline or quote would have turned it into
        assert!(json.starts_with("{\"details\":[{"), "{json}");
        assert!(json.ends_with("}]}"), "{json}");
        assert_eq!(json.matches("\\\\").count(), 1, "one backslash: {json}");
    }

    #[test]
    fn every_result_becomes_one_detail_and_empty_is_still_valid() {
        assert_eq!(report_json_impl(&[]), "{\"details\":[]}");

        let results = vec![
            CheckResultProfile::of(Kind::PingCheck),
            CheckResultProfile::of(Kind::DnsCheck),
        ];
        let json = report_json_impl(&results);
        assert_eq!(json.matches("\"detectType\":").count(), 2);
        assert!(json.contains("},{"), "the details are comma separated");
    }

    #[test]
    fn the_libraries_the_process_loaded() {
        assert_eq!(get_load_libraries_impl(), vec!["marsrsxlog".to_owned()]);
    }
}
