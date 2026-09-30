//! Drives the SDT C ABI the way an app's `SdtLogic` does: point it at the hosts
//! of the two links, ask what it is going to check, run the checks over a probe
//! this test answers, then take the JSON report they produced.
//!
//! The diagnosis is one process-wide value — the counterpart of the C++'s
//! singleton — so every test takes [`lock()`]: `cargo test` runs the cases of
//! this file on parallel threads and they would otherwise race over the same
//! request.

#![cfg(feature = "sdt")]

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::Duration;

use mars_ffi::sdt::{
    mars_sdt_cancel_active_check, mars_sdt_http_netcheck_cgi, mars_sdt_is_checking, mars_sdt_plan,
    mars_sdt_reset, mars_sdt_run_checks, mars_sdt_set_http_netcheck_cgi,
    mars_sdt_start_active_check, mars_sdt_take_report, MarsSdtAnswer, MarsSdtCheck, MarsSdtHosts,
    MarsSdtIpPort, MarsSdtKind, MarsSdtQuery, MARS_SDT_ERR_BAD_ARG, MARS_SDT_ERR_BUSY,
    MARS_SDT_ERR_NO_CHECK, MARS_SDT_ERR_NO_PROBE, MARS_SDT_ERR_NO_SPACE, MARS_SDT_ERR_NULL_OUT,
    MARS_SDT_ERR_PANIC, MARS_SDT_OK,
};

/// `NET_CHECK_BASIC | NET_CHECK_LONG | NET_CHECK_SHORT` of
/// `mars/sdt/constants.h`: ping and DNS, then the HTTP check, then the TCP one.
const NET_CHECK_ALL: c_int = 1 | 2 | 4;

/// What `comm::getNetInfo()` would answer here. Nothing in a check reads it; it
/// goes into every profile of the run.
const NET_WIFI: c_int = 1;

/// Serialises the tests that share the process-wide diagnosis.
fn lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    // A poisoned mutex only means an earlier assertion failed; the diagnosis is
    // still usable, so recover instead of cascading the panic.
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// What the probe was asked, so a test can see which probes the checks used.
fn asked() -> &'static Mutex<Vec<MarsSdtKind>> {
    static ASKED: OnceLock<Mutex<Vec<MarsSdtKind>>> = OnceLock::new();
    ASKED.get_or_init(Default::default)
}

/// The address a resolve answers.
///
/// The pointer to it is read after the probe has returned, so it outlives the
/// whole test rather than the call.
fn resolved_ip() -> *const c_char {
    static IP: OnceLock<&'static CStr> = OnceLock::new();
    IP.get_or_init(|| Box::leak(CString::new("1.2.3.4").unwrap().into_boxed_c_str()))
        .as_ptr()
}

/// The addresses a resolve answers, and how many there are.
///
/// The C side reads the pointers after the probe has returned, so they outlive
/// the whole test rather than the call. They are held as integers because a
/// static that holds raw pointers is not `Sync`, which a static has to be.
fn resolved_ips() -> (*const *const c_char, c_uint) {
    static IPS: Mutex<[usize; 1]> = Mutex::new([0]);
    let mut ips = IPS.lock().unwrap_or_else(|e| e.into_inner());
    ips[0] = resolved_ip() as usize;
    (ips.as_ptr() as *const *const c_char, ips.len() as c_uint)
}

/// The network, as a caller supplies it, only slower: a probe that takes its
/// time, so that a test can cancel a run *while* it is in flight and not
/// before it starts.
///
/// [`slow_probes`] counts them, because the counter a run thread writes
/// through `ctx` is the thread's own and not the cancelling thread's to read.
extern "C" fn slow_probe(ctx: *mut c_void, query: *const MarsSdtQuery, answer: *mut MarsSdtAnswer) {
    slow_probes().fetch_add(1, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(50));
    probe(ctx, query, answer);
}

/// How many probes [`slow_probe`] has been asked.
fn slow_probes() -> &'static AtomicUsize {
    static SLOW: AtomicUsize = AtomicUsize::new(0);
    &SLOW
}

/// The network, as a caller supplies it: every probe works, and answers at once.
///
/// This is the four probes of `mars/sdt/src/checkimpl/` in one function, which
/// is the shape [`mars_ffi::sdt::MarsSdtProbe`] asks for.
extern "C" fn probe(ctx: *mut c_void, query: *const MarsSdtQuery, answer: *mut MarsSdtAnswer) {
    // SAFETY: `mars_sdt_run_checks` hands a query and an answer it owns, and
    // reads the answer only after this call returns.
    let (query, answer) = unsafe { (&*query, &mut *answer) };
    assert!(!query.host.is_null(), "every probe is asked about a host");
    asked().lock().unwrap().push(query.kind);
    // SAFETY: `ctx` is the `&mut usize` the test handed to `mars_sdt_run_checks`,
    // which outlives the run.
    unsafe {
        *(ctx as *mut usize) += 1;
    }

    *answer = match query.kind {
        MarsSdtKind::Dns => {
            let (ips, ip_count) = resolved_ips();
            MarsSdtAnswer {
                kind: MarsSdtKind::Dns,
                error_code: 0,
                rtt: 5,
                ips,
                ip_count,
                ..MarsSdtAnswer::default()
            }
        }
        MarsSdtKind::Tcp => MarsSdtAnswer {
            kind: MarsSdtKind::Tcp,
            sent: 1,
            received: 1,
            is_noop_resp: 1,
            rtt: 12,
            ..MarsSdtAnswer::default()
        },
        MarsSdtKind::Http => MarsSdtAnswer {
            kind: MarsSdtKind::Http,
            error_code: 0,
            status_code: 200,
            rtt: 30,
            ..MarsSdtAnswer::default()
        },
        MarsSdtKind::Ping => MarsSdtAnswer {
            kind: MarsSdtKind::Ping,
            error_code: 0,
            rtt: 10,
            loss_rate: 0.0,
            avgrtt: 10.0,
            ..MarsSdtAnswer::default()
        },
        // A probe is never asked for nothing: `Nothing` is what an answer nobody
        // wrote carries.
        MarsSdtKind::Nothing => MarsSdtAnswer::default(),
    };
}

/// One host per link, which is the smallest diagnosis there is.
///
/// # Safety
///
/// The `CString`s behind the hosts outlive the call, as
/// `mars_sdt_start_active_check` requires.
unsafe fn start(mode: c_int) -> c_int {
    let longlink_name = CString::new("longlink").unwrap();
    let shortlink_name = CString::new("shortlink").unwrap();
    let longlink_ports = [MarsSdtIpPort {
        ip: resolved_ip(),
        port: 80,
    }];
    let shortlink_ports = [MarsSdtIpPort {
        ip: resolved_ip(),
        port: 443,
    }];
    let longlink = [MarsSdtHosts {
        name: longlink_name.as_ptr(),
        ports: longlink_ports.as_ptr(),
        port_count: longlink_ports.len() as c_uint,
    }];
    let shortlink = [MarsSdtHosts {
        name: shortlink_name.as_ptr(),
        ports: shortlink_ports.as_ptr(),
        port_count: shortlink_ports.len() as c_uint,
    }];
    // SAFETY: both arrays and the strings they point at are alive for the call.
    unsafe {
        mars_sdt_start_active_check(
            longlink.as_ptr(),
            longlink.len() as c_uint,
            shortlink.as_ptr(),
            shortlink.len() as c_uint,
            mode,
            5_000,
        )
    }
}

#[test]
fn the_cgi_round_trips_and_a_short_buffer_is_reported() {
    let _guard = lock();
    mars_sdt_reset();

    let cgi = CString::new("https://example.com/netcheck").unwrap();
    // SAFETY: `cgi` is a valid NUL-terminated string.
    unsafe {
        mars_sdt_set_http_netcheck_cgi(cgi.as_ptr());
    }

    let mut out = [0 as c_char; 64];
    // SAFETY: a valid buffer of 64 bytes.
    let written = unsafe { mars_sdt_http_netcheck_cgi(out.as_mut_ptr(), out.len() as c_uint) };
    assert_eq!(written as usize, cgi.to_bytes().len());
    // SAFETY: the call above wrote a NUL-terminated string into `out`.
    let read = unsafe { CStr::from_ptr(out.as_ptr()) };
    assert_eq!(read.to_bytes(), cgi.to_bytes());

    // No room for the URL and its NUL, and no room because there is no buffer.
    let mut tiny = [0 as c_char; 4];
    // SAFETY: a valid buffer of four bytes.
    assert_eq!(
        unsafe { mars_sdt_http_netcheck_cgi(tiny.as_mut_ptr(), tiny.len() as c_uint) },
        MARS_SDT_ERR_NO_SPACE
    );
    // SAFETY: null is explicitly allowed by the contract.
    assert_eq!(
        unsafe { mars_sdt_http_netcheck_cgi(std::ptr::null_mut(), 0) },
        MARS_SDT_ERR_NULL_OUT
    );
}

#[test]
fn a_diagnosis_runs_over_the_probe_and_reports_what_it_found() {
    let _guard = lock();
    mars_sdt_reset();
    asked().lock().unwrap().clear();

    // SAFETY: the hosts of `start` are alive for the call.
    assert_eq!(unsafe { start(NET_CHECK_ALL) }, MARS_SDT_OK);
    assert_eq!(mars_sdt_is_checking(), 1);
    // A second request is refused while the first is in flight.
    // SAFETY: the hosts of `start` are alive for the call.
    assert_eq!(unsafe { start(NET_CHECK_ALL) }, MARS_SDT_ERR_BUSY);

    // What it is going to check, asked for its size first and then for itself.
    // SAFETY: `out` may be null when `cap` is 0.
    let count = unsafe { mars_sdt_plan(std::ptr::null_mut(), 0) };
    assert_eq!(count, 4);
    let mut plan = vec![MarsSdtCheck::Ping; count as usize];
    // SAFETY: a valid buffer of `count` checks.
    let planned = unsafe { mars_sdt_plan(plan.as_mut_ptr(), count) };
    assert_eq!(planned, count);
    assert_eq!(
        plan,
        vec![
            MarsSdtCheck::Ping,
            MarsSdtCheck::Dns,
            MarsSdtCheck::Http,
            MarsSdtCheck::Tcp,
        ]
    );

    // Run it.
    let mut probes = 0usize;
    // SAFETY: `probes` outlives the run, and `probe` answers every question with
    // strings that outlive it too.
    assert_eq!(
        unsafe {
            mars_sdt_run_checks(
                &mut probes as *mut usize as *mut c_void,
                Some(probe),
                NET_WIFI,
            )
        },
        MARS_SDT_OK
    );
    assert!(probes >= 4, "every planned check asked a probe: {probes}");
    // The run is over, so nothing is in flight any more.
    assert_eq!(mars_sdt_is_checking(), 0);

    let asked = asked().lock().unwrap().clone();
    for kind in [
        MarsSdtKind::Ping,
        MarsSdtKind::Dns,
        MarsSdtKind::Http,
        MarsSdtKind::Tcp,
    ] {
        assert!(asked.contains(&kind), "{kind:?} was never asked: {asked:?}");
    }

    // What was found, as the JSON the C++ hands to the app.
    let mut report = vec![0 as c_char; 4096];
    // SAFETY: a valid buffer of 4096 bytes.
    let written = unsafe { mars_sdt_take_report(report.as_mut_ptr(), report.len() as c_uint) };
    assert!(written > 0);
    // SAFETY: the call above wrote a NUL-terminated string into `report`.
    let json = unsafe { CStr::from_ptr(report.as_ptr()) }
        .to_str()
        .unwrap()
        .to_owned();
    // One entry per result: a ping and a resolve for each of the two links,
    // plus the HTTP and the TCP one.
    assert_eq!(json.matches("\"detectType\":").count(), 6, "{json}");
    for check in [0, 1, 3, 4] {
        assert!(
            json.contains(&format!("\"detectType\":{check}")),
            "the {check} check is in the report: {json}"
        );
    }
    // And the answers of the probes are in it: the hosts a diagnosis was
    // started with, the address the resolve answered, the HTTP status and the
    // ping's own numbers.
    for field in [
        "\"dnsDomain\":\"longlink\"",
        "\"dnsDomain\":\"shortlink\"",
        "\"dnsIP1\":\"1.2.3.4\"",
        "\"httpStatusCode\":200",
        "\"pingLossRate\":\"0.000000\"",
    ] {
        assert!(json.contains(field), "{field} is missing from {json}");
    }

    // Taking the report empties it: a take hands over everything recorded
    // since the last one, and not only the results of the run that just
    // finished.
    // SAFETY: a valid buffer of 4096 bytes.
    let written = unsafe { mars_sdt_take_report(report.as_mut_ptr(), report.len() as c_uint) };
    assert!(written > 0);
    // SAFETY: the call above wrote a NUL-terminated string into `report`.
    assert_eq!(
        unsafe { CStr::from_ptr(report.as_ptr()) }.to_str().unwrap(),
        "{\"details\":[]}"
    );
}

#[test]
fn a_cancelled_diagnosis_runs_nothing() {
    let _guard = lock();
    mars_sdt_reset();

    // SAFETY: the hosts of `start` are alive for the call.
    assert_eq!(unsafe { start(NET_CHECK_ALL) }, MARS_SDT_OK);
    mars_sdt_cancel_active_check();

    let mut probes = 0usize;
    // SAFETY: `probes` outlives the run.
    assert_eq!(
        unsafe {
            mars_sdt_run_checks(
                &mut probes as *mut usize as *mut c_void,
                Some(probe),
                NET_WIFI,
            )
        },
        MARS_SDT_ERR_NO_CHECK
    );
    assert_eq!(probes, 0, "a cancelled run asks nothing");
}

#[test]
fn a_run_with_nothing_in_flight_has_nothing_to_do() {
    let _guard = lock();
    mars_sdt_reset();

    let mut probes = 0usize;
    // SAFETY: `probes` outlives the call.
    assert_eq!(
        unsafe {
            mars_sdt_run_checks(
                &mut probes as *mut usize as *mut c_void,
                Some(probe),
                NET_WIFI,
            )
        },
        MARS_SDT_ERR_NO_CHECK
    );
}

#[test]
fn a_null_probe_is_reported() {
    let _guard = lock();
    mars_sdt_reset();

    // SAFETY: the hosts of `start` are alive for the call.
    assert_eq!(unsafe { start(NET_CHECK_ALL) }, MARS_SDT_OK);
    // SAFETY: a null probe is explicitly allowed by the contract.
    assert_eq!(
        unsafe { mars_sdt_run_checks(std::ptr::null_mut(), None, NET_WIFI) },
        MARS_SDT_ERR_NO_PROBE
    );
    // Nothing ran, and the request is not left in flight behind the run that
    // could not ask anything: `MARS_SDT_ERR_NO_PROBE` is not a code a caller
    // retries — it has no probe to retry with — so a request left `Checking`
    // would answer `MARS_SDT_ERR_BUSY` to every start after it, for the life
    // of the process.
    assert_eq!(mars_sdt_is_checking(), 0);
    // SAFETY: the hosts of `start` are alive for the call.
    assert_eq!(unsafe { start(NET_CHECK_ALL) }, MARS_SDT_OK);
}

/// `MARS_SDT_ERR_BUSY` is the one code a caller retries — "a check is already
/// in flight, ask again" — so it must not be the answer to arguments that can
/// never start a check at all. Retrying those never ends.
#[test]
fn a_request_that_can_never_start_is_not_reported_as_busy() {
    let _guard = lock();
    mars_sdt_reset();

    // A count that promises hosts behind a null pointer: `hosts_from_c` would
    // read nothing and the request would start with a link it has no hosts
    // for.
    // SAFETY: a null array with a non-zero count is what the contract forbids,
    // and the call answers instead of reading through the pointer.
    assert_eq!(
        unsafe {
            mars_sdt_start_active_check(
                std::ptr::null(),
                1,
                std::ptr::null(),
                1,
                NET_CHECK_ALL,
                5_000,
            )
        },
        MARS_SDT_ERR_BAD_ARG
    );
    // A mode that asks for no check: three bits are the whole vocabulary, and
    // a request with an empty plan runs nothing and reports nothing.
    // SAFETY: the hosts of `start` are alive for the call.
    assert_eq!(unsafe { start(0) }, MARS_SDT_ERR_BAD_ARG);
    assert_eq!(
        unsafe { start(8) },
        MARS_SDT_ERR_BAD_ARG,
        "a bit nobody named"
    );

    // None of them started a request, so there is nothing in flight.
    assert_eq!(mars_sdt_is_checking(), 0);

    // A real one still starts, and is still refused with BUSY while it is in
    // flight — which is the answer a caller retries, and the only one.
    // SAFETY: the hosts of `start` are alive for the call.
    assert_eq!(unsafe { start(NET_CHECK_ALL) }, MARS_SDT_OK);
    assert_eq!(unsafe { start(NET_CHECK_ALL) }, MARS_SDT_ERR_BUSY);

    mars_sdt_reset();
}

/// A run that is cancelled *while it is asking probes* stops early.
///
/// This is the whole reason the cancellation flag does not live behind the
/// process-wide diagnosis: `mars_sdt_run_checks` holds it from the first probe
/// to the last, so a flag reached through that lock can only be set before a
/// run starts or after it has finished and reset itself — never while it is
/// in flight, which is the only time cancelling means anything.
#[test]
fn a_run_in_flight_can_be_cancelled() {
    let _guard = lock();

    // How many probes a whole run asks, to have something to compare against.
    mars_sdt_reset();
    // SAFETY: the hosts of `start` are alive for the call.
    assert_eq!(unsafe { start(NET_CHECK_ALL) }, MARS_SDT_OK);
    slow_probes().store(0, Ordering::SeqCst);
    let whole = Box::leak(Box::new(0usize));
    // SAFETY: `whole` outlives the run, and `slow_probe` answers with strings
    // that outlive it too.
    assert_eq!(
        unsafe {
            mars_sdt_run_checks(
                whole as *mut usize as *mut c_void,
                Some(slow_probe),
                NET_WIFI,
            )
        },
        MARS_SDT_OK
    );
    let whole = slow_probes().load(Ordering::SeqCst);
    assert!(whole >= 4, "a whole run asks every check: {whole}");

    // The same run again, cancelled from this thread while it is going.
    mars_sdt_reset();
    // SAFETY: the hosts of `start` are alive for the call.
    assert_eq!(unsafe { start(NET_CHECK_ALL) }, MARS_SDT_OK);
    slow_probes().store(0, Ordering::SeqCst);
    let counted = Box::leak(Box::new(0usize));
    let ctx = counted as *mut usize as usize;
    let run = thread::spawn(move || {
        // SAFETY: `counted` outlives the run — it is leaked — and `slow_probe`
        // answers with strings that outlive it too.
        unsafe { mars_sdt_run_checks(ctx as *mut c_void, Some(slow_probe), NET_WIFI) }
    });
    while slow_probes().load(Ordering::SeqCst) == 0 {
        thread::sleep(Duration::from_millis(1));
    }
    mars_sdt_cancel_active_check();
    let code = run.join().expect("the run thread did not panic");
    let asked = slow_probes().load(Ordering::SeqCst);

    assert_ne!(code, MARS_SDT_ERR_PANIC);
    assert!(
        asked < whole,
        "cancelling stopped the run: {asked} probes of {whole}"
    );
}

/// A report that did not fit is not taken: the retry a caller makes with a
/// bigger buffer gets the diagnosis, and not an empty one.
#[test]
fn a_report_that_does_not_fit_is_kept() {
    let _guard = lock();
    mars_sdt_reset();

    // SAFETY: the hosts of `start` are alive for the call.
    assert_eq!(unsafe { start(NET_CHECK_ALL) }, MARS_SDT_OK);
    let mut probes = 0usize;
    // SAFETY: `probes` outlives the run.
    assert_eq!(
        unsafe {
            mars_sdt_run_checks(
                &mut probes as *mut usize as *mut c_void,
                Some(probe),
                NET_WIFI,
            )
        },
        MARS_SDT_OK
    );

    // Eight bytes is not a report, and the call says so instead of handing
    // over nothing and calling it taken.
    let mut small = vec![0 as c_char; 8];
    // SAFETY: a valid buffer of eight bytes.
    assert_eq!(
        unsafe { mars_sdt_take_report(small.as_mut_ptr(), small.len() as c_uint) },
        MARS_SDT_ERR_NO_SPACE
    );

    // So the retry with room gets the whole thing.
    let mut report = vec![0 as c_char; 4096];
    // SAFETY: a valid buffer of 4096 bytes.
    let written = unsafe { mars_sdt_take_report(report.as_mut_ptr(), report.len() as c_uint) };
    assert!(written > 0);
    // SAFETY: the call above wrote a NUL-terminated string into `report`.
    let json = unsafe { CStr::from_ptr(report.as_ptr()) }
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(json.matches("\"detectType\":").count(), 6, "{json}");
}

#[test]
fn a_null_report_buffer_is_reported() {
    let _guard = lock();
    mars_sdt_reset();

    // SAFETY: null is explicitly allowed by the contract.
    assert_eq!(
        unsafe { mars_sdt_take_report(std::ptr::null_mut(), 0) },
        MARS_SDT_ERR_NULL_OUT
    );
}
