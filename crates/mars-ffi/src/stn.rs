//! `mars_stn_*` — the C ABI of the task pipeline (`mars/stn`).
//!
//! This is the seam `com/tencent/mars/stn/StnLogic.java` has in the C++
//! (`mars/stn/jni/*_Java2C.cc`), and the same one [`crate::abi`] is for xlog
//! and [`crate::sdt`] for the diagnosis: the calls of [`mars_stn::StnLogic`],
//! with C types on the outside, so that a Swift or C++ caller can run a task
//! without a Rust toolchain.
//!
//! The one thing a caller has to supply is the app: eighteen questions STN asks
//! while it runs a task — is the caller logged in, what does this task send,
//! how is the answer read, what is to be reported. The C++ asks them as
//! eighteen virtuals of a `Callback` the app inherits; the JNI asks them as
//! eighteen static methods of `StnLogic`. Here they are **one function pointer**
//! — [`MarsStnAsk`] — handed one tagged [`MarsStnQuestion`] and answering one
//! tagged [`MarsStnAnswer`], which is the JNI's own `Question` / `Answer` pair
//! with C types inside: a caller switches on the kind and reads the fields that
//! kind carries, and an answer of a kind the question did not ask for is no
//! answer at all, so [`mars_stn::App`]'s own is used — the one a host that has
//! no app gets.
//!
//! What a task is ([`MarsStnTask`]) and what a run leaves behind
//! ([`MarsStnCgiProfile`], [`MarsStnDnsProfile`]) cross as structs, because a
//! caller fills the first one in and reads the other two. The *report* of a
//! task crosses as the JSON [`mars_stn::task_profile_json`] writes, which is
//! the document the JNI hands Java: thirty-odd readings of a run, as one string
//! instead of a struct a caller would have to mirror.
//!
//! Panics: no Rust panic ever crosses this boundary. Every entry point is
//! wrapped in `catch_unwind` and reports [`MARS_STN_ERR_PANIC`] (or swallows it,
//! for the `void` symbols).

use std::collections::BTreeMap;
use std::ffi::{c_char, c_int, c_uint, c_void, CString};
use std::ptr::addr_of;
use std::sync::{Mutex, OnceLock};

use mars_stn::{
    gen_sequence_id, gen_task_id, task_profile_json, App as StnApp, CgiProfile, DnsProfile,
    ErrCmdType, ExtraInfo, HostRedirectType, IdentifyBuffer, LongLinkStatus, NetStatus, StnLogic,
    Task, TaskFailHandleType, TaskProfile, LOCAL_START_TASK_FAIL,
};

use crate::cstr;
use crate::guard;

/// `MARS_STN_OK` — the symbol did what it was asked.
pub const MARS_STN_OK: c_int = 0;
/// A panic was caught: nothing crossed into C, and the state is whatever the
/// panic left behind.
pub const MARS_STN_ERR_PANIC: c_int = -1;
/// `mars_stn_start_task` was handed no task.
pub const MARS_STN_ERR_NULL_TASK: c_int = -2;
/// The task is not one the queues would take — no channel to go out on, a
/// timeout the C++ refuses — which is what `StartTask` answers `false` for.
pub const MARS_STN_ERR_REFUSED: c_int = -3;

/// Which of the eighteen questions STN asked.
///
/// The integers are this ABI's own and nothing else's: they name a question,
/// the way the C++'s eighteen virtuals do by name and the JNI's
/// [`Question`](../../mars_jni/stn_c2java/enum.Question.html) does by variant.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarsStnQuestionKind {
    /// Nothing was asked: what a question starts out as, and what no question
    /// is ever asked as.
    Nothing = 0,
    /// `MakesureAuthed` — is the app logged in for this host and user?
    MakesureAuthed = 1,
    /// `TrafficData` — how much went out and came in.
    TrafficData = 2,
    /// `OnNewDns` — the ips the app knows for a host.
    OnNewDns = 3,
    /// `OnPush` — something the server sent that no task asked for.
    OnPush = 4,
    /// `Req2Buf` — what a task is to send.
    Req2Buf = 5,
    /// `Buf2Resp` — how the app reads an answer that came back.
    Buf2Resp = 6,
    /// `OnTaskEnd` — a task that is over.
    OnTaskEnd = 7,
    /// `ReportConnectStatus` — the connection as the app is asked to see it.
    ReportConnectStatus = 8,
    /// `OnLongLinkNetworkError` — the main long link's errors.
    LongLinkNetworkError = 9,
    /// `OnShortLinkNetworkError`.
    ShortLinkNetworkError = 10,
    /// `OnLongLinkStatusChange` — what the default long link is in.
    LongLinkStatusChange = 11,
    /// `GetLonglinkIdentifyCheckBuffer` — the check a new link is used with.
    IdentifyCheckBuffer = 12,
    /// `OnLonglinkIdentifyResponse` — whether the answer is the one the check
    /// asked for.
    IdentifyResponse = 13,
    /// `RequestSync` — the app is asked to sync.
    RequestSync = 14,
    /// `RequestNetCheckShortLinkHosts` — the hosts the network check may probe.
    NetCheckShortLinkHosts = 15,
    /// `ReportTaskProfile` — everything a task left behind.
    ReportTaskProfile = 16,
    /// `ReportTaskLimited` — a task the app asked to have limited.
    ReportTaskLimited = 17,
    /// `ReportDnsProfile` — how a dns question went.
    ReportDnsProfile = 18,
}

/// Which answer the caller wrote.
///
/// One answer per question, and an answer of another kind than the question
/// asked for is no answer: STN takes [`mars_stn::App`]'s own instead, which is
/// what a host with no app gets.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarsStnAnswerKind {
    /// Nobody answered — [`MarsStnAnswer::default`], and what an app that has
    /// nothing to say about a question writes.
    Nothing = 0,
    /// `MakesureAuthed`, `IdentifyResponse` — `yes` is `0` or `1`.
    Yes = 1,
    /// `OnNewDns`, `NetCheckShortLinkHosts` — `ips` / `ip_count`.
    Ips = 2,
    /// `Req2Buf` — `bytes` / `byte_count` is what the task sends.
    Encoded = 3,
    /// `Req2Buf` — `error_code` is what the task ends with, i.e. the C++'s
    /// `false` and the `errCode[0]` the app filled.
    Failed = 4,
    /// `Buf2Resp` — `error_code` is the code the task is remembered with and
    /// `handle` is one of the `kTaskFailHandle*` ints.
    Decoded = 5,
    /// `OnTaskEnd` — `error_code` is the code the task is remembered with.
    Ended = 6,
    /// `IdentifyCheckBuffer` — `mode`, `bytes`, `hash` and `cmdid`.
    Identified = 7,
    /// `ReportTaskLimited` — `limit` is what the limit is; `0` is "go ahead".
    Limit = 8,
}

/// One header of a task: a name and a value, both NUL-terminated.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MarsStnHeader {
    /// The name.
    pub name: *const c_char,
    /// The value.
    pub value: *const c_char,
}

/// A list of NUL-terminated strings, owned by the caller.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MarsStnStrings {
    /// `count` strings.
    pub items: *const *const c_char,
    /// How many strings `items` holds.
    pub count: c_uint,
}

impl Default for MarsStnStrings {
    /// A list with nothing in it.
    fn default() -> Self {
        Self {
            items: std::ptr::null(),
            count: 0,
        }
    }
}

/// One unit of work: [`mars_stn::Task`] with C types inside.
///
/// Every string is owned by the caller and read for the duration of the call it
/// was handed to; every list is a [`MarsStnStrings`] of the same. `send_only`
/// and friends are `0` or `1`, and `redirect_type` is one of
/// [`mars_stn::HostRedirectType`]'s: `0` none, `1` bare to https, `2` http to
/// https, `3` new host.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MarsStnTask {
    /// The id STN identifies the task by; [`mars_stn_gen_task_id`] hands one
    /// out.
    pub taskid: u32,
    /// The command, for a task that goes out on a long link.
    pub cmdid: u32,
    /// The channel, for a task the app keeps one per channel.
    pub channel_id: u64,
    /// One of the `Task::CHANNEL_*` values; `0` is no channel at all, which is
    /// a task the queues refuse.
    pub channel_select: i32,
    /// One of the `Task::TRANSPORT_PROTOCOL*` values.
    pub transport_protocol: i32,
    /// The cgi, for a task that goes out on a short link.
    pub cgi: *const c_char,
    /// Whether the task is a send with no answer to wait for.
    pub send_only: c_int,
    /// Whether the task waits for the app to be logged in.
    pub need_authed: c_int,
    /// Whether the task is weighed against the flow limit.
    pub limit_flow: c_int,
    /// Whether it is weighed against the frequency limit.
    pub limit_frequency: c_int,
    /// Whether a task with no network to go out on is ended instead of waiting.
    pub network_status_sensitive: c_int,
    /// One of the `Task::CHANNEL_*_STRATEGY` values.
    pub channel_strategy: i32,
    /// One of the `Task::TASK_PRIORITY_*` values.
    pub priority: i32,
    /// How many tries the task has; negative is the default.
    pub retry_count: i32,
    /// How long the server is expected to take, which is added to the timeout.
    pub server_process_cost: i32,
    /// How long the whole task may take, retries and all.
    pub total_timeout: i32,
    /// Whether the answer may come late.
    pub long_polling: c_int,
    /// How long a late answer may take.
    pub long_polling_timeout: i32,
    /// What the app's own report is keyed by.
    pub report_arg: *const c_char,
    /// The long link the task goes out on; empty is the default one.
    pub channel_name: *const c_char,
    /// The group the task belongs to.
    pub group_name: *const c_char,
    /// The user the task is sent for.
    pub user_id: *const c_char,
    /// The protocol the app speaks.
    pub protocol: i32,
    /// `header_count` headers.
    pub headers: *const MarsStnHeader,
    /// How many headers `headers` holds.
    pub header_count: c_uint,
    /// The short link's hosts.
    pub shortlink_host_list: MarsStnStrings,
    /// The hosts a short-link host falls back to.
    pub shortlink_fallback_hostlist: MarsStnStrings,
    /// The long link's hosts.
    pub longlink_host_list: MarsStnStrings,
    /// The hosts of the minor long links.
    pub minorlong_host_list: MarsStnStrings,
    /// The hosts a quic connection may be made to.
    pub quic_host_list: MarsStnStrings,
    /// How many minor long links the task may use.
    pub max_minorlinks: i32,
    /// What the task is, for the app's own report.
    pub function: *const c_char,
    /// What the cgi is prefixed with.
    pub cgi_prefix: *const c_char,
    /// One of [`mars_stn::HostRedirectType`]'s.
    pub redirect_type: c_int,
    /// The sequence the app numbers the task with.
    pub client_sequence_id: u16,
}

impl Default for MarsStnTask {
    /// A task with nothing in it: no channel to go out on, which is one the
    /// queues refuse, and no strings. A caller fills in what it wants — at
    /// least a `channel_select` and, for a short link, a `cgi`.
    fn default() -> Self {
        Self {
            taskid: 0,
            cmdid: 0,
            channel_id: 0,
            channel_select: 0,
            transport_protocol: 0,
            cgi: std::ptr::null(),
            send_only: 0,
            need_authed: 0,
            limit_flow: 0,
            limit_frequency: 0,
            network_status_sensitive: 0,
            channel_strategy: 0,
            priority: 0,
            retry_count: 0,
            server_process_cost: 0,
            total_timeout: 0,
            long_polling: 0,
            long_polling_timeout: 0,
            report_arg: std::ptr::null(),
            channel_name: std::ptr::null(),
            group_name: std::ptr::null(),
            user_id: std::ptr::null(),
            protocol: 0,
            headers: std::ptr::null(),
            header_count: 0,
            shortlink_host_list: MarsStnStrings::default(),
            shortlink_fallback_hostlist: MarsStnStrings::default(),
            longlink_host_list: MarsStnStrings::default(),
            minorlong_host_list: MarsStnStrings::default(),
            quic_host_list: MarsStnStrings::default(),
            max_minorlinks: 0,
            function: std::ptr::null(),
            cgi_prefix: std::ptr::null(),
            redirect_type: 0,
            client_sequence_id: 0,
        }
    }
}

/// The connect a task ran on, as the app's report wants it:
/// [`mars_stn::CgiProfile`] with C types inside. Every reading is a
/// `gettickcount()`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MarsStnCgiProfile {
    /// When the run began.
    pub start_time: u64,
    /// When the connect began.
    pub start_connect_time: u64,
    /// When it came back, socket or no socket.
    pub connect_successful_time: u64,
    /// When the request went out.
    pub start_send_packet_time: u64,
    /// When the write was done.
    pub send_packet_finished_time: u64,
    /// When the read of the answer began.
    pub start_read_packet_time: u64,
    /// When the last of it came back.
    pub read_packet_finished_time: u64,
    /// One of the `Task::CHANNEL_*` values.
    pub channel_type: i32,
    /// One of the `Task::TRANSPORT_PROTOCOL*` values.
    pub transport_protocol: i32,
    /// How long the pair that won took to answer.
    pub rtt: u32,
    /// The network the connect was made on.
    pub nettype: *const c_char,
}

/// How a dns question went: [`mars_stn::DnsProfile`] with C types inside.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MarsStnDnsProfile {
    /// When the question was asked.
    pub start_time: u64,
    /// When it came back; `0` while it has not.
    pub end_time: u64,
    /// What was asked for.
    pub host: *const c_char,
    /// One of [`mars_stn::ErrCmdType`]'s: `0` ok, `1` false, `2` dial, `3` dns,
    /// `4` socket, `5` http, `6` netmsgxp, `7` endecode, `8` server, `9` local,
    /// `10` canceld.
    pub err_type: c_int,
    /// The code the question ended with.
    pub err_code: c_int,
    /// Which dns it was: `1` the app's, `2` the platform's.
    pub dnstype: c_int,
}

/// One of the eighteen questions, in the arguments the C++ hands the app.
///
/// Every field is read for the [`MarsStnQuestionKind`] in `kind` and left alone
/// for the others, so a caller switches on the kind and reads what it names:
/// `host` is the host of `MakesureAuthed`, `OnNewDns` and
/// `ShortLinkNetworkError`; `ip` / `port` the pair of the two network errors;
/// `channel_id` the link of `OnPush`, `IdentifyCheckBuffer` and
/// `IdentifyResponse`; `body` what was pushed, what came back and what the
/// server answered. The strings and buffers are owned by STN and are only good
/// while the ask runs.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MarsStnQuestion {
    /// Which question.
    pub kind: MarsStnQuestionKind,
    /// The host: `MakesureAuthed`, `OnNewDns`, `ShortLinkNetworkError`.
    pub host: *const c_char,
    /// The user: `MakesureAuthed`, `Req2Buf`, `Buf2Resp`, `OnTaskEnd`.
    pub user_id: *const c_char,
    /// The link: `OnPush`, `IdentifyCheckBuffer`, `IdentifyResponse`.
    pub channel_id: *const c_char,
    /// The ip a network error happened on.
    pub ip: *const c_char,
    /// The port a network error happened on.
    pub port: u16,
    /// `OnPush`, `IdentifyCheckBuffer`.
    pub cmdid: u32,
    /// `OnPush`, `Req2Buf`, `Buf2Resp`, `OnTaskEnd`.
    pub taskid: u32,
    /// `TrafficData` — how much went out.
    pub send: i64,
    /// `TrafficData` — how much came in.
    pub recv: i64,
    /// `Req2Buf`, `Buf2Resp` — the channel the task is going out on.
    pub channel_select: i32,
    /// `Req2Buf` — the sequence the task goes out with.
    pub sequence: u16,
    /// `OnPush`, `Buf2Resp`, `IdentifyResponse` — the bytes.
    pub body: *const u8,
    /// How many bytes `body` holds.
    pub body_count: c_uint,
    /// `IdentifyResponse` — the hash the app handed out.
    pub hash: *const u8,
    /// How many bytes `hash` holds.
    pub hash_count: c_uint,
    /// `OnTaskEnd`, the two network errors — one of [`mars_stn::ErrCmdType`]'s.
    pub err_type: c_int,
    /// `OnTaskEnd`, the two network errors.
    pub err_code: c_int,
    /// `OnTaskEnd` — the connect the task ran on.
    pub profile: *const MarsStnCgiProfile,
    /// `ReportTaskProfile` — the report, as the JSON
    /// [`mars_stn::task_profile_json`] writes.
    pub profile_json: *const c_char,
    /// `ReportDnsProfile` — how the question went.
    pub dns: *const MarsStnDnsProfile,
    /// `ReportConnectStatus` — whether STN can reach anything at all: one of
    /// [`mars_stn::NetStatus`]'s, `-1` unknown, `0` unavailable, `1` gateway
    /// failed, `2` server failed, `3` connecting, `4` connected, `5` down.
    pub net_status_all: c_int,
    /// `ReportConnectStatus` — the long link, in the same integers.
    pub net_status_longlink: c_int,
    /// `LongLinkStatusChange` — one of [`mars_stn::LongLinkStatus`]'s: `0`
    /// idle, `1` connecting, `2` connected, `3` disconnected, `4` failed.
    pub link_status: c_int,
    /// `OnNewDns` — whether the host is a long-link one; `0` or `1`.
    pub longlink_host: c_int,
    /// `ReportTaskLimited` — what the task is being weighed against.
    pub check_type: c_int,
    /// `ReportTaskLimited` — the task itself.
    pub task: *const MarsStnTask,
}

impl Default for MarsStnQuestion {
    /// A question that was never asked.
    fn default() -> Self {
        Self {
            kind: MarsStnQuestionKind::Nothing,
            host: std::ptr::null(),
            user_id: std::ptr::null(),
            channel_id: std::ptr::null(),
            ip: std::ptr::null(),
            port: 0,
            cmdid: 0,
            taskid: 0,
            send: 0,
            recv: 0,
            channel_select: 0,
            sequence: 0,
            body: std::ptr::null(),
            body_count: 0,
            hash: std::ptr::null(),
            hash_count: 0,
            err_type: 0,
            err_code: 0,
            profile: std::ptr::null(),
            profile_json: std::ptr::null(),
            dns: std::ptr::null(),
            net_status_all: 0,
            net_status_longlink: 0,
            link_status: 0,
            longlink_host: 0,
            check_type: 0,
            task: std::ptr::null(),
        }
    }
}

/// What the caller answered.
///
/// Every field is read for the [`MarsStnAnswerKind`] in `kind` and left alone
/// for the others, so a caller fills in the one it answered and leaves the rest
/// zero. The strings and buffers the answer points at are owned by the caller
/// and are read before the ask returns, and never afterwards.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MarsStnAnswer {
    /// Which answer.
    pub kind: MarsStnAnswerKind,
    /// `Yes` — `0` or `1`.
    pub yes: c_int,
    /// `Ips` — `ip_count` NUL-terminated strings.
    pub ips: *const *const c_char,
    /// How many strings `ips` holds.
    pub ip_count: c_uint,
    /// `Encoded`, `Identified` — the bytes: what to send, or the check buffer.
    pub bytes: *const u8,
    /// How many bytes `bytes` holds.
    pub byte_count: c_uint,
    /// `Failed`, `Decoded`, `Ended` — the code a task is remembered with.
    pub error_code: c_int,
    /// `Decoded` — one of the `kTaskFailHandle*` ints: `0` normal, `-1` default,
    /// `-12` retry every task, `-13` session timeout, `-14` task end, `-15` task
    /// timeout, `-16` silent task end.
    pub handle: c_int,
    /// `Identified` — `0` sends it now, `1` asks again on the next connect,
    /// anything else stops asking.
    pub mode: c_int,
    /// `Identified` — the hash the response is judged against.
    pub hash: *const u8,
    /// How many bytes `hash` holds.
    pub hash_count: c_uint,
    /// `Identified` — the cmdid the buffer goes out with.
    pub cmdid: u32,
    /// `Limit` — what the limit is; `0` is "go ahead".
    pub limit: u32,
}

impl Default for MarsStnAnswer {
    /// An answer nobody wrote, which is [`mars_stn::App`]'s own for every
    /// question.
    fn default() -> Self {
        Self {
            kind: MarsStnAnswerKind::Nothing,
            yes: 0,
            ips: std::ptr::null(),
            ip_count: 0,
            bytes: std::ptr::null(),
            byte_count: 0,
            error_code: 0,
            handle: 0,
            mode: 0,
            hash: std::ptr::null(),
            hash_count: 0,
            cmdid: 0,
            limit: 0,
        }
    }
}

/// The app STN asks, as one function: the eighteen questions in, the eighteen
/// answers out, and `ctx` — what the caller handed to [`mars_stn_set_app`] —
/// handed back with every one.
///
/// `NULL` is an app that answers nothing, which is [`mars_stn::App`]'s own
/// answers: an app that is logged in, a task that is encoded to nothing, an
/// answer that is a good one.
pub type MarsStnAsk =
    Option<extern "C" fn(*mut c_void, *const MarsStnQuestion, *mut MarsStnAnswer)>;

/// The STN of the process: the counterpart of the `StnLogic` singleton the C++'s
/// Java2C calls reach, and of the `LOGIC` `mars-jni` keeps. A net core is made
/// with it, the way `OnCreate` makes one.
fn logic() -> &'static Mutex<StnLogic> {
    static LOGIC: OnceLock<Mutex<StnLogic>> = OnceLock::new();
    LOGIC.get_or_init(|| {
        let mut logic = StnLogic::new();
        logic.create();
        Mutex::new(logic)
    })
}

/// Runs `f` on the process-wide STN. A poisoned lock keeps the state a panic
/// left behind rather than resetting it, which is what the C++ would leave.
fn with_logic<R>(f: impl FnOnce(&mut StnLogic) -> R) -> R {
    let mut logic = logic()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut logic)
}

/// `SetCallback` — the app STN talks to: one function pointer the eighteen
/// questions are funnelled through, and `ctx`, which is handed back with every
/// one of them.
///
/// A `NULL` `ask` is an app that answers nothing, which gets
/// [`mars_stn::App`]'s own answers — the ones a host that never called this
/// gets too.
///
/// # Safety
///
/// `ctx` must be whatever `ask` expects, and must stay alive until another
/// `mars_stn_set_app` takes its place. STN asks the app while it holds the
/// process-wide logic, so `ask` must not call another `mars_stn_*`: it would
/// wait for the lock this call is holding. Everything a question carries is in
/// the question itself.
#[no_mangle]
pub unsafe extern "C" fn mars_stn_set_app(ctx: *mut c_void, ask: MarsStnAsk) {
    guard((), || {
        let ctx = ctx as usize;
        with_logic(|logic| match ask {
            Some(ask) => logic.set_callback(CApp { ask, ctx }),
            None => logic.set_callback(NoApp),
        });
    })
}

/// `Reset` — a net core made again from nothing, which is what the C++ does when
/// the app has moved to another account: the tasks, the signalling session and
/// the addresses the setters handed to the net source are gone with the one
/// before it. The app is not: it is the caller's, and the new core is asked
/// through the same [`MarsStnAsk`].
#[no_mangle]
pub extern "C" fn mars_stn_reset() {
    guard((), || with_logic(StnLogic::reset));
}

/// `ResetAndInitEncoderVersion` — reset plus the encoder the new net core is
/// made with.
///
/// # Safety
///
/// `name` must either be null or point to a valid NUL-terminated string that is
/// not mutated while the call runs.
#[no_mangle]
pub unsafe extern "C" fn mars_stn_reset_and_init_encoder_version(
    version: c_int,
    name: *const c_char,
) {
    guard((), || {
        // SAFETY: forwarded to `ptr_to_str_or_empty`, whose contract the caller
        // upholds (null or a valid NUL-terminated string).
        let name = unsafe { cstr::ptr_to_str_or_empty(name) };
        with_logic(|logic| logic.reset_with_encoder(version, name));
    })
}

/// `SetLonglinkSvrAddr` — the hosts and ports the long link goes out on, and
/// the ip `debug_ip` makes it reach without asking dns (`""` or null for none).
///
/// # Safety
///
/// `host` and `debug_ip` must each either be null or point to a valid
/// NUL-terminated string, and `ports` must either be null or point to
/// `port_count` initialised `unsigned short`s. All three are read for the
/// duration of the call.
#[no_mangle]
pub unsafe extern "C" fn mars_stn_set_longlink_svr_addr(
    host: *const c_char,
    ports: *const u16,
    port_count: c_uint,
    debug_ip: *const c_char,
) {
    guard((), || {
        // SAFETY: forwarded to `ptr_to_str_or_empty`, whose contract the caller
        // upholds for both strings.
        let (host, debug_ip) = unsafe {
            (
                cstr::ptr_to_str_or_empty(host),
                cstr::ptr_to_str_or_empty(debug_ip),
            )
        };
        let mut ports_vec = Vec::with_capacity(port_count as usize);
        if !ports.is_null() {
            for index in 0..port_count as usize {
                // SAFETY: `ports` is non-null and the caller promises
                // `port_count` readable `unsigned short`s.
                ports_vec.push(unsafe { *ports.add(index) });
            }
        }
        with_logic(|logic| logic.set_longlink_svr_addr(host, ports_vec, debug_ip));
    })
}

/// `SetShortlinkSvrAddr` — the port the short link goes out on, and the ip
/// `debug_ip` makes it reach without asking dns.
///
/// # Safety
///
/// `debug_ip` must either be null or point to a valid NUL-terminated string
/// that is not mutated while the call runs.
#[no_mangle]
pub unsafe extern "C" fn mars_stn_set_shortlink_svr_addr(port: u16, debug_ip: *const c_char) {
    guard((), || {
        // SAFETY: forwarded to `ptr_to_str_or_empty`, whose contract the caller
        // upholds (null or a valid NUL-terminated string).
        let debug_ip = unsafe { cstr::ptr_to_str_or_empty(debug_ip) };
        with_logic(|logic| logic.set_shortlink_svr_addr(port, debug_ip));
    })
}

/// `SetDebugIP` — a host that is reached without asking dns. An empty ip drops
/// the entry, so the host goes back to being resolved.
///
/// # Safety
///
/// `host` and `ip` must each either be null or point to a valid NUL-terminated
/// string that is not mutated while the call runs.
#[no_mangle]
pub unsafe extern "C" fn mars_stn_set_debug_ip(host: *const c_char, ip: *const c_char) {
    guard((), || {
        // SAFETY: forwarded to `ptr_to_str_or_empty`, whose contract the caller
        // upholds for both strings.
        let (host, ip) = unsafe {
            (
                cstr::ptr_to_str_or_empty(host),
                cstr::ptr_to_str_or_empty(ip),
            )
        };
        with_logic(|logic| logic.set_debug_ip(host, ip));
    })
}

/// `SetBackupIPs` — the ips a host falls back to. An empty list drops the host.
///
/// # Safety
///
/// `host` must either be null or point to a valid NUL-terminated string, and
/// `ips` must either be null or point to `ip_count` valid NUL-terminated
/// strings. All of them are read for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn mars_stn_set_backup_ips(
    host: *const c_char,
    ips: *const *const c_char,
    ip_count: c_uint,
) {
    guard((), || {
        // SAFETY: forwarded to `ptr_to_str_or_empty` and `strings_from_c`,
        // whose contracts the caller upholds.
        let (host, ips) = unsafe {
            (
                cstr::ptr_to_str_or_empty(host),
                strings_from_c(ips, ip_count),
            )
        };
        with_logic(|logic| logic.set_backup_ips(host, ips));
    })
}

/// `StartTask` — one unit of work, as [`MarsStnTask`] describes it.
///
/// @return [`MARS_STN_OK`], or [`MARS_STN_ERR_NULL_TASK`], or
/// [`MARS_STN_ERR_REFUSED`] when the task is not one the queues would take —
/// no channel to go out on, a timeout the C++ refuses — or
/// [`MARS_STN_ERR_PANIC`].
///
/// # Safety
///
/// `task` must either be null or point to an initialised [`MarsStnTask`] whose
/// strings, headers and host lists stay alive for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn mars_stn_start_task(task: *const MarsStnTask) -> c_int {
    guard(MARS_STN_ERR_PANIC, || {
        if task.is_null() {
            return MARS_STN_ERR_NULL_TASK;
        }
        // SAFETY: `task` is non-null and the caller promises an initialised
        // `MarsStnTask` whose strings and lists outlive this call.
        let task = unsafe { task_from_c(&*task) };
        let started = with_logic(|logic| logic.start_task(task));
        if started {
            MARS_STN_OK
        } else {
            MARS_STN_ERR_REFUSED
        }
    })
}

/// `StopTask` — the task is taken out of the queues.
///
/// @return `1` when it was one of ours, `0` when it was not, and
/// [`MARS_STN_ERR_PANIC`] when a panic was caught.
#[no_mangle]
pub extern "C" fn mars_stn_stop_task(taskid: u32) -> c_int {
    guard(MARS_STN_ERR_PANIC, || {
        with_logic(|logic| logic.stop_task(taskid)) as c_int
    })
}

/// `HasTask` — whether the task is in one of the queues.
///
/// @return `1` or `0`, and [`MARS_STN_ERR_PANIC`] when a panic was caught.
#[no_mangle]
pub extern "C" fn mars_stn_has_task(taskid: u32) -> c_int {
    guard(MARS_STN_ERR_PANIC, || {
        with_logic(|logic| logic.has_task(taskid)) as c_int
    })
}

/// `RedoTask` — every task that is out is run again.
#[no_mangle]
pub extern "C" fn mars_stn_redo_tasks() {
    guard((), || with_logic(StnLogic::redo_tasks));
}

/// `TouchTasks` — the queues are sorted again, which is what a task the app was
/// not logged in for is waiting for.
#[no_mangle]
pub extern "C" fn mars_stn_touch_tasks() {
    guard((), || with_logic(StnLogic::touch_tasks));
}

/// `ClearTask` — every task that is out is thrown away.
#[no_mangle]
pub extern "C" fn mars_stn_clear_tasks() {
    guard((), || with_logic(StnLogic::clear_tasks));
}

/// `MakesureLongLinkConnected` — the default long link is connected.
///
/// @return `1` when there was one to connect, `0` when there was not — which is
/// what the C++ does nothing at all for — and [`MARS_STN_ERR_PANIC`].
#[no_mangle]
pub extern "C" fn mars_stn_makesure_longlink_connected() -> c_int {
    guard(MARS_STN_ERR_PANIC, || {
        with_logic(|logic| {
            let connected = logic.default_link().is_some();
            logic.make_sure_default_long_link_connected();
            connected
        }) as c_int
    })
}

/// `SetSignallingStrategy` — for every keeper in the process. A period or a keep
/// time of `0` leaves the `SignallingKeeper` defaults alone, which is what the
/// C++'s `SetStrategy` does with them.
#[no_mangle]
pub extern "C" fn mars_stn_set_signalling_strategy(period: i64, keep_time: i64) {
    guard((), || {
        with_logic(|logic| {
            logic.set_signalling_strategy(period.max(0) as u64, keep_time.max(0) as u64)
        });
    })
}

/// `KeepSignalling`.
#[no_mangle]
pub extern "C" fn mars_stn_keep_signalling() {
    guard((), || with_logic(StnLogic::keep_signalling));
}

/// `StopSignalling`.
#[no_mangle]
pub extern "C" fn mars_stn_stop_signalling() {
    guard((), || with_logic(StnLogic::stop_signalling));
}

/// `SetClientVersion` — the version every long-link package goes out with, and
/// the only one the unpacker accepts back.
#[no_mangle]
pub extern "C" fn mars_stn_set_client_version(version: u32) {
    guard((), || mars_stn::longlink::set_client_version(version));
}

/// `GenTaskID` — one counter for the whole process, like the C++'s
/// `static uint32_t`.
#[no_mangle]
pub extern "C" fn mars_stn_gen_task_id() -> u32 {
    guard(0, gen_task_id)
}

/// `GenSequenceId` — a `unsigned short`, like the C++'s.
#[no_mangle]
pub extern "C" fn mars_stn_gen_sequence_id() -> u16 {
    guard(0, gen_sequence_id)
}

/// `TrigNooping` — `SmartHeartbeat::SetHeartBeat(0)` and a noop on the default
/// long link.
#[no_mangle]
pub extern "C" fn mars_stn_trig_nooping() {
    guard((), || {
        mars_stn::smart_heartbeat::set_heartbeat(0);
        with_logic(|logic| {
            if let Some(link) = logic.default_long_link() {
                link.lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .trig_noop();
            }
        });
    })
}

/// The app STN asks, when the app is C: one function pointer and the `ctx` that
/// goes with it, carried as a `usize` so that the app is `Send` — which
/// [`mars_stn::App`] requires, because a task may run on any thread.
struct CApp {
    ask: extern "C" fn(*mut c_void, *const MarsStnQuestion, *mut MarsStnAnswer),
    ctx: usize,
}

impl CApp {
    /// One question out, one answer back.
    fn ask(&self, question: &MarsStnQuestion) -> MarsStnAnswer {
        let mut answer = MarsStnAnswer::default();
        // The call is not `unsafe` — `ask` is a plain `fn` pointer — but what it
        // is handed is: `ctx` is the pointer the caller gave
        // `mars_stn_set_app`, alive by that contract, and the question and
        // answer it reads and writes are locals that outlive it.
        (self.ask)(
            self.ctx as *mut c_void,
            addr_of!(*question),
            &mut answer as *mut MarsStnAnswer,
        );
        answer
    }
}

/// The app a caller that handed no `ask` gets — and the one a host that never
/// called [`mars_stn_set_app`] gets: [`mars_stn::App`]'s own answers, which are
/// the C++'s for a callback that was never set.
struct NoApp;

impl StnApp for NoApp {}

impl StnApp for CApp {
    fn makesure_authed(&mut self, host: &str, user_id: &str) -> bool {
        let host = c_string(host);
        let user_id = c_string(user_id);
        let answer = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::MakesureAuthed,
            host: host.as_ptr(),
            user_id: user_id.as_ptr(),
            ..Default::default()
        });
        match answer.kind {
            MarsStnAnswerKind::Yes => answer.yes != 0,
            // `App`'s own answer: an app that did not answer is logged in.
            _ => true,
        }
    }

    fn traffic_data(&mut self, send: i64, recv: i64) {
        let _ = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::TrafficData,
            send,
            recv,
            ..Default::default()
        });
    }

    fn on_new_dns(&mut self, host: &str, longlink_host: bool, _extra: &ExtraInfo) -> Vec<String> {
        let host = c_string(host);
        let answer = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::OnNewDns,
            host: host.as_ptr(),
            longlink_host: c_int::from(longlink_host),
            ..Default::default()
        });
        match answer.kind {
            // SAFETY: `answer.ips` is what the caller wrote, and by
            // `mars_stn_set_app`'s contract its strings are alive until the ask
            // returns — which it has.
            MarsStnAnswerKind::Ips => unsafe { strings_from_c(answer.ips, answer.ip_count) },
            // The platform's own resolver, which is `App`'s own answer.
            _ => Vec::new(),
        }
    }

    fn on_push(&mut self, channel_id: &str, cmdid: u32, taskid: u32, body: &[u8]) {
        let channel_id = c_string(channel_id);
        let _ = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::OnPush,
            channel_id: channel_id.as_ptr(),
            cmdid,
            taskid,
            body: body.as_ptr(),
            body_count: body.len() as c_uint,
            ..Default::default()
        });
    }

    fn req2buf(
        &mut self,
        taskid: u32,
        user_id: &str,
        channel_select: i32,
        host: &str,
        sequence: u16,
    ) -> Result<Vec<u8>, i32> {
        let host = c_string(host);
        let user_id = c_string(user_id);
        let answer = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::Req2Buf,
            host: host.as_ptr(),
            user_id: user_id.as_ptr(),
            taskid,
            channel_select,
            sequence,
            ..Default::default()
        });
        match answer.kind {
            // SAFETY: `answer.bytes` is what the caller wrote, alive until the
            // ask returns — which it has.
            MarsStnAnswerKind::Encoded => {
                Ok(unsafe { bytes_from_c(answer.bytes, answer.byte_count) })
            }
            MarsStnAnswerKind::Failed => Err(answer.error_code),
            // A task nobody can encode is one that cannot be started.
            _ => Err(LOCAL_START_TASK_FAIL),
        }
    }

    fn buf2resp(
        &mut self,
        taskid: u32,
        user_id: &str,
        body: &[u8],
        channel_select: i32,
    ) -> (i32, TaskFailHandleType) {
        let user_id = c_string(user_id);
        let answer = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::Buf2Resp,
            user_id: user_id.as_ptr(),
            taskid,
            body: body.as_ptr(),
            body_count: body.len() as c_uint,
            channel_select,
            ..Default::default()
        });
        match answer.kind {
            MarsStnAnswerKind::Decoded => {
                (answer.error_code, TaskFailHandleType::of(answer.handle))
            }
            // An answer nobody read is a good one.
            _ => (0, TaskFailHandleType::Normal),
        }
    }

    fn on_task_end(
        &mut self,
        taskid: u32,
        user_id: &str,
        err_type: ErrCmdType,
        err_code: i32,
        profile: &CgiProfile,
    ) -> i32 {
        let user_id = c_string(user_id);
        let profile = ProfileView::of(profile);
        let answer = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::OnTaskEnd,
            user_id: user_id.as_ptr(),
            taskid,
            err_type: err_type as c_int,
            err_code,
            profile: profile.profile(),
            ..Default::default()
        });
        match answer.kind {
            MarsStnAnswerKind::Ended => answer.error_code,
            // The net core is done with the task.
            _ => 0,
        }
    }

    fn report_connect_status(&mut self, all: NetStatus, longlink: NetStatus) {
        let _ = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::ReportConnectStatus,
            net_status_all: all as c_int,
            net_status_longlink: longlink as c_int,
            ..Default::default()
        });
    }

    fn on_long_link_network_error(
        &mut self,
        err_type: ErrCmdType,
        err_code: i32,
        ip: &str,
        port: u16,
    ) {
        let ip = c_string(ip);
        let _ = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::LongLinkNetworkError,
            ip: ip.as_ptr(),
            port,
            err_type: err_type as c_int,
            err_code,
            ..Default::default()
        });
    }

    fn on_short_link_network_error(
        &mut self,
        err_type: ErrCmdType,
        err_code: i32,
        ip: &str,
        host: &str,
        port: u16,
    ) {
        let ip = c_string(ip);
        let host = c_string(host);
        let _ = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::ShortLinkNetworkError,
            ip: ip.as_ptr(),
            host: host.as_ptr(),
            port,
            err_type: err_type as c_int,
            err_code,
            ..Default::default()
        });
    }

    fn on_long_link_status_change(&mut self, status: LongLinkStatus) {
        let _ = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::LongLinkStatusChange,
            link_status: status as c_int,
            ..Default::default()
        });
    }

    fn identify_check_buffer(&mut self, channel_id: &str, cmdid: u32) -> IdentifyBuffer {
        let channel_id = c_string(channel_id);
        let answer = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::IdentifyCheckBuffer,
            channel_id: channel_id.as_ptr(),
            cmdid,
            ..Default::default()
        });
        match answer.kind {
            MarsStnAnswerKind::Identified => {
                // SAFETY: `answer.bytes` and `answer.hash` are what the caller
                // wrote, alive until the ask returns — which it has.
                let (bytes, hash) = unsafe {
                    (
                        bytes_from_c(answer.bytes, answer.byte_count),
                        bytes_from_c(answer.hash, answer.hash_count),
                    )
                };
                IdentifyBuffer::of(answer.mode, bytes, hash, answer.cmdid)
            }
            // Ask again on the next connect.
            _ => IdentifyBuffer::next(Vec::new()),
        }
    }

    fn identify_response(&mut self, channel_id: &str, response: &[u8], hash: &[u8]) -> bool {
        let channel_id = c_string(channel_id);
        let answer = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::IdentifyResponse,
            channel_id: channel_id.as_ptr(),
            body: response.as_ptr(),
            body_count: response.len() as c_uint,
            hash: hash.as_ptr(),
            hash_count: hash.len() as c_uint,
            ..Default::default()
        });
        match answer.kind {
            MarsStnAnswerKind::Yes => answer.yes != 0,
            // Not the answer the check asked for.
            _ => false,
        }
    }

    fn request_sync(&mut self) {
        let _ = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::RequestSync,
            ..Default::default()
        });
    }

    fn net_check_shortlink_hosts(&mut self) -> Vec<String> {
        let answer = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::NetCheckShortLinkHosts,
            ..Default::default()
        });
        match answer.kind {
            // SAFETY: `answer.ips` is what the caller wrote, alive until the
            // ask returns — which it has.
            MarsStnAnswerKind::Ips => unsafe { strings_from_c(answer.ips, answer.ip_count) },
            _ => Vec::new(),
        }
    }

    fn report_task_profile(&mut self, profile: &TaskProfile) {
        let json = c_string(&task_profile_json(profile));
        let _ = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::ReportTaskProfile,
            profile_json: json.as_ptr(),
            ..Default::default()
        });
    }

    fn report_task_limited(&mut self, check_type: i32, task: &Task) -> u32 {
        let task = TaskView::of(task);
        let answer = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::ReportTaskLimited,
            check_type,
            task: task.task(),
            ..Default::default()
        });
        match answer.kind {
            MarsStnAnswerKind::Limit => answer.limit,
            // `0` is "go ahead".
            _ => 0,
        }
    }

    fn report_dns_profile(&mut self, profile: &DnsProfile) {
        let profile = DnsView::of(profile);
        let _ = self.ask(&MarsStnQuestion {
            kind: MarsStnQuestionKind::ReportDnsProfile,
            dns: profile.dns(),
            ..Default::default()
        });
    }
}

/// A NUL-terminated copy of `value`, for the C side to read. A value with a NUL
/// in it is not one C can read, so it becomes the empty one — the way the rest
/// of this crate degrades.
fn c_string(value: &str) -> CString {
    CString::new(value).unwrap_or_default()
}

/// `count` [`MarsStnHeader`]s as the headers of a task.
///
/// # Safety
///
/// `headers` must either be null or point to `count` initialised
/// [`MarsStnHeader`]s whose strings stay alive for the duration of the call.
unsafe fn headers_from_c(headers: *const MarsStnHeader, count: c_uint) -> BTreeMap<String, String> {
    let mut items = BTreeMap::new();
    if headers.is_null() {
        return items;
    }
    for index in 0..count as usize {
        // SAFETY: `headers` is non-null and the caller promises `count`
        // initialised headers.
        let header = unsafe { &*headers.add(index) };
        // SAFETY: forwarded to `ptr_to_str_or_empty`, whose contract the caller
        // upholds for both strings.
        let (name, value) = unsafe {
            (
                cstr::ptr_to_str_or_empty(header.name),
                cstr::ptr_to_str_or_empty(header.value),
            )
        };
        items.insert(name.to_owned(), value.to_owned());
    }
    items
}

/// `count` NUL-terminated strings as the list a task or a setter carries.
///
/// # Safety
///
/// `items` must either be null or point to `count` valid NUL-terminated strings
/// that stay alive for the duration of the call.
unsafe fn strings_from_c(items: *const *const c_char, count: c_uint) -> Vec<String> {
    let mut strings = Vec::with_capacity(count as usize);
    if items.is_null() {
        return strings;
    }
    for index in 0..count as usize {
        // SAFETY: `items` is non-null and the caller promises `count` readable
        // pointers.
        let item = unsafe { *items.add(index) };
        // SAFETY: forwarded to `ptr_to_str_or_empty`, whose contract the caller
        // upholds (null or a valid NUL-terminated string).
        strings.push(unsafe { cstr::ptr_to_str_or_empty(item) }.to_owned());
    }
    strings
}

/// `count` bytes out of the caller's buffer: nothing at all when there is no
/// buffer, which is what an app that answered nothing handed over.
///
/// # Safety
///
/// `bytes` must either be null or point to `count` readable bytes that stay
/// alive for the duration of the call.
unsafe fn bytes_from_c(bytes: *const u8, count: c_uint) -> Vec<u8> {
    if bytes.is_null() || count == 0 {
        return Vec::new();
    }
    // SAFETY: `bytes` is non-null and the caller promises `count` readable
    // bytes, alive for this call.
    unsafe { std::slice::from_raw_parts(bytes, count as usize) }.to_vec()
}

/// One [`MarsStnTask`], as the [`mars_stn::Task`] the queues take.
///
/// # Safety
///
/// Every string, header and host list of `task` must be null or valid for the
/// duration of the call; see [`strings_from_c`] and [`headers_from_c`].
unsafe fn task_from_c(task: &MarsStnTask) -> Task {
    let mut item = Task::new(task.taskid, task.cmdid);
    item.channel_id = task.channel_id;
    item.channel_select = task.channel_select;
    item.transport_protocol = task.transport_protocol;
    item.cgi = unsafe { cstr::ptr_to_str_or_empty(task.cgi) }.to_owned();
    item.send_only = task.send_only != 0;
    item.need_authed = task.need_authed != 0;
    item.limit_flow = task.limit_flow != 0;
    item.limit_frequency = task.limit_frequency != 0;
    item.network_status_sensitive = task.network_status_sensitive != 0;
    item.channel_strategy = task.channel_strategy;
    item.priority = task.priority;
    item.retry_count = task.retry_count;
    item.server_process_cost = task.server_process_cost;
    item.total_timeout = task.total_timeout;
    item.long_polling = task.long_polling != 0;
    item.long_polling_timeout = task.long_polling_timeout;
    item.report_arg = unsafe { cstr::ptr_to_str_or_empty(task.report_arg) }.to_owned();
    item.channel_name = unsafe { cstr::ptr_to_str_or_empty(task.channel_name) }.to_owned();
    item.group_name = unsafe { cstr::ptr_to_str_or_empty(task.group_name) }.to_owned();
    item.user_id = unsafe { cstr::ptr_to_str_or_empty(task.user_id) }.to_owned();
    item.protocol = task.protocol;
    // SAFETY: forwarded to `headers_from_c`, whose contract the caller upholds.
    item.headers = unsafe { headers_from_c(task.headers, task.header_count) };
    // SAFETY: forwarded to `strings_from_c`, whose contract the caller upholds
    // for every list.
    item.shortlink_host_list = unsafe {
        strings_from_c(
            task.shortlink_host_list.items,
            task.shortlink_host_list.count,
        )
    };
    item.shortlink_fallback_hostlist = unsafe {
        strings_from_c(
            task.shortlink_fallback_hostlist.items,
            task.shortlink_fallback_hostlist.count,
        )
    };
    item.longlink_host_list =
        unsafe { strings_from_c(task.longlink_host_list.items, task.longlink_host_list.count) };
    item.minorlong_host_list = unsafe {
        strings_from_c(
            task.minorlong_host_list.items,
            task.minorlong_host_list.count,
        )
    };
    item.quic_host_list =
        unsafe { strings_from_c(task.quic_host_list.items, task.quic_host_list.count) };
    item.max_minorlinks = task.max_minorlinks;
    item.function = unsafe { cstr::ptr_to_str_or_empty(task.function) }.to_owned();
    item.cgi_prefix = unsafe { cstr::ptr_to_str_or_empty(task.cgi_prefix) }.to_owned();
    item.redirect_type = match task.redirect_type {
        1 => HostRedirectType::BareToHttps,
        2 => HostRedirectType::HttpToHttps,
        3 => HostRedirectType::NewHost,
        _ => HostRedirectType::None,
    };
    item.client_sequence_id = task.client_sequence_id;
    item
}

/// Pushes `value` onto `strings`, handing back the pointer C reads.
///
/// The pointer stays valid while `strings` lives: a `CString` owns its bytes on
/// the heap, so growing the `Vec` moves the `CString` and not what it points at.
fn push_cstr(strings: &mut Vec<CString>, value: &str) -> *const c_char {
    strings.push(c_string(value));
    strings
        .last()
        .map_or(std::ptr::null(), |value| value.as_ptr())
}

/// Pushes `values` onto `strings`, handing back the [`MarsStnStrings`] C reads.
fn push_list(
    strings: &mut Vec<CString>,
    lists: &mut Vec<Vec<*const c_char>>,
    values: &[String],
) -> MarsStnStrings {
    let mut items = Vec::with_capacity(values.len());
    for value in values {
        items.push(push_cstr(strings, value));
    }
    lists.push(items);
    let items = lists
        .last()
        .map_or(std::ptr::null(), |items| items.as_ptr());
    MarsStnStrings {
        items,
        count: values.len() as c_uint,
    }
}

/// A [`MarsStnTask`] a question carries, and the strings it points at: the
/// pointers of the struct are only good while the strings live, so the two
/// travel together.
struct TaskView {
    /// Every string of the task.
    strings: Vec<CString>,
    /// One array of pointers per list of the task.
    lists: Vec<Vec<*const c_char>>,
    /// The headers, whose strings are in `strings` too.
    headers: Vec<MarsStnHeader>,
    /// The struct the caller is handed.
    raw: MarsStnTask,
}

impl TaskView {
    /// The task of a question, as a pointer the C side reads.
    ///
    /// The strings, lists and headers the struct points at are this view's own,
    /// which is why they are read here: the view has to outlive the question,
    /// so the two travel together.
    fn task(&self) -> *const MarsStnTask {
        let _ = (self.strings.len(), self.lists.len(), self.headers.len());
        addr_of!(self.raw)
    }

    /// The task as the C side reads it.
    fn of(task: &Task) -> Self {
        let mut strings = Vec::new();
        let mut lists = Vec::new();
        let mut headers = Vec::with_capacity(task.headers.len());
        for (name, value) in &task.headers {
            headers.push(MarsStnHeader {
                name: push_cstr(&mut strings, name),
                value: push_cstr(&mut strings, value),
            });
        }
        let raw = MarsStnTask {
            taskid: task.taskid,
            cmdid: task.cmdid,
            channel_id: task.channel_id,
            channel_select: task.channel_select,
            transport_protocol: task.transport_protocol,
            cgi: push_cstr(&mut strings, &task.cgi),
            send_only: c_int::from(task.send_only),
            need_authed: c_int::from(task.need_authed),
            limit_flow: c_int::from(task.limit_flow),
            limit_frequency: c_int::from(task.limit_frequency),
            network_status_sensitive: c_int::from(task.network_status_sensitive),
            channel_strategy: task.channel_strategy,
            priority: task.priority,
            retry_count: task.retry_count,
            server_process_cost: task.server_process_cost,
            total_timeout: task.total_timeout,
            long_polling: c_int::from(task.long_polling),
            long_polling_timeout: task.long_polling_timeout,
            report_arg: push_cstr(&mut strings, &task.report_arg),
            channel_name: push_cstr(&mut strings, &task.channel_name),
            group_name: push_cstr(&mut strings, &task.group_name),
            user_id: push_cstr(&mut strings, &task.user_id),
            protocol: task.protocol,
            headers: headers.as_ptr(),
            header_count: headers.len() as c_uint,
            shortlink_host_list: push_list(&mut strings, &mut lists, &task.shortlink_host_list),
            shortlink_fallback_hostlist: push_list(
                &mut strings,
                &mut lists,
                &task.shortlink_fallback_hostlist,
            ),
            longlink_host_list: push_list(&mut strings, &mut lists, &task.longlink_host_list),
            minorlong_host_list: push_list(&mut strings, &mut lists, &task.minorlong_host_list),
            quic_host_list: push_list(&mut strings, &mut lists, &task.quic_host_list),
            max_minorlinks: task.max_minorlinks,
            function: push_cstr(&mut strings, &task.function),
            cgi_prefix: push_cstr(&mut strings, &task.cgi_prefix),
            redirect_type: task.redirect_type as c_int,
            client_sequence_id: task.client_sequence_id,
        };
        Self {
            strings,
            lists,
            headers,
            raw,
        }
    }
}

/// A [`MarsStnCgiProfile`] a question carries, and the string it points at.
struct ProfileView {
    /// The network the connect was made on.
    strings: Vec<CString>,
    /// The struct the caller is handed.
    raw: MarsStnCgiProfile,
}

impl ProfileView {
    /// The profile of a question, as a pointer the C side reads: `raw.nettype`
    /// points into `strings`, so the two travel together.
    fn profile(&self) -> *const MarsStnCgiProfile {
        let _ = self.strings.len();
        addr_of!(self.raw)
    }

    /// The connect as the C side reads it.
    fn of(profile: &CgiProfile) -> Self {
        let mut strings = Vec::new();
        let raw = MarsStnCgiProfile {
            start_time: profile.start_time,
            start_connect_time: profile.start_connect_time,
            connect_successful_time: profile.connect_successful_time,
            start_send_packet_time: profile.start_send_packet_time,
            send_packet_finished_time: profile.send_packet_finished_time,
            start_read_packet_time: profile.start_read_packet_time,
            read_packet_finished_time: profile.read_packet_finished_time,
            channel_type: profile.channel_type,
            transport_protocol: profile.transport_protocol,
            rtt: profile.rtt,
            nettype: push_cstr(&mut strings, &profile.nettype),
        };
        Self { strings, raw }
    }
}

/// A [`MarsStnDnsProfile`] a question carries, and the string it points at.
struct DnsView {
    /// The host the question was about.
    strings: Vec<CString>,
    /// The struct the caller is handed.
    raw: MarsStnDnsProfile,
}

impl DnsView {
    /// The dns profile of a question, as a pointer the C side reads:
    /// `raw.host` points into `strings`, so the two travel together.
    fn dns(&self) -> *const MarsStnDnsProfile {
        let _ = self.strings.len();
        addr_of!(self.raw)
    }

    /// The question as the C side reads it.
    fn of(profile: &DnsProfile) -> Self {
        let mut strings = Vec::new();
        let raw = MarsStnDnsProfile {
            start_time: profile.start_time,
            end_time: profile.end_time,
            host: push_cstr(&mut strings, &profile.host),
            err_type: profile.err_type as c_int,
            err_code: profile.err_code,
            dnstype: profile.dnstype as c_int,
        };
        Self { strings, raw }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mars_stn::PrepareProfile;
    use std::ffi::CString;
    use std::sync::Mutex;

    /// What a question said, in the terms a test reads. The strings and buffers
    /// of a question are only good while it is being answered, so what is
    /// written down is what they said and not where they were.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Seen {
        kind: MarsStnQuestionKind,
        taskid: u32,
        cmdid: u32,
        channel_id: String,
        send: i64,
        recv: i64,
        ip: String,
        host: String,
        port: u16,
        err_type: c_int,
        err_code: c_int,
        net_status_all: c_int,
        net_status_longlink: c_int,
        link_status: c_int,
        body: Vec<u8>,
        profile_json: String,
        /// Whether the question carried a [`MarsStnDnsProfile`] — the readings
        /// of it are the dns profile's own test's to check.
        has_dns: bool,
    }

    impl Default for Seen {
        fn default() -> Self {
            Self {
                kind: MarsStnQuestionKind::Nothing,
                taskid: 0,
                cmdid: 0,
                channel_id: String::new(),
                send: 0,
                recv: 0,
                ip: String::new(),
                host: String::new(),
                port: 0,
                err_type: 0,
                err_code: 0,
                net_status_all: 0,
                net_status_longlink: 0,
                link_status: 0,
                body: Vec::new(),
                profile_json: String::new(),
                has_dns: false,
            }
        }
    }

    /// An app that answers what a test tells it to, and writes down what it was
    /// asked. It is leaked, because `ctx` has to outlive the questions.
    struct App {
        asked: &'static Mutex<Vec<Seen>>,
        answer: MarsStnAnswer,
    }

    /// The [`MarsStnAsk`] of an [`App`]: `ctx` is one.
    extern "C" fn ask(
        ctx: *mut c_void,
        question: *const MarsStnQuestion,
        answer: *mut MarsStnAnswer,
    ) {
        // SAFETY: `ctx` is the leaked `App` the test handed over, and the
        // question and answer are the ones the caller owns for the call.
        let (app, question, answer) = unsafe { (&*(ctx as *const App), &*question, &mut *answer) };
        // SAFETY: every pointer a question carries is one the caller handed
        // over, and by `MarsStnAsk`'s contract it is alive for this call.
        let seen = unsafe {
            Seen {
                kind: question.kind,
                taskid: question.taskid,
                cmdid: question.cmdid,
                channel_id: cstr::ptr_to_str_or_empty(question.channel_id).to_owned(),
                send: question.send,
                recv: question.recv,
                ip: cstr::ptr_to_str_or_empty(question.ip).to_owned(),
                host: cstr::ptr_to_str_or_empty(question.host).to_owned(),
                port: question.port,
                err_type: question.err_type,
                err_code: question.err_code,
                net_status_all: question.net_status_all,
                net_status_longlink: question.net_status_longlink,
                link_status: question.link_status,
                body: bytes_from_c(question.body, question.body_count),
                profile_json: cstr::ptr_to_str_or_empty(question.profile_json).to_owned(),
                has_dns: !question.dns.is_null(),
            }
        };
        app.asked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(seen);
        *answer = app.answer;
    }

    /// One [`CApp`] answering `answer` to everything, and the questions it was
    /// asked.
    fn app_of(answer: MarsStnAnswer) -> (CApp, &'static Mutex<Vec<Seen>>) {
        let asked: &'static Mutex<Vec<Seen>> = Box::leak(Box::new(Mutex::new(Vec::new())));
        let app: &'static App = Box::leak(Box::new(App { asked, answer }));
        (
            CApp {
                ask,
                ctx: app as *const App as usize,
            },
            asked,
        )
    }

    #[test]
    fn a_question_nobody_asked_is_the_defaults() {
        let question = MarsStnQuestion::default();
        assert_eq!(question.kind, MarsStnQuestionKind::Nothing);
        assert!(question.host.is_null());
        assert!(question.task.is_null());
        assert_eq!(question.taskid, 0);
    }

    #[test]
    fn an_answer_nobody_wrote_is_nothing() {
        let answer = MarsStnAnswer::default();
        assert_eq!(answer.kind, MarsStnAnswerKind::Nothing);
        assert!(answer.ips.is_null());
        assert!(answer.bytes.is_null());
    }

    #[test]
    fn a_yes_is_what_the_app_is_asked_and_what_it_answers() {
        let (mut app, asked) = app_of(MarsStnAnswer {
            kind: MarsStnAnswerKind::Yes,
            yes: 1,
            ..Default::default()
        });
        assert!(app.makesure_authed("long.host.com", "user"));
        assert!(app.identify_response("longlink", b"answer", b"hash"));
        assert_eq!(
            asked.lock().unwrap().clone(),
            vec![
                Seen {
                    kind: MarsStnQuestionKind::MakesureAuthed,
                    host: "long.host.com".to_owned(),
                    ..Default::default()
                },
                Seen {
                    kind: MarsStnQuestionKind::IdentifyResponse,
                    channel_id: "longlink".to_owned(),
                    body: b"answer".to_vec(),
                    ..Default::default()
                }
            ]
        );
    }

    #[test]
    fn an_app_that_answers_nothing_gets_stns_own_answers() {
        let (mut app, asked) = app_of(MarsStnAnswer::default());
        // logged in, an answer nobody read is a good one, a task nobody can
        // encode cannot be started, a check that never happened is asked for
        // again on the next connect
        assert!(app.makesure_authed("host", "user"));
        assert_eq!(
            app.req2buf(7, "user", 0, "host", 0),
            Err(LOCAL_START_TASK_FAIL)
        );
        assert_eq!(
            app.buf2resp(7, "user", b"answer", 0),
            (0, TaskFailHandleType::Normal)
        );
        assert_eq!(app.report_task_limited(0, &Task::new(7, 8)), 0);
        assert_eq!(
            app.identify_check_buffer("longlink", 0),
            IdentifyBuffer::next(Vec::new())
        );
        assert!(!app.identify_response("longlink", b"answer", b"hash"));
        assert!(app.on_new_dns("host", false, &ExtraInfo::new()).is_empty());
        assert!(app.net_check_shortlink_hosts().is_empty());
        assert_eq!(asked.lock().unwrap().len(), 8);
    }

    #[test]
    fn the_bytes_the_app_answers_with_are_what_a_task_sends() {
        let body = b"body";
        let (mut app, _) = app_of(MarsStnAnswer {
            kind: MarsStnAnswerKind::Encoded,
            bytes: body.as_ptr(),
            byte_count: body.len() as c_uint,
            ..Default::default()
        });
        assert_eq!(app.req2buf(7, "user", 1, "host", 3), Ok(b"body".to_vec()));

        let (mut failed, _) = app_of(MarsStnAnswer {
            kind: MarsStnAnswerKind::Failed,
            error_code: -7,
            ..Default::default()
        });
        assert_eq!(failed.req2buf(7, "user", 1, "host", 3), Err(-7));
    }

    #[test]
    fn the_handle_the_app_answers_buf2resp_with_is_what_stn_does_about_the_task() {
        let (mut app, _) = app_of(MarsStnAnswer {
            kind: MarsStnAnswerKind::Decoded,
            handle: -14,
            error_code: -15,
            ..Default::default()
        });
        assert_eq!(
            app.buf2resp(7, "user", b"answer", 0),
            (-15, TaskFailHandleType::TaskEnd)
        );
    }

    #[test]
    fn the_ips_the_app_answers_with_are_the_ones_stn_tries() {
        let ip = CString::new("1.2.3.4").unwrap();
        let ips = [ip.as_ptr()];
        let (mut app, _) = app_of(MarsStnAnswer {
            kind: MarsStnAnswerKind::Ips,
            ips: ips.as_ptr(),
            ip_count: ips.len() as c_uint,
            ..Default::default()
        });
        assert_eq!(
            app.on_new_dns("host", true, &ExtraInfo::new()),
            vec!["1.2.3.4".to_owned()]
        );
        assert_eq!(app.net_check_shortlink_hosts(), vec!["1.2.3.4".to_owned()]);
    }

    #[test]
    fn the_buffer_the_app_answers_with_is_the_check_a_link_is_used_with() {
        let buffer = b"identify";
        let hash = b"hash";
        let (mut app, _) = app_of(MarsStnAnswer {
            kind: MarsStnAnswerKind::Identified,
            mode: 0,
            bytes: buffer.as_ptr(),
            byte_count: buffer.len() as c_uint,
            hash: hash.as_ptr(),
            hash_count: hash.len() as c_uint,
            cmdid: 17,
            ..Default::default()
        });
        assert_eq!(
            app.identify_check_buffer("longlink", 6),
            IdentifyBuffer::now(b"identify".to_vec(), b"hash".to_vec(), 17)
        );
    }

    #[test]
    fn a_task_the_app_is_asked_about_is_one_it_can_read() {
        let mut task = Task::new(7, 8);
        task.cgi = "/cgi-bin/7".to_owned();
        task.user_id = "user".to_owned();
        task.headers.insert("name".to_owned(), "value".to_owned());
        task.longlink_host_list = vec!["long.host.com".to_owned()];
        let view = TaskView::of(&task);
        // SAFETY: every string and list of the view is alive, and the header is
        // one the view owns.
        assert_eq!(
            unsafe {
                strings_from_c(
                    view.raw.longlink_host_list.items,
                    view.raw.longlink_host_list.count,
                )
            },
            vec!["long.host.com".to_owned()]
        );
        assert_eq!(view.raw.header_count, 1);
        let header = view.headers[0];
        // SAFETY: forwarded to `ptr_to_str_or_empty`, whose contract the view
        // upholds.
        assert_eq!(unsafe { cstr::ptr_to_str_or_empty(header.name) }, "name");
        assert_eq!(unsafe { cstr::ptr_to_str_or_empty(header.value) }, "value");

        // … and one the C side hands back is the same task
        // SAFETY: every string and list of the view is alive for this call.
        let read = unsafe { task_from_c(&view.raw) };
        assert_eq!(read, task);
    }

    #[test]
    fn a_task_with_nothing_in_it_is_one_the_queues_refuse() {
        // SAFETY: nulls and zero counts are explicitly allowed by the contract.
        let task = unsafe { task_from_c(&MarsStnTask::default()) };
        assert_eq!(task.channel_select, 0, "no channel to go out on");
        assert_eq!(task.retry_count, 0, "and no retry either");
        assert!(task.cgi.is_empty());
        assert!(task.headers.is_empty());
        assert!(task.longlink_host_list.is_empty());
    }

    #[test]
    fn no_lists_are_empty_ones() {
        // SAFETY: null is explicitly allowed by the contract.
        assert!(unsafe { strings_from_c(std::ptr::null(), 0) }.is_empty());
        // SAFETY: null is explicitly allowed by the contract.
        assert!(unsafe { bytes_from_c(std::ptr::null(), 0) }.is_empty());
        // SAFETY: null is explicitly allowed by the contract.
        assert!(unsafe { headers_from_c(std::ptr::null(), 0) }.is_empty());
    }

    /// What a task sent and what came back, which is the one question the app
    /// is told about without being asked anything.
    #[test]
    fn what_went_out_and_came_in_is_what_the_app_is_told() {
        let (mut app, asked) = app_of(MarsStnAnswer::default());
        app.traffic_data(100, 200);
        assert_eq!(
            asked.lock().unwrap().clone(),
            vec![Seen {
                kind: MarsStnQuestionKind::TrafficData,
                send: 100,
                recv: 200,
                ..Default::default()
            }]
        );
    }

    /// A push is something no task asked for, so what it carries is the link it
    /// came in on and the body that came with it.
    #[test]
    fn a_push_is_handed_over_with_the_link_it_came_in_on() {
        let (mut app, asked) = app_of(MarsStnAnswer::default());
        app.on_push("longlink", 12, 7, b"pushed");
        assert_eq!(
            asked.lock().unwrap().clone(),
            vec![Seen {
                kind: MarsStnQuestionKind::OnPush,
                channel_id: "longlink".to_owned(),
                cmdid: 12,
                taskid: 7,
                body: b"pushed".to_vec(),
                ..Default::default()
            }]
        );
    }

    /// The two network errors name the peer they happened on: an ip and a port
    /// for the long link, and the host the short one was talking to as well.
    #[test]
    fn the_two_network_errors_name_the_peer_they_happened_on() {
        let (mut app, asked) = app_of(MarsStnAnswer::default());
        app.on_long_link_network_error(ErrCmdType::Local, -5, "1.2.3.4", 80);
        app.on_short_link_network_error(ErrCmdType::Socket, -6, "1.2.3.4", "short.host.com", 443);
        assert_eq!(
            asked.lock().unwrap().clone(),
            vec![
                Seen {
                    kind: MarsStnQuestionKind::LongLinkNetworkError,
                    ip: "1.2.3.4".to_owned(),
                    port: 80,
                    err_type: ErrCmdType::Local as c_int,
                    err_code: -5,
                    ..Default::default()
                },
                Seen {
                    kind: MarsStnQuestionKind::ShortLinkNetworkError,
                    ip: "1.2.3.4".to_owned(),
                    host: "short.host.com".to_owned(),
                    port: 443,
                    err_type: ErrCmdType::Socket as c_int,
                    err_code: -6,
                    ..Default::default()
                }
            ]
        );
    }

    /// The status of the network and of the long link, in the ints the header
    /// spells out.
    #[test]
    fn the_status_of_the_links_is_what_the_app_is_told() {
        let (mut app, asked) = app_of(MarsStnAnswer::default());
        app.report_connect_status(NetStatus::Connected, NetStatus::Connecting);
        app.on_long_link_status_change(LongLinkStatus::Connected);
        assert_eq!(
            asked.lock().unwrap().clone(),
            vec![
                Seen {
                    kind: MarsStnQuestionKind::ReportConnectStatus,
                    net_status_all: NetStatus::Connected as c_int,
                    net_status_longlink: NetStatus::Connecting as c_int,
                    ..Default::default()
                },
                Seen {
                    kind: MarsStnQuestionKind::LongLinkStatusChange,
                    link_status: LongLinkStatus::Connected as c_int,
                    ..Default::default()
                }
            ]
        );
    }

    /// A sync is asked for and nothing is carried with it: the app is told to
    /// sync, and everything it needs to do so is its own.
    #[test]
    fn a_sync_is_asked_for_and_nothing_comes_with_it() {
        let (mut app, asked) = app_of(MarsStnAnswer::default());
        app.request_sync();
        assert_eq!(
            asked.lock().unwrap().clone(),
            vec![Seen {
                kind: MarsStnQuestionKind::RequestSync,
                ..Default::default()
            }]
        );
    }

    /// The report of a task that is over is the json the app parses, in the one
    /// spelling both seams hand it over.
    #[test]
    fn a_task_that_is_over_is_reported_as_the_json_the_app_parses() {
        let profile = TaskProfile::new_at(
            0,
            Task::new(7, 8),
            PrepareProfile {
                start_task_call_time: 0,
                begin_process_hosts_time: 0,
                end_process_hosts_time: 0,
            },
        );
        let (mut app, asked) = app_of(MarsStnAnswer::default());
        app.report_task_profile(&profile);
        assert_eq!(
            asked.lock().unwrap().clone(),
            vec![Seen {
                kind: MarsStnQuestionKind::ReportTaskProfile,
                profile_json: task_profile_json(&profile),
                ..Default::default()
            }]
        );
    }

    /// A dns question is reported as the profile the C side reads, whose fields
    /// are the dns profile's own test's to check.
    #[test]
    fn a_dns_question_is_reported_as_one_the_c_side_reads() {
        let (mut app, asked) = app_of(MarsStnAnswer::default());
        app.report_dns_profile(&DnsProfile::new_at(1, "host"));
        assert_eq!(
            asked.lock().unwrap().clone(),
            vec![Seen {
                kind: MarsStnQuestionKind::ReportDnsProfile,
                has_dns: true,
                ..Default::default()
            }]
        );
    }

    #[test]
    fn a_profile_and_a_dns_question_are_ones_the_c_side_reads() {
        let profile = CgiProfile {
            nettype: "wifi".to_owned(),
            rtt: 12,
            ..CgiProfile::default()
        };
        let view = ProfileView::of(&profile);
        // SAFETY: the string of the view is alive.
        assert_eq!(
            unsafe { cstr::ptr_to_str_or_empty(view.raw.nettype) },
            "wifi"
        );
        assert_eq!(view.raw.rtt, 12);

        let dns = DnsProfile::new_at(1, "host");
        let view = DnsView::of(&dns);
        // SAFETY: the string of the view is alive.
        assert_eq!(unsafe { cstr::ptr_to_str_or_empty(view.raw.host) }, "host");
        assert_eq!(view.raw.dnstype, 1);
    }
}
