//! `mars_sdt_*` — the C ABI of the network diagnosis (`mars/sdt`).
//!
//! This is the seam `com/tencent/mars/sdt/SdtLogic.java` has in the C++
//! (`mars/sdt/jni/*_Java2C.cc`), and the same one [`crate::abi`] is for xlog:
//! the calls of [`marsrs_sdt::SdtLogic`], with C types on the outside, so that a
//! Swift or C++ caller can start a diagnosis without a Rust toolchain.
//!
//! The one thing a caller has to supply is the network: DNS, TCP, HTTP and ping
//! are four probes the port refuses to open for itself (`mars/comm/socket` and
//! `mars/comm/dns` are the platform's), so they cross this boundary as one
//! function pointer — [`MarsSdtProbe`] — the way [`marsrs_sdt::checkimpl::Ask`]
//! hands them to the checkers. A caller that answers nothing gets the diagnosis
//! a host with no network gets: the check that asked fails, and a failed
//! check ends the run. A ping is the one exception — a ping nobody sent is
//! a check that did not run.
//!
//! What a run finds is handed back as the JSON
//! `SdtLogic.reportSignalDetectResults(String)` gets in the C++ — the document
//! [`marsrs_sdt::report_json`] builds, which is also what the JNI seam hands
//! over. Every field of it is [`CheckResultProfile`]'s, so the report crosses
//! as one string instead of as a struct the caller would have to mirror.
//!
//! Panics: no Rust panic ever crosses this boundary. Every entry point is
//! wrapped in `catch_unwind` and reports [`MARS_SDT_ERR_PANIC`] (or swallows it,
//! for the `void` symbols).

use std::ffi::{c_char, c_int, c_uint, c_void, CString};
use std::ptr::{addr_of, addr_of_mut};
use std::sync::{Arc, Mutex, OnceLock};

use marsrs_sdt::checkimpl::{Answer, Ask, PingStatus, Query};
use marsrs_sdt::netchecker_profile::CheckResultProfile;
use marsrs_sdt::sdt_core::CancelHandle;
use marsrs_sdt::{
    report_json, Callback, CheckIPPort, CheckIPPorts, NetCheckType, SdtLogic, NET_CHECK_BASIC,
    NET_CHECK_LONG, NET_CHECK_SHORT,
};

use crate::cstr;
use crate::guard;

/// `MARS_SDT_OK` — the symbol did what it was asked.
pub const MARS_SDT_OK: c_int = 0;
/// A panic was caught: nothing crossed into C, and the state is whatever the
/// panic left behind.
pub const MARS_SDT_ERR_PANIC: c_int = -1;
/// The `out` buffer is null.
pub const MARS_SDT_ERR_NULL_OUT: c_int = -2;
/// The `out` buffer has no room for the answer and its terminating NUL —
/// including `len == 0`.
pub const MARS_SDT_ERR_NO_SPACE: c_int = -3;
/// `mars_sdt_run_checks` was called without a probe, and there is no network to
/// run a diagnosis with.
pub const MARS_SDT_ERR_NO_PROBE: c_int = -4;
/// A check is already in flight, which is what `StartActiveCheck` answers
/// `false` for.
pub const MARS_SDT_ERR_BUSY: c_int = -5;
/// There is no check in flight, so there is nothing to run.
pub const MARS_SDT_ERR_NO_CHECK: c_int = -6;
/// What the caller handed [`mars_sdt_start_active_check`] cannot start a
/// check at all: a hosts array that promises `count` hosts behind a null
/// pointer, or a `mode` with none of the three `NET_CHECK_*` bits in it, so a
/// request with no check to make.
///
/// This is deliberately *not* [`MARS_SDT_ERR_BUSY`], which is the one code a
/// caller retries on — "a check is already in flight, ask again". Nothing
/// about this one changes with time: the same arguments are refused the same
/// way however often they are tried, so a caller that retried it would spin.
pub const MARS_SDT_ERR_BAD_ARG: c_int = -7;

/// Which probe is being asked, and which answer came back: the four of
/// `mars/sdt/src/checkimpl/`, plus `Nothing` for a probe nobody answered.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarsSdtKind {
    /// Nobody answered — a host with no network to probe with, which the
    /// check that asked reads as a failure, and a failed check ends the
    /// run. A ping is the one exception: a ping nobody sent is a check that
    /// did
    /// not run, and the plan goes on behind it.
    Nothing = 0,
    /// `socket_gethostbyname` — the resolve of one host name.
    Dns = 1,
    /// `TcpQuery` — a long-link noop out, and whatever comes back.
    Tcp = 2,
    /// `SendHttpQuery` — the request that tells the net-check CGI somebody is
    /// looking for it.
    Http = 3,
    /// `PingQuery::RunPingQuery`.
    Ping = 4,
}

impl MarsSdtKind {
    /// The integer the caller left in `kind`, as the probe it names —
    /// [`MarsSdtKind::Nothing`] for one no variant has, which is what a probe
    /// nobody answered is read as anyway.
    ///
    /// `kind` is the caller's to fill in, so what it holds is whatever that
    /// caller left there, and a number no variant has is a value the enum
    /// cannot hold: reading one is undefined behaviour of its own, before any
    /// match on it runs. So the field is read as the `i32` it is, here, and
    /// what leaves is a variant.
    pub fn of(raw: i32) -> Self {
        match raw {
            1 => Self::Dns,
            2 => Self::Tcp,
            3 => Self::Http,
            4 => Self::Ping,
            _ => Self::Nothing,
        }
    }
}

/// What one probe is asked: [`MarsSdtKind`], and the arguments the C++ hands
/// that probe. `host` is the domain for a resolve, the ip for a noop, the URL
/// for the HTTP request and the host for a ping; `timeout` is milliseconds for
/// the first three and seconds for a ping, which is what `RunPingQuery` takes.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MarsSdtQuery {
    /// Which probe.
    pub kind: MarsSdtKind,
    /// The host, as a NUL-terminated string owned by the caller.
    pub host: *const c_char,
    /// `TcpQuery`'s port; `0` for the other three.
    pub port: u16,
    /// Milliseconds, or seconds for a ping.
    pub timeout: u32,
}

/// What one probe answers. Every field is read for the [`MarsSdtKind`] in
/// `kind` and ignored for the others, so a caller fills in the one it answered
/// and leaves the rest zero: `ips` / `ip_count` are the resolve, `sent` /
/// `received` / `is_noop_resp` the noop round trip, `status_code` the HTTP
/// request and `loss_rate` / `avgrtt` the ping. `rtt` is what the C++ calls
/// `cost_time`, and every answer but `Nothing` carries it.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MarsSdtAnswer {
    /// Which probe this is the answer of; `Nothing` when nobody answered.
    pub kind: MarsSdtKind,
    /// `error_code` — the return value of the probe: `0` and above is a resolve
    /// or a request that worked, below `0` one that did not.
    pub error_code: i32,
    /// `cost_time` — how long the probe took.
    pub rtt: u64,
    /// The addresses a resolve found, in the order they came. Owned by the
    /// caller, and read only while the probe's answer is being taken.
    pub ips: *const *const c_char,
    /// How many addresses `ips` holds.
    pub ip_count: c_uint,
    /// `tcp_send` — `0` and above is a noop that went out.
    pub sent: i32,
    /// `tcp_receive` — `0` and above is an answer that came back.
    pub received: i32,
    /// Whether what came back was the answer to the noop; `0` or `1`.
    pub is_noop_resp: u8,
    /// The HTTP status of the answer to the net-check CGI.
    pub status_code: i32,
    /// `PingStatus::loss_rate` — `1.0` is every ping of the run lost.
    pub loss_rate: f32,
    /// `PingStatus::avgrtt` — what `profile.rtt_str` is made of.
    pub avgrtt: f32,
}

impl Default for MarsSdtAnswer {
    /// A probe nobody answered.
    fn default() -> Self {
        Self {
            kind: MarsSdtKind::Nothing,
            error_code: 0,
            rtt: 0,
            ips: std::ptr::null(),
            ip_count: 0,
            sent: 0,
            received: 0,
            is_noop_resp: 0,
            status_code: 0,
            loss_rate: 0.0,
            avgrtt: 0.0,
        }
    }
}

/// One check of a request: `NetCheckType` of `mars/sdt/sdt.h`, which is also the
/// `detectType` of every entry of the report, so a plan and a report name the
/// same checks with the same integers.
///
/// The four that ask a probe carry the same names as four of
/// [`MarsSdtKind`] — and *not* the same integers: this is what the request is
/// made of, which is `netcheck_type`'s spelling, while that one is what a probe
/// is asked, which is the probe's.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarsSdtCheck {
    /// `kPingCheck`.
    Ping = 0,
    /// `kDnsCheck`.
    Dns = 1,
    /// `kNewDnsCheck` — the resolve against another server, to compare with.
    NewDns = 2,
    /// `kTcpCheck`.
    Tcp = 3,
    /// `kHttpCheck`.
    Http = 4,
    /// `kTracerouteCheck` — planned, but no probe is asked for it yet.
    Traceroute = 5,
    /// `kReqBufCheck` — planned, but no probe is asked for it yet.
    ReqBuf = 6,
}

impl MarsSdtCheck {
    /// The check a plan entry is.
    fn of(check: NetCheckType) -> Self {
        match check {
            NetCheckType::PingCheck => Self::Ping,
            NetCheckType::DnsCheck => Self::Dns,
            NetCheckType::NewDnsCheck => Self::NewDns,
            NetCheckType::TcpCheck => Self::Tcp,
            NetCheckType::HttpCheck => Self::Http,
            NetCheckType::TracerouteCheck => Self::Traceroute,
            NetCheckType::ReqBufCheck => Self::ReqBuf,
        }
    }
}

/// One host and port of a link: `CheckIPPort` of `mars/sdt/sdt.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MarsSdtIpPort {
    /// The host, as an IP or a name.
    pub ip: *const c_char,
    pub port: u16,
}

/// The hosts of one link, keyed by the name they are known under — one entry of
/// the `CheckIPPorts` map a diagnosis is started with.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MarsSdtHosts {
    /// The name the host is known under.
    pub name: *const c_char,
    /// `port_count` host and port pairs.
    pub ports: *const MarsSdtIpPort,
    /// How many pairs `ports` holds.
    pub port_count: c_uint,
}

/// The four probes, asked the way [`marsrs_sdt::checkimpl::Ask`] asks them: one
/// question in, one answer out, and `ctx` — the pointer the caller handed to
/// [`mars_sdt_run_checks`] — handed back with every one.
pub type MarsSdtProbe = Option<extern "C" fn(*mut c_void, *const MarsSdtQuery, *mut MarsSdtAnswer)>;

/// The diagnosis of the process: the counterpart of the `SdtLogic` singleton the
/// C++'s Java2C calls reach, and of the `LOGIC` `marsrs-jni` keeps.
struct SdtState {
    logic: SdtLogic,
    reported: Arc<Mutex<Vec<CheckResultProfile>>>,
}

/// Where the results a run reported wait for [`mars_sdt_take_report`]: the
/// callback runs inside the run, which holds the state lock, so it records into
/// its own value instead of taking that lock again.
struct Sink(Arc<Mutex<Vec<CheckResultProfile>>>);

impl Callback for Sink {
    fn report_net_check_result(&self, check_results: &[CheckResultProfile]) {
        if let Ok(mut reported) = self.0.lock() {
            reported.extend_from_slice(check_results);
        }
    }
}

/// A diagnosis nobody has touched yet, with the callback that records what it
/// finds already installed.
fn new_state() -> SdtState {
    let reported = Arc::new(Mutex::new(Vec::new()));
    let mut logic = SdtLogic::new();
    logic.set_callback(Sink(Arc::clone(&reported)));
    // A new diagnosis is a new core, and a new cancellation flag with it: this
    // is the one [`mars_sdt_cancel_active_check`] sets, and it has to be the
    // one the logic the state holds answers to.
    *cancel()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = logic.cancel_handle();
    SdtState { logic, reported }
}

/// The cancellation flag of the request in flight, kept *outside* [`state()`].
///
/// That is the whole point: [`mars_sdt_run_checks`] holds the process-wide
/// diagnosis for as long as the checks take — which is exactly when a caller
/// wants to cancel — so a flag that had to be reached through that lock could
/// only ever be set before a run started or after it had already finished and
/// reset itself. [`marsrs_sdt::CancelHandle`] is shared for the same reason
/// inside the diagnosis, and this is the copy the boundary keeps of it.
fn cancel() -> &'static Mutex<CancelHandle> {
    static CANCEL: OnceLock<Mutex<CancelHandle>> = OnceLock::new();
    // Its own handle, and not [`new_state`]'s: that function is what writes
    // this slot, so reaching for it here would ask for the slot being filled.
    // Every state that follows overwrites it with the handle of the core it
    // holds, which is the one a run of that core reads.
    CANCEL.get_or_init(|| Mutex::new(CancelHandle::new()))
}

fn state() -> &'static Mutex<SdtState> {
    static STATE: OnceLock<Mutex<SdtState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(new_state()))
}

/// Runs `f` on the process-wide diagnosis. A poisoned lock keeps the state a
/// panic left behind rather than resetting it, which is what the C++ would
/// leave.
fn with_state<R>(f: impl FnOnce(&mut SdtState) -> R) -> R {
    let mut state = state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut state)
}

/// Throws the diagnosis away and starts over: the request, the check that may be
/// in flight and everything that was reported go away together, which is what
/// `SdtLogic::Reset` does.
#[no_mangle]
pub extern "C" fn mars_sdt_reset() {
    guard((), || with_state(|state| *state = new_state()));
}

/// `SdtLogic.setHttpNetcheckCGI` — the URL the HTTP check goes to.
///
/// # Safety
///
/// `cgi` must either be null or point to a valid NUL-terminated string that is
/// not mutated while the call runs.
#[no_mangle]
pub unsafe extern "C" fn mars_sdt_set_http_netcheck_cgi(cgi: *const c_char) {
    guard((), || {
        // SAFETY: forwarded to `ptr_to_str_or_empty`, whose contract the caller
        // upholds (null or a valid NUL-terminated string).
        let cgi = unsafe { cstr::ptr_to_str_or_empty(cgi) };
        with_state(|state| state.logic.set_http_netcheck_cgi(cgi));
    })
}

/// The URL [`mars_sdt_set_http_netcheck_cgi`] set, copied into `out`.
///
/// @return the number of bytes written excluding the terminating NUL, or
/// [`MARS_SDT_ERR_PANIC`], [`MARS_SDT_ERR_NULL_OUT`] or
/// [`MARS_SDT_ERR_NO_SPACE`] (including `len == 0`).
///
/// # Safety
///
/// `out` must be null, or point to at least `len` writable bytes that stay alive
/// for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn mars_sdt_http_netcheck_cgi(out: *mut c_char, len: c_uint) -> c_int {
    guard(MARS_SDT_ERR_PANIC, || {
        let cgi = with_state(|state| state.logic.http_netcheck_cgi().to_owned());
        // SAFETY: forwarded to `write_str_into`, whose contract the caller
        // upholds (null or `len` writable bytes).
        unsafe { write_str_into(cgi.as_bytes(), out, len) }
    })
}

/// `StartActiveCheck` — a diagnosis of the two links' hosts, in `mode` and with
/// `timeout` milliseconds to spend on it.
///
/// @return [`MARS_SDT_OK`], or [`MARS_SDT_ERR_BUSY`] when a check is already in
/// flight — the one answer worth retrying — or [`MARS_SDT_ERR_BAD_ARG`] when
/// the arguments cannot start a check at all, or [`MARS_SDT_ERR_PANIC`].
///
/// # Safety
///
/// `longlink` and `shortlink` must each either be null or point to
/// `longlink_count` / `shortlink_count` initialised [`MarsSdtHosts`] whose
/// strings and port arrays stay alive for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn mars_sdt_start_active_check(
    longlink: *const MarsSdtHosts,
    longlink_count: c_uint,
    shortlink: *const MarsSdtHosts,
    shortlink_count: c_uint,
    mode: c_int,
    timeout: c_uint,
) -> c_int {
    guard(MARS_SDT_ERR_PANIC, || {
        // A count that promises hosts the pointer cannot deliver. A caller
        // that gets [`MARS_SDT_ERR_BUSY`] here retries, and retrying this
        // would never stop: the request is the same every time.
        if (longlink.is_null() && longlink_count > 0)
            || (shortlink.is_null() && shortlink_count > 0)
        {
            return MARS_SDT_ERR_BAD_ARG;
        }
        // A mode with none of the three `NET_CHECK_*` bits in it is a request
        // with an empty plan: it runs nothing and reports nothing, which is
        // not what a caller that asked for a diagnosis meant.
        if mode & (NET_CHECK_BASIC | NET_CHECK_LONG | NET_CHECK_SHORT) == 0 {
            return MARS_SDT_ERR_BAD_ARG;
        }
        // SAFETY: forwarded to `hosts_from_c`, whose contract the caller
        // upholds for both links.
        let (longlink_items, shortlink_items) = unsafe {
            (
                hosts_from_c(longlink, longlink_count),
                hosts_from_c(shortlink, shortlink_count),
            )
        };
        let started = with_state(|state| {
            state
                .logic
                .start_active_check(&longlink_items, &shortlink_items, mode, timeout)
        });
        if started {
            MARS_SDT_OK
        } else {
            MARS_SDT_ERR_BUSY
        }
    })
}

/// `CancelActiveCheck` — the check in flight is asked to stop.
///
/// It stops a run that is *in* flight, not one that has not started: the flag
/// this sets is the one [`mars_sdt_run_checks`]' own read, and it is reached
/// without the lock that run holds, so a caller may cancel from another thread
/// while the probes are still being asked.
#[no_mangle]
pub extern "C" fn mars_sdt_cancel_active_check() {
    guard((), || {
        cancel()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancel()
    });
}

/// Whether a check is in flight.
///
/// @return `1` or `0`.
#[no_mangle]
pub extern "C" fn mars_sdt_is_checking() -> c_int {
    guard(0, || with_state(|state| state.logic.is_checking()) as c_int)
}

/// The checks the request is going to make, in order: as many [`MarsSdtCheck`]s
/// as there are checks, written into `out`.
///
/// @return how many checks there are, whether or not they all fit: `out` may be
/// null and `cap` may be `0`, which is how a caller asks for the size first.
///
/// # Safety
///
/// `out` must be null, or point to at least `cap` writable [`MarsSdtCheck`]s
/// that stay alive for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn mars_sdt_plan(out: *mut MarsSdtCheck, cap: c_uint) -> c_uint {
    guard(0, || {
        let plan = with_state(|state| {
            state
                .logic
                .plan()
                .iter()
                .map(|check| MarsSdtCheck::of(*check))
                .collect::<Vec<_>>()
        });
        if out.is_null() || cap == 0 {
            return plan.len() as c_uint;
        }
        let written = plan.len().min(cap as usize);
        // SAFETY: `out` is non-null and the caller promises `cap` writable
        // checks; `written <= cap` and `written <= plan.len()`.
        unsafe {
            std::ptr::copy_nonoverlapping(plan.as_ptr(), out, written);
        }
        plan.len() as c_uint
    })
}

/// Runs the planned checks — one probe per check, in order — and reports what
/// they recorded. This is the `__RunOn` thread of the C++, driven by the caller:
/// the port has no sockets of its own, so `probe` is the network.
///
/// `network_type` is the `comm::getNetInfo()` every check writes into its
/// profiles, which is the platform's to answer — on Android it is
/// `PlatformComm.getNetInfo`, on iOS the caller's own.
///
/// @return [`MARS_SDT_OK`], or [`MARS_SDT_ERR_NO_PROBE`],
/// [`MARS_SDT_ERR_NO_CHECK`] (nothing was in flight, so nothing ran) or
/// [`MARS_SDT_ERR_PANIC`].
///
/// # Safety
///
/// `probe` must be a valid function pointer, or null — and `ctx` must be
/// whatever that function expects, alive for the whole run, which is as long as
/// this call. The strings and address arrays of the answers it writes are read
/// before it is asked again, and never afterwards.
///
/// The run holds the process-wide diagnosis, so a probe must not call another
/// `mars_sdt_*` while it is answering: it would wait for the lock this call is
/// holding. Everything a probe needs is in the query it is given.
#[no_mangle]
pub unsafe extern "C" fn mars_sdt_run_checks(
    ctx: *mut c_void,
    probe: MarsSdtProbe,
    network_type: c_int,
) -> c_int {
    guard(MARS_SDT_ERR_PANIC, || {
        let Some(probe) = probe else {
            return MARS_SDT_ERR_NO_PROBE;
        };
        let probe = Probe { probe, ctx };
        let mut ask = Ask::new(move |query| probe.ask(query));
        let results = with_state(|state| state.logic.run_checks(&mut ask, network_type));
        if results.is_empty() {
            MARS_SDT_ERR_NO_CHECK
        } else {
            MARS_SDT_OK
        }
    })
}

/// Takes the JSON report of everything the checks have reported since the last
/// call — the document `SdtLogic.reportSignalDetectResults(String)` gets in the
/// C++, built by [`marsrs_sdt::report_json`].
///
/// A report that did not fit is *not* taken: on [`MARS_SDT_ERR_NO_SPACE`] (or
/// [`MARS_SDT_ERR_NULL_OUT`]) it stays where it was and the next call hands it
/// over again, which is what makes asking again with a bigger buffer work.
///
/// @return the number of bytes written excluding the terminating NUL, or
/// [`MARS_SDT_ERR_PANIC`], [`MARS_SDT_ERR_NULL_OUT`] or
/// [`MARS_SDT_ERR_NO_SPACE`].
///
/// # Safety
///
/// `out` must be null, or point to at least `len` writable bytes that stay alive
/// for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn mars_sdt_take_report(out: *mut c_char, len: c_uint) -> c_int {
    guard(MARS_SDT_ERR_PANIC, || {
        let results = with_state(|state| {
            state
                .reported
                .lock()
                .map(|mut reported| std::mem::take(&mut *reported))
                .unwrap_or_default()
        });
        let json = report_json(&results);
        // SAFETY: forwarded to `write_str_into`, whose contract the caller
        // upholds (null or `len` writable bytes).
        let written = unsafe { write_str_into(json.as_bytes(), out, len) };
        // Taking the report is what makes the next call report what happened
        // since — but only once it has been handed over. A report that did not
        // fit (or had nowhere to go) is put back whole, at the front: the
        // caller's next call is the retry it is asking for with a bigger
        // buffer, and a diagnosis that is lost because the first buffer was
        // 4 KB is one no retry can get back.
        if written < 0 && !results.is_empty() {
            with_state(|state| {
                if let Ok(mut reported) = state.reported.lock() {
                    let later = std::mem::take(&mut *reported);
                    *reported = results;
                    reported.extend(later);
                }
            });
        }
        written
    })
}

/// The caller's four probes, as [`marsrs_sdt::checkimpl::Ask`] wants them: a
/// function pointer and the `ctx` that goes with it.
///
/// `ctx` is kept as the pointer the caller handed over and not as the integer
/// it would round-trip through: an address turned into a `usize` and cast back
/// carries no provenance, and the code that reads through it is the caller's
/// C, on whatever thread a check asks it from. The single crossing is the one
/// [`mars_sdt_run_checks`] makes, from the caller's `void*` to this field.
#[derive(Clone, Copy)]
struct Probe {
    probe: extern "C" fn(*mut c_void, *const MarsSdtQuery, *mut MarsSdtAnswer),
    ctx: *mut c_void,
}

// SAFETY: `Ask::new` asks for `Send` because a diagnosis may be run from any
// thread, and a raw pointer is not one. What crosses is an address: this crate
// never dereferences `ctx`, it only hands it back to the C probe it came from,
// and the contract `mars_sdt_run_checks` states is that `ctx` stays alive for
// the whole run — whichever thread the run is on.
unsafe impl Send for Probe {}

impl Probe {
    /// One probe: the question out, the answer back.
    fn ask(&self, query: Query) -> Answer {
        let (host, kind, port, timeout) = match &query {
            Query::Dns { domain, timeout_ms } => {
                (domain.as_str(), MarsSdtKind::Dns, 0, *timeout_ms)
            }
            Query::Tcp {
                ip,
                port,
                timeout_ms,
            } => (ip.as_str(), MarsSdtKind::Tcp, *port, *timeout_ms),
            Query::Http { url, timeout_ms } => (url.as_str(), MarsSdtKind::Http, 0, *timeout_ms),
            Query::Ping { host, timeout_s } => (host.as_str(), MarsSdtKind::Ping, 0, *timeout_s),
        };
        // A NUL in a host name is not a host name the C side can read, so it
        // becomes the empty one — the way the rest of this crate degrades.
        let host = CString::new(host).unwrap_or_default();
        let query = MarsSdtQuery {
            kind,
            host: host.as_ptr(),
            port,
            timeout,
        };
        let mut answer = MarsSdtAnswer::default();
        // The call is not `unsafe` — `probe` is a plain `fn` pointer — but what
        // it is handed is: `ctx` is the pointer the caller gave
        // `mars_sdt_run_checks`, alive for the whole run by that contract, and
        // the query and answer it reads and writes are locals that outlive it.
        (self.probe)(self.ctx, addr_of!(query), addr_of_mut!(answer));
        // What the caller left in `kind` is an `i32`, and not necessarily one
        // of the numbers that are variants — a caller that writes anything
        // else, or nothing at all, has written a value the enum cannot hold.
        //
        // SAFETY: `MarsSdtKind` is a fieldless `#[repr(i32)]` enum, so the
        // field is four bytes holding that integer, and reading them as one
        // yields no value that type cannot hold. Reading the field as the
        // enum would, and that is undefined behaviour before the first match.
        let raw = unsafe { addr_of!(answer.kind).cast::<i32>().read() };
        answer.kind = MarsSdtKind::of(raw);
        answer_from_c(&answer)
    }
}

/// The answer the caller wrote, as the [`Answer`] a checker reads.
fn answer_from_c(answer: &MarsSdtAnswer) -> Answer {
    match answer.kind {
        MarsSdtKind::Nothing => Answer::Nothing,
        MarsSdtKind::Dns => {
            let mut ips = Vec::with_capacity(answer.ip_count as usize);
            if !answer.ips.is_null() {
                for index in 0..answer.ip_count as usize {
                    // SAFETY: `ips` is non-null and the caller promises
                    // `ip_count` readable pointers, alive for this call.
                    let ip = unsafe { *answer.ips.add(index) };
                    // SAFETY: forwarded to `ptr_to_str_or_empty`.
                    ips.push(unsafe { cstr::ptr_to_str_or_empty(ip) }.to_owned());
                }
            }
            Answer::Dns {
                error_code: answer.error_code,
                rtt: answer.rtt,
                // `MarsSdtAnswer` carries no resolver and no connect time,
                // so a host on this seam reports neither: the profile keeps
                // what it started with.
                local_dns: String::new(),
                ips,
            }
        }
        MarsSdtKind::Tcp => Answer::Tcp {
            sent: answer.sent,
            received: answer.received,
            is_noop_resp: answer.is_noop_resp != 0,
            conntime: 0,
            rtt: answer.rtt,
        },
        MarsSdtKind::Http => Answer::Http {
            error_code: answer.error_code,
            status_code: answer.status_code,
            rtt: answer.rtt,
        },
        MarsSdtKind::Ping => Answer::Ping {
            error_code: answer.error_code,
            rtt: answer.rtt,
            status: Some(PingStatus::new(answer.loss_rate, answer.avgrtt)),
        },
    }
}

/// `count` [`MarsSdtHosts`] as the [`CheckIPPorts`] a diagnosis is started with:
/// one map entry per host, keyed by the name it is known under, which is what
/// the C++'s `std::map` is.
///
/// # Safety
///
/// `hosts` must either be null or point to `count` initialised [`MarsSdtHosts`]
/// whose `name`, `ports` and `ports[..].ip` stay alive for the duration of the
/// call.
unsafe fn hosts_from_c(hosts: *const MarsSdtHosts, count: c_uint) -> CheckIPPorts {
    let mut items = CheckIPPorts::new();
    if hosts.is_null() {
        return items;
    }
    for index in 0..count as usize {
        // SAFETY: `hosts` is non-null and the caller promises `count`
        // initialised entries.
        let host = unsafe { &*hosts.add(index) };
        // SAFETY: `host.name` is null or a valid NUL-terminated string, per the
        // caller's contract.
        let name = unsafe { cstr::ptr_to_str_or_empty(host.name) };
        let mut ports = Vec::with_capacity(host.port_count as usize);
        if !host.ports.is_null() {
            for port_index in 0..host.port_count as usize {
                // SAFETY: `host.ports` is non-null and the caller promises
                // `port_count` initialised pairs.
                let port = unsafe { &*host.ports.add(port_index) };
                // SAFETY: `port.ip` is null or a valid NUL-terminated string.
                let ip = unsafe { cstr::ptr_to_str_or_empty(port.ip) };
                ports.push(CheckIPPort::new(ip, port.port));
            }
        }
        items.insert(name.to_owned(), ports);
    }
    items
}

/// Copies `bytes` plus a terminating NUL into the caller's buffer.
///
/// # Safety
///
/// `out` must be null, or point to at least `len` writable bytes.
unsafe fn write_str_into(bytes: &[u8], out: *mut c_char, len: c_uint) -> c_int {
    if out.is_null() {
        return MARS_SDT_ERR_NULL_OUT;
    }
    if len == 0 {
        return MARS_SDT_ERR_NO_SPACE;
    }
    // `+ 1` for the terminating NUL.
    if bytes.len() + 1 > len as usize {
        return MARS_SDT_ERR_NO_SPACE;
    }
    // SAFETY: `out` is non-null, the caller promises `len` writable bytes, and
    // `bytes.len() + 1 <= len` was just checked.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, bytes.len());
        *out.add(bytes.len()) = 0;
    }
    bytes.len() as c_int
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn a_query_round_trips_as_the_kind_the_checker_asked() {
        let query = Query::Dns {
            domain: "example.com".to_owned(),
            timeout_ms: 3_000,
        };
        let (_, kind, port, timeout) = match &query {
            Query::Dns { domain, timeout_ms } => {
                (domain.as_str(), MarsSdtKind::Dns, 0, *timeout_ms)
            }
            _ => unreachable!(),
        };
        assert_eq!(kind, MarsSdtKind::Dns);
        assert_eq!(port, 0);
        assert_eq!(timeout, 3_000);
    }

    #[test]
    fn an_answer_nobody_wrote_is_nothing() {
        assert_eq!(answer_from_c(&MarsSdtAnswer::default()), Answer::Nothing);
    }

    /// A probe that fills `kind` in with a number of its own, the way a C
    /// caller writing the field itself would.
    extern "C" fn probe_of_raw(
        _ctx: *mut c_void,
        _query: *const MarsSdtQuery,
        answer: *mut MarsSdtAnswer,
    ) {
        // SAFETY: `answer` is the caller's, alive for this call.
        let answer = unsafe { &mut *answer };
        answer.rtt = 7;
        // SAFETY: `MarsSdtKind` is a fieldless `#[repr(i32)]` enum, so the
        // field is an `i32` and writing one leaves that integer in it.
        unsafe {
            addr_of_mut!(answer.kind).cast::<i32>().write(99);
        }
    }

    #[test]
    fn a_kind_no_variant_has_is_a_probe_nobody_answered() {
        let probe = Probe {
            probe: probe_of_raw,
            ctx: std::ptr::null_mut(),
        };
        let answer = probe.ask(Query::Dns {
            domain: "example.com".to_owned(),
            timeout_ms: 1,
        });
        // The `rtt` of `7` is in the answer the caller wrote, and `Nothing`
        // is what a probe that names no probe is read as.
        assert_eq!(answer, Answer::Nothing);
    }

    #[test]
    fn an_answer_reads_the_fields_of_its_kind() {
        let ip = CString::new("1.2.3.4").unwrap();
        let ips = [ip.as_ptr()];
        let answer = MarsSdtAnswer {
            kind: MarsSdtKind::Dns,
            error_code: 0,
            rtt: 7,
            ips: ips.as_ptr(),
            ip_count: ips.len() as c_uint,
            ..MarsSdtAnswer::default()
        };
        assert_eq!(
            answer_from_c(&answer),
            Answer::Dns {
                error_code: 0,
                rtt: 7,
                local_dns: String::new(),
                ips: vec!["1.2.3.4".to_owned()],
            }
        );
    }

    #[test]
    fn a_short_buffer_is_reported_and_not_written() {
        let mut buffer = [0 as c_char; 2];
        // SAFETY: a valid buffer of two bytes.
        let written =
            unsafe { write_str_into(b"abc", buffer.as_mut_ptr(), buffer.len() as c_uint) };
        assert_eq!(written, MARS_SDT_ERR_NO_SPACE);
        assert_eq!(buffer[0], 0);
    }

    #[test]
    fn a_null_out_is_reported() {
        // SAFETY: null is explicitly allowed by the contract.
        assert_eq!(
            unsafe { write_str_into(b"", std::ptr::null_mut(), 0) },
            MARS_SDT_ERR_NULL_OUT
        );
    }

    #[test]
    fn no_hosts_is_an_empty_request() {
        // SAFETY: null is explicitly allowed by the contract.
        let items = unsafe { hosts_from_c(std::ptr::null(), 0) };
        assert!(items.is_empty());
    }

    #[test]
    fn hosts_read_as_the_map_a_diagnosis_is_started_with() {
        let ip = CString::new("1.2.3.4").unwrap();
        let ports = [MarsSdtIpPort {
            ip: ip.as_ptr(),
            port: 80,
        }];
        let name = CString::new("longlink").unwrap();
        let hosts = [MarsSdtHosts {
            name: name.as_ptr(),
            ports: ports.as_ptr(),
            port_count: ports.len() as c_uint,
        }];
        // SAFETY: `hosts` points to one initialised entry whose strings and
        // port array outlive this call.
        let items = unsafe { hosts_from_c(hosts.as_ptr(), hosts.len() as c_uint) };
        assert_eq!(
            items.get("longlink").map(Vec::as_slice),
            Some([CheckIPPort::new("1.2.3.4", 80)].as_slice())
        );
    }
}
