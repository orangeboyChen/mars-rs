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
//! `fun_network_report_` carries a `__LINE__` in the C++, which is a line
//! number in a file the port does not have; what is reported here is the error
//! and the pair it happened on.

use mars_comm::local_ipstack::LocalIpStack;
use mars_comm::{ProxyInfo, ProxyType, SocketAddress};

use crate::longlink::{longlink_pack, longlink_unpack, LongLinkEncoder, Unpacked};
use crate::longlink_connect_monitor::LongLinkStatus;
use crate::net_source::LonglinkConfig;
use crate::simple_ipport_sort::{IpPortItem, IpSourceType};
use crate::smart_heartbeat::SmartHeartbeat;
use crate::socket_operator::{SocketFd, SocketOperator, SocketProfile};
use crate::task::Task;
use crate::task_profile::{ConnectProfile, ErrCmdType};

/// `kEctDnsMakeSocketPrepared` — no address to connect to at all.
pub const ECT_DNS_MAKE_SOCKET_PREPARED: i32 = -10606;
/// `kEctSocketMakeSocketPrepared` — the connect itself failed.
pub const ECT_SOCKET_MAKE_SOCKET_PREPARED: i32 = -10087;
/// `EBADMSG` — what the C++ reports for an answer it could not read.
pub const EBADMSG: i32 = 74;
/// The buffer the connect's verification reads into: `64 * 1024`, which is what
/// the C++'s `__RunReadWrite` uses too.
pub const RECV_BUFFER_LEN: usize = 64 * 1024;

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
}

impl LongLink {
    /// `LongLink(..., _config, _encoder)` — a link that has not been made, with
    /// the default encoder.
    pub fn new(config: LonglinkConfig) -> Self {
        Self::with_encoder(config, LongLinkEncoder::new())
    }

    /// The same, with the encoder the app asked for.
    pub fn with_encoder(config: LonglinkConfig, encoder: LongLinkEncoder) -> Self {
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

        let mut addresses: Vec<SocketAddress> = items
            .iter()
            .map(|item| {
                let mut address = SocketAddress::new(&item.ip, item.port);
                // a proxy is reached at the address it is; anything else is
                // mapped onto the stack the local network carries
                if !use_proxy {
                    address.v4_to_v6_address(stack);
                }
                address
            })
            .collect();

        if addresses.is_empty() {
            self.set_status(LongLinkStatus::ConnectFailed);
            self.run_response_error(ErrCmdType::Dns, ECT_DNS_MAKE_SOCKET_PREPARED, true);
            return Err(ConnectFail::NoAddress);
        }

        // what the profile says until the connect comes back with the pair that
        // actually won
        if let Some(first) = items.first() {
            self.profile.proxy_info = proxy.clone();
            self.profile.ip_items = items.clone();
            self.profile.host = first.host.clone();
            self.profile.ip_type = first.source_type;
            self.profile.ip = first.ip.clone();
            self.profile.port = first.port;
            self.profile.dns_endtime = now;
        }

        // `ComplexConnect` goes through the proxy only when it was given an
        // address for it, which is what no proxy at all is here
        let proxy_address = match self.proxy_address(&mut addresses, &proxy, use_proxy, stack) {
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
        if let Some(item) = items.get(usize::try_from(connected.index).unwrap_or(usize::MAX)) {
            self.profile.host = item.host.clone();
            self.profile.ip_type = item.source_type;
            self.profile.ip = item.ip.clone();
            self.profile.port = item.port;
        }

        // the ports of the candidates that lost: 80 and 443 are the two the
        // C++ cares about, because a firewall that lets one through may not
        // let the other
        for address in addresses
            .iter()
            .take(usize::try_from(connected.index).unwrap_or(0))
        {
            match address.port() {
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
        if let Err(error_code) = self.send(socket, &request, -1) {
            self.network_report(ErrCmdType::Socket, error_code);
            return false;
        }
        match self.recv(socket) {
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

    /// `~LongLink()` — `Disconnect(kObjectDestruct)`.
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
        addresses: &mut Vec<SocketAddress>,
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

        if address.is_v4() && addresses.len() > 1 {
            addresses.retain(|address| !address.is_v6());
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

    fn send(&mut self, socket: SocketFd, buffer: &[u8], timeout_ms: i32) -> Result<usize, i32> {
        match self.operator.as_mut() {
            Some(operator) => operator.send(socket, buffer, timeout_ms),
            None => Err(0),
        }
    }

    fn recv(&mut self, socket: SocketFd) -> Result<Vec<u8>, i32> {
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
}
