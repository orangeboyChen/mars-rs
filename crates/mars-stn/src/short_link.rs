//! `mars/stn/src/shortlink.cc` — one task, one socket, one http request.
//!
//! What is here is the connect: [`ShortLink::connect_at`] is the C++'s
//! `__RunConnect`, which turns a host list into a socket. The pairs the link
//! may be made on come from `NetSource::GetShortLinkItems` — or from the app,
//! which is what [`ShortLink::set_connect_params`] is for — and then the proxy
//! decides which of them is reached: an http proxy is connected *to* (and
//! pushed onto the front of the list as a pair of its own), while a tunnel or a
//! socks5 one is connected *through*. A task that asked to be kept alive does
//! not connect at all when the pool still has a socket for one of the pairs.
//!
//! And the run on the socket it made: [`ShortLink::write_at`] and
//! [`ShortLink::read_at`] are the C++'s `__RunReadWrite`, which writes one
//! request and reads one answer — a read at a time, which is what
//! [`ShortLink::run_at`] takes the socket's reads as. What goes out is
//! [`crate::shortlink`]'s, and so is the fork on what comes back: a status of
//! 200 is the body of the answer, anything else is `kEctHttp` with the status.
//! The socket is left open — and handed back to the pool — when both the task
//! and the server asked for it, and [`ShortLink::end_run`] closes it when one of
//! them did not.
//!
//! Two things of the C++ are not ported. `__UpdateProfile`, which copies a
//! profile into another one and keeps the two tls flags doing so: this link
//! *is* the profile it fills in, so there is nothing to copy — the flags are
//! simply never cleared, which [`ConnectProfile::reset`] does not do either.
//! And the `#ifdef _WIN32` block that drops the v6 pairs when the proxy is a
//! v4 one.

use mars_comm::http::{Parser, RecvStatus};
use mars_comm::tickcount::gettickcount;
use mars_comm::{LocalIpStack, ProxyInfo, ProxyType, SocketAddress};

use crate::long_link::{
    ECT_DNS_MAKE_SOCKET_PREPARED, ECT_SOCKET_MAKE_SOCKET_PREPARED, ECT_SOCKET_RECV_ERR,
    ECT_SOCKET_SHUTDOWN,
};
use crate::net_source::TimeoutSource;
use crate::shortlink::{
    answer, is_keep_alive, keep_alive, pack, request_headers, request_url, Answer, KeepAlive,
};
use crate::socket_operator::{SocketFd, SocketOperator, SocketProfile};
use crate::{ConnectProfile, ErrCmdType, IpPortItem, IpSourceType, Task};

/// `kMobile` of `mars/comm/platform_comm.h` — the class `getCurrNetLabel`
/// answers for a mobile network, and the only one whose label is a number.
pub const K_MOBILE: i32 = 2;

/// `kWifi` of `mars/comm/platform_comm.h` — the network the C++ reads the
/// signal of when a run is over.
pub const K_WIFI: i32 = 1;

/// `SOCKET_ERRNO(ETIMEDOUT)` on linux — what a pair that was still connecting
/// when another one won is reported with. What a platform calls it is the
/// host's to say, so the port takes the one mars was written for.
pub const ETIMEDOUT: i32 = 110;

/// `SOCKET_ERRNO(ENOTCONN)` on linux — the read that tells a quic link to fall
/// back to tcp, which is what `is_fast_fallback_tcp` on the profile says.
pub const ENOTCONN: i32 = 107;

/// `v4connect_timeout_ms_`, `v6connect_timeout_ms_` — what a link that was
/// given pairs of its own races them with.
pub const DEFAULT_CONNECT_TIMEOUT_MS: u32 = 1000;

/// `int timeout = 5000` — what a read of the answer waits for, unless the link
/// is a quic one and the net source has a timeout of its own for the cgi.
pub const DEFAULT_RW_TIMEOUT_MS: u32 = 5000;

/// `KBufferSize` — how much of the answer one read asks the socket for, which is
/// what a host hands its `Recv`: the port's own reads are arguments.
pub const K_BUFFER_SIZE: usize = 8 * 1024;

/// `kEctSocketWritenWithNonBlock` — a write that went nowhere and left no error
/// for it, which is what the C++ writes when `_err_code` is still `0`.
pub const ECT_SOCKET_WRITEN_WITH_NON_BLOCK: i32 = -10088;

/// `kEctSocketReadOnce` — a read that went nowhere and left no error for it.
pub const ECT_SOCKET_READ_ONCE: i32 = -10089;

/// `kEctHttpParseStatusLine` — what came back was not a status line.
pub const ECT_HTTP_PARSE_STATUS_LINE: i32 = -10195;

/// `kEctHttpSplitHttpHeadAndBody` — the head was never ended, or the body was
/// not the one the head said.
pub const ECT_HTTP_SPLIT_HTTP_HEAD_AND_BODY: i32 = -10194;

/// Why a connect did not happen.
///
/// The C++ answers `INVALID_SOCKET` for all of them and says which it was on
/// the profile instead: [`ConnectFail::err_cmd_type`] and
/// [`ConnectFail::err_code`] are the two fields it leaves behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectFail {
    /// `ip_items.empty()` — no pair at all, which the C++ reports as `kEctDns`.
    NoAddress,
    /// The proxy is a host, and dns could not name it. The C++ says nothing at
    /// all about this one: no report, and nothing on the profile.
    NoProxyIp,
    /// The connect failed; the code is the platform's.
    Socket {
        /// `SocketProfile::errorCode`.
        error_code: i32,
    },
    /// The connect failed because the app broke it off, which is `kEctCanceld`
    /// rather than a socket error.
    Canceld {
        /// `SocketProfile::errorCode`.
        error_code: i32,
    },
}

impl ConnectFail {
    /// The `ErrCmdType` the C++ leaves on the profile for this.
    pub fn err_cmd_type(&self) -> ErrCmdType {
        match self {
            Self::NoAddress => ErrCmdType::Dns,
            Self::NoProxyIp => ErrCmdType::Ok,
            Self::Socket { .. } => ErrCmdType::Socket,
            Self::Canceld { .. } => ErrCmdType::Canceld,
        }
    }

    /// The `errcode` the C++ leaves on the profile with it, which is `0` for a
    /// proxy dns that said nothing at all.
    pub fn err_code(&self) -> i32 {
        match self {
            Self::NoAddress => ECT_DNS_MAKE_SOCKET_PREPARED,
            Self::NoProxyIp => 0,
            Self::Socket { .. } => ECT_SOCKET_MAKE_SOCKET_PREPARED,
            Self::Canceld { .. } => 0,
        }
    }

    /// The platform's own word for it, which is what the connect left on the
    /// profile.
    pub fn error_code(&self) -> i32 {
        match self {
            Self::Socket { error_code } | Self::Canceld { error_code } => *error_code,
            Self::NoAddress | Self::NoProxyIp => 0,
        }
    }
}

/// What `getCurrNetLabel(_nettype)` / `getRealtimeNetLabel(_nettype)` answer:
/// the label it wrote, and which kind of network it is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetworkLabel {
    /// `kNoNet` / `kWifi` / `kMobile` / `kOtherNet`.
    pub kind: i32,
    /// `_nettype` — the label itself. On a mobile network it *is* the isp code,
    /// which is what the C++ reads back out of it with `strtoll`.
    pub label: String,
}

/// `NetSource::GetShortLinkItems(_hostlist, _ipport_items, _dns_util, _cgi,
/// _extra_info)` — the pairs the link may be made on, in order. The
/// `_extra_info` of the C++ is not ported: [`crate::Task`] has none.
pub type ShortLinkItems = dyn FnMut(&[String], &str) -> Vec<IpPortItem> + Send;
/// `AppManager::GetProxyInfo(_host)` — the proxy to go through, if any.
pub type Proxy = dyn FnMut(&str) -> ProxyInfo + Send;
/// `NetSource::GetShortLinkDebugIP()` — a debug ip is where the link goes, so a
/// proxy is not asked about.
pub type DebugIp = dyn FnMut() -> String + Send;
/// `NetSource::GetShortLinkPort()` — the port of the pair an http proxy is
/// pushed onto the list as.
pub type ShortLinkPort = dyn FnMut() -> u16 + Send;
/// `NetSource::GetIpConnectTimeout()` — the `(v4, v6)` timeouts, in
/// milliseconds, that the connect races its pairs with.
pub type IpConnectTimeout = dyn FnMut() -> (u32, u32) + Send;
/// `DnsUtil::GetDNS().GetHostByName(_host)` — the ips of a host, which is how a
/// proxy named by host is reached.
pub type Dns = dyn FnMut(&str) -> Vec<String> + Send;
/// `local_ipstack_detect()` — what the local network carries.
pub type LocalStack = dyn FnMut() -> LocalIpStack + Send;
/// `getCurrNetLabel` / `getRealtimeNetLabel` — the network the connect is made
/// on.
pub type NetLabel = dyn FnMut() -> NetworkLabel + Send;
/// `socket_address::getsockname(_sock)` — the near end of a socket.
pub type LocalAddress = dyn FnMut(SocketFd) -> Option<SocketAddress> + Send;
/// `func_network_report_` without its `__LINE__` — how a pair or a connect
/// failed, and which pair it was.
pub type NetworkReport = dyn FnMut(ErrCmdType, i32, &str, &str, u16) + Send;
/// `GetCacheSocket(_address)` — a socket the pool still has for a pair.
/// [`None`] is the C++'s `INVALID_SOCKET`.
pub type CacheSocket = dyn FnMut(&IpPortItem) -> Option<SocketFd> + Send;
/// `WeakNetworkLogic::OnConnectEvent(_is_suc, _rtt, _index)` — whether the
/// connect got a socket, how long the pair that won took, and which it was.
pub type ConnectEvent = dyn FnMut(bool, u32, i32) + Send;

/// `OnSend` — the socket is up and the request is about to go out on it.
pub type OnSend = dyn FnMut(SocketFd) + Send;

/// `OnRecv(_cached_size, _total_size)` — how much of the answer one read
/// brought, and how much of it there is so far.
pub type OnRecv = dyn FnMut(usize, usize) + Send;

/// `fun_shortlink_response_(_status_code)` — the status of the answer, which
/// the C++ is told whatever became of the task.
pub type ResponseStatus = dyn FnMut(i32) + Send;

/// `OnResponse(...)` — what the app is handed when a run is over: the error (or
/// `kEctOK`), the status, the body — which is empty for anything that is not
/// one — and the profile the run wrote.
///
/// The C++ calls this *instead of* its own `__OnResponseImp`, which is what
/// [`ShortLink`] falls back to: see [`ShortLink::set_response_status`],
/// [`ShortLink::set_pool_report`] and [`ShortLink::set_pool_cache`].
pub type OnResponse = dyn FnMut(ErrCmdType, i32, &[u8], &ConnectProfile) + Send;

/// `OnSocketPoolReport(_is_reused, _has_received, _is_decode_ok)` — what the
/// pool is told about a socket it handed out.
pub type PoolReport = dyn FnMut(bool, bool, bool) + Send;

/// `OnSocketPoolTryAddCache(_item, _conn_profile)` — the socket the server said
/// to keep, which the pool may keep for the next task on the same pair. What it
/// needs of the run is on the profile: the socket, and how long it is good for.
pub type PoolCache = dyn FnMut(&IpPortItem, &ConnectProfile) + Send;

/// `NetSource::GetQUICRWTimeoutMs(_cgi, &_source)` — the timeout a quic link
/// reads with, and where it came from.
pub type QuicRwTimeout = dyn FnMut(&str) -> (u32, TimeoutSource) + Send;

/// `mars::comm::getNetTypeForStatistics()` — the network the profile reports.
pub type NetTypeForReport = dyn FnMut() -> i32 + Send;

/// `::getSignal(_is_wifi)` — the signal the network had when the run was over.
pub type Signal = dyn FnMut(bool) -> i32 + Send;

/// `xlogger_tid()` — the thread the run is on, which the profile names.
pub type Tid = dyn FnMut() -> i64 + Send;

/// Why a run came back with no body — the `ErrCmdType` and the code the C++
/// left on the profile, told as one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunFail {
    /// The connect never happened. [`ConnectFail`] is what the C++ says on the
    /// profile, and this is the same thing for a caller that only wants to know
    /// whether the task got an answer.
    Connect(ConnectFail),
    /// `kEctSocket` — the write or a read failed. The code is the platform's own
    /// word when it has one, and mars's when it does not.
    Socket {
        /// `disconn_errcode`.
        err_code: i32,
    },
    /// `kEctHttp` — what came back was not a 200, or was not an answer at all.
    /// The code is the status, or one of the `kEctHttp*` for an answer that
    /// could not be read.
    Http {
        /// `disconn_errcode`.
        err_code: i32,
    },
    /// `kEctCanceld` — the app broke the run off, which is no error at all.
    Canceld,
}

impl RunFail {
    /// The `ErrCmdType` the C++ leaves on the profile for this.
    pub fn err_cmd_type(&self) -> ErrCmdType {
        match self {
            Self::Connect(fail) => fail.err_cmd_type(),
            Self::Socket { .. } => ErrCmdType::Socket,
            Self::Http { .. } => ErrCmdType::Http,
            Self::Canceld => ErrCmdType::Canceld,
        }
    }

    /// The `disconn_errcode` that goes with it: `0` for a run the app broke off,
    /// which the C++ leaves at whatever it was.
    pub fn err_code(&self) -> i32 {
        match self {
            Self::Connect(fail) => fail.err_code(),
            Self::Socket { err_code } | Self::Http { err_code } => *err_code,
            Self::Canceld => 0,
        }
    }
}

/// What one read of the socket did — the C++'s read loop either goes round
/// again or is over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    /// The answer is not whole yet: a read that timed out, or one that brought
    /// part of it.
    Again,
    /// The run is over: the body of a 200, or why there is none.
    Done(Result<Vec<u8>, RunFail>),
}

/// `ShortLink` — one task on its own socket.
pub struct ShortLink {
    task: Task,
    profile: ConnectProfile,
    /// `use_proxy_` — whether the link may go through a proxy at all.
    use_proxy: bool,
    /// `is_keep_alive_` — `CheckKeepAlive(_task)`, which is a `Connection:
    /// Keep-Alive` of the task's own.
    keep_alive: bool,
    /// `sent_count` — how many times the task has gone out.
    sent_count: i32,
    /// `outter_vec_addr_` — pairs the app handed the link instead of letting it
    /// resolve the host itself: newdns, for instance.
    outer: Vec<IpPortItem>,
    /// `v4connect_timeout_ms_`.
    v4_connect_timeout_ms: u32,
    /// `v6connect_timeout_ms_`.
    v6_connect_timeout_ms: u32,

    /// `http::Parser` — the answer, as far as it has come in. [`write_at`]
    /// throws away whatever a run before this one read.
    answer: Parser,
    /// `recv_pos` — how much of the answer has come in, which is the "total" of
    /// `OnRecv`.
    received: usize,

    operator: Option<Box<dyn SocketOperator>>,
    items: Option<Box<ShortLinkItems>>,
    proxy: Option<Box<Proxy>>,
    debug_ip: Option<Box<DebugIp>>,
    port: Option<Box<ShortLinkPort>>,
    ip_connect_timeout: Option<Box<IpConnectTimeout>>,
    dns: Option<Box<Dns>>,
    local_stack: Option<Box<LocalStack>>,
    net_label: Option<Box<NetLabel>>,
    local_address: Option<Box<LocalAddress>>,
    network_report: Option<Box<NetworkReport>>,
    cache_socket: Option<Box<CacheSocket>>,
    connect_event: Option<Box<ConnectEvent>>,
    on_send: Option<Box<OnSend>>,
    on_recv: Option<Box<OnRecv>>,
    response_status: Option<Box<ResponseStatus>>,
    on_response: Option<Box<OnResponse>>,
    pool_report: Option<Box<PoolReport>>,
    pool_cache: Option<Box<PoolCache>>,
    quic_rw_timeout: Option<Box<QuicRwTimeout>>,
    net_type_for_report: Option<Box<NetTypeForReport>>,
    signal: Option<Box<Signal>>,
    tid: Option<Box<Tid>>,
}

impl ShortLink {
    /// `ShortLink(..., _task, _use_proxy)` — a link that has not been made: the
    /// task it is for, and whether it may go through a proxy.
    ///
    /// Whether the socket is left open afterwards is the task's own
    /// `Connection: Keep-Alive` ([`is_keep_alive`]), so it is decided here and
    /// not asked again.
    pub fn new(task: Task, use_proxy: bool) -> Self {
        Self {
            profile: ConnectProfile {
                task_id: task.taskid,
                cgi: task.cgi.clone(),
                link_type: Task::CHANNEL_SHORT,
                ..ConnectProfile::new()
            },
            keep_alive: is_keep_alive(&task),
            task,
            use_proxy,
            sent_count: 0,
            outer: Vec::new(),
            v4_connect_timeout_ms: DEFAULT_CONNECT_TIMEOUT_MS,
            v6_connect_timeout_ms: DEFAULT_CONNECT_TIMEOUT_MS,
            answer: Parser::new(),
            received: 0,
            operator: None,
            items: None,
            proxy: None,
            debug_ip: None,
            port: None,
            ip_connect_timeout: None,
            dns: None,
            local_stack: None,
            net_label: None,
            local_address: None,
            network_report: None,
            cache_socket: None,
            connect_event: None,
            on_send: None,
            on_recv: None,
            response_status: None,
            on_response: None,
            pool_report: None,
            pool_cache: None,
            quic_rw_timeout: None,
            net_type_for_report: None,
            signal: None,
            tid: None,
        }
    }

    /// `task_` — what the link was built for.
    pub fn task(&self) -> &Task {
        &self.task
    }

    /// `Profile()` — the profile the connect has been filling in.
    pub fn profile(&self) -> &ConnectProfile {
        &self.profile
    }

    /// `IsKeepAlive()` — whether the socket is left open when the answer is in.
    pub fn is_keep_alive(&self) -> bool {
        self.keep_alive
    }

    /// `sent_count` — how many times the task has gone out.
    pub fn sent_count(&self) -> i32 {
        self.sent_count
    }

    /// `SetSentCount(_sent_count)`.
    pub fn set_sent_count(&mut self, sent_count: i32) {
        self.sent_count = sent_count;
    }

    /// `SetConnectParams(_out_addr, _v4timeout_ms, _v6timeout_ms)` — the pairs
    /// the link is made on instead of the ones it would have resolved itself,
    /// and the timeouts it races them with.
    pub fn set_connect_params(
        &mut self,
        out_addr: Vec<IpPortItem>,
        v4_timeout_ms: u32,
        v6_timeout_ms: u32,
    ) {
        self.outer = out_addr;
        self.v4_connect_timeout_ms = v4_timeout_ms;
        self.v6_connect_timeout_ms = v6_timeout_ms;
    }

    /// `outter_vec_addr_` — the pairs the app handed the link, which is also
    /// what decides whose connect timeouts are used.
    pub fn outer(&self) -> &[IpPortItem] {
        &self.outer
    }

    /// `SocketOperator` — the sockets the link is made on. Unset, a connect has
    /// no socket to hand back.
    pub fn set_socket_operator(&mut self, operator: impl SocketOperator + 'static) {
        self.operator = Some(Box::new(operator));
    }

    /// `NetSource::GetShortLinkItems` — unset answers nothing, which is a
    /// connect that fails with [`ConnectFail::NoAddress`].
    pub fn set_shortlink_items(
        &mut self,
        items: impl FnMut(&[String], &str) -> Vec<IpPortItem> + Send + 'static,
    ) {
        self.items = Some(Box::new(items));
    }

    /// `AppManager::GetProxyInfo(_host)` — unset answers no proxy at all.
    pub fn set_proxy(&mut self, proxy: impl FnMut(&str) -> ProxyInfo + Send + 'static) {
        self.proxy = Some(Box::new(proxy));
    }

    /// `NetSource::GetShortLinkDebugIP` — unset answers none, which is what
    /// lets an http proxy in.
    pub fn set_debug_ip(&mut self, debug_ip: impl FnMut() -> String + Send + 'static) {
        self.debug_ip = Some(Box::new(debug_ip));
    }

    /// `NetSource::GetShortLinkPort` — unset answers `0`.
    pub fn set_shortlink_port(&mut self, port: impl FnMut() -> u16 + Send + 'static) {
        self.port = Some(Box::new(port));
    }

    /// `NetSource::GetIpConnectTimeout` — unset answers no timeouts at all,
    /// which is a connect that waits.
    pub fn set_ip_connect_timeout(&mut self, timeout: impl FnMut() -> (u32, u32) + Send + 'static) {
        self.ip_connect_timeout = Some(Box::new(timeout));
    }

    /// `DnsUtil::GetDNS().GetHostByName` — unset answers no ips, which is a
    /// proxy host that cannot be reached.
    pub fn set_dns(&mut self, dns: impl FnMut(&str) -> Vec<String> + Send + 'static) {
        self.dns = Some(Box::new(dns));
    }

    /// `local_ipstack_detect` — unset answers [`LocalIpStack::None`], which is a
    /// network that carries nothing.
    pub fn set_local_ip_stack(&mut self, stack: impl FnMut() -> LocalIpStack + Send + 'static) {
        self.local_stack = Some(Box::new(stack));
    }

    /// `getCurrNetLabel` — unset answers the empty label of a network that is
    /// none of the kinds it knows.
    pub fn set_net_label(&mut self, label: impl FnMut() -> NetworkLabel + Send + 'static) {
        self.net_label = Some(Box::new(label));
    }

    /// `socket_address::getsockname` — unset leaves the near end of the socket
    /// out of the profile.
    pub fn set_local_address(
        &mut self,
        local_address: impl FnMut(SocketFd) -> Option<SocketAddress> + Send + 'static,
    ) {
        self.local_address = Some(Box::new(local_address));
    }

    /// `func_network_report_` — what the C++ reports a connect's failures to.
    pub fn set_network_report(
        &mut self,
        report: impl FnMut(ErrCmdType, i32, &str, &str, u16) + Send + 'static,
    ) {
        self.network_report = Some(Box::new(report));
    }

    /// `GetCacheSocket` — unset answers none, which is a link that always
    /// connects.
    pub fn set_cache_socket(
        &mut self,
        cache_socket: impl FnMut(&IpPortItem) -> Option<SocketFd> + Send + 'static,
    ) {
        self.cache_socket = Some(Box::new(cache_socket));
    }

    /// `WeakNetworkLogic::OnConnectEvent` — unset reports nothing.
    pub fn set_connect_event(
        &mut self,
        connect_event: impl FnMut(bool, u32, i32) + Send + 'static,
    ) {
        self.connect_event = Some(Box::new(connect_event));
    }

    /// `OnSend` — unset says nothing.
    pub fn set_on_send(&mut self, on_send: impl FnMut(SocketFd) + Send + 'static) {
        self.on_send = Some(Box::new(on_send));
    }

    /// `OnRecv` — unset says nothing.
    pub fn set_on_recv(&mut self, on_recv: impl FnMut(usize, usize) + Send + 'static) {
        self.on_recv = Some(Box::new(on_recv));
    }

    /// `fun_shortlink_response_` — unset says nothing.
    pub fn set_response_status(&mut self, status: impl FnMut(i32) + Send + 'static) {
        self.response_status = Some(Box::new(status));
    }

    /// `OnResponse` — unset leaves the answer to the link's own `__OnResponseImp`
    /// ([`ShortLink::set_response_status`] and the two pool callbacks).
    pub fn set_on_response(
        &mut self,
        on_response: impl FnMut(ErrCmdType, i32, &[u8], &ConnectProfile) + Send + 'static,
    ) {
        self.on_response = Some(Box::new(on_response));
    }

    /// `OnSocketPoolReport` — unset reports nothing.
    pub fn set_pool_report(&mut self, report: impl FnMut(bool, bool, bool) + Send + 'static) {
        self.pool_report = Some(Box::new(report));
    }

    /// `OnSocketPoolTryAddCache` — unset keeps nothing.
    pub fn set_pool_cache(
        &mut self,
        cache: impl FnMut(&IpPortItem, &ConnectProfile) + Send + 'static,
    ) {
        self.pool_cache = Some(Box::new(cache));
    }

    /// `NetSource::GetQUICRWTimeoutMs` — unset answers
    /// [`DEFAULT_RW_TIMEOUT_MS`] out of [`TimeoutSource::ClientDefault`], which
    /// is what a link that is not a quic one reads with anyway.
    pub fn set_quic_rw_timeout(
        &mut self,
        timeout: impl FnMut(&str) -> (u32, TimeoutSource) + Send + 'static,
    ) {
        self.quic_rw_timeout = Some(Box::new(timeout));
    }

    /// `mars::comm::getNetTypeForStatistics` — unset leaves the profile's
    /// `nettype_for_report` where it is.
    pub fn set_net_type_for_report(&mut self, net_type: impl FnMut() -> i32 + Send + 'static) {
        self.net_type_for_report = Some(Box::new(net_type));
    }

    /// `::getSignal(_is_wifi)` — unset leaves `disconn_signal` at `0`.
    pub fn set_signal(&mut self, signal: impl FnMut(bool) -> i32 + Send + 'static) {
        self.signal = Some(Box::new(signal));
    }

    /// `xlogger_tid` — unset leaves `tid` at `0`, which is a run on no thread in
    /// particular.
    pub fn set_tid(&mut self, tid: impl FnMut() -> i64 + Send + 'static) {
        self.tid = Some(Box::new(tid));
    }

    /// `__RunConnect(_conn_profile)` — the socket the link is made on, or why
    /// there is none.
    ///
    /// The profile is filled in as it goes: the pairs, the proxy, the pair that
    /// won, the timings, and the near end of the socket.
    pub fn connect_at(&mut self, now: u64) -> Result<SocketFd, ConnectFail> {
        self.profile.dns_time = now;
        if let Some(host) = self.task.shortlink_host_list.first() {
            self.profile.host = host.clone();
        }
        if self.use_proxy {
            let host = self.profile.host.clone();
            self.profile.proxy_info = self.proxy(&host);
        }
        let proxy = self.profile.proxy_info.clone();

        let stack = self.local_stack();
        self.profile.local_net_stack = stack;

        // pairs the app handed the link — newdns, for instance — are used as
        // they are, and so are the timeouts they came with
        self.profile.ip_items = if self.outer.is_empty() {
            self.shortlink_items()
        } else {
            self.outer.clone()
        };
        if self.profile.ip_items.is_empty() {
            self.run_response_error(ErrCmdType::Dns, ECT_DNS_MAKE_SOCKET_PREPARED, false);
            return Err(ConnectFail::NoAddress);
        }

        // a debug ip is where the link goes, so there is no proxy to speak of
        let use_proxy = proxy.is_address_valid()
            && self
                .profile
                .ip_items
                .first()
                .is_some_and(|item| item.source_type != IpSourceType::Debug);

        if use_proxy && proxy.kind == ProxyType::Http && self.debug_ip().is_empty() {
            self.profile.ip = proxy.ip.clone();
            self.profile.port = proxy.port;
            self.profile.ip_type = IpSourceType::Proxy;
            // an http proxy is connected *to*, so it goes on the list as a pair
            // of its own, at the front — on the short-link port rather than on
            // the proxy's, which is what the request is then written for
            let port = self.shortlink_port();
            let mut item = IpPortItem::new(&proxy.ip, port);
            item.source_type = IpSourceType::Proxy;
            item.host = self.profile.host.clone();
            self.profile.ip_items.insert(0, item);
        } else {
            let protocol = self.protocol();
            if protocol == Task::TRANSPORT_PROTOCOL_QUIC {
                // the C++ asserts there is no proxy here: quic does not go
                // through one
                for item in &mut self.profile.ip_items {
                    item.transport_protocol = Task::TRANSPORT_PROTOCOL_QUIC;
                }
            }
        }
        if let Some(first) = self.profile.ip_items.first().cloned() {
            self.profile.host = first.host;
            self.profile.ip_type = first.source_type;
            self.profile.ip = first.ip;
            self.profile.port = first.port;
        }

        // what a proxy named by host is reached at, which is the one dns named
        let proxy_ip = if use_proxy && !proxy.kind.is_none() {
            if proxy.ip.is_empty() && !proxy.host.is_empty() {
                match self.dns(&proxy.host).into_iter().next() {
                    Some(ip) => Some(ip),
                    // the C++ comes back with no socket and says nothing at all
                    None => return Err(ConnectFail::NoProxyIp),
                }
            } else {
                Some(proxy.ip.clone())
            }
        } else {
            None
        };

        // a tunnel or a socks5 proxy is connected *through*: the pairs stay
        // where they are and the operator is told where the proxy is. The C++
        // hands it the nat64-mapped address of the proxy separately; the port
        // hands it the whole [`ProxyInfo`] instead, so mapping it is the
        // operator's.
        if use_proxy && (proxy.kind == ProxyType::HttpTunnel || proxy.kind == ProxyType::Socks5) {
            self.profile.ip_type = IpSourceType::Proxy;
        }
        // the proxy the operator is given is the one dns named, not the host the
        // app told it about
        let connect_proxy = match &proxy_ip {
            Some(ip) => ProxyInfo {
                ip: ip.clone(),
                port: proxy.port,
                ..proxy.clone()
            },
            None => ProxyInfo::none(),
        };

        let addresses: Vec<SocketAddress> = if use_proxy && proxy.kind == ProxyType::Http {
            let ip = proxy_ip.clone().unwrap_or_default();
            let mut address = SocketAddress::new(&ip, proxy.port);
            address.v4_to_v6_address(stack);
            vec![address]
        } else {
            self.profile
                .ip_items
                .iter()
                .map(|item| {
                    let mut address = SocketAddress::new(&item.ip, item.port);
                    // a proxy is reached at the address it is; anything else is
                    // mapped onto the stack the local network carries
                    if !use_proxy || proxy.kind.is_none() {
                        address.v4_to_v6_address(stack);
                    }
                    address
                })
                .collect()
        };
        if addresses.is_empty() {
            self.run_response_error(ErrCmdType::Dns, ECT_DNS_MAKE_SOCKET_PREPARED, false);
            return Err(ConnectFail::NoAddress);
        }

        self.profile.nat64 = stack == LocalIpStack::IPv6;
        self.profile.dns_endtime = now;
        self.profile.transport_protocol = self.protocol();

        let label = self.net_label();
        self.profile.net_type = label.label;
        if label.kind == K_MOBILE {
            // the label of a mobile network is the isp code
            self.profile.ispcode = self.profile.net_type.parse().unwrap_or(0);
        }

        if self.profile.ip_type != IpSourceType::Proxy && self.keep_alive {
            let items = self.profile.ip_items.clone();
            for (index, item) in items.iter().enumerate() {
                let Some(socket) = self.cache_socket(item) else {
                    continue;
                };
                self.profile.conn_rtt = 0;
                self.profile.ip_index = i32::try_from(index).unwrap_or(0);
                self.profile.conn_cost = 0;
                self.profile.host = item.host.clone();
                self.profile.ip_type = item.source_type;
                self.profile.ip = item.ip.clone();
                self.profile.port = item.port;
                self.profile.transport_protocol = item.transport_protocol;
                self.profile.conn_time = now;
                self.profile.start_connect_time = now;
                self.profile.connect_successful_time = now;
                if let Some(local) = self.local_address(socket) {
                    self.profile.local_ip = local.ip().to_string();
                    self.profile.local_port = local.port();
                }
                self.profile.socket_fd = socket;
                self.profile.is_reused_fd = true;
                let identify = self.identify(socket);
                self.profile.connection_identify = format!("{identify}@REUSE");
                return Ok(socket);
            }
        }

        self.profile.start_connect_time = now;
        let (v4, v6) = if self.outer.is_empty() {
            self.ip_connect_timeout()
        } else {
            (self.v4_connect_timeout_ms, self.v6_connect_timeout_ms)
        };
        if let Some(operator) = self.operator.as_mut() {
            operator.set_ip_connection_timeout(v4, v6);
        }
        let socket = self.open(&addresses, &connect_proxy);
        self.profile.connect_successful_time = now;

        let connected = self.operator_profile();
        self.profile.conn_rtt = connected.rtt;
        self.profile.ip_index = connected.index;
        self.profile.conn_cost = u64::from(connected.total_cost);
        self.profile.is0rtt = connected.is_0rtt;

        self.connect_event(socket.is_valid(), connected.rtt, connected.index);

        if !socket.is_valid() {
            self.profile.conn_errcode = connected.error_code;
            if self.is_broken() {
                // a link the app took down itself is not a socket error
                self.profile.disconn_errtype = ErrCmdType::Canceld;
                return Err(ConnectFail::Canceld {
                    error_code: connected.error_code,
                });
            }
            self.run_response_error(ErrCmdType::Socket, ECT_SOCKET_MAKE_SOCKET_PREPARED, false);
            return Err(ConnectFail::Socket {
                error_code: connected.error_code,
            });
        }

        let index = usize::try_from(connected.index).unwrap_or(usize::MAX);
        // the pairs that lost: the C++ reports the ones its connect had
        // *started*, which the host does not say, so every pair before the one
        // that won is reported
        let losers: Vec<IpPortItem> = self.profile.ip_items.iter().take(index).cloned().collect();
        for item in &losers {
            self.report(
                ErrCmdType::Socket,
                ETIMEDOUT,
                &item.ip,
                &item.host,
                item.port,
            );
        }

        if let Some(item) = self.profile.ip_items.get(index) {
            self.profile.host = item.host.clone();
            self.profile.ip_type = item.source_type;
            self.profile.ip = item.ip.clone();
            // the C++ leaves the port at the first pair's, whichever pair won
        }
        self.profile.conn_time = now;
        if let Some(local) = self.local_address(socket) {
            self.profile.local_ip = local.ip().to_string();
            self.profile.local_port = local.port();
        }
        self.profile.connection_identify = self.identify(socket);
        if first_is_v6(&addresses) && connected.index > 0 {
            self.profile.ipv6_connect_failed = true;
        }

        Ok(socket)
    }

    /// The same, with the reading of the clock the host's `gettickcount()`.
    pub fn connect(&mut self) -> Result<SocketFd, ConnectFail> {
        self.connect_at(gettickcount())
    }

    /// `int timeout` of `__RunReadWrite` — what a read of the answer waits for:
    /// the net source's own timeout for the cgi on a quic link, and
    /// [`DEFAULT_RW_TIMEOUT_MS`] otherwise. Which it was is on the profile.
    ///
    /// The C++ works it out just before its read loop starts; the port works it
    /// out with the write, which is the call before it.
    pub fn read_timeout_ms(&self) -> i32 {
        i32::try_from(self.profile.quic_rw_timeout_ms).unwrap_or(i32::MAX)
    }

    /// The write step of `__RunReadWrite` — the request goes out on the socket.
    ///
    /// The request is [`crate::shortlink`]'s: the url and the head the connect
    /// decided, and the body this is handed. What the profile gets is when the
    /// write started, and how long a read then waits; how long the write itself
    /// took is worked out at the read that follows it, which is the next time
    /// the port is handed a reading of the clock.
    pub fn write_at(&mut self, now: u64, socket: SocketFd, body: &[u8]) -> Result<usize, RunFail> {
        // a new answer to read, and nothing of the one before it
        self.answer = Parser::new();
        self.received = 0;
        self.profile.start_read_packet_time = 0;

        let url = request_url(&self.profile, &self.task.cgi);
        let headers = request_headers(&self.profile, &self.task);
        let request = pack(&url, &headers, body);

        let (timeout, source) = if self.protocol() == Task::TRANSPORT_PROTOCOL_QUIC {
            let cgi = self.task.cgi.clone();
            self.quic_rw_timeout(&cgi)
        } else {
            (DEFAULT_RW_TIMEOUT_MS, TimeoutSource::ClientDefault)
        };
        self.profile.quic_rw_timeout_ms = timeout;
        self.profile.quic_rw_timeout_source = source;

        self.profile.start_send_packet_time = now;
        match self.socket_send(socket, &request) {
            // `send_ret < 0` is the C++'s only failure; a write that brought
            // part of the request out is one too, since what would come back
            // is the answer to a request that was never finished
            Ok(len) if len >= request.len() => Ok(len),
            Ok(_) | Err(0) => {
                self.keep_alive = false;
                Err(self.fail(
                    RunFail::Socket {
                        err_code: ECT_SOCKET_WRITEN_WITH_NON_BLOCK,
                    },
                    true,
                ))
            }
            Err(err_code) => {
                // a request that did not go out is a socket that is not kept
                self.keep_alive = false;
                let err_code = if err_code == 0 {
                    ECT_SOCKET_WRITEN_WITH_NON_BLOCK
                } else {
                    err_code
                };
                Err(self.fail(RunFail::Socket { err_code }, true))
            }
        }
    }

    /// The same, with the reading of the clock the host's `gettickcount()`.
    pub fn write(&mut self, socket: SocketFd, body: &[u8]) -> Result<usize, RunFail> {
        self.write_at(gettickcount(), socket, body)
    }

    /// The read step of `__RunReadWrite` — what one `Recv` of the socket gave,
    /// which is the C++'s `(recv_ret, _errcode)` pair as a [`Result`]: [`Err`]
    /// is a read that brought nothing, with the platform's word for why, and
    /// [`Ok`] is what it brought, which is nothing at all when the peer hung up.
    ///
    /// [`Read::Again`] is a read that brought part of the answer — and one that
    /// timed out without bringing any, which is only an error on quic.
    /// [`Read::Done`] is the whole answer, and every way a run ends without one.
    pub fn read_at(&mut self, now: u64, socket: SocketFd, read: Result<&[u8], i32>) -> Read {
        let err_code = read.err().unwrap_or(0);
        if self.profile.start_read_packet_time == 0 {
            // the C++ reads the clock and the net label again just after the
            // write, which is where its read loop starts
            self.profile.start_read_packet_time = now;
            self.profile.send_request_cost =
                now.saturating_sub(self.profile.start_send_packet_time);
            let label = self.net_label();
            self.profile.net_type = label.label;
            if label.kind == K_MOBILE {
                self.profile.ispcode = self.profile.net_type.parse().unwrap_or(0);
            }
        }
        self.profile.rw_errcode = err_code;
        self.profile.read_packet_finished_time = now;
        self.profile.recv_reponse_cost = now.saturating_sub(self.profile.start_read_packet_time);

        if err_code == ENOTCONN && self.protocol() == Task::TRANSPORT_PROTOCOL_QUIC {
            // a quic link that never connected falls back to tcp, and does not
            // keep the socket
            self.profile.is_fast_fallback_tcp = 1;
            self.keep_alive = false;
        }

        // a read that failed is an error before anything else the C++ asks —
        // even before whether the app broke the run off
        if read.is_err() && err_code != ETIMEDOUT {
            self.keep_alive = false;
            let err_code = if err_code == 0 {
                ECT_SOCKET_READ_ONCE
            } else {
                err_code
            };
            return self.over(RunFail::Socket { err_code }, true);
        }

        // `socketOperator_->Breaker().IsBreak()` — the app's own cancel, which
        // the C++ writes on the profile and says nothing about to anyone
        if self.is_broken() {
            self.profile.disconn_errtype = ErrCmdType::Canceld;
            return Read::Done(Err(RunFail::Canceld));
        }

        let bytes = match read {
            Ok(bytes) => bytes,
            Err(_) => {
                // a read that timed out: on quic, that is the end of the run
                if self.protocol() == Task::TRANSPORT_PROTOCOL_QUIC {
                    self.keep_alive = false;
                    return self.over(
                        RunFail::Socket {
                            err_code: ECT_SOCKET_RECV_ERR,
                        },
                        true,
                    );
                }
                return Read::Again;
            }
        };
        if bytes.is_empty() {
            if self.protocol() == Task::TRANSPORT_PROTOCOL_QUIC {
                // quic has no hang-up to speak of: nothing came, so read again
                return Read::Again;
            }
            // the peer hung up, which a socket the pool handed out is not
            // reported for: the C++ has already had its turn with that one
            let report = !self.profile.is_reused_fd;
            return self.over(
                RunFail::Socket {
                    err_code: ECT_SOCKET_SHUTDOWN,
                },
                report,
            );
        }

        self.received += bytes.len();
        self.on_recv(bytes.len(), self.received);

        match self.answer.recv(bytes) {
            RecvStatus::FirstLineError => self.over(
                RunFail::Http {
                    err_code: ECT_HTTP_PARSE_STATUS_LINE,
                },
                true,
            ),
            RecvStatus::HeaderFieldsError | RecvStatus::BodyError => self.over(
                RunFail::Http {
                    err_code: ECT_HTTP_SPLIT_HTTP_HEAD_AND_BODY,
                },
                true,
            ),
            RecvStatus::End => self.answered(socket),
            // a first line, a head or a body that is not whole yet
            _ => Read::Again,
        }
    }

    /// The same, with the reading of the clock the host's `gettickcount()`.
    pub fn read(&mut self, socket: SocketFd, read: Result<&[u8], i32>) -> Read {
        self.read_at(gettickcount(), socket, read)
    }

    /// `__Run` — the whole run: the connect, the write, and the reads until the
    /// answer is whole.
    ///
    /// `reads` is the socket: one `Recv` per item, each with the reading of the
    /// clock it came at, which is the `gettickcount()` the C++ reads inside its
    /// loop. The one reading this is handed is the start of the run, which the
    /// C++ reads once for the connect and again for the write. [`None`] is a run whose reads ran out with the answer still not
    /// whole, which is where the C++ would still be blocked on the socket —
    /// [`ShortLink::end_run`] is still to be called for such a run.
    ///
    /// The `req2buf` thread the C++ starts and waits for is not here: turning a
    /// task into a body is the app's own encoder, which comes with the task
    /// manager.
    pub fn run_at(
        &mut self,
        now: u64,
        body: &[u8],
        reads: impl Iterator<Item = (Result<Vec<u8>, i32>, u64)>,
    ) -> Option<Result<Vec<u8>, RunFail>> {
        self.profile.start_time = now;
        self.profile.tid = self.tid();
        self.profile.nettype_for_report = self.net_type_for_report();

        // a connect that failed has already said so on the profile, and has
        // already answered the app
        let socket = match self.connect_at(now) {
            Ok(socket) => socket,
            Err(fail) => return Some(Err(RunFail::Connect(fail))),
        };
        self.on_send(socket);
        if let Err(fail) = self.write_at(now, socket, body) {
            // the C++ leaves `__RunReadWrite` here too, and closes in `__Run`;
            // the socket is one this run is not keeping either way
            self.end_run(socket);
            return Some(Err(fail));
        }
        // `socketOperator_->Breaker().IsBreak()` — the C++ asks once more, just
        // after the write, and says nothing at all about it: the port says the
        // same thing it says for a break during the reads
        if self.is_broken() {
            self.profile.disconn_errtype = ErrCmdType::Canceld;
            // no answer was read, so the socket is not one the pool can hand
            // out again — the C++ keeps `is_keep_alive_` here, and loses it
            self.keep_alive = false;
            self.end_run(socket);
            return Some(Err(RunFail::Canceld));
        }
        for (read, at) in reads {
            match self.read_at(at, socket, read.as_deref().map_err(|code| *code)) {
                Read::Again => continue,
                Read::Done(done) => {
                    self.end_run(socket);
                    return Some(done);
                }
            }
        }
        None
    }

    /// The same, with the reading of the clock the host's `gettickcount()` for
    /// the start of the run.
    pub fn run(
        &mut self,
        body: &[u8],
        reads: impl Iterator<Item = (Result<Vec<u8>, i32>, u64)>,
    ) -> Option<Result<Vec<u8>, RunFail>> {
        self.run_at(gettickcount(), body, reads)
    }

    /// The end of `__Run` — the signal the network had, and the socket: one the
    /// link is not keeping is closed.
    pub fn end_run(&mut self, socket: SocketFd) {
        let is_wifi = self.net_label().kind == K_WIFI;
        self.profile.disconn_signal = self.signal(is_wifi);
        if !self.keep_alive {
            self.socket_close(socket);
        }
    }

    /// The `kEnd` of the parser — the answer is whole: what the server said
    /// about keeping the socket, and then the fork on the status.
    fn answered(&mut self, socket: SocketFd) -> Read {
        if self.keep_alive {
            let transport = self.profile.transport_protocol;
            match keep_alive(self.answer.fields(), true, transport) {
                KeepAlive::Reuse { timeout } => {
                    self.profile.keepalive_timeout = timeout;
                    // the socket the pool may keep is the one the answer came on
                    self.profile.socket_fd = socket;
                }
                // the server said close, so the task's own `Keep-Alive` is not
                // one after all
                KeepAlive::Closed => self.keep_alive = false,
            }
        }

        match answer(&self.answer) {
            Some(Answer::Ok(body)) => {
                // the body is copied out before the link is asked to answer with
                // it, which is a mutable borrow of the whole link
                let body = body.to_vec();
                self.respond(ErrCmdType::Ok, 200, &body, false);
                Read::Done(Ok(body))
            }
            Some(Answer::Http(status)) => self.over(RunFail::Http { err_code: status }, true),
            // the parser said `kEnd` and the answer says it did not, which is
            // not a thing that happens: read again
            None => Read::Again,
        }
    }

    /// `__RunResponseError` — the connect's way of saying how the link went
    /// away: the same [`ShortLink::respond`] an answer goes through, with no
    /// body to hand over.
    fn run_response_error(&mut self, err_type: ErrCmdType, err_code: i32, report: bool) {
        self.respond(err_type, err_code, &[], report);
    }

    /// `__OnResponse` — the run is over, one way or the other: what the C++
    /// writes on the profile, the report it makes when it is asked to, and the
    /// answer it hands to the app — or to [`ShortLink::respond_default`], which
    /// is the C++'s own `__OnResponseImp` when the app has not asked for one.
    fn respond(&mut self, err_type: ErrCmdType, err_code: i32, body: &[u8], report: bool) {
        self.profile.disconn_errtype = err_type;
        self.profile.disconn_errcode = err_code;
        self.profile.channel_type = Task::CHANNEL_SHORT;
        if report && err_type != ErrCmdType::Ok {
            let ip = self.profile.ip.clone();
            let host = self.profile.host.clone();
            let port = self.profile.port;
            self.report(err_type, err_code, &ip, &host, port);
        }
        if let Some(on_response) = self.on_response.as_mut() {
            on_response(err_type, err_code, body, &self.profile);
        } else {
            self.respond_default(err_type, err_code);
        }
    }

    /// `__OnResponseImp` — what the link does with an answer the app did not ask
    /// to be handed: the status is said, and the socket is closed or handed back
    /// to the pool.
    ///
    /// The parts of the C++'s that need the app's decoder — `Buf2Resp`, the
    /// retry it may ask for, and the statistic — are not here: they come with
    /// the task manager.
    fn respond_default(&mut self, err_type: ErrCmdType, err_code: i32) {
        self.response_status(err_code);
        if !self.keep_alive || !self.profile.socket_fd.is_valid() {
            return;
        }
        if err_type != ErrCmdType::Ok {
            let socket = self.profile.socket_fd;
            self.socket_close(socket);
            // a server that hung up is not a socket the pool got wrong
            if err_code != ECT_SOCKET_SHUTDOWN {
                self.pool_report(self.profile.is_reused_fd, false, false);
            }
        } else {
            // the C++ asserts that the pair that won is on the list, which it
            // always is: the port asks the list instead
            let index = usize::try_from(self.profile.ip_index).unwrap_or(usize::MAX);
            let item = self.profile.ip_items.get(index).cloned();
            if let Some(item) = item {
                self.pool_cache(&item);
            }
        }
    }

    /// The run is over without an answer: the profile says why, the app hears
    /// about it, and the run is handed back as a [`RunFail`].
    fn fail(&mut self, fail: RunFail, report: bool) -> RunFail {
        self.respond(fail.err_cmd_type(), fail.err_code(), &[], report);
        fail
    }

    /// The same, as what a [`Read`] hands back.
    fn over(&mut self, fail: RunFail, report: bool) -> Read {
        Read::Done(Err(self.fail(fail, report)))
    }

    fn report(&mut self, err_type: ErrCmdType, err_code: i32, ip: &str, host: &str, port: u16) {
        if let Some(report) = self.network_report.as_mut() {
            report(err_type, err_code, ip, host, port);
        }
    }

    fn shortlink_items(&mut self) -> Vec<IpPortItem> {
        let hosts = self.task.shortlink_host_list.clone();
        let cgi = self.task.cgi.clone();
        match self.items.as_mut() {
            Some(items) => items(&hosts, &cgi),
            None => Vec::new(),
        }
    }

    fn proxy(&mut self, host: &str) -> ProxyInfo {
        match self.proxy.as_mut() {
            Some(proxy) => proxy(host),
            None => ProxyInfo::none(),
        }
    }

    fn debug_ip(&mut self) -> String {
        match self.debug_ip.as_mut() {
            Some(debug_ip) => debug_ip(),
            None => String::new(),
        }
    }

    fn shortlink_port(&mut self) -> u16 {
        match self.port.as_mut() {
            Some(port) => port(),
            None => 0,
        }
    }

    fn ip_connect_timeout(&mut self) -> (u32, u32) {
        match self.ip_connect_timeout.as_mut() {
            Some(timeout) => timeout(),
            None => (0, 0),
        }
    }

    fn dns(&mut self, host: &str) -> Vec<String> {
        match self.dns.as_mut() {
            Some(dns) => dns(host),
            None => Vec::new(),
        }
    }

    fn local_stack(&mut self) -> LocalIpStack {
        match self.local_stack.as_mut() {
            Some(stack) => stack(),
            None => LocalIpStack::None,
        }
    }

    fn net_label(&mut self) -> NetworkLabel {
        match self.net_label.as_mut() {
            Some(label) => label(),
            None => NetworkLabel::default(),
        }
    }

    fn local_address(&mut self, socket: SocketFd) -> Option<SocketAddress> {
        self.local_address.as_mut().and_then(|local| local(socket))
    }

    fn cache_socket(&mut self, item: &IpPortItem) -> Option<SocketFd> {
        self.cache_socket.as_mut().and_then(|cache| cache(item))
    }

    fn connect_event(&mut self, is_success: bool, rtt: u32, index: i32) {
        if let Some(event) = self.connect_event.as_mut() {
            event(is_success, rtt, index);
        }
    }

    fn on_send(&mut self, socket: SocketFd) {
        if let Some(on_send) = self.on_send.as_mut() {
            on_send(socket);
        }
    }

    fn on_recv(&mut self, cached_size: usize, total_size: usize) {
        if let Some(on_recv) = self.on_recv.as_mut() {
            on_recv(cached_size, total_size);
        }
    }

    fn response_status(&mut self, status: i32) {
        if let Some(response_status) = self.response_status.as_mut() {
            response_status(status);
        }
    }

    fn pool_report(&mut self, is_reused: bool, has_received: bool, is_decode_ok: bool) {
        if let Some(report) = self.pool_report.as_mut() {
            report(is_reused, has_received, is_decode_ok);
        }
    }

    fn pool_cache(&mut self, item: &IpPortItem) {
        if let Some(cache) = self.pool_cache.as_mut() {
            cache(item, &self.profile);
        }
    }

    /// `NetSource::GetQUICRWTimeoutMs` — how long a read waits on a quic link,
    /// which the net source answers for the cgi. Unset is
    /// [`DEFAULT_RW_TIMEOUT_MS`] out of the client, which is the C++'s own
    /// `kDefaultRWTimeoutMs` default.
    fn quic_rw_timeout(&mut self, cgi: &str) -> (u32, TimeoutSource) {
        match self.quic_rw_timeout.as_mut() {
            Some(timeout) => timeout(cgi),
            None => (DEFAULT_RW_TIMEOUT_MS, TimeoutSource::ClientDefault),
        }
    }

    /// `mars::comm::getNetTypeForStatistics` — the network for the report, which
    /// the C++ only asks for once, before the connect.
    fn net_type_for_report(&mut self) -> i32 {
        match self.net_type_for_report.as_mut() {
            Some(net_type) => net_type(),
            None => K_WIFI,
        }
    }

    /// `::getSignal(_is_wifi)` — how strong the network was when the run ended.
    fn signal(&mut self, is_wifi: bool) -> i32 {
        match self.signal.as_mut() {
            Some(signal) => signal(is_wifi),
            None => 0,
        }
    }

    /// `xlogger_tid` — which thread the run was on.
    fn tid(&mut self) -> i64 {
        match self.tid.as_mut() {
            Some(tid) => tid(),
            None => 0,
        }
    }

    fn open(&mut self, addresses: &[SocketAddress], proxy: &ProxyInfo) -> SocketFd {
        match self.operator.as_mut() {
            Some(operator) => operator.connect(addresses, proxy),
            None => SocketFd::INVALID,
        }
    }

    /// `socketOperator_->Send` — the request goes out, whole and once: the C++
    /// hands `-1` for the timeout, which is the socket's own.
    ///
    /// An operator that is not there has no socket to write on, which is a write
    /// that did not happen.
    fn socket_send(&mut self, socket: SocketFd, request: &[u8]) -> Result<usize, i32> {
        match self.operator.as_mut() {
            Some(operator) => operator.send(socket, request, -1),
            None => Err(0),
        }
    }

    fn socket_close(&mut self, socket: SocketFd) {
        if let Some(operator) = self.operator.as_mut() {
            operator.close(socket);
        }
    }

    fn operator_profile(&self) -> SocketProfile {
        match self.operator.as_ref() {
            Some(operator) => operator.profile(),
            None => SocketProfile::default(),
        }
    }

    /// `SocketOperator::Protocol` — unset is tcp, which is what the C++'s
    /// `TcpSocketOperator` answers.
    fn protocol(&self) -> i32 {
        match self.operator.as_ref() {
            Some(operator) => operator.protocol(),
            None => Task::TRANSPORT_PROTOCOL_TCP,
        }
    }

    /// `SocketOperator::Identify` — what the socket is called in a log. An
    /// operator that is not there has no name for it, so the log gets the
    /// number.
    fn identify(&self, socket: SocketFd) -> String {
        match self.operator.as_ref() {
            Some(operator) => operator.identify(socket),
            None => socket.0.to_string(),
        }
    }

    /// Whether the app broke the connect off: `socketOperator_->Breaker()`.
    fn is_broken(&mut self) -> bool {
        match self.operator.as_mut() {
            Some(operator) => operator.breaker().is_break(),
            None => false,
        }
    }
}

impl std::fmt::Debug for ShortLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShortLink")
            .field("cgi", &self.task.cgi)
            .field("taskid", &self.task.taskid)
            .field("ip", &self.profile.ip)
            .field("port", &self.profile.port)
            .field("keep_alive", &self.keep_alive)
            .finish()
    }
}

/// `ShortLink::__ContainIPv6` — whether the **first** of the addresses is a v6
/// one, which is not the same question as whether any of them is: what the C++
/// wants to know is whether a v6 pair was tried *first*.
fn first_is_v6(addresses: &[SocketAddress]) -> bool {
    addresses.first().is_some_and(SocketAddress::is_v6)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::socket_operator::OpBreaker;

    /// What the host was asked for, and what it answers with.
    #[derive(Clone, Default)]
    struct Seen {
        addresses: Arc<Mutex<Vec<Vec<SocketAddress>>>>,
        proxies: Arc<Mutex<Vec<ProxyInfo>>>,
        timeouts: Arc<Mutex<Vec<(u32, u32)>>>,
        reports: Arc<Mutex<Vec<Reported>>>,
        events: Arc<Mutex<Vec<(bool, u32, i32)>>>,
        cached: Arc<Mutex<Vec<IpPortItem>>>,
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
        closed: Arc<Mutex<Vec<SocketFd>>>,
        /// A write that does not happen: `None` is one that does.
        fail_send: Arc<Mutex<Option<i32>>>,
        /// A write that brings only part of the request out.
        short_send: Arc<Mutex<bool>>,
    }

    impl Seen {
        fn sent(&self) -> Vec<Vec<u8>> {
            self.sent.lock().unwrap().clone()
        }

        fn closed(&self) -> Vec<SocketFd> {
            self.closed.lock().unwrap().clone()
        }
    }

    /// One thing a link reported: the error, and the pair it was about.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Reported {
        err_type: ErrCmdType,
        err_code: i32,
        ip: String,
        host: String,
        port: u16,
    }

    /// A pipe that is never woken unless a test wakes it: the port has nothing
    /// blocking to give up.
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
        protocol: i32,
    }

    impl Host {
        /// The same, on a quic socket.
        fn quic(mut self) -> Self {
            self.protocol = Task::TRANSPORT_PROTOCOL_QUIC;
            self
        }

        fn new(seen: Seen) -> Self {
            Self {
                seen,
                next: 3,
                profile: SocketProfile::default(),
                breaker: Breaker::default(),
                protocol: Task::TRANSPORT_PROTOCOL_TCP,
            }
        }
    }

    impl SocketOperator for Host {
        fn connect(&mut self, addresses: &[SocketAddress], proxy: &ProxyInfo) -> SocketFd {
            self.seen.addresses.lock().unwrap().push(addresses.to_vec());
            self.seen.proxies.lock().unwrap().push(proxy.clone());
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
            timeout_ms: i32,
        ) -> Result<usize, i32> {
            // the C++ hands `-1`, which is the socket's own timeout
            assert_eq!(timeout_ms, -1);
            self.seen.sent.lock().unwrap().push(buffer.to_vec());
            if *self.seen.short_send.lock().unwrap() {
                return Ok(buffer.len().saturating_sub(1));
            }
            match *self.seen.fail_send.lock().unwrap() {
                None => Ok(buffer.len()),
                Some(error) => Err(error),
            }
        }

        fn recv(
            &mut self,
            _socket: SocketFd,
            _max_size: usize,
            _timeout_ms: i32,
            _wait_full_size: bool,
        ) -> Result<Vec<u8>, i32> {
            Ok(Vec::new())
        }

        fn close(&mut self, socket: SocketFd) {
            self.seen.closed.lock().unwrap().push(socket);
        }

        fn identify(&self, socket: SocketFd) -> String {
            format!("{}@TCP", socket.0)
        }

        fn protocol(&self) -> i32 {
            self.protocol
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

        fn create_stream(&mut self, _socket: SocketFd) -> SocketFd {
            SocketFd::INVALID
        }

        fn set_ip_connection_timeout(&mut self, v4_timeout_ms: u32, v6_timeout_ms: u32) {
            self.seen
                .timeouts
                .lock()
                .unwrap()
                .push((v4_timeout_ms, v6_timeout_ms));
        }
    }

    /// A task on one host, and the host list that goes with it.
    fn task() -> Task {
        let mut task = Task::new(7, 12);
        task.cgi = "/cgi-bin/micromsg-bin/short".to_string();
        task.shortlink_host_list = vec!["short.weixin.qq.com".to_string()];
        task
    }

    /// One pair the link may be made on.
    fn item(ip: &str, port: u16) -> IpPortItem {
        let mut item = IpPortItem::new(ip, port);
        item.source_type = IpSourceType::Dns;
        item.host = "short.weixin.qq.com".to_string();
        item
    }

    /// A link on one pair, with the host's sockets behind it.
    fn link(seen: &Seen) -> ShortLink {
        link_of(seen, false)
    }

    /// The same, one that may go through a proxy.
    fn proxy_link(seen: &Seen) -> ShortLink {
        link_of(seen, true)
    }

    fn link_of(seen: &Seen, use_proxy: bool) -> ShortLink {
        link_for(seen, task(), use_proxy)
    }

    /// The same, for a task of the test's own.
    fn link_for(seen: &Seen, task: Task, use_proxy: bool) -> ShortLink {
        let mut link = ShortLink::new(task, use_proxy);
        link.set_socket_operator(Host::new(seen.clone()));
        link.set_shortlink_items(|_, _| vec![item("183.3.226.35", 80)]);
        link.set_shortlink_port(|| 80);
        link.set_ip_connect_timeout(|| (1500, 2500));
        link.set_local_ip_stack(|| LocalIpStack::IPv4);
        link.set_network_report({
            let seen = seen.clone();
            move |err_type, err_code, ip, host, port| {
                seen.reports.lock().unwrap().push(Reported {
                    err_type,
                    err_code,
                    ip: ip.to_string(),
                    host: host.to_string(),
                    port,
                });
            }
        });
        link.set_connect_event({
            let seen = seen.clone();
            move |is_success, rtt, index| seen.events.lock().unwrap().push((is_success, rtt, index))
        });
        link.set_cache_socket({
            let seen = seen.clone();
            move |item| {
                seen.cached.lock().unwrap().push(item.clone());
                None
            }
        });
        link
    }

    #[test]
    fn a_link_that_has_not_been_made_is_the_task_it_is_for() {
        let link = ShortLink::new(task(), true);
        assert_eq!(link.task().cgi, "/cgi-bin/micromsg-bin/short");
        assert_eq!(link.profile().task_id, 7);
        assert_eq!(link.profile().cgi, "/cgi-bin/micromsg-bin/short");
        assert_eq!(link.profile().link_type, Task::CHANNEL_SHORT);
        assert_eq!(link.sent_count(), 0);
        assert!(!link.is_keep_alive());
        assert!(link.outer().is_empty());

        // a task of its own `Connection: Keep-Alive` is one that is kept
        let mut kept = task();
        kept.headers
            .insert("Connection".to_string(), "Keep-Alive".to_string());
        assert!(ShortLink::new(kept, false).is_keep_alive());

        let mut sent = ShortLink::new(task(), false);
        sent.set_sent_count(3);
        assert_eq!(sent.sent_count(), 3);
    }

    #[test]
    fn a_connect_lands_on_the_pair_it_was_given() {
        let seen = Seen::default();
        let mut link = link(&seen);
        let socket = link.connect_at(1000).unwrap();

        assert!(socket.is_valid());
        assert_eq!(link.profile().ip, "183.3.226.35");
        assert_eq!(link.profile().port, 80);
        assert_eq!(link.profile().host, "short.weixin.qq.com");
        assert_eq!(link.profile().ip_index, 0);
        assert_eq!(link.profile().dns_time, 1000);
        assert_eq!(link.profile().dns_endtime, 1000);
        assert_eq!(link.profile().start_connect_time, 1000);
        assert_eq!(link.profile().connect_successful_time, 1000);
        assert_eq!(link.profile().conn_time, 1000);
        assert_eq!(link.profile().connection_identify, "3@TCP");
        assert!(!link.profile().is_reused_fd);
        assert!(!link.profile().ipv6_connect_failed);

        // the near end of the socket is the host's to answer, and none did
        assert_eq!(link.profile().local_ip, "");

        let addresses = seen.addresses.lock().unwrap();
        assert_eq!(addresses.len(), 1);
        assert_eq!(addresses[0].len(), 1);
        assert_eq!(addresses[0][0].ip(), "183.3.226.35");
    }

    #[test]
    fn a_link_with_no_pair_at_all_says_so_on_the_profile() {
        let seen = Seen::default();
        let mut link = link(&seen);
        link.set_shortlink_items(|_, _| Vec::new());

        assert_eq!(link.connect_at(1000), Err(ConnectFail::NoAddress));
        assert_eq!(link.profile().disconn_errtype, ErrCmdType::Dns);
        assert_eq!(link.profile().disconn_errcode, ECT_DNS_MAKE_SOCKET_PREPARED);
        assert!(
            seen.addresses.lock().unwrap().is_empty(),
            "no connect at all"
        );
        // `false` is what the C++ hands `__RunResponseError` here: no report
        assert!(seen.reports.lock().unwrap().is_empty());
    }

    #[test]
    fn a_link_with_no_operator_has_no_socket() {
        let mut link = ShortLink::new(task(), false);
        link.set_shortlink_items(|_, _| vec![item("183.3.226.35", 80)]);

        assert_eq!(
            link.connect_at(1000),
            Err(ConnectFail::Socket { error_code: 0 })
        );
        assert_eq!(link.profile().disconn_errtype, ErrCmdType::Socket);
        assert_eq!(link.profile().conn_errcode, 0);
        assert_eq!(
            link.profile().connection_identify,
            "",
            "no socket, so nothing to name"
        );

        // and nothing can be written on one either
        assert_eq!(
            link.write_at(1100, SocketFd::INVALID, b"hello"),
            Err(RunFail::Socket {
                err_code: ECT_SOCKET_WRITEN_WITH_NON_BLOCK
            })
        );
    }

    #[test]
    fn a_connect_that_failed_reports_a_socket_error() {
        let seen = Seen::default();
        let mut link = link(&seen);
        // the host's connect answers no socket, with the platform's word for it
        let mut host = Host::new(seen.clone());
        host.profile.error_code = 110;
        link.set_socket_operator(host);

        assert_eq!(
            link.connect_at(1000),
            Err(ConnectFail::Socket { error_code: 110 })
        );
        assert_eq!(link.profile().disconn_errtype, ErrCmdType::Socket);
        assert_eq!(
            link.profile().disconn_errcode,
            ECT_SOCKET_MAKE_SOCKET_PREPARED
        );
        assert_eq!(link.profile().conn_errcode, 110);
        assert_eq!(
            seen.events.lock().unwrap().as_slice(),
            &[(false, 0, 0)],
            "the connect event is told it got no socket"
        );
        // and it is not reported: the C++ hands `__RunResponseError` a `false`
        assert!(seen.reports.lock().unwrap().is_empty());
    }

    #[test]
    fn a_connect_the_app_broke_off_is_cancelled_and_not_reported() {
        let seen = Seen::default();
        let mut link = link(&seen);
        let mut host = Host::new(seen.clone());
        host.profile.error_code = 110;
        host.breaker.broken = true;
        link.set_socket_operator(host);

        assert_eq!(
            link.connect_at(1000),
            Err(ConnectFail::Canceld { error_code: 110 })
        );
        assert_eq!(
            link.profile().disconn_errtype,
            ErrCmdType::Canceld,
            "not a socket error"
        );
        assert_eq!(
            link.profile().disconn_errcode,
            0,
            "nothing is reported, so nothing is on the profile"
        );
        assert_eq!(
            ConnectFail::Canceld { error_code: 110 }.err_cmd_type(),
            ErrCmdType::Canceld
        );
    }

    #[test]
    fn the_pairs_that_lost_are_reported_and_the_one_that_won_is_the_profile_s() {
        let seen = Seen::default();
        let mut link = link(&seen);
        link.set_shortlink_items(|_, _| {
            vec![
                item("183.3.226.35", 80),
                item("183.3.226.36", 80),
                item("183.3.226.37", 8080),
            ]
        });
        let mut host = Host::new(seen.clone());
        host.profile.index = 2;
        host.profile.rtt = 40;
        host.profile.total_cost = 60;
        link.set_socket_operator(host);

        let socket = link.connect_at(1000).unwrap();
        assert!(socket.is_valid());
        assert_eq!(link.profile().ip, "183.3.226.37");
        assert_eq!(
            link.profile().port,
            80,
            "the C++ leaves the port at the first pair's"
        );
        assert_eq!(link.profile().ip_index, 2);
        assert_eq!(link.profile().conn_rtt, 40);
        assert_eq!(link.profile().conn_cost, 60);

        assert_eq!(
            seen.reports.lock().unwrap().as_slice(),
            &[
                Reported {
                    err_type: ErrCmdType::Socket,
                    err_code: ETIMEDOUT,
                    ip: "183.3.226.35".to_string(),
                    host: "short.weixin.qq.com".to_string(),
                    port: 80,
                },
                Reported {
                    err_type: ErrCmdType::Socket,
                    err_code: ETIMEDOUT,
                    ip: "183.3.226.36".to_string(),
                    host: "short.weixin.qq.com".to_string(),
                    port: 80,
                },
            ],
            "every pair before the one that won"
        );
    }

    #[test]
    fn a_v6_pair_that_lost_is_said_so() {
        let seen = Seen::default();
        let mut lost = link(&seen);
        lost.set_shortlink_items(|_, _| vec![item("::1", 80), item("183.3.226.35", 80)]);
        let mut host = Host::new(seen.clone());
        host.profile.index = 1;
        lost.set_socket_operator(host);

        lost.connect_at(1000).unwrap();
        assert!(lost.profile().ipv6_connect_failed);

        // … a v6 pair that won is not one that failed
        let seen = Seen::default();
        let mut won = link(&seen);
        won.set_shortlink_items(|_, _| vec![item("::1", 80), item("183.3.226.35", 80)]);
        won.connect_at(1000).unwrap();
        assert!(!won.profile().ipv6_connect_failed);

        // nor is a v4 pair that lost to a later v4 one
        let seen = Seen::default();
        let mut v4s = link(&seen);
        v4s.set_shortlink_items(|_, _| vec![item("183.3.226.35", 80), item("183.3.226.36", 80)]);
        let mut host = Host::new(seen.clone());
        host.profile.index = 1;
        v4s.set_socket_operator(host);
        v4s.connect_at(1000).unwrap();
        assert!(!v4s.profile().ipv6_connect_failed);
    }

    #[test]
    fn a_task_that_asked_to_be_kept_takes_the_socket_the_pool_has() {
        let mut task = task();
        task.headers
            .insert("Connection".to_string(), "Keep-Alive".to_string());
        let seen = Seen::default();
        let mut link = ShortLink::new(task, false);
        link.set_socket_operator(Host::new(seen.clone()));
        link.set_shortlink_items(|_, _| vec![item("183.3.226.35", 80), item("183.3.226.36", 80)]);
        link.set_cache_socket(|item| {
            // only the second pair has one
            (item.ip == "183.3.226.36").then_some(SocketFd(9))
        });

        assert_eq!(link.connect_at(1000), Ok(SocketFd(9)));
        assert_eq!(link.profile().ip, "183.3.226.36");
        assert_eq!(link.profile().ip_index, 1);
        assert_eq!(link.profile().conn_rtt, 0);
        assert_eq!(link.profile().conn_cost, 0);
        assert_eq!(link.profile().conn_time, 1000);
        assert_eq!(link.profile().start_connect_time, 1000);
        assert_eq!(link.profile().connect_successful_time, 1000);
        assert_eq!(link.profile().socket_fd, SocketFd(9));
        assert!(link.profile().is_reused_fd);
        assert_eq!(link.profile().connection_identify, "9@TCP@REUSE");
        assert!(
            seen.addresses.lock().unwrap().is_empty(),
            "a reused socket is not connected"
        );
    }

    #[test]
    fn a_task_that_did_not_ask_to_be_kept_does_not_look_in_the_pool() {
        let seen = Seen::default();
        let mut link = link(&seen);
        link.connect_at(1000).unwrap();
        assert!(
            seen.cached.lock().unwrap().is_empty(),
            "the pool is only asked for a link that is kept"
        );
    }

    #[test]
    fn an_http_proxy_is_connected_to_and_goes_on_the_list_first() {
        let seen = Seen::default();
        let mut link = proxy_link(&seen);
        link.set_proxy(|_| ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "", ""));

        link.connect_at(1000).unwrap();

        assert_eq!(link.profile().ip_type, IpSourceType::Proxy);
        assert_eq!(link.profile().ip, "10.0.0.1");
        assert_eq!(
            link.profile().port,
            80,
            "the short-link port of the pair it was pushed as, not the proxy's"
        );
        assert_eq!(link.profile().ip_items.len(), 2, "the proxy, then the pair");
        assert_eq!(link.profile().ip_items[0].source_type, IpSourceType::Proxy);

        // one address only: the proxy's
        let addresses = seen.addresses.lock().unwrap();
        assert_eq!(addresses[0].len(), 1);
        assert_eq!(addresses[0][0].ip(), "10.0.0.1");
        assert_eq!(addresses[0][0].port(), 8080);
        assert_eq!(
            seen.proxies.lock().unwrap()[0],
            ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "", "")
        );
        assert!(
            seen.cached.lock().unwrap().is_empty(),
            "a link through a proxy is never reused"
        );
    }

    #[test]
    fn a_proxy_named_by_host_is_the_one_dns_named() {
        let seen = Seen::default();
        let mut link = proxy_link(&seen);
        link.set_proxy(|_| ProxyInfo::new(ProxyType::Http, "proxy.example", "", 8080, "", ""));
        link.set_dns(|host| {
            assert_eq!(host, "proxy.example");
            vec!["10.0.0.2".to_string()]
        });

        link.connect_at(1000).unwrap();
        let addresses = seen.addresses.lock().unwrap();
        assert_eq!(addresses[0][0].ip(), "10.0.0.2");
        assert_eq!(seen.proxies.lock().unwrap()[0].ip, "10.0.0.2");
    }

    #[test]
    fn a_proxy_host_dns_could_not_name_is_no_socket_at_all() {
        let seen = Seen::default();
        let mut link = proxy_link(&seen);
        link.set_proxy(|_| ProxyInfo::new(ProxyType::Http, "proxy.example", "", 8080, "", ""));
        link.set_dns(|_| Vec::new());

        assert_eq!(link.connect_at(1000), Err(ConnectFail::NoProxyIp));
        assert_eq!(
            link.profile().disconn_errtype,
            ErrCmdType::Ok,
            "the C++ says nothing at all about this one"
        );
        assert!(seen.reports.lock().unwrap().is_empty());
        assert!(seen.addresses.lock().unwrap().is_empty());
    }

    #[test]
    fn a_tunnel_proxy_is_connected_through() {
        let seen = Seen::default();
        let mut link = proxy_link(&seen);
        link.set_proxy(|_| {
            ProxyInfo::new(
                ProxyType::HttpTunnel,
                "",
                "10.0.0.1",
                8080,
                "mars",
                "secret",
            )
        });

        link.connect_at(1000).unwrap();

        // the pair, not the proxy: a tunnel is connected *through*
        let addresses = seen.addresses.lock().unwrap();
        assert_eq!(addresses[0].len(), 1);
        assert_eq!(addresses[0][0].ip(), "183.3.226.35");
        let proxies = seen.proxies.lock().unwrap();
        assert_eq!(proxies[0].kind, ProxyType::HttpTunnel);
        assert_eq!(proxies[0].ip, "10.0.0.1");
        assert_eq!(proxies[0].username, "mars");
        assert_eq!(
            link.profile().ip_type,
            IpSourceType::Dns,
            "the C++ marks the profile as going through a proxy, and then overwrites it with the pair that won"
        );
    }

    #[test]
    fn a_debug_pair_is_where_the_link_goes_and_not_through_a_proxy() {
        let seen = Seen::default();
        let mut link = proxy_link(&seen);
        let mut debug = item("127.0.0.1", 8080);
        debug.source_type = IpSourceType::Debug;
        link.set_shortlink_items(move |_, _| vec![debug.clone()]);
        link.set_proxy(|_| ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "", ""));

        link.connect_at(1000).unwrap();
        assert_eq!(link.profile().ip_type, IpSourceType::Debug);
        assert_eq!(link.profile().ip, "127.0.0.1");
        let addresses = seen.addresses.lock().unwrap();
        assert_eq!(addresses[0][0].ip(), "127.0.0.1", "not the proxy's");
    }

    #[test]
    fn a_debug_ip_of_the_host_keeps_an_http_proxy_out() {
        let seen = Seen::default();
        let mut link = proxy_link(&seen);
        link.set_debug_ip(|| "127.0.0.1".to_string());
        link.set_proxy(|_| ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "", ""));

        link.connect_at(1000).unwrap();
        assert_eq!(link.profile().ip_type, IpSourceType::Dns);
        assert_eq!(link.profile().ip, "183.3.226.35");
        let addresses = seen.addresses.lock().unwrap();
        assert_eq!(
            addresses[0][0].ip(),
            "10.0.0.1",
            "the debug ip keeps the proxy off the list, but the connect still goes to it"
        );
    }

    #[test]
    fn the_pairs_the_app_handed_the_link_are_the_ones_used() {
        let seen = Seen::default();
        let mut link = link(&seen);
        link.set_connect_params(vec![item("10.0.0.9", 443)], 700, 900);
        link.set_shortlink_items(|_, _| vec![item("183.3.226.35", 80)]);

        link.connect_at(1000).unwrap();
        assert_eq!(link.profile().ip, "10.0.0.9");
        assert_eq!(
            seen.timeouts.lock().unwrap().as_slice(),
            &[(700, 900)],
            "the timeouts they came with, not the net source's"
        );
        assert_eq!(link.outer().len(), 1);
    }

    #[test]
    fn a_link_that_resolved_its_own_pairs_uses_the_net_source_timeouts() {
        let seen = Seen::default();
        let mut link = link(&seen);
        link.connect_at(1000).unwrap();
        assert_eq!(seen.timeouts.lock().unwrap().as_slice(), &[(1500, 2500)]);
    }

    #[test]
    fn the_network_the_connect_was_made_on_is_the_label_the_host_gave() {
        let seen = Seen::default();
        let mut wifi = link(&seen);
        wifi.set_net_label(|| NetworkLabel {
            kind: 1,
            label: "WIFI".to_string(),
        });

        wifi.connect_at(1000).unwrap();
        assert_eq!(wifi.profile().net_type, "WIFI");
        assert_eq!(wifi.profile().ispcode, 0, "only a mobile label is a number");

        // a mobile label *is* the isp code
        let mut mobile = link(&seen);
        mobile.set_net_label(|| NetworkLabel {
            kind: K_MOBILE,
            label: "46000".to_string(),
        });
        mobile.connect_at(1000).unwrap();
        assert_eq!(mobile.profile().net_type, "46000");
        assert_eq!(mobile.profile().ispcode, 46000);

        // and one that is not a number at all is no code
        let mut named = link(&seen);
        named.set_net_label(|| NetworkLabel {
            kind: K_MOBILE,
            label: "cmnet".to_string(),
        });
        named.connect_at(1000).unwrap();
        assert_eq!(named.profile().ispcode, 0);
    }

    #[test]
    fn a_network_that_carries_only_v6_is_nat64() {
        let seen = Seen::default();
        let mut link = link(&seen);
        link.set_local_ip_stack(|| LocalIpStack::IPv6);

        link.connect_at(1000).unwrap();
        assert!(link.profile().nat64);
        assert_eq!(link.profile().local_net_stack, LocalIpStack::IPv6);
        // a v4 pair on an IPv6-only network is reached at a nat64 address
        let addresses = seen.addresses.lock().unwrap();
        assert!(addresses[0][0].is_v6());
    }

    #[test]
    fn the_protocol_of_the_operator_is_the_one_the_profile_says() {
        let seen = Seen::default();
        let mut link = link(&seen);
        let mut host = Host::new(seen.clone());
        host.protocol = Task::TRANSPORT_PROTOCOL_QUIC;
        link.set_socket_operator(host);

        link.connect_at(1000).unwrap();
        assert_eq!(
            link.profile().transport_protocol,
            Task::TRANSPORT_PROTOCOL_QUIC
        );
        assert_eq!(
            link.profile().ip_items[0].transport_protocol,
            Task::TRANSPORT_PROTOCOL_QUIC,
            "every pair is marked quic, which is what the C++ does"
        );
    }

    #[test]
    fn the_near_end_of_the_socket_is_the_host_s_to_answer() {
        let seen = Seen::default();
        let mut link = link(&seen);
        link.set_local_address(|_| Some(SocketAddress::new("192.168.1.2", 34567)));

        link.connect_at(1000).unwrap();
        assert_eq!(link.profile().local_ip, "192.168.1.2");
        assert_eq!(link.profile().local_port, 34567);
    }

    #[test]
    fn why_a_connect_did_not_happen_is_what_the_profile_says() {
        assert_eq!(
            ConnectFail::NoAddress.err_code(),
            ECT_DNS_MAKE_SOCKET_PREPARED
        );
        assert_eq!(
            ConnectFail::Socket { error_code: 110 }.err_code(),
            ECT_SOCKET_MAKE_SOCKET_PREPARED
        );
        assert_eq!(ConnectFail::NoProxyIp.err_code(), 0);
        assert_eq!(ConnectFail::Canceld { error_code: 1 }.err_code(), 0);
        assert_eq!(ConnectFail::Socket { error_code: 110 }.error_code(), 110);
        assert_eq!(ConnectFail::NoAddress.error_code(), 0);
        assert_eq!(ConnectFail::NoAddress.err_cmd_type(), ErrCmdType::Dns);
        assert_eq!(ConnectFail::NoProxyIp.err_cmd_type(), ErrCmdType::Ok);
    }

    #[test]
    fn the_methods_that_take_no_reading_ask_the_clock_themselves() {
        let seen = Seen::default();
        let started = gettickcount();

        let mut first = link(&seen);
        let socket = first.connect().unwrap();
        assert!(first.profile().dns_time >= started);

        first.write(socket, b"hello").unwrap();
        assert!(first.profile().start_send_packet_time >= first.profile().dns_time);
        assert_eq!(
            first.read(socket, Ok(&answer_200())),
            Read::Done(Ok(b"hello".to_vec()))
        );
        assert!(first.profile().start_read_packet_time >= first.profile().start_send_packet_time);
        assert!(
            first.profile().read_packet_finished_time >= first.profile().start_read_packet_time
        );

        let mut run = link(&seen);
        let reads = vec![(Ok(answer_200()), gettickcount())];
        assert_eq!(
            run.run(b"hello", reads.into_iter()),
            Some(Ok(b"hello".to_vec()))
        );
        assert!(run.profile().start_time >= started);
    }

    #[test]
    fn the_first_address_is_the_one_a_v6_connect_is_judged_by() {
        let v4 = SocketAddress::new("1.1.1.1", 80);
        let v6 = SocketAddress::new("::1", 80);
        assert!(first_is_v6(&[v6.clone(), v4.clone()]));
        assert!(!first_is_v6(&[v4, v6]));
        assert!(!first_is_v6(&[]));
    }

    /// An answer of 200, with a body of `hello`.
    fn answer_200() -> Vec<u8> {
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec()
    }

    /// The same, one the C++ forks on: any status but 200 is `kEctHttp` with it.
    fn answer_500() -> Vec<u8> {
        b"HTTP/1.1 500 Server Error\r\nContent-Length: 0\r\n\r\n".to_vec()
    }

    /// A task that asked for the socket to be kept.
    fn kept_task() -> Task {
        let mut task = task();
        task.headers
            .insert("Connection".to_string(), "Keep-Alive".to_string());
        task
    }

    /// A link on a socket, ready to write.
    fn connected(seen: &Seen) -> (ShortLink, SocketFd) {
        let mut link = link(seen);
        let socket = link.connect_at(1000).unwrap();
        (link, socket)
    }

    /// A link on a socket the pool had, which is one that is reused.
    fn reused(seen: &Seen) -> (ShortLink, SocketFd) {
        let mut link = link_for(seen, kept_task(), false);
        link.set_cache_socket(|_| Some(SocketFd(11)));
        let socket = link.connect_at(1000).unwrap();
        (link, socket)
    }

    /// A link on a quic socket, which reads for as long as the net source says.
    fn quic_link_for(seen: &Seen, task: Task) -> ShortLink {
        let mut link = link_for(seen, task, false);
        link.set_socket_operator(Host::new(seen.clone()).quic());
        link
    }

    #[test]
    fn the_request_that_goes_out_is_the_one_the_task_asked_for() {
        let seen = Seen::default();
        let (mut link, socket) = connected(&seen);

        let request = pack(
            "/cgi-bin/micromsg-bin/short",
            &request_headers(link.profile(), &task()),
            b"hello",
        );
        assert_eq!(
            link.write_at(1100, socket, b"hello").unwrap(),
            request.len()
        );
        assert_eq!(seen.sent(), vec![request]);

        assert_eq!(link.profile().start_send_packet_time, 1100);
        assert_eq!(link.read_timeout_ms(), 5000, "tcp reads for five seconds");
        assert_eq!(
            link.profile().quic_rw_timeout_source,
            TimeoutSource::ClientDefault
        );
    }

    #[test]
    fn an_answer_of_two_hundred_is_the_body_of_it() {
        let seen = Seen::default();
        let (mut link, socket) = connected(&seen);
        let statuses = Arc::new(Mutex::new(Vec::new()));
        link.set_response_status({
            let statuses = statuses.clone();
            move |status| statuses.lock().unwrap().push(status)
        });
        link.write_at(1100, socket, b"hello").unwrap();

        assert_eq!(
            link.read_at(1200, socket, Ok(&answer_200())),
            Read::Done(Ok(b"hello".to_vec()))
        );
        assert_eq!(link.profile().disconn_errtype, ErrCmdType::Ok);
        assert_eq!(link.profile().disconn_errcode, 200);
        assert_eq!(link.profile().channel_type, Task::CHANNEL_SHORT);
        assert_eq!(link.profile().start_read_packet_time, 1200);
        assert_eq!(link.profile().read_packet_finished_time, 1200);
        assert_eq!(
            link.profile().send_request_cost,
            100,
            "how long the write took, which the C++ asks between the two"
        );
        assert_eq!(link.profile().recv_reponse_cost, 0);
        assert_eq!(*statuses.lock().unwrap(), vec![200]);
        assert!(
            seen.reports.lock().unwrap().is_empty(),
            "an answer that came back is not reported"
        );
    }

    #[test]
    fn an_answer_that_is_not_two_hundred_is_the_status() {
        let seen = Seen::default();
        let (mut link, socket) = connected(&seen);
        link.write_at(1100, socket, b"hello").unwrap();

        assert_eq!(
            link.read_at(1200, socket, Ok(&answer_500())),
            Read::Done(Err(RunFail::Http { err_code: 500 }))
        );
        assert_eq!(link.profile().disconn_errtype, ErrCmdType::Http);
        assert_eq!(link.profile().disconn_errcode, 500);
        assert_eq!(
            seen.reports.lock().unwrap().as_slice(),
            &[Reported {
                err_type: ErrCmdType::Http,
                err_code: 500,
                ip: "183.3.226.35".to_string(),
                host: "short.weixin.qq.com".to_string(),
                port: 80,
            }]
        );
    }

    #[test]
    fn an_answer_that_came_in_two_reads_is_whole_on_the_second() {
        let seen = Seen::default();
        let (mut link, socket) = connected(&seen);
        let recvs = Arc::new(Mutex::new(Vec::new()));
        link.set_on_recv({
            let recvs = recvs.clone();
            move |cached, total| recvs.lock().unwrap().push((cached, total))
        });
        link.write_at(1100, socket, b"hello").unwrap();

        let answer = answer_200();
        let half = answer.len() - 5;
        assert_eq!(link.read_at(1200, socket, Ok(&answer[..half])), Read::Again);
        assert_eq!(
            link.read_at(1300, socket, Ok(&answer[half..])),
            Read::Done(Ok(b"hello".to_vec()))
        );
        assert_eq!(
            *recvs.lock().unwrap(),
            vec![(half, half), (5, answer.len())],
            "what came, and how much of the answer that is"
        );
        // the C++ reads the clock for the whole read once, at its start
        assert_eq!(link.profile().start_read_packet_time, 1200);
        assert_eq!(link.profile().read_packet_finished_time, 1300);
        assert_eq!(link.profile().recv_reponse_cost, 100);
    }

    #[test]
    fn a_socket_that_hung_up_is_a_shutdown_and_reported() {
        let seen = Seen::default();
        let (mut link, socket) = connected(&seen);
        link.write_at(1100, socket, b"hello").unwrap();

        assert_eq!(
            link.read_at(1200, socket, Ok(&[])),
            Read::Done(Err(RunFail::Socket {
                err_code: ECT_SOCKET_SHUTDOWN
            }))
        );
        assert_eq!(link.profile().disconn_errtype, ErrCmdType::Socket);
        assert_eq!(
            seen.reports.lock().unwrap().as_slice(),
            &[Reported {
                err_type: ErrCmdType::Socket,
                err_code: ECT_SOCKET_SHUTDOWN,
                ip: "183.3.226.35".to_string(),
                host: "short.weixin.qq.com".to_string(),
                port: 80,
            }]
        );
    }

    #[test]
    fn a_socket_the_pool_handed_out_that_hung_up_is_not_reported() {
        let seen = Seen::default();
        let (mut link, socket) = reused(&seen);
        assert!(link.profile().is_reused_fd);
        link.write_at(1100, socket, b"hello").unwrap();

        // the C++ has already had its turn with this one, so it says nothing —
        // but it does close it, and it does not blame the pool
        assert_eq!(
            link.read_at(1200, socket, Ok(&[])),
            Read::Done(Err(RunFail::Socket {
                err_code: ECT_SOCKET_SHUTDOWN
            }))
        );
        assert_eq!(seen.closed(), vec![socket]);
        assert!(seen.reports.lock().unwrap().is_empty());
    }

    #[test]
    fn a_read_that_timed_out_is_read_again() {
        let seen = Seen::default();
        let (mut link, socket) = connected(&seen);
        link.write_at(1100, socket, b"hello").unwrap();

        assert_eq!(link.read_at(1200, socket, Err(ETIMEDOUT)), Read::Again);
        assert!(seen.reports.lock().unwrap().is_empty());
        assert_eq!(link.profile().disconn_errtype, ErrCmdType::Ok);
    }

    #[test]
    fn a_read_that_timed_out_on_quic_is_the_end_of_the_run() {
        let seen = Seen::default();
        let mut link = quic_link_for(&seen, task());
        let socket = link.connect_at(1000).unwrap();
        link.write_at(1100, socket, b"hello").unwrap();

        assert_eq!(
            link.read_at(1200, socket, Err(ETIMEDOUT)),
            Read::Done(Err(RunFail::Socket {
                err_code: ECT_SOCKET_RECV_ERR
            }))
        );
    }

    #[test]
    fn a_quic_link_that_read_notconn_falls_back_to_tcp() {
        let seen = Seen::default();
        let mut link = quic_link_for(&seen, kept_task());
        let socket = link.connect_at(1000).unwrap();
        link.write_at(1100, socket, b"hello").unwrap();

        assert_eq!(
            link.read_at(1200, socket, Err(ENOTCONN)),
            Read::Done(Err(RunFail::Socket { err_code: ENOTCONN }))
        );
        assert_eq!(link.profile().is_fast_fallback_tcp, 1);
        assert!(!link.is_keep_alive(), "and it is not kept");
    }

    #[test]
    fn a_read_that_failed_is_reported_with_the_platform_s_word_for_it() {
        let seen = Seen::default();
        let (mut link, socket) = connected(&seen);
        link.write_at(1100, socket, b"hello").unwrap();

        assert_eq!(
            link.read_at(1200, socket, Err(104)),
            Read::Done(Err(RunFail::Socket { err_code: 104 }))
        );
        assert_eq!(link.profile().rw_errcode, 104);
        assert_eq!(link.profile().disconn_errcode, 104);
        assert_eq!(seen.reports.lock().unwrap()[0].err_code, 104);
    }

    #[test]
    fn a_read_that_left_no_error_at_all_is_read_once() {
        let seen = Seen::default();
        let (mut link, socket) = connected(&seen);
        link.write_at(1100, socket, b"hello").unwrap();

        assert_eq!(
            link.read_at(1200, socket, Err(0)),
            Read::Done(Err(RunFail::Socket {
                err_code: ECT_SOCKET_READ_ONCE
            }))
        );
    }

    #[test]
    fn a_write_that_did_not_happen_is_not_kept() {
        let seen = Seen::default();
        let mut link = link_for(&seen, kept_task(), false);
        let socket = link.connect_at(1000).unwrap();
        assert!(link.is_keep_alive());

        *seen.fail_send.lock().unwrap() = Some(0);
        assert_eq!(
            link.write_at(1100, socket, b"hello"),
            Err(RunFail::Socket {
                err_code: ECT_SOCKET_WRITEN_WITH_NON_BLOCK
            })
        );
        assert_eq!(link.profile().disconn_errtype, ErrCmdType::Socket);
        assert_eq!(
            link.profile().disconn_errcode,
            ECT_SOCKET_WRITEN_WITH_NON_BLOCK
        );
        assert!(
            !link.is_keep_alive(),
            "a socket the request never went out on is not one to keep"
        );
    }

    #[test]
    fn a_write_that_brought_only_part_of_the_request_out_is_one_that_failed() {
        let seen = Seen::default();
        let mut link = link_for(&seen, kept_task(), false);
        let socket = link.connect_at(1000).unwrap();
        *seen.short_send.lock().unwrap() = true;

        assert_eq!(
            link.write_at(1100, socket, b"hello"),
            Err(RunFail::Socket {
                err_code: ECT_SOCKET_WRITEN_WITH_NON_BLOCK
            })
        );
        assert!(
            !link.is_keep_alive(),
            "the answer to a request that was never finished is not one to wait for"
        );
    }

    #[test]
    fn a_run_whose_write_did_not_happen_closes_the_socket_it_made() {
        let seen = Seen::default();
        let mut link = link_for(&seen, kept_task(), false);
        *seen.fail_send.lock().unwrap() = Some(0);

        assert_eq!(
            link.run_at(1000, b"hello", std::iter::empty()),
            Some(Err(RunFail::Socket {
                err_code: ECT_SOCKET_WRITEN_WITH_NON_BLOCK
            }))
        );
        assert_eq!(
            seen.closed(),
            vec![SocketFd(3)],
            "the connect did happen, so the socket is the run's to close"
        );
    }

    #[test]
    fn a_run_the_app_broke_off_is_cancelled_and_not_reported() {
        let seen = Seen::default();
        let mut link = link_for(&seen, kept_task(), false);
        let mut host = Host::new(seen.clone());
        host.breaker.broken = true;
        link.set_socket_operator(host);

        // the C++ asks once more, just after the write
        assert_eq!(
            link.run_at(1000, b"hello", std::iter::empty()),
            Some(Err(RunFail::Canceld))
        );
        assert_eq!(link.profile().disconn_errtype, ErrCmdType::Canceld);
        assert!(seen.reports.lock().unwrap().is_empty());
        assert_eq!(seen.sent().len(), 1, "the request did go out");
        assert_eq!(
            seen.closed(),
            vec![SocketFd(3)],
            "no answer was read, so the socket is not one to keep for the next task"
        );
    }

    #[test]
    fn a_kept_socket_goes_back_to_the_pool() {
        let seen = Seen::default();
        let mut link = link_for(&seen, kept_task(), false);
        let cached = Arc::new(Mutex::new(Vec::new()));
        link.set_pool_cache({
            let cached = cached.clone();
            move |item, profile| {
                cached
                    .lock()
                    .unwrap()
                    .push((item.clone(), profile.keepalive_timeout))
            }
        });
        let socket = link.connect_at(1000).unwrap();
        link.write_at(1100, socket, b"hello").unwrap();

        let answer = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: Keep-Alive\r\n\
                       Keep-Alive: timeout=15\r\n\r\nhello";
        assert_eq!(
            link.read_at(1200, socket, Ok(answer)),
            Read::Done(Ok(b"hello".to_vec()))
        );
        assert_eq!(link.profile().keepalive_timeout, 15);
        assert_eq!(link.profile().socket_fd, socket);
        assert_eq!(
            cached.lock().unwrap().as_slice(),
            &[(item("183.3.226.35", 80), 15)],
            "the pair that won, good for how long the server said"
        );
        assert!(
            seen.closed().is_empty(),
            "a socket that is kept is not closed"
        );
    }

    #[test]
    fn a_kept_socket_that_failed_is_closed_and_the_pool_told() {
        let seen = Seen::default();
        let (mut link, socket) = reused(&seen);
        let reported = Arc::new(Mutex::new(Vec::new()));
        link.set_pool_report({
            let reported = reported.clone();
            move |is_reused, has_received, is_decode_ok| {
                reported
                    .lock()
                    .unwrap()
                    .push((is_reused, has_received, is_decode_ok))
            }
        });
        link.write_at(1100, socket, b"hello").unwrap();

        // the server said keep it, so it is on the profile — and the status was
        // not 200, so it is closed and the pool is told
        let answer = b"HTTP/1.1 500 Server Error\r\nContent-Length: 0\r\n\
                       Connection: Keep-Alive\r\n\r\n";
        assert_eq!(
            link.read_at(1200, socket, Ok(answer)),
            Read::Done(Err(RunFail::Http { err_code: 500 }))
        );
        assert_eq!(seen.closed(), vec![socket]);
        assert_eq!(
            reported.lock().unwrap().as_slice(),
            &[(true, false, false)],
            "reused, and no answer was decoded out of it"
        );
    }

    /// A socket gets onto the profile as `socket_fd` only once the pool handed
    /// it out or the server said to keep it. A peer that hung up before its head
    /// came did neither, so the C++ closes nothing at all: the socket is still
    /// open when the run is over, which is the C++'s own leak and not the
    /// port's.
    #[test]
    fn a_socket_the_profile_does_not_name_is_not_one_the_link_closes() {
        let seen = Seen::default();
        let mut link = link_for(&seen, kept_task(), false);
        let reported = Arc::new(Mutex::new(Vec::new()));
        link.set_pool_report({
            let reported = reported.clone();
            move |is_reused, _, _| reported.lock().unwrap().push(is_reused)
        });
        let socket = link.connect_at(1000).unwrap();
        link.write_at(1100, socket, b"hello").unwrap();

        assert_eq!(
            link.read_at(1200, socket, Ok(&[])),
            Read::Done(Err(RunFail::Socket {
                err_code: ECT_SOCKET_SHUTDOWN
            }))
        );
        assert!(
            link.is_keep_alive(),
            "nothing said close, so the run leaves it"
        );
        assert_eq!(seen.closed(), Vec::new());
        assert!(reported.lock().unwrap().is_empty());
    }

    #[test]
    fn a_server_that_said_close_is_not_one_that_is_kept() {
        let seen = Seen::default();
        let mut link = link_for(&seen, kept_task(), false);
        let cached = Arc::new(Mutex::new(Vec::new()));
        link.set_pool_cache({
            let cached = cached.clone();
            move |item, _| cached.lock().unwrap().push(item.clone())
        });

        let answer = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
        let answer = vec![(Ok(answer.to_vec()), 1100u64)];
        assert_eq!(
            link.run_at(1000, b"hello", answer.into_iter()),
            Some(Ok(b"hello".to_vec()))
        );
        assert!(!link.is_keep_alive(), "the server has the last word");
        assert!(cached.lock().unwrap().is_empty());
        assert_eq!(seen.closed().len(), 1);
    }

    #[test]
    fn an_answer_that_is_not_a_status_line_is_an_http_error() {
        let seen = Seen::default();
        let (mut link, socket) = connected(&seen);
        link.write_at(1100, socket, b"hello").unwrap();

        assert_eq!(
            link.read_at(1200, socket, Ok(b"nonsense\r\n")),
            Read::Done(Err(RunFail::Http {
                err_code: ECT_HTTP_PARSE_STATUS_LINE
            }))
        );
        assert_eq!(link.profile().disconn_errtype, ErrCmdType::Http);
    }

    #[test]
    fn a_body_that_is_not_the_one_the_head_said_is_an_http_error() {
        let seen = Seen::default();
        let (mut link, socket) = connected(&seen);
        link.write_at(1100, socket, b"hello").unwrap();

        assert_eq!(
            link.read_at(
                1200,
                socket,
                Ok(b"HTTP/1.1 200 OK\r\nContent-Length: 4294967297\r\n\r\n")
            ),
            Read::Done(Err(RunFail::Http {
                err_code: ECT_HTTP_SPLIT_HTTP_HEAD_AND_BODY
            }))
        );
    }

    #[test]
    fn the_whole_run_is_one_connect_one_write_and_the_reads() {
        let seen = Seen::default();
        let mut link = link(&seen);
        let sends = Arc::new(Mutex::new(Vec::new()));
        link.set_on_send({
            let sends = sends.clone();
            move |socket| sends.lock().unwrap().push(socket)
        });
        link.set_tid(|| 42);
        link.set_net_type_for_report(|| K_WIFI);

        let reads = vec![(Ok(answer_200()), 1200u64)];
        assert_eq!(
            link.run_at(1000, b"hello", reads.into_iter()),
            Some(Ok(b"hello".to_vec()))
        );
        assert_eq!(link.profile().start_time, 1000);
        assert_eq!(link.profile().tid, 42);
        assert_eq!(link.profile().nettype_for_report, K_WIFI);
        assert_eq!(*sends.lock().unwrap(), vec![SocketFd(3)]);
        assert_eq!(seen.sent().len(), 1);
        assert_eq!(seen.closed(), vec![SocketFd(3)], "not kept, so closed");
    }

    #[test]
    fn a_run_whose_reads_ran_out_has_no_answer() {
        let seen = Seen::default();
        let mut link = link(&seen);

        assert_eq!(link.run_at(1000, b"hello", std::iter::empty()), None);
        assert_eq!(seen.sent().len(), 1);
        assert_eq!(
            seen.closed(),
            Vec::new(),
            "a run that is still reading has not ended"
        );
    }

    #[test]
    fn a_run_on_a_link_that_did_not_connect_says_why() {
        let seen = Seen::default();
        let mut link = link(&seen);
        link.set_shortlink_items(|_, _| Vec::new());

        assert_eq!(
            link.run_at(1000, b"hello", std::iter::empty()),
            Some(Err(RunFail::Connect(ConnectFail::NoAddress)))
        );
        assert_eq!(
            RunFail::Connect(ConnectFail::NoAddress).err_cmd_type(),
            ErrCmdType::Dns
        );
        assert!(seen.sent().is_empty(), "nothing went out");
    }

    #[test]
    fn the_app_is_handed_the_answer_when_it_asked_for_it() {
        let seen = Seen::default();
        let mut link = link(&seen);
        let answered = Arc::new(Mutex::new(Vec::new()));
        link.set_on_response({
            let answered = answered.clone();
            move |err_type, err_code, body, profile| {
                answered.lock().unwrap().push((
                    err_type,
                    err_code,
                    body.to_vec(),
                    profile.disconn_errtype,
                ))
            }
        });
        let statuses = Arc::new(Mutex::new(Vec::new()));
        link.set_response_status({
            let statuses = statuses.clone();
            move |status| statuses.lock().unwrap().push(status)
        });

        let reads = vec![(Ok(answer_200()), 1200u64)];
        assert_eq!(
            link.run_at(1000, b"hello", reads.into_iter()),
            Some(Ok(b"hello".to_vec()))
        );
        assert_eq!(
            answered.lock().unwrap().as_slice(),
            &[(ErrCmdType::Ok, 200, b"hello".to_vec(), ErrCmdType::Ok)]
        );
        assert!(
            statuses.lock().unwrap().is_empty(),
            "the app took the answer, so the link's own way of saying it is not used"
        );
    }

    #[test]
    fn the_network_the_read_was_made_on_is_the_label_the_host_gave() {
        let seen = Seen::default();
        let mut link = link(&seen);
        // the C++ asks for the label again for the read, and a mobile one *is*
        // the isp code, which it reads back out of it
        link.set_net_label(|| NetworkLabel {
            kind: K_MOBILE,
            label: "46000".to_string(),
        });
        let socket = link.connect_at(1000).unwrap();
        link.write_at(1100, socket, b"hello").unwrap();
        assert_eq!(
            link.read_at(1200, socket, Ok(&answer_200())),
            Read::Done(Ok(b"hello".to_vec()))
        );

        assert_eq!(link.profile().net_type, "46000");
        assert_eq!(link.profile().ispcode, 46000);
    }

    #[test]
    fn the_signal_the_network_had_is_what_the_run_ended_with() {
        let seen = Seen::default();
        let mut link = link(&seen);
        link.set_net_label(|| NetworkLabel {
            kind: K_MOBILE,
            label: "46000".to_string(),
        });
        link.set_signal(|is_wifi| if is_wifi { -55 } else { -70 });
        let socket = link.connect_at(1000).unwrap();

        link.end_run(socket);
        assert_eq!(
            link.profile().disconn_signal,
            -70,
            "not a wifi network, so the C++ asks for the mobile signal"
        );
        assert_eq!(seen.closed(), vec![socket]);
    }

    #[test]
    fn a_socket_that_is_kept_is_left_open() {
        let seen = Seen::default();
        let mut link = link_for(&seen, kept_task(), false);
        link.set_signal(|_| -55);
        let socket = link.connect_at(1000).unwrap();

        link.end_run(socket);
        assert_eq!(link.profile().disconn_signal, -55);
        assert!(seen.closed().is_empty());
    }

    #[test]
    fn a_quic_link_reads_for_as_long_as_the_net_source_said() {
        let seen = Seen::default();
        let mut link = quic_link_for(&seen, task());
        link.set_quic_rw_timeout(|cgi| {
            assert_eq!(cgi, "/cgi-bin/micromsg-bin/short");
            (900, TimeoutSource::CgiSpecial)
        });
        let socket = link.connect_at(1000).unwrap();
        link.write_at(1100, socket, b"hello").unwrap();

        assert_eq!(link.read_timeout_ms(), 900);
        assert_eq!(link.profile().quic_rw_timeout_ms, 900);
        assert_eq!(
            link.profile().quic_rw_timeout_source,
            TimeoutSource::CgiSpecial
        );
    }
}
