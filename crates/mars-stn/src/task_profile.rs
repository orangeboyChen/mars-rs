//! `mars/stn/task_profile.h` — the outcome of a finished task.
//!
//! The C++ `TaskProfile` is the whole record of a task that is running: the
//! [`crate::Task`] it came from, the prepare/transfer/connect profiles, the
//! history of its retries, and the fields `stn_logic` fills in as it goes.
//! What is here is what the code that reads a *finished* task needs: the two
//! error fields, the two times, and the two numbers of the transfer profile
//! that `TaskProfile::GetFailStep()` looks at. The rest of the struct comes
//! with the code that runs the tasks.
//!
//! `ErrCmdType` is the one of `mars/stn/stn.h`. Its companion is not an enum
//! there either: `err_code` is an `int` that carries either a server code or
//! one of the `kEctLocal*` values, so those are constants here.
//!
//! The task that is *running* is here too, now that the code that runs the
//! tasks is: [`TaskProfile`] is the record of one, [`TransferProfile`] is what
//! its run filled in, and [`PrepareProfile`] is what the caller did before it
//! handed the task over. A running task is not the link it runs on — that is
//! the host's — but a [`RunId`] it answers for, which is what the C++'s
//! `intptr_t running_id` is for. The three free functions of `task_profile.cc`
//! are here as well: [`read_write_timeout`], [`first_pkg_timeout`] and
//! [`compare_task`].
//!
//! The other struct of that header is [`ConnectProfile`], which is the record
//! of one connect — a long link's or a short link's — and it comes with the
//! code that makes the connect: [`ConnectProfile::reset`] is what the C++
//! `Reset()` clears, and the fields are the ones that code fills in. The
//! timings that came with the later versions of mars (the tls handshake, the
//! QUIC ones, mmtls) are not here yet; they come with the code that reads
//! them.

use std::cmp::Ordering;

use mars_comm::tickcount::gettickcount;
use mars_comm::{LocalIpStack, ProxyInfo};

use crate::config::{
    BASE_FIRST_PACKAGE_GPRS_TIMEOUT, BASE_FIRST_PACKAGE_WIFI_TIMEOUT,
    DYN_TIME_FIRST_PACKAGE_GPRS_TIMEOUT, DYN_TIME_FIRST_PACKAGE_WIFI_TIMEOUT,
    MAX_FIRST_PACKAGE_GPRS_TIMEOUT, MAX_FIRST_PACKAGE_WIFI_TIMEOUT, MAX_RECV_LEN, MOBILE_MIN_RATE,
    MOBILE_TASK_DELAY, WIFI_MIN_RATE, WIFI_TASK_DELAY,
};
use crate::dynamic_timeout::DynamicTimeoutStatus;
use crate::net_source::{TimeoutSource, DEFAULT_QUIC_RW_TIMEOUT_MS};
use crate::simple_ipport_sort::{IpPortItem, IpSourceType};
use crate::socket_operator::SocketFd;
use crate::task::Task;

/// `ErrCmdType` of `mars/stn/stn.h` — how a task failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrCmdType {
    /// `kEctOK`
    #[default]
    Ok = 0,
    /// `kEctFalse`
    False = 1,
    /// `kEctDial`
    Dial = 2,
    /// `kEctDns`
    Dns = 3,
    /// `kEctSocket`
    Socket = 4,
    /// `kEctHttp`
    Http = 5,
    /// `kEctNetMsgXP`
    NetMsgXp = 6,
    /// `kEctEnDecode`
    EnDecode = 7,
    /// `kEctServer`
    Server = 8,
    /// `kEctLocal`
    Local = 9,
    /// `kEctCanceld`
    Canceld = 10,
}

/// `kEctLocalTaskTimeout` — what `TaskProfile::err_code` carries when the task
/// timed out locally.
pub const LOCAL_TASK_TIMEOUT: i32 = -1;
/// `kEctLocalTaskRetry`.
pub const LOCAL_TASK_RETRY: i32 = -2;
/// `kEctLocalStartTaskFail`.
pub const LOCAL_START_TASK_FAIL: i32 = -3;
/// `kEctLocalAntiAvalanche`.
pub const LOCAL_ANTI_AVALANCHE: i32 = -4;
/// `kEctLocalChannelSelect`.
pub const LOCAL_CHANNEL_SELECT: i32 = -5;
/// `kEctLocalNoNet`.
pub const LOCAL_NO_NET: i32 = -6;
/// `kEctLocalCancel`.
pub const LOCAL_CANCEL: i32 = -7;
/// `kEctLocalClear`.
pub const LOCAL_CLEAR: i32 = -8;
/// `kEctLocalReset`.
pub const LOCAL_RESET: i32 = -9;
/// `kEctLocalTaskParam`.
pub const LOCAL_TASK_PARAM: i32 = -12;
/// `kEctLocalCgiFrequcencyLimit`.
pub const LOCAL_CGI_FREQUENCY_LIMIT: i32 = -13;
/// `kEctLocalChannelID`.
pub const LOCAL_CHANNEL_ID: i32 = -14;
/// `kEctLocalLongLinkReleased`.
pub const LOCAL_LONG_LINK_RELEASED: i32 = -15;
/// `kEctLocalLongLinkUnAvailable`.
pub const LOCAL_LONG_LINK_UNAVAILABLE: i32 = -16;
/// `kEctLongFirstPkgTimeout`.
pub const LONG_FIRST_PKG_TIMEOUT: i32 = -500;
/// `kEctLongPkgPkgTimeout`.
pub const LONG_PKG_PKG_TIMEOUT: i32 = -501;
/// `kEctLongReadWriteTimeout`.
pub const LONG_READ_WRITE_TIMEOUT: i32 = -502;
/// `kEctLongTaskTimeout`.
pub const LONG_TASK_TIMEOUT: i32 = -503;

/// `kEctHttpFirstPkgTimeout` — a short-link task that did not get the first
/// package of its answer in time. The same number as
/// [`LONG_FIRST_PKG_TIMEOUT`]: the two enums of `stn.h` share it.
pub const HTTP_FIRST_PKG_TIMEOUT: i32 = -500;
/// `kEctHttpPkgPkgTimeout`.
pub const HTTP_PKG_PKG_TIMEOUT: i32 = -501;
/// `kEctHttpReadWriteTimeout`.
pub const HTTP_READ_WRITE_TIMEOUT: i32 = -502;
/// `kEctHttpLongPollingTimeout`.
pub const HTTP_LONG_POLLING_TIMEOUT: i32 = -503;
/// `kEctHandshakeMisunderstand` — a task whose run gets another try because the
/// two ends did not agree on the handshake.
pub const HANDSHAKE_MISUNDERSTAND: i32 = -10096;

/// `TaskProfile::ComputeTaskTimeout` — how long a wait there is for an answer
/// that nobody said how long takes (`15 * 1000`), and the margin the whole task
/// gets on top of it (`5 * 1000`).
pub const BASE_TASK_TIMEOUT: u64 = 15 * 1000;
pub const TASK_TIMEOUT_MARGIN: u64 = 5 * 1000;

/// `TaskFailHandleType` of `mars/stn/stn.h` — what the app is told to do about
/// a task that failed.
///
/// The C++ hands it to the callback as a plain `int`, and not every value it
/// carries is one of these; the port uses the enum everywhere it is produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskFailHandleType {
    /// `kTaskFailHandleNormal` / `kTaskFailHandleNoError`
    #[default]
    Normal = 0,
    /// `kTaskFailHandleDefault`
    Default = -1,
    /// `kTaskFailHandleRetryAllTasks`
    RetryAllTasks = -12,
    /// `kTaskFailHandleSessionTimeout`
    SessionTimeout = -13,
    /// `kTaskFailHandleTaskEnd` — the task is over, do not retry it.
    TaskEnd = -14,
    /// `kTaskFailHandleTaskTimeout`
    TaskTimeout = -15,
    /// `kTaskSlientHandleTaskEnd`
    SlientTaskEnd = -16,
}

/// `NoopProfile` — one heartbeat of a link: how long after the last one it
/// went out, how long after that it *actually* went out (an alarm that fires
/// late is how a dozing network shows itself), how long its answer took, and
/// whether there was one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoopProfile {
    /// `success`
    pub success: bool,
    /// `noop_internal` — the interval the alarm was set to.
    pub noop_internal: u64,
    /// `noop_actual_internal` — the interval it really was.
    pub noop_actual_internal: u64,
    /// `noop_cost` — how long the answer took.
    pub noop_cost: u64,
    /// `noop_starttime` — when the heartbeat went out.
    pub noop_starttime: u64,
}

/// `ConnectProfile` — the record of one connect.
///
/// The C++ fills it in from two places: the link that is being made
/// (`__RunConnect`) and the run that came before it, whose
/// [`ConnectProfile::disconn_errcode`] is the [`ConnectProfile::conn_reason`]
/// of this one. A profile whose [`ConnectProfile::disconn_time`] is not `0` is
/// one a link has finished with, which is what the C++ broadcasts.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectProfile {
    /// `net_type` — `getCurrNetLabel`, the network the connect was made on.
    pub net_type: String,
    /// `nettype_for_report` — `-1` until something sets it.
    pub nettype_for_report: i32,
    /// `ispcode` — the isp code of a mobile network, which is what
    /// `getCurrNetLabel` writes into `net_type` for one: the label of a mobile
    /// network *is* the isp code, as a number.
    pub ispcode: i32,
    /// `task_id` — the task the connect was made for.
    pub task_id: u32,
    /// `cgi` — what the task asked for.
    pub cgi: String,
    /// `start_time` — when the run began.
    pub start_time: u64,
    /// `dns_time` — when the ips were asked for.
    pub dns_time: u64,
    /// `dns_endtime` — when the last of them came back.
    pub dns_endtime: u64,
    /// `ip_items` — the candidates, in the order they were tried.
    pub ip_items: Vec<IpPortItem>,
    /// `conn_reason` — why the connect is being made: the previous run's
    /// `disconn_errcode`.
    pub conn_reason: i32,
    /// `conn_time` — when the connect finished.
    pub conn_time: u64,
    /// `conn_errcode` — why it failed, in the platform's words.
    pub conn_errcode: i32,
    /// `rw_errcode` — why reading and writing failed, for a link that got that
    /// far.
    pub rw_errcode: i32,
    /// `ip`, `port`, `host` — the pair that won.
    pub ip: String,
    pub port: u16,
    pub host: String,
    /// `ip_type` — where the pair came from.
    pub ip_type: IpSourceType,
    /// `conn_rtt` — how long the pair that won took to answer.
    pub conn_rtt: u32,
    /// `conn_cost` — how long the whole connect took.
    pub conn_cost: u64,
    /// `tryip_count` — how many candidates were tried.
    pub tryip_count: i32,
    /// `is0rtt` — whether the connect was a 0-rtt one.
    pub is0rtt: i32,
    /// `proxy_info` — the proxy the connect went through, if any.
    pub proxy_info: ProxyInfo,
    /// `local_ip`, `local_port` — the near end of the socket, which is
    /// `getsockname` and therefore the host's to answer.
    pub local_ip: String,
    pub local_port: u16,
    /// `ip_index` — which candidate won; `-1` for a connect that never had
    /// one.
    pub ip_index: i32,
    /// `transport_protocol` — one of the `Task::TRANSPORT_PROTOCOL*` values.
    pub transport_protocol: i32,
    /// `link_type` — one of the `Task::CHANNEL_*` values.
    pub link_type: i32,
    /// `tried_443port` — whether a candidate on 443 was tried first.
    pub tried_443port: i32,
    /// `tried_80port` — whether a candidate on 80 was tried first.
    pub tried_80port: i32,
    /// `disconn_time` — when the link went away; `0` while it has not.
    pub disconn_time: u64,
    /// `disconn_errtype`.
    pub disconn_errtype: ErrCmdType,
    /// `disconn_errcode`.
    pub disconn_errcode: i32,
    /// `disconn_signal` — `getSignal` at the time the link went away.
    pub disconn_signal: i32,
    /// `nat64` — whether the local network is IPv6-only, which is what turns a
    /// v4 address into a NAT64 one.
    pub nat64: bool,
    /// `local_net_stack` — what the local network carries, which is what the
    /// nat64 flag above comes from. [`LocalIpStack`] is the C++'s `int`, with
    /// the same numbers.
    pub local_net_stack: LocalIpStack,
    /// `start_connect_time` — when the connect began.
    pub start_connect_time: u64,
    /// `connect_successful_time` — when it came back, socket or no socket.
    pub connect_successful_time: u64,
    /// `socket_fd` — the socket the connect came back with; a reused one is
    /// this too, with [`ConnectProfile::is_reused_fd`] saying so. A connect that
    /// opened its own writes it here only once the server has said to keep it,
    /// which is what the C++ does too: the run closes and hands back nothing but
    /// a socket that is named here.
    pub socket_fd: SocketFd,
    /// `is_reused_fd` — whether the socket came out of the pool rather than
    /// from a connect.
    pub is_reused_fd: bool,
    /// `connection_identify` — what the operator calls the socket in a log; one
    /// that came out of the pool is logged with `@REUSE` after it.
    pub connection_identify: String,
    /// `ipv6_connect_failed` — a v6 pair was tried first and the connect
    /// landed on a later one.
    pub ipv6_connect_failed: bool,
    /// `noop_profiles` — every heartbeat this link sent.
    pub noop_profiles: Vec<NoopProfile>,
    /// `tls_handshake_mismatch` — what a run keeps across a profile update.
    pub tls_handshake_mismatch: bool,
    /// `tls_handshake_success`.
    pub tls_handshake_success: bool,
    /// `tid` — `xlogger_tid`, which thread the run was on.
    pub tid: i64,
    /// `channel_type` — one of the `Task::CHANNEL_*` values, which is what a
    /// short link writes here once it has answered.
    pub channel_type: i32,
    /// `start_send_packet_time` — when the request went out.
    pub start_send_packet_time: u64,
    /// `send_request_cost` — how long the write took, which the C++ works out
    /// between the write and the first read.
    pub send_request_cost: u64,
    /// `start_read_packet_time` — when the read of the answer began.
    pub start_read_packet_time: u64,
    /// `read_packet_finished_time` — when the last read came back.
    pub read_packet_finished_time: u64,
    /// `recv_reponse_cost` — how long the whole read took, from its first byte.
    pub recv_reponse_cost: u64,
    /// `quic_rw_timeout_ms` — how long a read waits on a quic link; `5000` until
    /// something sets it.
    pub quic_rw_timeout_ms: u32,
    /// `quic_rw_timeout_source` — where the timeout above came from.
    pub quic_rw_timeout_source: TimeoutSource,
    /// `keepalive_timeout` — how long the server said the socket is good for.
    pub keepalive_timeout: u32,
    /// `is_fast_fallback_tcp` — a quic link that read `ENOTCONN` and so fell
    /// back to tcp.
    pub is_fast_fallback_tcp: i32,
}

impl ConnectProfile {
    /// `ConnectProfile()` — `Reset()`, which is a link that has not been made.
    pub fn new() -> Self {
        Self::default()
    }

    /// `Reset()` — every field back to what the C++ constructor gives it:
    /// `ip_index` is `-1`, `nettype_for_report` is `-1`, and the two tls flags
    /// are *not* cleared (a profile update keeps them, which is the one thing
    /// the C++'s `__UpdateProfile` carries over).
    pub fn reset(&mut self) {
        *self = Self {
            tls_handshake_mismatch: self.tls_handshake_mismatch,
            tls_handshake_success: self.tls_handshake_success,
            ..Self::default()
        };
    }

    /// Whether the link this profile is about has finished: the C++ broadcasts
    /// a profile only once `disconn_time` is set.
    pub fn is_finished(&self) -> bool {
        self.disconn_time != 0
    }
}

impl Default for ConnectProfile {
    fn default() -> Self {
        Self {
            net_type: String::new(),
            nettype_for_report: -1,
            ispcode: 0,
            task_id: 0,
            cgi: String::new(),
            start_time: 0,
            dns_time: 0,
            dns_endtime: 0,
            ip_items: Vec::new(),
            conn_reason: 0,
            conn_time: 0,
            conn_errcode: 0,
            rw_errcode: 0,
            ip: String::new(),
            port: 0,
            host: String::new(),
            ip_type: IpSourceType::Null,
            conn_rtt: 0,
            conn_cost: 0,
            tryip_count: 0,
            is0rtt: 0,
            proxy_info: ProxyInfo::none(),
            local_ip: String::new(),
            local_port: 0,
            ip_index: -1,
            transport_protocol: Task::TRANSPORT_PROTOCOL_TCP,
            link_type: Task::CHANNEL_LONG,
            tried_443port: 0,
            tried_80port: 0,
            disconn_time: 0,
            disconn_errtype: ErrCmdType::Ok,
            disconn_errcode: 0,
            disconn_signal: 0,
            nat64: false,
            local_net_stack: LocalIpStack::None,
            start_connect_time: 0,
            connect_successful_time: 0,
            socket_fd: SocketFd::INVALID,
            is_reused_fd: false,
            connection_identify: String::new(),
            ipv6_connect_failed: false,
            noop_profiles: Vec::new(),
            tls_handshake_mismatch: false,
            tls_handshake_success: false,
            tid: 0,
            channel_type: 0,
            start_send_packet_time: 0,
            send_request_cost: 0,
            start_read_packet_time: 0,
            read_packet_finished_time: 0,
            recv_reponse_cost: 0,
            quic_rw_timeout_ms: DEFAULT_QUIC_RW_TIMEOUT_MS,
            quic_rw_timeout_source: TimeoutSource::default(),
            keepalive_timeout: 0,
            is_fast_fallback_tcp: 0,
        }
    }
}

/// `TaskFailStep` — "do not insert or delete": the C++ turns the value into a
/// report key by adding it to an offset, so the discriminants are part of the
/// contract with the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskFailStep {
    /// `kStepSucc`
    Succ = 0,
    /// `kStepDns`
    Dns = 1,
    /// `kStepConnect`
    Connect = 2,
    /// `kStepFirstPkg`
    FirstPkg = 3,
    /// `kStepPkgPkg`
    PkgPkg = 4,
    /// `kStepDecode`
    Decode = 5,
    /// `kStepOther`
    Other = 6,
    /// `kStepTimeout`
    Timeout = 7,
    /// `kStepServer`
    Server = 8,
}

/// What a finished task reports.
///
/// `start_task_time` and `end_task_time` are `::gettickcount()` readings; the
/// other four fields are the ones `TaskProfile::GetFailStep()` and
/// `WeakNetworkLogic::OnTaskEvent()` read, with the defaults the C++
/// constructor gives them (`ip_index` is `-1`, `last_receive_pkg_time` is `0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskOutcome {
    /// `err_type`
    pub err_type: ErrCmdType,
    /// `err_code` — 0, a server code, or one of the `LOCAL_*` constants.
    pub err_code: i32,
    /// `transfer_profile.connect_profile.ip_index` — `-1` when the task never
    /// got an ip to connect to.
    pub ip_index: i32,
    /// `transfer_profile.last_receive_pkg_time` — `0` when no package ever
    /// arrived.
    pub last_receive_pkg_time: u64,
    /// `start_task_time`
    pub start_task_time: u64,
    /// `end_task_time` — 0 while the task has not finished.
    pub end_task_time: u64,
}

impl TaskOutcome {
    /// A task that succeeded: `err_type` is `kEctOK` and `err_code` is 0.
    pub fn new(start_task_time: u64, end_task_time: u64) -> Self {
        Self {
            err_type: ErrCmdType::Ok,
            err_code: 0,
            ip_index: -1,
            last_receive_pkg_time: 0,
            start_task_time,
            end_task_time,
        }
    }

    /// How the task failed, by the same rules as the C++: `err_type` first,
    /// then how far the task got.
    pub fn fail_step(&self) -> TaskFailStep {
        if self.err_type == ErrCmdType::Ok && self.err_code == 0 {
            return TaskFailStep::Succ;
        }
        if self.err_type == ErrCmdType::Dns {
            return TaskFailStep::Dns;
        }
        if self.ip_index == -1 {
            return TaskFailStep::Connect;
        }
        if self.last_receive_pkg_time == 0 {
            return TaskFailStep::FirstPkg;
        }
        if self.err_type == ErrCmdType::EnDecode {
            return TaskFailStep::Decode;
        }
        if self.err_type == ErrCmdType::Socket
            || self.err_type == ErrCmdType::Http
            || self.err_type == ErrCmdType::NetMsgXp
        {
            return TaskFailStep::PkgPkg;
        }
        if self.err_code == LOCAL_TASK_TIMEOUT {
            return TaskFailStep::Timeout;
        }
        if self.err_type == ErrCmdType::Server
            || (self.err_type == ErrCmdType::Ok && self.err_code != 0)
        {
            return TaskFailStep::Server;
        }
        TaskFailStep::Other
    }

    /// `end_task_time - start_task_time` — what the C++ compares against
    /// `GOOD_TASK_SPAN` and `WEAK_TASK_SPAN`.
    pub fn cost(&self) -> u64 {
        self.end_task_time.saturating_sub(self.start_task_time)
    }
}

/// `running_id` — which run of a task is out, in the words of whoever is
/// running it.
///
/// The C++ keeps an `intptr_t` to the worker and finds a task with it
/// (`__LocateBySeq`), because the worker is what answers. A run is the host's
/// here — the port has no sockets and no threads — so what the port keeps is
/// this: a number the host handed out when it started a run and hands back
/// with every answer, which is all a task manager needs of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunId(pub u64);

/// `PrepareProfile` — what the caller did before the task was handed over:
/// when it asked, and how long working out the hosts took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrepareProfile {
    /// `start_task_call_time` — when the app asked for the task.
    pub start_task_call_time: u64,
    /// `begin_process_hosts_time`.
    pub begin_process_hosts_time: u64,
    /// `end_process_hosts_time`.
    pub end_process_hosts_time: u64,
}

impl PrepareProfile {
    /// `PrepareProfile()` — `Reset()`, which is a caller that has just asked.
    pub fn new() -> Self {
        Self::new_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn new_at(now: u64) -> Self {
        Self {
            start_task_call_time: now,
            begin_process_hosts_time: 0,
            end_process_hosts_time: 0,
        }
    }

    /// `Reset()` — what a retry does to it, which is to say the task was asked
    /// for again.
    pub fn reset_at(&mut self, now: u64) {
        *self = Self::new_at(now);
    }
}

impl Default for PrepareProfile {
    fn default() -> Self {
        Self::new()
    }
}

/// `TransferProfile` — what a run of a task filled in: the connect it was made
/// on, the five readings of how far it got, and the two timeouts it was given.
///
/// [`TransferProfile::task`] is a copy, which is what the C++ made it too —
/// a reference there outlived the task it pointed at.
#[derive(Debug, Clone, PartialEq)]
pub struct TransferProfile {
    /// `task` — the task as it was when this run was started; a retry hands it
    /// the fallback hosts, and that is this copy's to keep.
    pub task: Task,
    /// `connect_profile` — the connect this run was made on.
    pub connect_profile: ConnectProfile,
    /// `loop_start_task_time` — when the run was started.
    pub loop_start_task_time: u64,
    /// `first_start_send_time` — when the request first went out; a retry does
    /// not move it.
    pub first_start_send_time: u64,
    /// `start_send_time` — when the request went out.
    pub start_send_time: u64,
    /// `last_receive_pkg_time` — when the last package of the answer came in.
    pub last_receive_pkg_time: u64,
    /// `read_write_timeout` — how long the whole read may take.
    pub read_write_timeout: u64,
    /// `first_pkg_timeout` — how long the first package may take.
    pub first_pkg_timeout: u64,
    /// `sent_size`.
    pub sent_size: usize,
    /// `send_data_size` — how long the request was.
    pub send_data_size: usize,
    /// `received_size`.
    pub received_size: usize,
    /// `receive_data_size` — how long the answer was.
    pub receive_data_size: usize,
    /// `external_ip` — the near end of the socket, as the world sees it.
    pub external_ip: String,
    /// `error_type`.
    pub error_type: ErrCmdType,
    /// `error_code`.
    pub error_code: i32,
}

impl TransferProfile {
    /// `TransferProfile(_task)` — `Reset()`, which is a run that has not begun.
    pub fn new(task: Task) -> Self {
        Self {
            task,
            connect_profile: ConnectProfile::new(),
            loop_start_task_time: 0,
            first_start_send_time: 0,
            start_send_time: 0,
            last_receive_pkg_time: 0,
            read_write_timeout: 0,
            first_pkg_timeout: 0,
            sent_size: 0,
            send_data_size: 0,
            received_size: 0,
            receive_data_size: 0,
            external_ip: String::new(),
            error_type: ErrCmdType::Ok,
            error_code: 0,
        }
    }

    /// `Reset()` — every reading back to nothing, and the task kept: a retry is
    /// the same task, not a new one.
    pub fn reset(&mut self) {
        let task = std::mem::replace(&mut self.task, Task::new(0, 0));
        *self = Self::new(task);
    }
}

/// `TaskProfile` — one task, from the moment it was asked for to the moment it
/// was answered, and everything its retries left behind.
///
/// The C++ has a few fields this does not: `is_weak_network` and
/// `first_auth_flag`, which only the report reads, and `channel_name`, which
/// only a minor long link has. They come with the code that writes the report.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskProfile {
    /// `task`.
    pub task: Task,
    /// `prepare_profile`.
    pub prepare_profile: PrepareProfile,
    /// `transfer_profile`.
    pub transfer_profile: TransferProfile,
    /// `running_id` — [`None`] while the task is waiting for a run.
    pub running: Option<RunId>,
    /// `task_timeout` — how long the whole task may take, retries and all;
    /// [`compute_task_timeout`] at the moment the task was asked for.
    pub task_timeout: u64,
    /// `start_task_time` — when the task was asked for.
    pub start_task_time: u64,
    /// `end_task_time` — `0` until the task is over.
    pub end_task_time: u64,
    /// `retry_start_time` — when the last retry was due; `0` for one that is
    /// due at once.
    pub retry_start_time: u64,
    /// `remain_retry_count` — how many tries the task has left.
    pub remain_retry_count: i32,
    /// `force_no_retry` — a run that asked for the task not to be tried again.
    pub force_no_retry: bool,
    /// `last_failed_dyntime_status` — what the network was called when the try
    /// before this one failed.
    pub last_failed_dyntime_status: DynamicTimeoutStatus,
    /// `current_dyntime_status` — what it is called now.
    pub current_dyntime_status: DynamicTimeoutStatus,
    /// `use_proxy` — whether this try goes through a proxy.
    pub use_proxy: bool,
    /// `retry_time_interval` — how long a retry waits.
    pub retry_time_interval: u64,
    /// `err_type`.
    pub err_type: ErrCmdType,
    /// `err_code`.
    pub err_code: i32,
    /// `link_type` — one of the `Task::CHANNEL_*` values.
    pub link_type: i32,
    /// `allow_sessiontimeout_retry` — whether a session timeout may still give
    /// this task another try; one try each.
    pub allow_sessiontimeout_retry: bool,
    /// `history_transfer_profiles` — every try but the one that is out. Not
    /// empty is what makes the next try a fallback one.
    pub history: Vec<TransferProfile>,
}

impl TaskProfile {
    /// `TaskProfile(_task, _prepare_profile)`.
    pub fn new(task: Task, prepare_profile: PrepareProfile) -> Self {
        Self::new_at(gettickcount(), task, prepare_profile)
    }

    /// The same, with the reading `start_task_time` is set to handed in.
    pub fn new_at(now: u64, task: Task, prepare_profile: PrepareProfile) -> Self {
        Self {
            transfer_profile: TransferProfile::new(task.clone()),
            remain_retry_count: task.retry_count,
            task_timeout: compute_task_timeout(&task),
            task,
            prepare_profile,
            running: None,
            start_task_time: now,
            end_task_time: 0,
            retry_start_time: 0,
            force_no_retry: false,
            last_failed_dyntime_status: DynamicTimeoutStatus::default(),
            current_dyntime_status: DynamicTimeoutStatus::default(),
            use_proxy: false,
            retry_time_interval: 0,
            err_type: ErrCmdType::Ok,
            err_code: 0,
            link_type: 0,
            allow_sessiontimeout_retry: true,
            history: Vec::new(),
        }
    }

    /// `running_id != 0` — whether a run of this task is out.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// `PushHistory()` — the try that is over, kept.
    pub fn push_history(&mut self) {
        self.history.push(self.transfer_profile.clone());
    }

    /// `InitSendParam()` — what a retry starts from: the run is gone and the
    /// readings of it are cleared, which is what makes the timeouts of the next
    /// try start over.
    pub fn init_send_param_at(&mut self, now: u64) {
        self.prepare_profile.reset_at(now);
        self.transfer_profile.reset();
        self.running = None;
    }

    /// `__SetLastFailedStatus()` — remember what the network was called, but
    /// only for a task that has another try coming: what a task that is over
    /// failed on is not what the next one cares about.
    pub fn set_last_failed_status(&mut self) {
        if self.remain_retry_count > 0 {
            self.last_failed_dyntime_status = self.current_dyntime_status;
        }
    }

    /// What a finished task reports.
    pub fn outcome(&self) -> TaskOutcome {
        TaskOutcome {
            err_type: self.err_type,
            err_code: self.err_code,
            ip_index: self.transfer_profile.connect_profile.ip_index,
            last_receive_pkg_time: self.transfer_profile.last_receive_pkg_time,
            start_task_time: self.start_task_time,
            end_task_time: self.end_task_time,
        }
    }

    /// `GetFailStep()` — how the task failed, by the rules of
    /// [`TaskOutcome::fail_step`].
    pub fn fail_step(&self) -> TaskFailStep {
        self.outcome().fail_step()
    }

    /// `end_task_time - start_task_time` — how long the task took, in all.
    ///
    /// `0` until it is over: a task that has not finished has not taken
    /// anything.
    pub fn cost(&self) -> u64 {
        self.end_task_time.saturating_sub(self.start_task_time)
    }
}

/// `TaskProfile::ComputeTaskTimeout(_task)` — how long the whole task may take:
/// [`BASE_TASK_TIMEOUT`] plus what the server said it needs, for every try the
/// task has in it, and [`TASK_TIMEOUT_MARGIN`] on top. A long-polling task gets
/// its own `long_polling_timeout` and no more, and `total_timeout` is a ceiling
/// on all of it.
pub fn compute_task_timeout(task: &Task) -> u64 {
    let wait = if task.server_process_cost > 0 {
        BASE_TASK_TIMEOUT + task.server_process_cost as u64
    } else {
        BASE_TASK_TIMEOUT
    };
    // `int trycount = 0; if (0 <= retry_count) trycount = retry_count; trycount++;`
    let tries = if task.retry_count >= 0 {
        task.retry_count + 1
    } else {
        1
    };
    let mut timeout = (wait + TASK_TIMEOUT_MARGIN) * tries as u64;
    if task.long_polling {
        timeout = task.long_polling_timeout.max(0) as u64 + TASK_TIMEOUT_MARGIN;
    }
    if task.total_timeout > 0 && (task.total_timeout as u64) < timeout {
        timeout = task.total_timeout as u64;
    }
    timeout
}

/// `__ReadWriteTimeout(_first_pkg_timeout)` — how long the whole read may take:
/// the first package's wait, plus the time [`MAX_RECV_LEN`] takes to come in at
/// the slowest rate the network is assumed to manage.
pub fn read_write_timeout(first_pkg_timeout: u64, mobile: bool) -> u64 {
    let rate = if mobile {
        MOBILE_MIN_RATE
    } else {
        WIFI_MIN_RATE
    };
    first_pkg_timeout + 1000 * MAX_RECV_LEN / rate
}

/// `__FirstPkgTimeout(_init_first_pkg_timeout, _sendlen, _send_count,
/// _dynamictimeout_status)` — how long the first package is waited for.
///
/// `init_first_pkg_timeout` is the task's `server_process_cost`: what the server
/// said it needs. `0` is "it said nothing", which is either the network's own
/// short wait (a network that has been [`DynamicTimeoutStatus::Excellent`]) or
/// the base wait grown by how long the request is and clipped to the maximum.
/// Either way, every task that is already out makes it longer.
pub fn first_pkg_timeout(
    init_first_pkg_timeout: i64,
    send_len: usize,
    send_count: i32,
    dyntime_status: DynamicTimeoutStatus,
    mobile: bool,
) -> u64 {
    // `std::min(_send_count, 5)`, and no less than nothing: a count is never
    // negative, which the C++'s `uint64_t` arithmetic would have wrapped.
    let sent_count = send_count.clamp(0, 5) as u64;
    let delay = sent_count
        * if mobile {
            MOBILE_TASK_DELAY
        } else {
            WIFI_TASK_DELAY
        };

    let wait = if dyntime_status == DynamicTimeoutStatus::Excellent && init_first_pkg_timeout == 0 {
        if mobile {
            DYN_TIME_FIRST_PACKAGE_GPRS_TIMEOUT
        } else {
            DYN_TIME_FIRST_PACKAGE_WIFI_TIMEOUT
        }
    } else {
        let rate = if mobile {
            MOBILE_MIN_RATE
        } else {
            WIFI_MIN_RATE
        };
        let base = if mobile {
            BASE_FIRST_PACKAGE_GPRS_TIMEOUT
        } else {
            BASE_FIRST_PACKAGE_WIFI_TIMEOUT
        };
        let max = if mobile {
            MAX_FIRST_PACKAGE_GPRS_TIMEOUT
        } else {
            MAX_FIRST_PACKAGE_WIFI_TIMEOUT
        };
        if init_first_pkg_timeout > 0 {
            init_first_pkg_timeout as u64 + 1000 * send_len as u64 / rate
        } else {
            (base + 1000 * send_len as u64 / rate).min(max)
        }
    };
    wait + delay
}

/// `__CompareTask` — the order a task manager keeps its tasks in: most urgent
/// first.
///
/// [`Ordering`] rather than the C++'s `bool`, so a stable sort leaves two tasks
/// of the same priority in the order they were asked for — which is what
/// `std::list::sort` does too.
pub fn compare_task(first: &TaskProfile, second: &TaskProfile) -> Ordering {
    first.task.priority.cmp(&second.task.priority)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_is_a_link_that_has_not_been_made() {
        let profile = ConnectProfile::new();
        assert_eq!(profile.ip_index, -1, "the C++ starts at -1");
        assert_eq!(profile.nettype_for_report, -1);
        assert_eq!(profile.transport_protocol, Task::TRANSPORT_PROTOCOL_TCP);
        assert_eq!(profile.link_type, Task::CHANNEL_LONG);
        assert_eq!(profile.disconn_errtype, ErrCmdType::Ok);
        assert!(profile.ip_type == IpSourceType::Null);
        assert!(!profile.is_finished(), "no disconn_time yet");
        assert_eq!(
            profile.socket_fd,
            SocketFd::INVALID,
            "the C++ resets it to INVALID_SOCKET"
        );
        assert_eq!(profile.local_net_stack, LocalIpStack::None);
        assert!(!profile.is_reused_fd);
    }

    #[test]
    fn resetting_a_profile_keeps_the_two_tls_flags() {
        let mut profile = ConnectProfile::new();
        profile.ip = "1.1.1.1".to_string();
        profile.disconn_time = 700;
        profile.tls_handshake_success = true;
        assert!(profile.is_finished());

        profile.reset();
        assert_eq!(profile.ip, "");
        assert!(!profile.is_finished());
        // what `__UpdateProfile` carries over from the profile before it
        assert!(profile.tls_handshake_success);
        assert!(!profile.tls_handshake_mismatch);
    }

    #[test]
    fn a_noop_profile_is_one_heartbeat() {
        let noop = NoopProfile {
            success: true,
            noop_internal: 210_000,
            noop_actual_internal: 240_000,
            noop_starttime: 1_000,
            ..NoopProfile::default()
        };
        assert_eq!(noop.noop_cost, 0, "until the answer comes back");
        assert_eq!(noop.noop_actual_internal - noop.noop_internal, 30_000);
    }

    #[test]
    fn a_task_that_succeeded_has_no_fail_step() {
        let outcome = TaskOutcome::new(100, 700);
        assert_eq!(outcome.err_type, ErrCmdType::Ok);
        assert_eq!(outcome.fail_step(), TaskFailStep::Succ);
        assert_eq!(outcome.cost(), 600);
        // and it never got an ip, which is what the C++ starts from
        assert_eq!(outcome.ip_index, -1);
        assert_eq!(outcome.last_receive_pkg_time, 0);
    }

    #[test]
    fn the_fail_step_is_decided_by_the_error_first() {
        let dns = TaskOutcome {
            err_type: ErrCmdType::Dns,
            ip_index: 2,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(dns.fail_step(), TaskFailStep::Dns, "before the ip check");

        let decode = TaskOutcome {
            err_type: ErrCmdType::EnDecode,
            ip_index: 2,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(decode.fail_step(), TaskFailStep::Decode);
    }

    #[test]
    fn the_fail_step_is_decided_by_how_far_the_task_got() {
        let no_ip = TaskOutcome {
            err_type: ErrCmdType::Socket,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(no_ip.fail_step(), TaskFailStep::Connect);

        let no_pkg = TaskOutcome {
            err_type: ErrCmdType::Socket,
            ip_index: 0,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(no_pkg.fail_step(), TaskFailStep::FirstPkg);

        let pkg_pkg = TaskOutcome {
            err_type: ErrCmdType::Socket,
            ip_index: 0,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(pkg_pkg.fail_step(), TaskFailStep::PkgPkg);
    }

    #[test]
    fn a_local_timeout_is_a_timeout_and_a_server_code_is_the_server() {
        let timeout = TaskOutcome {
            err_type: ErrCmdType::Local,
            err_code: LOCAL_TASK_TIMEOUT,
            ip_index: 0,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(timeout.fail_step(), TaskFailStep::Timeout);

        let server = TaskOutcome {
            err_type: ErrCmdType::Server,
            ip_index: 0,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(server.fail_step(), TaskFailStep::Server);

        // a success with a server code is the server's fault too
        let code = TaskOutcome {
            err_code: -100,
            ip_index: 0,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(code.fail_step(), TaskFailStep::Server);

        // ... and anything else is `kStepOther`
        let other = TaskOutcome {
            err_type: ErrCmdType::Canceld,
            err_code: LOCAL_CANCEL,
            ip_index: 0,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(other.fail_step(), TaskFailStep::Other);
    }
}
