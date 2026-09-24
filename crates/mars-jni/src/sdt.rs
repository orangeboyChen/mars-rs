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
//! Everything the JVM touches lives in [`crate::jni_bridge`]; what is here is
//! plain Rust and is covered by `cargo test`.

use std::sync::{Arc, Mutex, OnceLock};

use mars_sdt::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use mars_sdt::{Callback, CheckIPPorts, NetCheckType, SdtLogic};

/// What `getLoadLibraries` reports: the C++ lists the modules the process
/// loaded, which in this port is this one library.
pub const LOAD_LIBRARIES: &[&str] = &["marsxlog"];

/// Keeps the results `SdtLogic` reported, so the host can pick them up.
struct Sink(Arc<Mutex<Vec<CheckResultProfile>>>);

impl Callback for Sink {
    fn report_net_check_result(&self, check_results: &[CheckResultProfile]) {
        if let Ok(mut reported) = self.0.lock() {
            reported.extend_from_slice(check_results);
        }
    }
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

/// Drops the diagnosis state and starts over.
///
/// A cancelled `SdtCore` is cancelled for good — the C++ never clears
/// `cancel_`, so mars-sdt says it "has to be replaced" — and this is the only
/// way back from [`cancel_active_check_impl`].
pub fn reset_impl() {
    let reported = Arc::new(Mutex::new(Vec::new()));
    let mut logic = SdtLogic::new();
    logic.set_callback(Sink(Arc::clone(&reported)));
    *state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = SdtState { logic, reported };
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

/// `SdtManagerJniCallback::ReportNetCheckResult()` — the JSON the app gets
/// through `SdtLogic.reportSignalDetectResults`, field for field the C++'s.
pub fn report_json_impl(check_results: &[CheckResultProfile]) -> String {
    let mut json = String::from("{\"details\":[");
    for (index, result) in check_results.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            "{{\"detectType\":{},\"errorCode\":{},\"networkType\":{},\"detectIP\":\"{}\",\"port\":{},\"conntime\":{},\"rtt\":{},\"rttStr\":\"{}\",\"httpStatusCode\":{},\"pingCheckCount\":{},\"pingLossRate\":\"{}\",\"dnsDomain\":\"{}\",\"localDns\":\"{}\",\"dnsIP1\":\"{}\",\"dnsIP2\":\"{}\"}}",
            result.netcheck_type,
            result.error_code,
            result.network_type,
            result.ip,
            result.port,
            result.conntime,
            result.rtt,
            result.rtt_str,
            result.status_code,
            result.checkcount,
            result.loss_rate,
            result.domain_name,
            result.local_dns,
            result.ip1,
            result.ip2,
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
