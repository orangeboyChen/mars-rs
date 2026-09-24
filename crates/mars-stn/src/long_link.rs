//! `mars/stn/src/longlink.cc` — the long link, and the connect it is made on.
//!
//! A long link is one socket to one ip/port pair, kept open, with a heartbeat
//! going out on it and the server pushing down it. This is the link itself:
//! [`LongLinkStatus`] is where it is, [`LongLink::make_sure_connected`] is what
//! a task says when it wants one, and [`LongLink::connect_at`] is the dns, the
//! proxy and the socket of it.
//!
//! `LongLinkStatus` came with [`crate::longlink_connect_monitor`], which reads
//! the same enum off the link to decide when a connect is due.
//!
//! What the C++ does on a thread is a step the host calls here: there is no
//! `__Run` that blocks on a `SocketSelect`, and no pipe to wake it, so
//! [`LongLink::disconnect`] does not stop anything — it writes down *why* the
//! link is going away, and the host's run asks [`LongLink::disconnect_code`]
//! between steps. The same goes for the connect: [`SocketOperator`] is the
//! host's, and the socket it hands back is the link's.
//!
//! Two things are lost with the C++'s `ComplexConnect`, and both are said
//! where they happen: which of the candidates were still in flight when another
//! one won (only the host's connect knows), and the falling back to the next
//! candidate when the verification of one fails.
//!
//! The heartbeat is two alarms the host's run reads instead of waiting on:
//! [`LongLink::noop_due`] is the reading the next noop is due at, and
//! [`LongLink::noop_timeout_due`] the one the noop in flight has to answer by.
//! The C++'s alarms run on a message queue of their own and call back into the
//! link; here the host calls [`LongLink::on_noop_alarm_at`] when a reading has
//! come, and [`LongLink::send_heartbeat_at`] is the step the C++'s
//! `__RunReadWrite` runs whenever the interval alarm is not waiting.
//!
//! Two of the C++'s pieces have no place here: it has *two* noop-timeout alarms
//! — a member `TrigNoop` binds and a local one in `__RunReadWrite` — and an
//! `OnNoopAlarmSet` that `longlink_task_manager` wires up on Android but
//! nothing ever calls. The port has one alarm, and no `OnNoopAlarmSet`.
//!
//! Whether the app is in the foreground and whether the network is a mobile one
//! are `ActiveLogic` and `getNetInfo` in the C++, which are singletons; here
//! they are arguments, like the ones [`crate::AntiAvalanche`] takes.
//!
//! `fun_network_report_` carries a `__LINE__` in the C++, which is a line
//! number in a file the port does not have; what is reported here is the error
//! and the pair it happened on.

use std::collections::VecDeque;

use mars_comm::local_ipstack::LocalIpStack;
use mars_comm::{ProxyInfo, ProxyType, SocketAddress};

use crate::config::MIN_HEART_INTERVAL;
use crate::longlink::{longlink_pack, longlink_unpack, LongLinkEncoder, Unpacked};
use crate::longlink_connect_monitor::LongLinkStatus;
use crate::longlink_identify_checker::{IdentifyBuffer, LongLinkIdentifyChecker};
use crate::net_source::LonglinkConfig;
use crate::simple_ipport_sort::{IpPortItem, IpSourceType};
use crate::smart_heartbeat::SmartHeartbeat;
use crate::socket_operator::{SocketFd, SocketOperator, SocketProfile};
use crate::task::Task;
use crate::task_profile::{ConnectProfile, ErrCmdType, NoopProfile};

/// `kEctDnsMakeSocketPrepared` — no address to connect to at all.
pub const ECT_DNS_MAKE_SOCKET_PREPARED: i32 = -10606;
/// `kEctSocketMakeSocketPrepared` — the connect itself failed.
pub const ECT_SOCKET_MAKE_SOCKET_PREPARED: i32 = -10087;
/// `EBADMSG` — what the C++ reports for an answer it could not read.
pub const EBADMSG: i32 = 74;
/// The buffer the connect's verification reads into: `64 * 1024`, which is what
/// the C++'s `__RunReadWrite` uses too.
pub const RECV_BUFFER_LEN: usize = 64 * 1024;
/// How long a noop has to answer: the `8 * 1000` of the C++'s `__NoopReq`.
pub const NOOP_TIMEOUT: u64 = 8 * 1000;
/// The same, for a noop that is going out late: the `5 * 1000` the C++ asks
/// for when the last heartbeat came back a quarter of an hour late, which is
/// what a dozing network looks like.
pub const NOOP_ACTIVE_TIMEOUT: u64 = 5 * 1000;
/// `has_late_toomuch` — how late a heartbeat has to be before the noop that
/// follows it is given the short timeout: `15 * 60 * 1000`.
pub const NOOP_LATE_TOO_MUCH: u64 = 15 * 60 * 1000;

/// `LongLinkErrCode::TDisconnectInternalCode` — why a link is being taken
/// down. "Note: Never Delete Item!!!Just Add!!!" is the C++'s, and so are the
/// values: they are what a report carries, not an index into anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisconnectInternalCode {
    /// `kNone` — nothing asked for the link to go away.
    #[default]
    None = 0,
    /// `kReset` — "no use".
    Reset = 10000,
    /// `kRemoteClosed` — the peer hung up.
    RemoteClosed = 10001,
    /// `kUnknownErr`.
    UnknownErr = 10002,
    /// `kNoopTimeout` — a heartbeat was not answered.
    NoopTimeout = 10003,
    /// `kDecodeError`.
    DecodeError = 10004,
    /// `kUnknownRead`.
    UnknownRead = 10005,
    /// `kUnknownWrite`.
    UnknownWrite = 10006,
    /// `kDecodeErr`.
    DecodeErr = 10007,
    /// `kTaskTimeout`.
    TaskTimeout = 10008,
    /// `kNetworkChange`.
    NetworkChange = 10009,
    /// `kIDCChange`.
    IdcChange = 10010,
    /// `kNetworkLost`.
    NetworkLost = 10011,
    /// `kSelectError`.
    SelectError = 10012,
    /// `kPipeError`.
    PipeError = 10013,
    /// `kHasNewDnsIP`.
    HasNewDnsIp = 10014,
    /// `kSelectException`.
    SelectException = 10015,
    /// `kLinkCheckTimeout`.
    LinkCheckTimeout = 10016,
    /// `kForceNewGetDns`.
    ForceNewGetDns = 10017,
    /// `kLinkCheckError`.
    LinkCheckError = 10018,
    /// `kTimeCheckSucc` — the timer check found a better pair.
    TimeCheckSucc = 10019,
    /// `kObjectDestruct` — the link itself is gone; nothing will run again.
    ObjectDestruct = 10020,
    /// `kLinkDetectEnd`.
    LinkDetectEnd = 10021,
}

impl DisconnectInternalCode {
    /// `LongLinkErrCode::kNone != disconnectinternalcode_` — whether something
    /// asked for the link to go away.
    pub fn is_set(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// What [`LongLink::make_sure_connected`] answers, which is the C++'s `bool`
/// and the `bool` it writes to `_newone`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakeSure {
    /// `kConnected == ConnectStatus()`: the link is there, nothing to start.
    Connected,
    /// The C++'s `false`: a run has to be started. `new_one` is what it writes
    /// to `*_newone` — `true` when this call is what started it.
    Run {
        /// `*_newone`
        new_one: bool,
    },
    /// `disconnectinternalcode_ == LongLinkErrCode::kObjectDestruct`: the link
    /// was released, and the C++ says so rather than starting a thread for it.
    Released,
}

/// Why a connect did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectFail {
    /// No candidate at all: the C++'s `vecaddr.empty()`, which it reports as
    /// `kEctDns`.
    NoAddress,
    /// The proxy is a host, and dns could not name it.
    NoProxyIp,
    /// The connect failed; the code is the platform's.
    Connect {
        /// `ComplexConnect::ErrorCode()`.
        error_code: i32,
    },
    /// The socket answered, but not with a package the encoder could read.
    Verify,
}

/// `NetSource::GetLongLinkItems(_config, ...)` — the candidates, in order.
pub type LongLinkItems = dyn FnMut(&LonglinkConfig) -> Vec<IpPortItem> + Send;
/// `AppManager::GetProxyInfo("")` — the proxy to go through, if any.
pub type Proxy = dyn FnMut() -> ProxyInfo + Send;
/// `NetSource::GetLongLinkDebugIP()` / `GetMinorLongLinkDebugIP()`.
pub type DebugIp = dyn FnMut() -> String + Send;
/// `DnsUtil::GetDNS().GetHostByName(_host)` — the ips of a host.
pub type Dns = dyn FnMut(&str) -> Vec<String> + Send;
/// `local_ipstack_detect()` — what the local network carries.
pub type LocalStack = dyn FnMut() -> LocalIpStack + Send;
/// `getCurrNetLabel()` — the network a link was made on, for the profile.
pub type NetLabel = dyn FnMut() -> String + Send;
/// `getNetInfo()` — the same, as the number the smart heartbeat wants.
pub type NetType = dyn FnMut() -> i32 + Send;
/// `socket_address::getsockname(_sock)` — the near end of a socket.
pub type LocalAddress = dyn FnMut(SocketFd) -> SocketAddress + Send;
/// `fun_network_report_` without its `__LINE__`.
pub type NetworkReport = dyn FnMut(ErrCmdType, i32, &str, u16) + Send;
/// `OnResponse(...)` for an error the link ran into: the channel, how it
/// failed, and the profile.
pub type ResponseError = dyn FnMut(&str, ErrCmdType, i32, &ConnectProfile) + Send;
/// `SignalConnection` — the status went from one thing to another.
pub type Connection = dyn FnMut(LongLinkStatus, &str) + Send;
/// `broadcast_linkstatus_signal_` — a profile of a link that has finished.
pub type LinkStatus = dyn FnMut(&ConnectProfile) + Send;
/// `OnNoopAlarmReceived(_noop_timeout)` — one of the two noop alarms went off,
/// which on Android is what the connect monitor counts.
pub type NoopAlarmReceived = dyn FnMut(bool) + Send;

/// `comm::Alarm::TAlarmStatus` — where a one-shot timer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlarmStatus {
    /// `kInit` — never started.
    #[default]
    Init,
    /// `kStart` — waiting for its reading.
    Start,
    /// `kCancel` — cancelled.
    Cancel,
    /// `kOnAlarm` — it went off, and nothing has been started on it since.
    OnAlarm,
}

/// `comm::Alarm` as the port models one: a reading it is due at.
///
/// The C++'s alarm runs on a message queue of its own and calls the link back
/// when it goes off; the port has no thread, so what is here is the reading a
/// host waits for ([`NoopAlarm::due`]) and the two numbers the heartbeat is
/// judged by: `After()` is the interval it was started with
/// ([`NoopAlarm::after`]) and `ElapseTime()` the one it really was
/// ([`NoopAlarm::elapse_at`]), which is what a network that dozes shows itself
/// in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoopAlarm {
    /// The reading it goes off at; [`None`] when it is not waiting.
    due: Option<u64>,
    /// `after_` — the interval it was started with.
    after: u64,
    /// `start_tick_` — the reading it was started at.
    started: u64,
    status: AlarmStatus,
}

impl NoopAlarm {
    /// `Alarm()` — `kInit`, which is an alarm that was never started.
    pub fn new() -> Self {
        Self::default()
    }

    /// `Status()`.
    pub fn status(&self) -> AlarmStatus {
        self.status
    }

    /// The reading it goes off at; [`None`] while it is not waiting.
    pub fn due(&self) -> Option<u64> {
        self.due
    }

    /// `After()` — the interval it was started with.
    pub fn after(&self) -> u64 {
        self.after
    }

    /// `ElapseTime()` — how long ago it was started.
    pub fn elapse_at(&self, now: u64) -> u64 {
        now.saturating_sub(self.started)
    }

    /// `IsWaiting()` — started, and its reading has not come yet.
    pub fn is_waiting(&self) -> bool {
        self.status == AlarmStatus::Start
    }

    /// Whether its reading has come: what the host's run asks before it calls
    /// [`LongLink::on_noop_alarm_at`].
    pub fn is_due_at(&self, now: u64) -> bool {
        self.due.is_some_and(|due| now >= due)
    }

    /// `Start(_wait)` — from `now`, in `wait_ms`.
    pub fn start_at(&mut self, now: u64, wait_ms: u64) {
        self.due = Some(now + wait_ms);
        self.after = wait_ms;
        self.started = now;
        self.status = AlarmStatus::Start;
    }

    /// `Cancel()` — what the C++ does before it starts one again, and when the
    /// noop it was waiting for never went out.
    pub fn cancel(&mut self) {
        self.due = None;
        self.status = AlarmStatus::Cancel;
    }

    /// `OnAlarm` — whether it went off at `now`, which is what turns
    /// [`AlarmStatus::Start`] into [`AlarmStatus::OnAlarm`]. An alarm that is
    /// not waiting, or whose reading has not come, is left alone.
    pub fn on_alarm_at(&mut self, now: u64) -> bool {
        if self.status != AlarmStatus::Start || !self.is_due_at(now) {
            return false;
        }
        self.due = None;
        self.status = AlarmStatus::OnAlarm;
        true
    }
}

/// One thing queued to go out on the link: the task it is, and what the encoder
/// made of it. `pos` is how much of `buffer` has been written, which is what
/// the C++'s `AutoBuffer::Pos()` is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendData {
    /// The task the buffer came from; a noop is one too.
    pub task: Task,
    /// What the encoder made of the body: header first, then the body.
    pub buffer: Vec<u8>,
    /// How much of [`SendData::buffer`] has gone out.
    pub pos: usize,
}

/// One pair the link may be made on: the [`IpPortItem`] it came from and the
/// [`SocketAddress`] it is reached at.
///
/// The C++ keeps two vectors — `ip_items` and `vecaddr` — and an index into
/// them, and the two come apart: a v4 proxy takes the v6 addresses out of
/// `vecaddr` only, so the index the connect comes back with lands on a
/// different item than the one it won on. Here the item and its address are one
/// entry, so anything that filters the addresses filters the items with them.
#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    item: IpPortItem,
    address: SocketAddress,
}

impl Candidate {
    fn new(item: &IpPortItem, stack: LocalIpStack, use_proxy: bool) -> Self {
        let mut address = SocketAddress::new(&item.ip, item.port);
        // a proxy is reached at the address it is; anything else is mapped onto
        // the stack the local network carries
        if !use_proxy {
            address.v4_to_v6_address(stack);
        }
        Self {
            item: item.clone(),
            address,
        }
    }
}

/// `LongLink`.
pub struct LongLink {
    config: LonglinkConfig,
    encoder: LongLinkEncoder,
    profile: ConnectProfile,
    status: LongLinkStatus,
    disconnect_code: DisconnectInternalCode,
    /// `thread_.isruning()` — a run in flight.
    running: bool,
    /// `svr_trig_off_` — the server hung up on the link.
    server_triggered_off: bool,
    heartbeat: Option<SmartHeartbeat>,
    /// `identifychecker_` — the check the app answers before the link is used.
    identify: LongLinkIdentifyChecker,
    /// `lstsenddata_` — what is queued to go out, in order.
    queue: VecDeque<SendData>,
    /// `isnooping_` — a heartbeat is out and has not answered.
    nooping: bool,
    /// `lastheartbeat_` — the interval the heartbeat in force is on.
    last_heartbeat: u64,
    /// `alarmnoopinterval` — when the next heartbeat is due. The C++'s is a
    /// local of `__RunReadWrite`, so a new run starts with a new one.
    noop_interval: NoopAlarm,
    /// `alarmnooptimeout_` — when the heartbeat in flight has to answer by.
    noop_timeout: NoopAlarm,
    /// `first_noop_sent` — the C++'s, which is what keeps the doze judgement
    /// off the first heartbeat of a run: there is no interval to compare it to
    /// yet.
    first_noop_sent: bool,

    operator: Option<Box<dyn SocketOperator>>,
    items: Option<Box<LongLinkItems>>,
    proxy: Option<Box<Proxy>>,
    debug_ip: Option<Box<DebugIp>>,
    minorlong_debug_ip: Option<Box<DebugIp>>,
    dns: Option<Box<Dns>>,
    local_stack: Option<Box<LocalStack>>,
    net_label: Option<Box<NetLabel>>,
    net_type: Option<Box<NetType>>,
    local_address: Option<Box<LocalAddress>>,
    network_report: Option<Box<NetworkReport>>,
    on_response: Option<Box<ResponseError>>,
    on_connection: Option<Box<Connection>>,
    on_link_status: Option<Box<LinkStatus>>,
    on_noop_alarm_received: Option<Box<NoopAlarmReceived>>,
}

impl LongLink {
    /// `LongLink(..., _config, _encoder)` — a link that has not been made, with
    /// the default encoder.
    pub fn new(config: LonglinkConfig) -> Self {
        Self::with_encoder(config, LongLinkEncoder::new())
    }

    /// The same, with the encoder the app asked for.
    pub fn with_encoder(config: LonglinkConfig, encoder: LongLinkEncoder) -> Self {
        // `identifychecker_(_context, _encoder, _config.name, kChannelMinorLong
        // == _config.link_type)`
        let mut identify = LongLinkIdentifyChecker::new(
            &config.name,
            config.link_type == Task::CHANNEL_MINOR_LONG,
        );
        identify.set_encoder(encoder);
        Self {
            profile: ConnectProfile {
                link_type: config.link_type,
                ..ConnectProfile::new()
            },
            config,
            encoder,
            status: LongLinkStatus::ConnectIdle,
            disconnect_code: DisconnectInternalCode::None,
            running: false,
            server_triggered_off: false,
            heartbeat: None,
            identify,
            queue: VecDeque::new(),
            nooping: false,
            last_heartbeat: 0,
            noop_interval: NoopAlarm::new(),
            noop_timeout: NoopAlarm::new(),
            first_noop_sent: false,
            operator: None,
            items: None,
            proxy: None,
            debug_ip: None,
            minorlong_debug_ip: None,
            dns: None,
            local_stack: None,
            net_label: None,
            net_type: None,
            local_address: None,
            network_report: None,
            on_response: None,
            on_connection: None,
            on_link_status: None,
            on_noop_alarm_received: None,
        }
    }

    /// `config_` — what the link was built from.
    pub fn config(&self) -> &LonglinkConfig {
        &self.config
    }

    /// `Profile()` — the record of the connect, and of the run that came
    /// before it.
    pub fn profile(&self) -> &ConnectProfile {
        &self.profile
    }

    /// `ConnectStatus()`.
    pub fn connect_status(&self) -> LongLinkStatus {
        self.status
    }

    /// `thread_.isruning()` — whether a run is in flight.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// What the host's run says when it is over: `thread_.isruning()` is no
    /// longer true, so the next [`LongLink::make_sure_connected`] starts one
    /// from scratch.
    pub fn end_run(&mut self) {
        self.running = false;
    }

    /// `disconnectinternalcode_` — why the link is going away,
    /// [`DisconnectInternalCode::None`] while nothing asked it to.
    pub fn disconnect_code(&self) -> DisconnectInternalCode {
        self.disconnect_code
    }

    /// `svr_trig_off_` — the peer hung up, which the next run writes on the
    /// profile rather than reporting.
    pub fn is_server_triggered_off(&self) -> bool {
        self.server_triggered_off
    }

    /// `smartheartbeat_` — [`None`] until the app hands one over, which is the
    /// C++'s Android-only `#ifdef` as a value: a link without one never asks it
    /// for an interval.
    pub fn heartbeat(&self) -> Option<&SmartHeartbeat> {
        self.heartbeat.as_ref()
    }

    /// `isnooping_` — a heartbeat is out and has not answered yet.
    pub fn is_nooping(&self) -> bool {
        self.nooping
    }

    /// `lastheartbeat_` — the interval the heartbeat in force is on, which is
    /// `0` until the first one goes out.
    pub fn last_heartbeat(&self) -> u64 {
        self.last_heartbeat
    }

    /// `identifychecker_` — what the link asks the app before it uses the
    /// connection.
    pub fn identify(&self) -> &LongLinkIdentifyChecker {
        &self.identify
    }

    /// `lstsenddata_` — what is queued to go out, in the order it was queued.
    pub fn queued(&self) -> &VecDeque<SendData> {
        &self.queue
    }

    /// `has_data_to_send` — whether anything is waiting to go out, which is
    /// what the C++'s `__RunReadWrite` puts the socket in the select's write
    /// set for.
    pub fn has_data_to_send(&self) -> bool {
        !self.queue.is_empty()
    }

    /// `GetLonglinkIdentifyCheckBuffer` — the buffer the app answers the
    /// identify check with.
    pub fn set_identify_check_buffer(
        &mut self,
        check_buffer: impl FnMut(&str, u32) -> IdentifyBuffer + Send + 'static,
    ) {
        self.identify.set_check_buffer(check_buffer);
    }

    /// `OnLonglinkIdentifyResponse` — whether the answer the server sent back
    /// is the one the app handed out.
    pub fn set_identify_on_response(
        &mut self,
        on_response: impl FnMut(&str, &[u8], &[u8]) -> bool + Send + 'static,
    ) {
        self.identify.set_on_response(on_response);
    }

    /// `OnNoopAlarmReceived(_noop_timeout)` — one of the two noop alarms went
    /// off. The C++ wires this to the connect monitor on Android only.
    pub fn set_on_noop_alarm_received(
        &mut self,
        on_noop_alarm_received: impl FnMut(bool) + Send + 'static,
    ) {
        self.on_noop_alarm_received = Some(Box::new(on_noop_alarm_received));
    }

    /// `SocketOperator` — the sockets the link is made on. Unset, a connect
    /// has no socket to hand back.
    pub fn set_socket_operator(&mut self, operator: impl SocketOperator + 'static) {
        self.operator = Some(Box::new(operator));
    }

    /// `NetSource::GetLongLinkItems` — unset answers nothing, which is a
    /// connect that fails with [`ConnectFail::NoAddress`].
    pub fn set_longlink_items(
        &mut self,
        items: impl FnMut(&LonglinkConfig) -> Vec<IpPortItem> + Send + 'static,
    ) {
        self.items = Some(Box::new(items));
    }

    /// `AppManager::GetProxyInfo("")` — unset answers no proxy at all.
    pub fn set_proxy(&mut self, proxy: impl FnMut() -> ProxyInfo + Send + 'static) {
        self.proxy = Some(Box::new(proxy));
    }

    /// `NetSource::GetLongLinkDebugIP` — unset answers none.
    pub fn set_longlink_debug_ip(&mut self, debug_ip: impl FnMut() -> String + Send + 'static) {
        self.debug_ip = Some(Box::new(debug_ip));
    }

    /// `NetSource::GetMinorLongLinkDebugIP` — unset answers none.
    pub fn set_minorlong_debug_ip(&mut self, debug_ip: impl FnMut() -> String + Send + 'static) {
        self.minorlong_debug_ip = Some(Box::new(debug_ip));
    }

    /// `DnsUtil::GetDNS().GetHostByName` — unset answers no ips, which is a
    /// proxy host that cannot be reached.
    pub fn set_dns(&mut self, dns: impl FnMut(&str) -> Vec<String> + Send + 'static) {
        self.dns = Some(Box::new(dns));
    }

    /// `local_ipstack_detect` — unset answers [`LocalIpStack::None`], which is
    /// a network that carries nothing.
    pub fn set_local_ip_stack(&mut self, stack: impl FnMut() -> LocalIpStack + Send + 'static) {
        self.local_stack = Some(Box::new(stack));
    }

    /// `getCurrNetLabel` — unset answers the empty label, which is what the
    /// smart heartbeat takes for "no network detail".
    pub fn set_net_label(&mut self, label: impl FnMut() -> String + Send + 'static) {
        self.net_label = Some(Box::new(label));
    }

    /// `getNetInfo` — unset answers `0`.
    pub fn set_net_type(&mut self, net_type: impl FnMut() -> i32 + Send + 'static) {
        self.net_type = Some(Box::new(net_type));
    }

    /// `socket_address::getsockname` — unset leaves the near end of the socket
    /// out of the profile.
    pub fn set_local_address(
        &mut self,
        local_address: impl FnMut(SocketFd) -> SocketAddress + Send + 'static,
    ) {
        self.local_address = Some(Box::new(local_address));
    }

    /// `fun_network_report_` — what the C++ reports a connect's failures to.
    pub fn set_network_report(
        &mut self,
        report: impl FnMut(ErrCmdType, i32, &str, u16) + Send + 'static,
    ) {
        self.network_report = Some(Box::new(report));
    }

    /// `OnResponse(...)` for an error: `__RunResponseError` hands it the
    /// channel, how the link failed and the profile, and no task of its own.
    pub fn set_on_response(
        &mut self,
        on_response: impl FnMut(&str, ErrCmdType, i32, &ConnectProfile) + Send + 'static,
    ) {
        self.on_response = Some(Box::new(on_response));
    }

    /// `SignalConnection` — the status and the channel it changed on.
    pub fn set_on_connection(
        &mut self,
        on_connection: impl FnMut(LongLinkStatus, &str) + Send + 'static,
    ) {
        self.on_connection = Some(Box::new(on_connection));
    }

    /// `broadcast_linkstatus_signal_` — a profile of a link that has finished.
    pub fn set_on_link_status(
        &mut self,
        on_link_status: impl FnMut(&ConnectProfile) + Send + 'static,
    ) {
        self.on_link_status = Some(Box::new(on_link_status));
    }

    /// `smartheartbeat_` — the heartbeat interval the link asks for. The C++
    /// has one on Android only, which is why this starts out [`None`].
    pub fn set_smart_heartbeat(&mut self, heartbeat: SmartHeartbeat) {
        self.heartbeat = Some(heartbeat);
    }

    /// `MakeSureConnected(_newone)` — what a task says when it wants a link.
    ///
    /// The C++ starts a thread and answers `false` while the link is not up
    /// yet; the host is the thread here, so what comes back is what the host
    /// has to do: nothing, start a run, or give up on a link that was
    /// released. A run that is already in flight is told to carry on with the
    /// state it has — only the first call resets it.
    pub fn make_sure_connected(&mut self) -> MakeSure {
        // `svr_trig_off_` is cleared and forgotten: the C++ only logs it
        self.server_triggered_off = false;

        if self.status == LongLinkStatus::Connected {
            return MakeSure::Connected;
        }
        if self.disconnect_code == DisconnectInternalCode::ObjectDestruct {
            return MakeSure::Released;
        }

        let new_one = !self.running;
        if new_one {
            self.running = true;
            // `conn_reason` is the previous run's failure, which is why it is
            // read before the profile is reset
            let reason = self.profile.disconn_errcode;
            self.profile.reset();
            self.profile.link_type = self.config.link_type;
            self.profile.conn_reason = reason;
            self.status = LongLinkStatus::ConnectIdle;
            self.disconnect_code = DisconnectInternalCode::None;
            self.server_triggered_off = false;
            // what the C++'s `__RunReadWrite` starts with, and what its local
            // `alarmnoopinterval` is: a run asks the identify check again and
            // sends its first heartbeat at once
            self.identify.reset();
            self.queue.clear();
            self.noop_interval = NoopAlarm::new();
            self.noop_timeout = NoopAlarm::new();
            self.first_noop_sent = false;
        }

        MakeSure::Run { new_one }
    }

    /// `Disconnect(_scene)` — why the link is going away.
    ///
    /// The C++ breaks two pipes and joins its thread; neither is a thing here,
    /// so this writes the scene down for the host's run to see, and a link with
    /// no run in flight is left alone — which is what the C++'s
    /// `if (!thread_.isruning()) return;` is.
    pub fn disconnect(&mut self, scene: DisconnectInternalCode) {
        if !self.running {
            return;
        }
        self.disconnect_code = scene;
    }

    /// `__RunConnect(_conn_profile)` — the socket the link is made on, or why
    /// there is none.
    ///
    /// `kConnecting` and then `kConnected` or `kConnectFailed` are said as it
    /// goes, and the profile is filled in with what the connect found out.
    pub fn connect_at(&mut self, now: u64) -> Result<SocketFd, ConnectFail> {
        self.profile.net_type = self.net_label();
        self.profile.start_time = now;
        self.set_status(LongLinkStatus::Connecting);
        self.profile.dns_time = now;

        let items = self.longlink_items();
        let proxy = self.proxy();
        // a debug ip is where the link goes, so the proxy is not asked about
        let use_proxy = proxy.is_valid()
            && !proxy.kind.is_none()
            && proxy.kind != ProxyType::Http
            && self.debug_ip().is_empty()
            && !(self.config.link_type == Task::CHANNEL_MINOR_LONG
                && !self.minorlong_debug_ip().is_empty());

        let stack = self.local_ip_stack();
        self.profile.nat64 = stack == LocalIpStack::IPv6;

        let mut candidates: Vec<Candidate> = items
            .iter()
            .map(|item| Candidate::new(item, stack, use_proxy))
            .collect();

        if candidates.is_empty() {
            self.set_status(LongLinkStatus::ConnectFailed);
            self.run_response_error(ErrCmdType::Dns, ECT_DNS_MAKE_SOCKET_PREPARED, true);
            return Err(ConnectFail::NoAddress);
        }

        // what the profile says until the connect comes back with the pair that
        // actually won
        if let Some(first) = candidates.first() {
            self.profile.proxy_info = proxy.clone();
            self.profile.ip_items = items.clone();
            self.profile.host = first.item.host.clone();
            self.profile.ip_type = first.item.source_type;
            self.profile.ip = first.item.ip.clone();
            self.profile.port = first.item.port;
            self.profile.dns_endtime = now;
        }

        // `ComplexConnect` goes through the proxy only when it was given an
        // address for it, which is what no proxy at all is here
        let proxy_address = match self.proxy_address(&mut candidates, &proxy, use_proxy, stack) {
            Ok(address) => address,
            Err(fail) => {
                self.set_status(LongLinkStatus::ConnectFailed);
                self.run_response_error(ErrCmdType::Dns, ECT_DNS_MAKE_SOCKET_PREPARED, true);
                return Err(fail);
            }
        };
        // `ComplexConnect` goes through the proxy only when it was given an
        // address for it, and what it is given is the one dns named — not the
        // host the app told it about
        let connect_proxy = match &proxy_address {
            Some(address) => ProxyInfo {
                ip: address.ip().to_string(),
                port: address.port(),
                ..proxy
            },
            None => ProxyInfo::none(),
        };

        let addresses: Vec<SocketAddress> = candidates
            .iter()
            .map(|candidate| candidate.address.clone())
            .collect();
        let socket = self.open(&addresses, &connect_proxy);
        let connected = self.operator_profile();
        self.profile.conn_time = now;
        self.profile.conn_errcode = connected.error_code;
        self.profile.conn_rtt = connected.rtt;
        self.profile.conn_cost = u64::from(connected.total_cost);
        self.profile.is0rtt = connected.is_0rtt;

        if !socket.is_valid() {
            self.set_status(LongLinkStatus::ConnectFailed);
            // a link the app took down itself does not report the connect
            if !self.disconnect_code.is_set() {
                self.run_response_error(ErrCmdType::Socket, ECT_SOCKET_MAKE_SOCKET_PREPARED, false);
            }
            return Err(ConnectFail::Connect {
                error_code: connected.error_code,
            });
        }

        self.profile.ip_index = connected.index;
        if let Some(candidate) =
            candidates.get(usize::try_from(connected.index).unwrap_or(usize::MAX))
        {
            self.profile.host = candidate.item.host.clone();
            self.profile.ip_type = candidate.item.source_type;
            self.profile.ip = candidate.item.ip.clone();
            self.profile.port = candidate.item.port;
        }

        // the ports of the candidates that lost: 80 and 443 are the two the
        // C++ cares about, because a firewall that lets one through may not
        // let the other
        for candidate in candidates
            .iter()
            .take(usize::try_from(connected.index).unwrap_or(0))
        {
            match candidate.address.port() {
                443 => self.profile.tried_443port = 1,
                80 => self.profile.tried_80port = 1,
                _ => {}
            }
        }

        if let Some(local) = self.local_address(socket) {
            self.profile.local_ip = local.ip().to_string();
            self.profile.local_port = local.port();
        }

        if self.encoder.complexconnect_need_verify() && !self.verify(socket) {
            self.set_status(LongLinkStatus::ConnectFailed);
            return Err(ConnectFail::Verify);
        }

        self.set_status(LongLinkStatus::Connected);
        Ok(socket)
    }

    /// The same, with the reading of the clock the host's `gettickcount()`.
    pub fn connect(&mut self) -> Result<SocketFd, ConnectFail> {
        self.connect_at(mars_comm::tickcount::gettickcount())
    }

    /// `LongLinkConnectObserver::OnVerifySend` and `OnVerifyRecv` — a noop on
    /// a socket that is already up, which is how the C++ checks that the
    /// connect really reached the server.
    ///
    /// The C++ drops this candidate and tries the next one when it fails; the
    /// host's connect has already picked one, so here it fails the connect.
    pub fn verify(&mut self, socket: SocketFd) -> bool {
        let request = longlink_pack(self.encoder.noop_cmdid(), Task::NOOP_TASK_ID, &[]);
        if let Err(error_code) = self.write(socket, &request, -1) {
            self.network_report(ErrCmdType::Socket, error_code);
            return false;
        }
        match self.read(socket) {
            Err(error_code) => {
                self.network_report(ErrCmdType::Socket, error_code);
                false
            }
            Ok(bytes) => match longlink_unpack(&bytes) {
                Unpacked::Package { .. } => true,
                _ => {
                    self.network_report(ErrCmdType::Socket, EBADMSG);
                    false
                }
            },
        }
    }

    /// `__ConnectStatus(_status)` — the status went from one thing to another,
    /// and what that is told to: the smart heartbeat, the network report and
    /// `SignalConnection`. A status it is already at is said once.
    pub fn set_status(&mut self, status: LongLinkStatus) {
        if status == self.status {
            return;
        }
        self.status = status;
        self.notify_heartbeat(status);
        if status == LongLinkStatus::Connected {
            self.network_report(ErrCmdType::Ok, 0);
        }
        let name = self.config.name.clone();
        if let Some(on_connection) = self.on_connection.as_mut() {
            on_connection(status, &name);
        }
    }

    /// `~LongLink()` — `Disconnect(kObjectDestruct)`, which is the one scene
    /// [`LongLink::make_sure_connected`] answers [`MakeSure::Released`] for.
    ///
    /// It is the C++'s `Disconnect` with that scene and nothing else, so a link
    /// with no run in flight is left alone — and the C++ never sets
    /// `kObjectDestruct` itself: it is a scene the app hands to `Disconnect`,
    /// and `MakeSureConnected` is the only thing that reads it.
    pub fn release(&mut self) {
        self.disconnect(DisconnectInternalCode::ObjectDestruct);
    }

    /// `__UpdateProfile(_conn_profile)` — the profile of a link that has
    /// finished is what the C++ broadcasts. [`ConnectProfile::is_finished`] is
    /// the test: it is one whose `disconn_time` is set.
    pub fn broadcast_profile(&mut self) {
        if self.profile.is_finished() {
            let profile = self.profile.clone();
            if let Some(on_link_status) = self.on_link_status.as_mut() {
                on_link_status(&profile);
            }
        }
    }

    /// `Send(_body, _extension, _task)` — a task going out on the link.
    ///
    /// What goes out is what the encoder made of the body, and it waits in the
    /// queue until the host writes it; [`LongLink::queued`] is what it is
    /// waiting in. The C++ also clears `start_read_packet_time` and
    /// `start_connect_time` here, which are two profile fields that come with
    /// the code that reads them.
    ///
    /// The C++'s `_extension` has no home in the port's packer, which writes a
    /// header and a body, so it is not an argument.
    pub fn send(&mut self, task: Task, body: &[u8]) -> bool {
        if self.status != LongLinkStatus::Connected {
            return false;
        }
        self.push(task, body);
        true
    }

    /// `SendWhenNoData(_body, _extension, _cmdid, _taskid)` — the same, but
    /// only when nothing else is waiting: a heartbeat queued behind a task
    /// would not be answered any sooner than the task is.
    pub fn send_when_no_data(&mut self, cmdid: u32, taskid: u32, body: &[u8]) -> bool {
        if self.status != LongLinkStatus::Connected {
            return false;
        }
        if !self.queue.is_empty() {
            return false;
        }
        // `Task task(_taskid); task.send_only = true;`
        let mut task = Task::new(taskid, cmdid);
        task.send_only = true;
        self.push(task, body);
        true
    }

    /// `Stop(_taskid)` — a task that has not started going out is taken out of
    /// the queue. One that has (`pos != 0`) is not, which is the C++'s
    /// `0 == it->second->Pos()`.
    pub fn stop(&mut self, taskid: u32) -> bool {
        let Some(index) = self
            .queue
            .iter()
            .position(|data| data.task.taskid == taskid && data.pos == 0)
        else {
            return false;
        };
        self.queue.remove(index);
        true
    }

    /// What the host's run says when it has written: `len` bytes of what is at
    /// the head of the queue went out, and a thing that is fully written is
    /// done with.
    ///
    /// The C++ writes off `lstsenddata_.front()` the same way, and takes it out
    /// when `Pos()` is at the end of it.
    pub fn wrote(&mut self, len: usize) {
        let Some(data) = self.queue.front_mut() else {
            return;
        };
        data.pos += len;
        if data.pos >= data.buffer.len() {
            self.queue.pop_front();
        }
    }

    /// `__GetNextHeartbeatInterval()` — how long until the next heartbeat: the
    /// encoder's interval when it has one of its own, the minimum when there is
    /// no smart heartbeat, and the heartbeat's own answer otherwise.
    ///
    /// `is_active` is the app being in the foreground, which pins the interval
    /// to the minimum.
    pub fn next_heartbeat_interval(&mut self, is_active: bool) -> u64 {
        if self.encoder.noop_interval() > 0 {
            return u64::from(self.encoder.noop_interval());
        }
        match self.heartbeat.as_mut() {
            Some(heartbeat) => u64::from(heartbeat.get_next_heartbeat_interval(is_active)),
            None => u64::from(MIN_HEART_INTERVAL),
        }
    }

    /// `alarmnoopinterval` — the reading the next heartbeat is due at; [`None`]
    /// when none is waiting, which is also what a link with no interval at all
    /// answers.
    pub fn noop_due(&self) -> Option<u64> {
        self.noop_interval.due()
    }

    /// `alarmnooptimeout_` — the reading the heartbeat in flight has to answer
    /// by; [`None`] when none is in flight.
    pub fn noop_timeout_due(&self) -> Option<u64> {
        self.noop_timeout.due()
    }

    /// Where the interval alarm is, which is what the C++'s
    /// `while (!alarmnoopinterval.IsWaiting())` asks.
    pub fn noop_interval_status(&self) -> AlarmStatus {
        self.noop_interval.status()
    }

    /// Where the noop-timeout alarm is: [`AlarmStatus::OnAlarm`] is a heartbeat
    /// that did not answer in time.
    pub fn noop_timeout_status(&self) -> AlarmStatus {
        self.noop_timeout.status()
    }

    /// Whether the heartbeat that is due has not been sent yet — the C++'s
    /// `while (!alarmnoopinterval.IsWaiting())`, which is what the host's run
    /// asks before [`LongLink::send_heartbeat_at`].
    ///
    /// An alarm that was never started is one, which is the first heartbeat of
    /// a run: it goes out at once. A cancelled one is not, which is what stops
    /// a link whose interval came out as `0`.
    pub fn is_heartbeat_due_at(&self, now: u64) -> bool {
        match self.noop_interval.status() {
            AlarmStatus::Init | AlarmStatus::OnAlarm => true,
            AlarmStatus::Start => self.noop_interval.is_due_at(now),
            AlarmStatus::Cancel => false,
        }
    }

    /// The step the C++'s `__RunReadWrite` runs whenever the interval alarm is
    /// not waiting: the noop goes out, the heartbeat is told, and the alarm is
    /// started again on the interval that is in force now.
    ///
    /// `is_mobile` and `is_active` are `kMobile == getNetInfo()` and
    /// `ActiveLogic::Instance()->IsActive()`.
    pub fn send_heartbeat_at(&mut self, now: u64, is_mobile: bool, is_active: bool) -> bool {
        // what the alarm was set to, and what it really was — which is the
        // difference a dozing network shows itself in
        self.noop_interval.on_alarm_at(now);
        let on_alarm = self.noop_interval.status() == AlarmStatus::OnAlarm;
        let last_interval = self.noop_interval.after();
        let last_actual_interval = if on_alarm {
            self.noop_interval.elapse_at(now)
        } else {
            0
        };

        // the first heartbeat of a run has no interval to judge the network by
        if self.first_noop_sent && on_alarm {
            self.judge_doze_style_at(now, is_mobile, is_active);
        }

        let sent = self.noop_req_at(now, last_actual_interval >= NOOP_LATE_TOO_MUCH);
        if sent {
            self.nooping = true;
            self.notify_heartbeat_heart_req_at(now, last_interval, last_actual_interval);
        }

        self.first_noop_sent = true;
        self.last_heartbeat = self.next_heartbeat_interval(is_active);
        self.noop_interval.cancel();
        if self.last_heartbeat != 0 {
            self.noop_interval.start_at(now, self.last_heartbeat);
        }
        sent
    }

    /// `TrigNoop()` — the heartbeat the app asks for, which is a noop that is
    /// not the one the interval asked for.
    ///
    /// `isnooping_` is set *before* the noop goes out, which is the C++'s
    /// "in case the network is faster than the call" — an answer that arrives
    /// while the request is still being made is an answer all the same.
    pub fn trig_noop_at(&mut self, now: u64) {
        self.nooping = true;
        let sent = self.noop_req_at(now, false);
        if !sent && self.nooping {
            self.nooping = false;
        }
    }

    /// The same, with the reading of the clock the host's `gettickcount()`.
    pub fn trig_noop(&mut self) {
        self.trig_noop_at(mars_comm::tickcount::gettickcount())
    }

    /// `__NoopReq(_log, _alarm, need_active_timeout)` — the noop itself, or the
    /// identify check the app answered with when there is one to send.
    ///
    /// Either way the timeout alarm is started, and cancelled again when
    /// nothing went out: the C++ starts it, sends, and starts it once more,
    /// which is the same reading twice.
    pub fn noop_req_at(&mut self, now: u64, need_active_timeout: bool) -> bool {
        let wait = if need_active_timeout {
            NOOP_ACTIVE_TIMEOUT
        } else {
            NOOP_TIMEOUT
        };
        self.noop_timeout.cancel();
        self.noop_timeout.start_at(now, wait);

        let sent = match self.identify.get_identify_buffer() {
            Some((buffer, cmdid)) => {
                // `Task task(kLongLinkIdentifyCheckerTaskID); task.cmdid = …`
                let task = Task::new(Task::LONG_LINK_IDENTIFY_CHECKER_TASK_ID, cmdid);
                let sent = self.send(task, &buffer);
                self.identify
                    .set_id(Task::LONG_LINK_IDENTIFY_CHECKER_TASK_ID);
                sent
            }
            None => self.send_noop_when_no_data(),
        };

        if !sent {
            self.noop_timeout.cancel();
        }
        sent
    }

    /// `__NoopResp(...)` — whether what came back is the answer to the
    /// heartbeat that is out: the identify check's, or the noop's.
    ///
    /// An answer to a heartbeat that is out ends it: the timeout alarm is
    /// cancelled and the smart heartbeat is told it succeeded. The C++ pulls
    /// the noop's body out of the package here
    /// (`longlink_noop_resp_body`); the port's unpacker has already handed the
    /// caller the body, so there is nothing left to take out.
    pub fn noop_resp_at(&mut self, now: u64, cmdid: u32, taskid: u32, body: &[u8]) -> bool {
        let mut is_noop = false;

        if self.identify.is_identify_resp(taskid) {
            is_noop = true;
            if self.identify.on_identify_resp(body) {
                self.network_report(ErrCmdType::Ok, 0);
            }
        }

        if self.encoder.noop_isresp(Task::NOOP_TASK_ID, cmdid) {
            is_noop = true;
        }

        if is_noop && self.nooping {
            self.nooping = false;
            self.noop_timeout.cancel();
            self.notify_heartbeat_heart_result_at(now, true, false);
        }

        is_noop
    }

    /// `__OnAlarm(_noop_timeout)` — one of the two noop alarms went off, which
    /// in the C++ is the alarm's own thread calling back into the link; the
    /// host's run calls it when the reading [`LongLink::noop_due`] or
    /// [`LongLink::noop_timeout_due`] handed it has come.
    ///
    /// Whether it really went off is what comes back, and only then is
    /// `OnNoopAlarmReceived` told.
    pub fn on_noop_alarm_at(&mut self, now: u64, noop_timeout: bool) -> bool {
        let went_off = if noop_timeout {
            self.noop_timeout.on_alarm_at(now)
        } else {
            self.noop_interval.on_alarm_at(now)
        };
        if went_off {
            if let Some(received) = self.on_noop_alarm_received.as_mut() {
                received(noop_timeout);
            }
        }
        went_off
    }

    /// `__NotifySmartHeartbeatJudgeDozeStyle()` — whether the network delivered
    /// the last heartbeat when it was due.
    ///
    /// `is_mobile` is `kMobile == getNetInfo()` and `is_active` is
    /// `ActiveLogic::Instance()->IsActive()`; a network that is not a mobile
    /// one, or an app in the foreground, is not judged.
    pub fn judge_doze_style_at(&mut self, now: u64, is_mobile: bool, is_active: bool) {
        // an encoder with an interval of its own is not the smart heartbeat's
        // business
        if self.encoder.noop_interval() > 0 {
            return;
        }
        if let Some(heartbeat) = self.heartbeat.as_mut() {
            heartbeat.judge_doze_style(now, is_mobile, is_active);
        }
    }

    fn send_noop_when_no_data(&mut self) -> bool {
        // `__SendNoopWhenNoData()`: the C++ asks the encoder for the noop's
        // body and extension; the default encoder's are empty, and the port's
        // packer carries a body only
        let cmdid = self.encoder.noop_cmdid();
        self.send_when_no_data(cmdid, Task::NOOP_TASK_ID, &[])
    }

    fn push(&mut self, task: Task, body: &[u8]) {
        let buffer = longlink_pack(task.cmdid, task.taskid, body);
        self.queue.push_back(SendData {
            task,
            buffer,
            pos: 0,
        });
    }

    /// `__NotifySmartHeartbeatHeartReq(_profile, _internal, _actual_internal)`
    /// — the heartbeat that is going out is written on the profile before it
    /// does, with the interval the alarm was set to and the one it really was.
    fn notify_heartbeat_heart_req_at(&mut self, now: u64, internal: u64, actual_internal: u64) {
        if self.encoder.noop_interval() > 0 || self.heartbeat.is_none() {
            return;
        }
        self.profile.noop_profiles.push(NoopProfile {
            noop_internal: internal,
            noop_actual_internal: actual_internal,
            noop_starttime: now,
            ..NoopProfile::default()
        });
        if let Some(heartbeat) = self.heartbeat.as_mut() {
            heartbeat.on_heartbeat_start(now);
        }
    }

    /// `__NotifySmartHeartbeatHeartResult(_succes, _fail_of_timeout, _profile)`
    /// — how long the heartbeat took to answer, and whether it did.
    fn notify_heartbeat_heart_result_at(&mut self, now: u64, success: bool, fail_of_timeout: bool) {
        if self.encoder.noop_interval() > 0 || self.heartbeat.is_none() {
            return;
        }
        if let Some(noop) = self.profile.noop_profiles.last_mut() {
            noop.noop_cost = now.saturating_sub(noop.noop_starttime);
            noop.success = success;
        }
        if let Some(heartbeat) = self.heartbeat.as_mut() {
            heartbeat.on_heart_result(success, fail_of_timeout, seconds(now));
        }
    }

    fn longlink_items(&mut self) -> Vec<IpPortItem> {
        match self.items.as_mut() {
            Some(items) => items(&self.config),
            None => Vec::new(),
        }
    }

    fn proxy(&mut self) -> ProxyInfo {
        match self.proxy.as_mut() {
            Some(proxy) => proxy(),
            None => ProxyInfo::none(),
        }
    }

    fn debug_ip(&mut self) -> String {
        match self.debug_ip.as_mut() {
            Some(debug_ip) => debug_ip(),
            None => String::new(),
        }
    }

    fn minorlong_debug_ip(&mut self) -> String {
        match self.minorlong_debug_ip.as_mut() {
            Some(debug_ip) => debug_ip(),
            None => String::new(),
        }
    }

    fn local_ip_stack(&mut self) -> LocalIpStack {
        match self.local_stack.as_mut() {
            Some(stack) => stack(),
            None => LocalIpStack::None,
        }
    }

    fn net_label(&mut self) -> String {
        match self.net_label.as_mut() {
            Some(label) => label(),
            None => String::new(),
        }
    }

    fn dns(&mut self, host: &str) -> Vec<String> {
        match self.dns.as_mut() {
            Some(dns) => dns(host),
            None => Vec::new(),
        }
    }

    fn local_address(&mut self, socket: SocketFd) -> Option<SocketAddress> {
        self.local_address.as_mut().map(|local| local(socket))
    }

    /// The address of the proxy, and what a v4 proxy does to the candidates:
    /// a v4 proxy cannot carry v6 traffic, so those go.
    fn proxy_address(
        &mut self,
        candidates: &mut Vec<Candidate>,
        proxy: &ProxyInfo,
        use_proxy: bool,
        stack: LocalIpStack,
    ) -> Result<Option<SocketAddress>, ConnectFail> {
        // a debug ip is where the link goes, so there is no proxy to speak of
        if !use_proxy || self.profile.ip_type == IpSourceType::Debug {
            return Ok(None);
        }

        let proxy_ip = if proxy.ip.is_empty() && !proxy.host.is_empty() {
            match self.dns(&proxy.host).into_iter().next() {
                Some(ip) => ip,
                None => return Err(ConnectFail::NoProxyIp),
            }
        } else {
            proxy.ip.clone()
        };

        let mut address = SocketAddress::new(&proxy_ip, proxy.port);
        address.v4_to_v6_address(stack);
        self.profile.ip_type = IpSourceType::Proxy;

        if address.is_v4() && candidates.len() > 1 {
            // the item goes out with the address, so the pair the connect comes
            // back with is still the one it won on
            candidates.retain(|candidate| !candidate.address.is_v6());
        }
        Ok(Some(address))
    }

    fn open(&mut self, addresses: &[SocketAddress], proxy: &ProxyInfo) -> SocketFd {
        match self.operator.as_mut() {
            Some(operator) => operator.connect(addresses, proxy),
            None => SocketFd::INVALID,
        }
    }

    fn operator_profile(&self) -> SocketProfile {
        match self.operator.as_ref() {
            Some(operator) => operator.profile(),
            None => SocketProfile::default(),
        }
    }

    /// What the socket is asked for, which is the host's to answer.
    fn write(&mut self, socket: SocketFd, buffer: &[u8], timeout_ms: i32) -> Result<usize, i32> {
        match self.operator.as_mut() {
            Some(operator) => operator.send(socket, buffer, timeout_ms),
            None => Err(0),
        }
    }

    fn read(&mut self, socket: SocketFd) -> Result<Vec<u8>, i32> {
        match self.operator.as_mut() {
            Some(operator) => operator.recv(socket, RECV_BUFFER_LEN, -1, false),
            None => Err(0),
        }
    }

    /// `__RunResponseError(...)` — an error with no task of its own:
    /// `OnResponse` with the invalid task id, and then the network report,
    /// unless the call says to leave it out.
    fn run_response_error(&mut self, err_type: ErrCmdType, err_code: i32, report: bool) {
        let name = self.config.name.clone();
        let profile = self.profile.clone();
        if let Some(on_response) = self.on_response.as_mut() {
            on_response(&name, err_type, err_code, &profile);
        }
        if report {
            self.network_report(err_type, err_code);
        }
    }

    fn network_report(&mut self, err_type: ErrCmdType, err_code: i32) {
        let ip = self.profile.ip.clone();
        let port = self.profile.port;
        if let Some(report) = self.network_report.as_mut() {
            report(err_type, err_code, &ip, port);
        }
    }

    /// `__NotifySmartHeartbeatConnectStatus(_status)` — a link that came up
    /// starts the heartbeat over, and one that went away counts as a heartbeat
    /// that never answered. An encoder with an interval of its own is left to
    /// it.
    fn notify_heartbeat(&mut self, status: LongLinkStatus) {
        if self.encoder.noop_interval() > 0 {
            return;
        }
        match status {
            LongLinkStatus::Connected => {
                let net_detail = self.net_label();
                let net_type = self.net_type();
                if let Some(heartbeat) = self.heartbeat.as_mut() {
                    heartbeat.on_longlink_established(&net_detail, net_type);
                }
            }
            LongLinkStatus::ConnectFailed | LongLinkStatus::DisConnected => {
                let now = now_seconds();
                if let Some(heartbeat) = self.heartbeat.as_mut() {
                    heartbeat.on_longlink_disconnect(now);
                }
            }
            _ => {}
        }
    }

    fn net_type(&mut self) -> i32 {
        match self.net_type.as_mut() {
            Some(net_type) => net_type(),
            None => 0,
        }
    }
}

/// `SmartHeartbeat::OnLongLinkDisconnect` wants seconds, which is what the
/// C++'s `gettickcount() / 1000` is.
fn now_seconds() -> i64 {
    (mars_comm::tickcount::gettickcount() / 1000) as i64
}

/// The same, for a reading the host handed in.
fn seconds(now: u64) -> i64 {
    (now / 1000) as i64
}

impl std::fmt::Debug for LongLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LongLink")
            .field("name", &self.config.name)
            .field("status", &self.status)
            .field("disconnect_code", &self.disconnect_code)
            .field("running", &self.running)
            .field("ip", &self.profile.ip)
            .field("port", &self.profile.port)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::longlink::{longlink_pack, NOOP_CMDID};
    use crate::socket_operator::OpBreaker;

    /// What the host was asked for, and what it answers with.
    #[derive(Clone, Default)]
    struct Seen {
        proxies: Arc<Mutex<Vec<Option<ProxyInfo>>>>,
        addresses: Arc<Mutex<Vec<Vec<SocketAddress>>>>,
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
        closed: Arc<Mutex<Vec<SocketFd>>>,
        answer: Arc<Mutex<Vec<u8>>>,
    }

    /// A pipe that is never woken: the port has nothing blocking to give up.
    #[derive(Default)]
    struct Breaker {
        broken: bool,
    }

    impl OpBreaker for Breaker {
        fn is_break(&mut self) -> bool {
            self.broken
        }

        fn break_(&mut self) -> bool {
            self.broken = true;
            true
        }
    }

    /// The host's sockets: the numbers it hands out, and what it was asked to
    /// do with them.
    struct Host {
        seen: Seen,
        next: i64,
        profile: SocketProfile,
        breaker: Breaker,
    }

    impl Host {
        fn new(seen: Seen) -> Self {
            Self {
                seen,
                next: 3,
                profile: SocketProfile::default(),
                breaker: Breaker::default(),
            }
        }
    }

    impl SocketOperator for Host {
        fn connect(&mut self, addresses: &[SocketAddress], proxy: &ProxyInfo) -> SocketFd {
            self.seen.addresses.lock().unwrap().push(addresses.to_vec());
            self.seen
                .proxies
                .lock()
                .unwrap()
                .push(if proxy.kind.is_none() {
                    None
                } else {
                    Some(proxy.clone())
                });
            if self.profile.error_code != 0 {
                return SocketFd::INVALID;
            }
            let socket = SocketFd(self.next);
            self.next += 1;
            socket
        }

        fn send(
            &mut self,
            _socket: SocketFd,
            buffer: &[u8],
            _timeout_ms: i32,
        ) -> Result<usize, i32> {
            self.seen.sent.lock().unwrap().push(buffer.to_vec());
            Ok(buffer.len())
        }

        fn recv(
            &mut self,
            _socket: SocketFd,
            _max_size: usize,
            _timeout_ms: i32,
            _wait_full_size: bool,
        ) -> Result<Vec<u8>, i32> {
            Ok(self.seen.answer.lock().unwrap().clone())
        }

        fn close(&mut self, socket: SocketFd) {
            self.seen.closed.lock().unwrap().push(socket);
        }

        fn identify(&self, socket: SocketFd) -> String {
            crate::socket_operator::tcp_identify(socket)
        }

        fn protocol(&self) -> i32 {
            Task::TRANSPORT_PROTOCOL_TCP
        }

        fn error_desc(&self, error_code: i32) -> String {
            format!("error {error_code}")
        }

        fn profile(&self) -> SocketProfile {
            self.profile
        }

        fn breaker(&mut self) -> &mut dyn OpBreaker {
            &mut self.breaker
        }

        fn create_stream(&mut self, socket: SocketFd) -> SocketFd {
            SocketFd(socket.0 + 100)
        }

        fn set_ip_connection_timeout(&mut self, _v4: u32, _v6: u32) {}
    }

    fn item(ip: &str, port: u16, host: &str) -> IpPortItem {
        IpPortItem {
            ip: ip.to_string(),
            port,
            host: host.to_string(),
            source_type: IpSourceType::Dns,
            ..IpPortItem::new(ip, port)
        }
    }

    /// A link with one candidate on `1.1.1.1:80`, and the host's record of it.
    fn link() -> (LongLink, Seen) {
        let seen = Seen::default();
        let mut link = LongLink::new(LonglinkConfig::new("long.example"));
        link.set_longlink_items(|_| vec![item("1.1.1.1", 80, "long.example")]);
        link.set_local_ip_stack(|| LocalIpStack::IPv4);
        link.set_socket_operator(Host::new(seen.clone()));
        (link, seen)
    }

    /// Where a callback writes: they are all `'static`, so a test reads what
    /// they wrote through a shared `Vec`.
    fn sink<T: Send + 'static>() -> (Arc<Mutex<Vec<T>>>, impl FnMut(T) + Send + 'static) {
        let seen: Arc<Mutex<Vec<T>>> = Arc::new(Mutex::new(Vec::new()));
        let record = {
            let seen = seen.clone();
            move |value| seen.lock().unwrap().push(value)
        };
        (seen, record)
    }

    #[test]
    fn a_new_link_is_idle_and_has_not_been_made() {
        let (link, _) = link();
        assert_eq!(link.connect_status(), LongLinkStatus::ConnectIdle);
        assert!(!link.is_running());
        assert_eq!(link.disconnect_code(), DisconnectInternalCode::None);
        assert!(!link.disconnect_code().is_set());
        assert_eq!(link.config().name, "long.example");
        assert_eq!(link.profile().ip_index, -1);
        assert_eq!(link.profile().link_type, Task::CHANNEL_LONG);
        assert!(link.heartbeat().is_none());
        assert!(!link.is_server_triggered_off());
    }

    #[test]
    fn making_sure_starts_one_run_and_says_which_call_did() {
        let (mut link, _) = link();
        assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: true });
        // the second call is the C++'s `thread_.start` on a thread that is
        // already running: the state it has is the state it keeps
        link.profile.conn_rtt = 30;
        assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: false });
        assert_eq!(link.profile().conn_rtt, 30);
        assert!(link.is_running());

        link.set_status(LongLinkStatus::Connected);
        assert_eq!(link.make_sure_connected(), MakeSure::Connected);
    }

    #[test]
    fn a_new_run_carries_the_failure_of_the_one_before_it() {
        let (mut link, _) = link();
        link.profile.disconn_errcode = -10087;
        assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: true });
        // `conn_reason` is the run before this one
        assert_eq!(link.profile().conn_reason, -10087);
        assert_eq!(link.profile().disconn_errcode, 0);

        link.end_run();
        assert!(!link.is_running());
        link.profile.disconn_errcode = 0;
        assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: true });
        assert_eq!(link.profile().conn_reason, 0);
    }

    #[test]
    fn a_link_that_was_released_will_not_run_again() {
        let (mut link, _) = link();
        link.running = true;
        link.release();
        assert_eq!(
            link.disconnect_code(),
            DisconnectInternalCode::ObjectDestruct
        );
        assert_eq!(link.make_sure_connected(), MakeSure::Released);
    }

    #[test]
    fn disconnecting_a_link_that_is_not_running_does_nothing() {
        let (mut link, _) = link();
        link.disconnect(DisconnectInternalCode::NetworkChange);
        assert_eq!(link.disconnect_code(), DisconnectInternalCode::None);

        link.running = true;
        link.disconnect(DisconnectInternalCode::NetworkChange);
        assert_eq!(
            link.disconnect_code(),
            DisconnectInternalCode::NetworkChange
        );
    }

    #[test]
    fn a_connect_with_no_candidate_fails_with_dns() {
        let mut link = LongLink::new(LonglinkConfig::new("long.example"));
        let (reported, mut record) = sink();
        link.set_on_response(move |_name, err_type, err_code, _profile| {
            record((err_type, err_code))
        });
        assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: true });
        assert_eq!(link.connect_at(1_000), Err(ConnectFail::NoAddress));
        assert_eq!(link.connect_status(), LongLinkStatus::ConnectFailed);
        assert_eq!(
            *reported.lock().unwrap(),
            vec![(ErrCmdType::Dns, ECT_DNS_MAKE_SOCKET_PREPARED)]
        );
    }

    #[test]
    fn a_connect_that_fails_is_reported_unless_the_app_took_the_link_down() {
        let (mut link, _) = link();
        link.operator = Some(Box::new(Host {
            // the platform's word for a connect that did not happen
            profile: SocketProfile {
                error_code: ECT_SOCKET_MAKE_SOCKET_PREPARED,
                ..SocketProfile::default()
            },
            ..Host::new(Seen::default())
        }));
        let (responses, mut record_response) = sink();
        link.set_on_response(move |_name, err_type, err_code, _profile| {
            record_response((err_type, err_code))
        });
        let (reported, mut record) = sink();
        link.set_network_report(move |err_type, err_code, _ip, _port| record((err_type, err_code)));
        link.make_sure_connected();

        assert_eq!(
            link.connect_at(1_000),
            Err(ConnectFail::Connect {
                error_code: ECT_SOCKET_MAKE_SOCKET_PREPARED
            })
        );
        assert_eq!(link.connect_status(), LongLinkStatus::ConnectFailed);
        // `OnResponse` gets it, and `fun_network_report_` does not: that is the
        // `false` the C++ hands `__RunResponseError` for a connect
        assert_eq!(
            *responses.lock().unwrap(),
            vec![(ErrCmdType::Socket, ECT_SOCKET_MAKE_SOCKET_PREPARED)]
        );
        assert!(reported.lock().unwrap().is_empty());

        // a link the app disconnected itself is not even answered
        responses.lock().unwrap().clear();
        link.disconnect_code = DisconnectInternalCode::None;
        link.running = true;
        link.disconnect(DisconnectInternalCode::NetworkLost);
        assert_eq!(
            link.connect_at(2_000),
            Err(ConnectFail::Connect {
                error_code: ECT_SOCKET_MAKE_SOCKET_PREPARED
            })
        );
        assert!(responses.lock().unwrap().is_empty());
        assert!(reported.lock().unwrap().is_empty());
    }

    #[test]
    fn a_connect_fills_the_profile_with_the_pair_that_won() {
        let (mut link, _) = link();
        link.operator = Some(Box::new(Host {
            profile: SocketProfile {
                rtt: 30,
                index: 1,
                total_cost: 90,
                ..SocketProfile::default()
            },
            ..Host::new(Seen::default())
        }));
        link.set_longlink_items(|_| {
            vec![
                item("1.1.1.1", 443, "long.example"),
                item("2.2.2.2", 80, "long.example"),
            ]
        });
        link.set_local_address(|_| SocketAddress::new("10.0.0.1", 40_000));
        link.set_net_label(|| "wifi".to_string());
        link.make_sure_connected();

        assert_eq!(link.connect_at(1_000), Ok(SocketFd(3)));
        assert_eq!(link.connect_status(), LongLinkStatus::Connected);

        let profile = link.profile();
        assert_eq!(profile.ip, "2.2.2.2");
        assert_eq!(profile.port, 80);
        assert_eq!(profile.ip_index, 1);
        assert_eq!(profile.host, "long.example");
        // the candidate that lost was on 443
        assert_eq!(profile.tried_443port, 1);
        assert_eq!(profile.tried_80port, 0);
        assert_eq!(profile.local_ip, "10.0.0.1");
        assert_eq!(profile.local_port, 40_000);
        assert_eq!(profile.net_type, "wifi");
        assert_eq!(profile.conn_rtt, 30);
        assert_eq!(profile.conn_cost, 90);
        assert_eq!(profile.start_time, 1_000);
        assert_eq!(profile.dns_time, 1_000);
        assert_eq!(profile.dns_endtime, 1_000);
        assert!(!profile.nat64, "the local stack is v4");
        assert_eq!(profile.ip_items.len(), 2);
    }

    #[test]
    fn a_v4_proxy_drops_the_v6_pairs_and_the_one_that_won_is_still_the_one_it_won_on() {
        let (mut link, seen) = link();
        link.set_longlink_items(|_| {
            vec![
                item("2001:db8::1", 80, "long.example"),
                item("1.1.1.1", 80, "long.example"),
                item("2.2.2.2", 80, "long.example"),
            ]
        });
        link.set_proxy(|| ProxyInfo::new(ProxyType::Socks5, "", "10.0.0.2", 1080, "", ""));
        // the second pair the host was given: `2.2.2.2`, not `1.1.1.1`
        link.operator = Some(Box::new(Host {
            profile: SocketProfile {
                index: 1,
                ..SocketProfile::default()
            },
            ..Host::new(seen.clone())
        }));
        link.make_sure_connected();
        assert!(link.connect_at(1_000).is_ok());

        let addresses = seen.addresses.lock().unwrap();
        assert_eq!(addresses[0].len(), 2, "the v6 pair is not offered");
        assert!(addresses[0].iter().all(|address| !address.is_v6()));
        drop(addresses);
        // and the index is read off what is left, which is the pair itself
        assert_eq!(link.profile().ip, "2.2.2.2");
        assert_eq!(link.profile().ip_index, 1);
    }

    #[test]
    fn a_v6_only_network_makes_the_addresses_nat64_ones() {
        let (mut link, seen) = link();
        link.set_local_ip_stack(|| LocalIpStack::IPv6);
        link.make_sure_connected();
        assert!(link.connect_at(1_000).is_ok());
        assert!(link.profile().nat64);
        let addresses = seen.addresses.lock().unwrap();
        assert!(addresses[0][0].is_v6(), "1.1.1.1 became a nat64 one");
    }

    #[test]
    fn a_proxy_is_gone_through_only_when_the_candidates_are_not_debug_ones() {
        let proxy = ProxyInfo::new(ProxyType::Socks5, "", "10.0.0.2", 1080, "", "");
        for (source_type, want) in [(IpSourceType::Dns, true), (IpSourceType::Debug, false)] {
            let (mut link, seen) = link();
            link.set_longlink_items(move |_| {
                vec![IpPortItem {
                    source_type,
                    ..item("1.1.1.1", 80, "long.example")
                }]
            });
            link.set_proxy({
                let proxy = proxy.clone();
                move || proxy.clone()
            });
            link.make_sure_connected();
            assert!(link.connect_at(1_000).is_ok());
            assert_eq!(
                seen.proxies.lock().unwrap()[0].is_some(),
                want,
                "a debug ip is where the link goes"
            );
        }
    }

    #[test]
    fn a_v4_proxy_cannot_carry_v6_traffic() {
        let (mut link, seen) = link();
        link.set_longlink_items(|_| {
            vec![
                item("1.1.1.1", 80, "long.example"),
                item("::1", 80, "long.example"),
            ]
        });
        link.set_proxy(|| ProxyInfo::new(ProxyType::Socks5, "", "10.0.0.2", 1080, "", ""));
        link.make_sure_connected();
        assert!(link.connect_at(1_000).is_ok());

        // ... so the v6 candidate is dropped
        let addresses = seen.addresses.lock().unwrap();
        assert_eq!(addresses[0].len(), 1);
        assert!(addresses[0][0].is_v4());
    }

    #[test]
    fn a_proxy_host_that_does_not_resolve_fails_the_connect() {
        let (mut link, seen) = link();
        link.set_proxy(|| ProxyInfo::new(ProxyType::Socks5, "proxy.example", "", 1080, "", ""));
        link.make_sure_connected();

        // no dns callback at all: nothing resolves
        assert_eq!(link.connect_at(1_000), Err(ConnectFail::NoProxyIp));
        assert_eq!(link.connect_status(), LongLinkStatus::ConnectFailed);
        assert!(seen.proxies.lock().unwrap().is_empty());

        // ... and with dns for it, the proxy is where the connect goes
        link.set_dns(|_host| vec!["10.0.0.3".to_string()]);
        link.end_run();
        link.make_sure_connected();
        assert!(link.connect_at(2_000).is_ok());
        assert_eq!(
            seen.proxies.lock().unwrap()[0].as_ref().unwrap().ip,
            "10.0.0.3"
        );
        // `ip_type` says proxy until the pair that won overwrites it, which is
        // what the C++ does: "after connect, the ip info will be overwritten"
        assert_eq!(link.profile().ip_type, IpSourceType::Dns);
    }

    #[test]
    fn the_status_is_said_once_and_a_connected_link_is_reported() {
        let (mut link, _) = link();
        let (said, mut record_said) = sink();
        link.set_on_connection(move |status, name| record_said((status, name.to_string())));
        let (reported, mut record) = sink();
        link.set_network_report(move |err_type, err_code, _ip, _port| record((err_type, err_code)));
        link.make_sure_connected();
        assert!(link.connect_at(1_000).is_ok());

        assert_eq!(
            *said.lock().unwrap(),
            vec![
                (LongLinkStatus::Connecting, "long.example".to_string()),
                (LongLinkStatus::Connected, "long.example".to_string()),
            ]
        );
        assert_eq!(*reported.lock().unwrap(), vec![(ErrCmdType::Ok, 0)]);

        // and saying it again says nothing
        link.set_status(LongLinkStatus::Connected);
        assert_eq!(said.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_link_without_a_socket_operator_has_no_socket() {
        let mut link = LongLink::new(LonglinkConfig::new("long.example"));
        link.set_longlink_items(|_| vec![item("1.1.1.1", 80, "long.example")]);
        link.make_sure_connected();
        assert_eq!(
            link.connect_at(1_000),
            Err(ConnectFail::Connect { error_code: 0 })
        );
    }

    #[test]
    fn the_verification_is_a_noop_the_server_answers() {
        let (mut link, seen) = link();
        let noop = longlink_pack(NOOP_CMDID, Task::NOOP_TASK_ID, &[]);
        *seen.answer.lock().unwrap() = noop.clone();
        link.make_sure_connected();

        assert!(link.verify(SocketFd(3)));
        assert_eq!(*seen.sent.lock().unwrap(), vec![noop]);

        // ... and an answer that is not a package is not an answer
        seen.answer.lock().unwrap().clear();
        assert!(!link.verify(SocketFd(3)));
    }

    #[test]
    fn a_profile_of_a_link_that_finished_is_broadcast() {
        let (mut link, _) = link();
        let (broadcast, mut record) = sink();
        link.set_on_link_status(move |profile| record(profile.clone()));

        link.broadcast_profile();
        assert!(
            broadcast.lock().unwrap().is_empty(),
            "a link that is still up"
        );

        link.profile.disconn_time = 700;
        link.broadcast_profile();
        let broadcast = broadcast.lock().unwrap();
        assert_eq!(broadcast.len(), 1);
        assert_eq!(broadcast[0].disconn_time, 700);
    }

    #[test]
    fn a_link_that_came_up_starts_the_heartbeat_over() {
        let (mut link, _) = link();
        link.set_smart_heartbeat(SmartHeartbeat::new());
        link.set_net_label(|| "wifi".to_string());
        link.set_net_type(|| 1);
        link.make_sure_connected();
        assert!(link.connect_at(1_000).is_ok());

        // `OnLongLinkEstablished` is what wrote the network down
        let heartbeat = link.heartbeat().unwrap();
        assert_eq!(heartbeat.info().net_detail, "wifi");
        assert_eq!(heartbeat.info().net_type, 1);
    }

    #[test]
    fn the_methods_that_take_no_reading_ask_the_clock_themselves() {
        // `connect` is `connect_at` with the host's `gettickcount`
        let (mut link, _) = link();
        link.make_sure_connected();
        assert!(link.connect().is_ok());
        assert_eq!(link.connect_status(), LongLinkStatus::Connected);
        assert_eq!(link.profile().conn_time, link.profile().start_time);

        // ... and `trig_noop` is `trig_noop_at` with the same reading
        link.trig_noop();
        assert!(link.is_nooping());
    }

    #[test]
    fn a_link_that_went_away_counts_as_a_heartbeat_that_never_answered() {
        let (mut link, _) = link();
        let mut heartbeat = SmartHeartbeat::new();
        heartbeat.on_heartbeat_start(0);
        heartbeat.info_mut().succ_heart_count = 3;
        link.set_smart_heartbeat(heartbeat);
        link.make_sure_connected();
        link.set_status(LongLinkStatus::DisConnected);

        // `OnLongLinkDisconnect` is `OnHeartResult(false, false)` and then a
        // success count of zero: whatever the network did is forgotten
        assert_eq!(link.heartbeat().unwrap().info().succ_heart_count, 0);
    }

    /// A link that is up: what every heartbeat test starts from.
    fn connected() -> LongLink {
        let (mut link, _) = link();
        link.make_sure_connected();
        assert!(link.connect_at(1_000).is_ok());
        link
    }

    /// What the host's run does between two heartbeats: what was queued has
    /// gone out, so the queue is empty again.
    fn written(link: &mut LongLink) {
        let len = link
            .queued()
            .front()
            .map(|data| data.buffer.len())
            .unwrap_or(0);
        link.wrote(len);
    }

    #[test]
    fn an_alarm_is_a_reading_it_is_due_at() {
        let mut alarm = NoopAlarm::new();
        assert_eq!(alarm.status(), AlarmStatus::Init);
        assert_eq!(alarm.due(), None);
        assert!(!alarm.is_waiting());

        alarm.start_at(1_000, 8_000);
        assert_eq!(alarm.status(), AlarmStatus::Start);
        assert_eq!(alarm.after(), 8_000);
        assert_eq!(alarm.due(), Some(9_000));
        assert!(alarm.is_waiting());
        assert!(!alarm.is_due_at(8_999));
        assert!(!alarm.on_alarm_at(8_999));

        // `ElapseTime` is where a network that dozes shows itself: the reading
        // came a minute later than the interval it was started with
        assert!(alarm.is_due_at(69_000));
        assert!(alarm.on_alarm_at(69_000));
        assert_eq!(alarm.status(), AlarmStatus::OnAlarm);
        assert_eq!(alarm.elapse_at(69_000), 68_000);
        assert_eq!(alarm.due(), None);
        assert!(!alarm.is_waiting());
        // ... and it goes off once
        assert!(!alarm.on_alarm_at(70_000));

        alarm.cancel();
        assert_eq!(alarm.status(), AlarmStatus::Cancel);
        assert_eq!(alarm.due(), None);
        // `After` is what the heartbeat before this one was set to, which is
        // what the next one is judged against
        assert_eq!(alarm.after(), 8_000);
    }

    #[test]
    fn a_task_that_went_out_is_queued_until_the_host_writes_it() {
        let mut link = connected();
        assert!(!link.has_data_to_send());

        assert!(link.send(Task::new(7, 12), b"hello"));
        assert!(link.has_data_to_send());
        let queued = link.queued();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].task.taskid, 7);
        assert_eq!(queued[0].pos, 0, "nothing of it has gone out");
        assert_eq!(queued[0].buffer, longlink_pack(12, 7, b"hello"));

        // `Stop` takes out a task that has not started going out
        assert!(link.stop(7));
        assert!(link.queued().is_empty());
        assert!(!link.stop(7));

        link.send(Task::new(8, 12), b"hello");
        link.queue[0].pos = 4;
        assert!(!link.stop(8), "one that is already going out");

        // and what the host wrote is taken out when the whole of it is out
        link.wrote(3);
        assert_eq!(link.queued()[0].pos, 7);
        let rest = link.queued()[0].buffer.len() - 7;
        link.wrote(rest);
        assert!(link.queued().is_empty());
        link.wrote(1);
        assert!(link.queued().is_empty(), "nothing is waiting");
    }

    #[test]
    fn a_link_that_is_not_up_sends_nothing() {
        let (mut link, _) = link();
        assert!(!link.send(Task::new(7, 12), b"hello"));
        assert!(!link.send_when_no_data(12, 7, b"hello"));
        assert!(link.queued().is_empty());

        // ... and a noop only goes out when nothing else is waiting
        link.set_status(LongLinkStatus::Connected);
        assert!(link.send(Task::new(7, 12), b"hello"));
        assert!(!link.send_when_no_data(NOOP_CMDID, Task::NOOP_TASK_ID, &[]));
    }

    #[test]
    fn the_heartbeat_goes_out_when_its_reading_comes() {
        // the heartbeat interval is one value for the whole process
        let _lock = crate::test_lock();
        let mut link = connected();
        // the first heartbeat of a run goes out at once: the C++'s interval
        // alarm is a local of `__RunReadWrite`, so it is `kInit`, not waiting
        assert_eq!(link.noop_interval_status(), AlarmStatus::Init);
        assert!(link.is_heartbeat_due_at(2_000));

        assert!(link.send_heartbeat_at(2_000, false, false));
        assert!(link.is_nooping());
        assert_eq!(link.last_heartbeat(), 210_000, "`MinHeartInterval`");
        assert_eq!(link.noop_due(), Some(2_000 + 210_000));
        assert_eq!(link.noop_timeout_due(), Some(2_000 + NOOP_TIMEOUT));

        // what went out is the noop: `kNoopTaskID` with `kNoopCmdID`
        let queued = link.queued();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].task.taskid, Task::NOOP_TASK_ID);
        assert_eq!(queued[0].task.cmdid, NOOP_CMDID);
        assert!(queued[0].task.send_only);

        // and a heartbeat that is waiting does not go out again
        assert!(!link.is_heartbeat_due_at(2_000 + 210_000 - 1));
        assert!(link.is_heartbeat_due_at(2_000 + 210_000));

        // the interval alarm going off is told the same way the timeout one
        // is, and what it says is which of the two it was
        let (received, record) = sink();
        link.set_on_noop_alarm_received(record);
        assert!(link.on_noop_alarm_at(2_000 + 210_000, false));
        assert_eq!(*received.lock().unwrap(), vec![false]);
    }

    #[test]
    fn the_answer_of_the_heartbeat_is_written_on_the_profile() {
        // the heartbeat interval is one value for the whole process
        let _lock = crate::test_lock();
        let mut link = connected();
        link.set_smart_heartbeat(SmartHeartbeat::new());
        assert!(link.send_heartbeat_at(1_000, false, false));
        assert_eq!(link.profile().noop_profiles.len(), 1);
        // there was no alarm before this one, so there is no interval to say
        assert_eq!(link.profile().noop_profiles[0].noop_internal, 0);
        assert_eq!(link.profile().noop_profiles[0].noop_actual_internal, 0);
        assert_eq!(link.profile().noop_profiles[0].noop_starttime, 1_000);
        assert!(
            !link.profile().noop_profiles[0].success,
            "it has not answered"
        );

        // the noop's answer, 500 later
        assert!(link.noop_resp_at(1_500, NOOP_CMDID, Task::NOOP_TASK_ID, &[]));
        assert!(!link.is_nooping());
        assert_eq!(link.noop_timeout_due(), None, "the alarm was cancelled");
        let noop = link.profile().noop_profiles[0];
        assert!(noop.success);
        assert_eq!(noop.noop_cost, 500);

        // ... and a package that is neither the noop nor the identify check is
        // not the answer to a heartbeat
        assert!(!link.noop_resp_at(1_600, 12, 7, &[]));
    }

    #[test]
    fn a_heartbeat_that_is_late_is_given_the_short_timeout() {
        // the heartbeat interval is one value for the whole process
        let _lock = crate::test_lock();
        let mut link = connected();
        link.set_smart_heartbeat(SmartHeartbeat::new());
        assert!(link.send_heartbeat_at(1_000, false, false));
        assert_eq!(link.noop_timeout_due(), Some(1_000 + NOOP_TIMEOUT));

        // the next one, twenty minutes after the interval it was set to, which
        // is more than `has_late_toomuch` allows
        written(&mut link);
        let late = 1_000 + 210_000 + 20 * 60 * 1000;
        assert!(link.is_heartbeat_due_at(late));
        assert!(link.send_heartbeat_at(late, false, false));
        assert_eq!(link.noop_timeout_due(), Some(late + NOOP_ACTIVE_TIMEOUT));

        // and the profile says what the interval was, and what it really was
        let noop = link.profile().noop_profiles[1];
        assert_eq!(noop.noop_internal, 210_000);
        assert_eq!(noop.noop_actual_internal, late - 1_000);
    }

    #[test]
    fn a_network_that_dozed_is_judged_from_the_second_heartbeat_on() {
        // the heartbeat interval is one value for the whole process
        let _lock = crate::test_lock();
        let mut link = connected();
        link.set_smart_heartbeat(SmartHeartbeat::new());
        // the first heartbeat of a run has no interval to be late for
        assert!(link.send_heartbeat_at(1_000, true, false));
        assert!(!link.heartbeat().unwrap().is_doze_style());

        // the second one is thirty past its interval, which is outside the
        // window `JudgeDozeStyle` allows
        written(&mut link);
        let late = 1_000 + 210_000 + 30_000;
        assert!(link.send_heartbeat_at(late, true, false));
        assert!(
            !link.heartbeat().unwrap().is_doze_style(),
            "one late heartbeat is not a pattern"
        );

        // ... and the third one makes two of them
        written(&mut link);
        assert!(link.send_heartbeat_at(late + 210_000 + 30_000, true, false));
        assert!(link.heartbeat().unwrap().is_doze_style());

        // a network that is not a mobile one is never judged
        let mut link = connected();
        link.set_smart_heartbeat(SmartHeartbeat::new());
        assert!(link.send_heartbeat_at(1_000, false, false));
        written(&mut link);
        assert!(link.send_heartbeat_at(late, false, false));
        assert!(!link.heartbeat().unwrap().is_doze_style());
    }

    #[test]
    fn the_heartbeat_the_app_asks_for_starts_the_timeout_alarm() {
        let mut link = connected();
        let (received, record) = sink();
        link.set_on_noop_alarm_received(record);

        // `TrigNoop` is the noop the app asked for, not the one the interval
        // did: the interval alarm is not what it is waiting on
        link.trig_noop_at(3_000);
        assert!(link.is_nooping());
        assert_eq!(link.noop_timeout_due(), Some(3_000 + NOOP_TIMEOUT));
        assert_eq!(link.noop_due(), None);

        assert!(!link.on_noop_alarm_at(3_000 + NOOP_TIMEOUT - 1, true));
        assert!(link.on_noop_alarm_at(3_000 + NOOP_TIMEOUT, true));
        assert_eq!(link.noop_timeout_status(), AlarmStatus::OnAlarm);
        assert_eq!(*received.lock().unwrap(), vec![true]);

        // ... and a noop that never went out leaves no alarm waiting
        let mut down = LongLink::new(LonglinkConfig::new("long.example"));
        down.trig_noop_at(3_000);
        assert!(!down.is_nooping(), "a link that is not up");
        assert_eq!(down.noop_timeout_due(), None);
    }

    #[test]
    fn the_identify_check_goes_out_before_the_first_noop() {
        // the heartbeat interval is one value for the whole process
        let _lock = crate::test_lock();
        let mut link = connected();
        link.set_identify_check_buffer(|_channel_id, _cmdid| {
            IdentifyBuffer::now(b"check".to_vec(), b"hash".to_vec(), 99)
        });
        let (checked, mut record) = sink();
        link.set_identify_on_response(move |_channel_id, response, _hash| {
            let accepted = response == b"hash";
            record((response.to_vec(), accepted));
            accepted
        });
        let (reported, mut record_report) = sink();
        link.set_network_report(move |err_type, err_code, _ip, _port| {
            record_report((err_type, err_code))
        });

        assert!(link.send_heartbeat_at(1_000, false, false));
        let queued = link.queued();
        assert_eq!(
            queued[0].task.taskid,
            Task::LONG_LINK_IDENTIFY_CHECKER_TASK_ID
        );
        assert_eq!(queued[0].task.cmdid, 99);
        assert_eq!(
            queued[0].buffer,
            longlink_pack(99, Task::LONG_LINK_IDENTIFY_CHECKER_TASK_ID, b"check")
        );
        assert_eq!(
            link.identify().taskid(),
            Task::LONG_LINK_IDENTIFY_CHECKER_TASK_ID
        );

        // the answer the server sent back is the hash the app handed out
        assert!(link.noop_resp_at(1_100, 99, Task::LONG_LINK_IDENTIFY_CHECKER_TASK_ID, b"hash"));
        assert!(link.identify().has_checked());
        assert!(!link.is_nooping());
        assert_eq!(*checked.lock().unwrap(), vec![(b"hash".to_vec(), true)]);
        assert_eq!(*reported.lock().unwrap(), vec![(ErrCmdType::Ok, 0)]);

        // ... and the next heartbeat is a plain noop, because the link is
        // checked now
        written(&mut link);
        assert!(link.send_heartbeat_at(2_000, false, false));
        assert_eq!(link.queued()[0].task.taskid, Task::NOOP_TASK_ID);
    }

    #[test]
    fn a_new_run_starts_the_heartbeat_over() {
        // the heartbeat interval is one value for the whole process
        let _lock = crate::test_lock();
        let mut link = connected();
        link.set_smart_heartbeat(SmartHeartbeat::new());
        assert!(link.send_heartbeat_at(1_000, false, false));
        assert_eq!(link.queued().len(), 1);
        assert_eq!(link.noop_due(), Some(1_000 + 210_000));
        link.identify
            .set_id(Task::LONG_LINK_IDENTIFY_CHECKER_TASK_ID);

        link.set_status(LongLinkStatus::ConnectFailed);
        link.end_run();
        assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: true });
        // `identifychecker_.Reset()` and `lstsenddata_.clear()`, and a new
        // interval alarm, which is the one the first heartbeat of a run is
        // sent on
        assert!(link.queued().is_empty());
        assert_eq!(link.identify().taskid(), 0);
        assert_eq!(link.noop_interval_status(), AlarmStatus::Init);
        assert_eq!(link.noop_due(), None);
    }

    #[test]
    fn a_link_with_no_interval_sends_no_more_heartbeats() {
        let _lock = crate::test_lock();
        // `SetHeartBeat(0)`, which is what a noop triggered from Java leaves
        // behind: an interval of `0` is no interval at all
        crate::smart_heartbeat::set_heartbeat(0);
        let mut link = connected();
        link.set_smart_heartbeat(SmartHeartbeat::new());

        assert!(link.send_heartbeat_at(1_000, false, false));
        assert_eq!(link.last_heartbeat(), 0);
        assert_eq!(link.noop_due(), None, "no alarm was started");
        // the C++'s `if (lastheartbeat_ == 0) break;`
        assert!(!link.is_heartbeat_due_at(1_000 + 600_000));

        crate::smart_heartbeat::set_heartbeat(-1);
    }
}
