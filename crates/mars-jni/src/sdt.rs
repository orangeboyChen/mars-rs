//! `io.github.marsrs.sdt.SdtLogic` — the network diagnosis.
//!
//! `com/tencent/mars/sdt/SdtLogic.java` declares two `native` methods,
//! `setHttpNetcheckCGI` and `getLoadLibraries`, and the C++
//! (`mars/sdt/jni/com_tencent_mars_sdt_SdtLogic_Java2C.cc`) forwards the first
//! one to `mars::sdt::SetHttpNetcheckCGI`. This module is that call, on top of
//! [`mars_sdt::SdtLogic`], plus the two things the CGI is for: running a check
//! and turning what it found into the JSON
//! `SdtLogic.reportSignalDetectResults(String)` hands to the app.
//!
//! That hand-over goes the other way round from every other call in this
//! crate: `reportSignalDetectResults` is a static **Java** method the native
//! side calls when a diagnosis ends
//! (`com_tencent_mars_sdt_SdtLogic_C2Java.cc`), not a `native` method Java
//! calls. It needs a live `JNIEnv`, so it lives in
//! [`crate::jni_bridge::report_signal_detect_results`] and is left out of the
//! tests; everything up to it — the JSON, and the record of what was handed
//! over — is here and is covered by `cargo test`.
//!
//! Everything else the JVM touches lives in [`crate::jni_bridge`]; what is here
//! is plain Rust and is covered by `cargo test`.

use std::sync::{Arc, Mutex, OnceLock};

use mars_sdt::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use mars_sdt::{Callback, CheckIPPorts, NetCheckType, SdtLogic};

/// What `getLoadLibraries` reports: the C++ lists the modules the process
/// loaded, which in this port is this one library.
pub const LOAD_LIBRARIES: &[&str] = &["marsxlog"];

/// Keeps the results `SdtLogic` reported, so the host can pick them up — and
/// hands them to Java as well, which is what a report is for.
struct Sink(Arc<Mutex<Vec<CheckResultProfile>>>);

impl Callback for Sink {
    fn report_net_check_result(&self, check_results: &[CheckResultProfile]) {
        if let Ok(mut reported) = self.0.lock() {
            reported.extend_from_slice(check_results);
        }
        deliver_report_impl(check_results);
    }
}

/// The JSON reports handed to Java since the last call.
///
/// This lives outside [`SdtState`] on purpose: the callback runs *inside*
/// [`run_checks_impl`], which holds the state lock for the whole run, so
/// taking that lock again to record a report would deadlock.
fn delivered() -> &'static Mutex<Vec<String>> {
    static DELIVERED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    DELIVERED.get_or_init(|| Mutex::new(Vec::new()))
}

struct SdtState {
    logic: SdtLogic,
    reported: Arc<Mutex<Vec<CheckResultProfile>>>,
}

fn state() -> &'static Mutex<SdtState> {
    static STATE: OnceLock<Mutex<SdtState>> = OnceLock::new();
    STATE.get_or_init(|| {
        let reported = Arc::new(Mutex::new(Vec::new()));
        let mut logic = SdtLogic::new();
        logic.set_callback(Sink(Arc::clone(&reported)));
        Mutex::new(SdtState { logic, reported })
    })
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
    let reported = Arc::new(Mutex::new(Vec::new()));
    let mut logic = SdtLogic::new();
    logic.set_callback(Sink(Arc::clone(&reported)));
    *state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = SdtState { logic, reported };
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

/// `StartActiveCheck` — `false` when a check is already in flight.
///
/// Java does not declare this one; it is the Rust side of the same state, and
/// the reason [`set_http_netcheck_cgi_impl`] exists.
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
pub fn cancel_active_check_impl() {
    with_state(|state| state.logic.cancel_active_check())
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
pub fn run_checks_impl(
    do_check: impl FnMut(NetCheckType, &mut CheckRequestProfile),
) -> Vec<CheckResultProfile> {
    with_state(|state| state.logic.run(do_check))
}

/// Takes everything the checks have reported since the last call.
pub fn take_reported_impl() -> Vec<CheckResultProfile> {
    with_state(|state| {
        state
            .reported
            .lock()
            .map(|mut reported| std::mem::take(&mut *reported))
            .unwrap_or_default()
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

/// A JSON string field: the C++ writes `iter->ip` straight into the document,
/// so a quote or a newline in a domain name — and the domain names come from
/// the caller — used to break the whole report.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for char in value.chars() {
        match char {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            control if control < ' ' => out.push_str(&format!("\\u{:04x}", control as u32)),
            _ => out.push(char),
        }
    }
    out.push('"');
    out
}

/// `SdtManagerJniCallback::ReportNetCheckResult()` — the JSON the app gets
/// through `SdtLogic.reportSignalDetectResults`, field for field the C++'s,
/// with the string fields escaped (`json_string`).
pub fn report_json_impl(check_results: &[CheckResultProfile]) -> String {
    let mut json = String::from("{\"details\":[");
    for (index, result) in check_results.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            "{{\"detectType\":{},\"errorCode\":{},\"networkType\":{},\"detectIP\":{},\"port\":{},\"conntime\":{},\"rtt\":{},\"rttStr\":{},\"httpStatusCode\":{},\"pingCheckCount\":{},\"pingLossRate\":{},\"dnsDomain\":{},\"localDns\":{},\"dnsIP1\":{},\"dnsIP2\":{}}}",
            result.netcheck_type,
            result.error_code,
            result.network_type,
            json_string(&result.ip),
            result.port,
            result.conntime,
            result.rtt,
            json_string(&result.rtt_str),
            result.status_code,
            result.checkcount,
            json_string(&result.loss_rate),
            json_string(&result.domain_name),
            json_string(&result.local_dns),
            json_string(&result.ip1),
            json_string(&result.ip2),
        ));
    }
    json.push_str("]}");
    json
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
    use mars_sdt::{CheckIPPort, NetCheckType as Kind, NET_CHECK_BASIC, UNUSE_TIMEOUT};
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
        assert_eq!(get_load_libraries_impl(), vec!["marsxlog".to_owned()]);
    }
}
