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
//! What goes out on the socket and what comes back for it are
//! [`crate::shortlink`]; the run that writes and reads them comes with the
//! slice after this one.
//!
//! Two things of the C++ are not ported. `__UpdateProfile`, which copies a
//! profile into another one and keeps the two tls flags doing so: this link
//! *is* the profile it fills in, so there is nothing to copy — the flags are
//! simply never cleared, which [`ConnectProfile::reset`] does not do either.
//! And the `#ifdef _WIN32` block that drops the v6 pairs when the proxy is a
//! v4 one.

use mars_comm::tickcount::gettickcount;
use mars_comm::{LocalIpStack, ProxyInfo, ProxyType, SocketAddress};

use crate::long_link::{ECT_DNS_MAKE_SOCKET_PREPARED, ECT_SOCKET_MAKE_SOCKET_PREPARED};
use crate::shortlink::is_keep_alive;
use crate::socket_operator::{SocketFd, SocketOperator, SocketProfile};
use crate::{ConnectProfile, ErrCmdType, IpPortItem, IpSourceType, Task};

/// `kMobile` of `mars/comm/platform_comm.h` — the class `getCurrNetLabel`
/// answers for a mobile network, and the only one whose label is a number.
pub const K_MOBILE: i32 = 2;

/// `SOCKET_ERRNO(ETIMEDOUT)` on linux — what a pair that was still connecting
/// when another one won is reported with. What a platform calls it is the
/// host's to say, so the port takes the one mars was written for.
pub const ETIMEDOUT: i32 = 110;

/// `v4connect_timeout_ms_`, `v6connect_timeout_ms_` — what a link that was
/// given pairs of its own races them with.
pub const DEFAULT_CONNECT_TIMEOUT_MS: u32 = 1000;

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

    /// `__RunResponseError` — the profile says how the link went away, and the
    /// network report hears about it when it is asked to.
    ///
    /// What else the C++'s `__OnResponse` does — answering the task, and handing
    /// the socket back to the pool — comes with the run that has an answer to
    /// hand over.
    fn run_response_error(&mut self, err_type: ErrCmdType, err_code: i32, report: bool) {
        self.profile.disconn_errtype = err_type;
        self.profile.disconn_errcode = err_code;
        if report && err_type != ErrCmdType::Ok {
            let ip = self.profile.ip.clone();
            let host = self.profile.host.clone();
            let port = self.profile.port;
            self.report(err_type, err_code, &ip, &host, port);
        }
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
            _timeout_ms: i32,
        ) -> Result<usize, i32> {
            Ok(buffer.len())
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

        fn close(&mut self, _socket: SocketFd) {}

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
        let mut link = ShortLink::new(task(), use_proxy);
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
    fn the_first_address_is_the_one_a_v6_connect_is_judged_by() {
        let v4 = SocketAddress::new("1.1.1.1", 80);
        let v6 = SocketAddress::new("::1", 80);
        assert!(first_is_v6(&[v6.clone(), v4.clone()]));
        assert!(!first_is_v6(&[v4, v6]));
        assert!(!first_is_v6(&[]));
    }
}
