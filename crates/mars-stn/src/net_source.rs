//! `mars/stn/src/net_source.cc` — where an ip/port pair comes from.
//!
//! `NetSource` is the table behind every connection in STN: the long-link
//! hosts and ports the app set, the debug ip that overrides dns for one host
//! or one cgi, the backup ips a host falls back to, and the history that says
//! which pairs failed ([`SimpleIpPortSort`]). What it answers a caller is a
//! [`Vec`] of [`IpPortItem`] — at most [`NUM_MAKE_COUNT`] ip/port pairs, in the
//! order they should be tried in.
//!
//! Two things decide the whole list: a host with a debug ip never reaches dns
//! at all, and a pair that came from dns is sorted and filtered by its history
//! while a pair that came from the backup list is only shuffled. The count
//! each host is allowed to add is [`NUM_MAKE_COUNT`] while the app is in the
//! foreground, and one fewer spread over the hosts while it is not.
//!
//! The dns, the network, and whether the app is in the foreground are three
//! callbacks here ([`NewDns`], [`Dns`], [`NetInfo`], [`IsActive`]) — the port
//! has no socket and no platform — and the `rand()` that shuffles the backup
//! pairs is a fourth ([`crate::Random`]). What the C++ does with files
//! (`ipportrecords2.xml`) is [`SimpleIpPortSort::load_records`] and
//! [`SimpleIpPortSort::save_records`], which the host calls.
//!
//! What the C++ keeps as function objects and manager hooks is left out: the
//! dns profile report (`ReportDnsProfileFunc`), the quic/ipv6 policy of the
//! app, and `OnNewDns` beyond the callback. Two of the C++'s own stubs are
//! kept as stubs: [`NetSource::longlink_speed_test_ips`] answers an empty list
//! where the C++ answers `true` and fills nothing in, and
//! [`NetSource::report_longlink_speed_test_result`] does nothing.

use std::collections::{BTreeMap, BTreeSet};

use crate::simple_ipport_sort::{IpPortItem, IpSourceType, SimpleIpPortSort};
use crate::task::Task;
use crate::Random;
use mars_comm::tickcount::gettickcount;

/// `kItemDelimiter` — what [`NetSource::dump_table`] puts between the fields of
/// one item, with a `|` between the items themselves.
pub const ITEM_DELIMITER: &str = ":";
/// `kNumMakeCount` — how many pairs a host list is made up of at most.
pub const NUM_MAKE_COUNT: usize = 5;
/// `kNoNet` — what `getNetInfo()` answers with no network, and what an unset
/// [`NetInfo`] answers too.
pub const NO_NET: i32 = -1;
/// `DEFAULT_LONGLINK_GROUP`.
pub const DEFAULT_LONGLINK_GROUP: &str = "default-group";
/// `sg_quic_default_rw_timeoutms`.
pub const DEFAULT_QUIC_RW_TIMEOUT_MS: u32 = 5_000;
/// `quic_default_conn_timeoutms_`.
pub const DEFAULT_QUIC_CONNECT_TIMEOUT_MS: u32 = 250;
/// What `DisableQUIC` is called with when the caller says nothing.
pub const DISABLE_QUIC_SECONDS: i64 = 20 * 60;
/// The port `SetCgiDebugIP` writes down for a cgi whose port is zero.
pub const CGI_DEBUG_DEFAULT_PORT: u16 = 80;

/// The C++'s `front()` of a host list it never checked: an empty one has no
/// host to give.
fn first_host(host_list: &[String]) -> &str {
    host_list.first().map_or("", String::as_str)
}

/// The `item` the C++ fills in inside `__Get*DebugIPPort`: one pair, from a
/// debug ip.
fn debug_item(ip: &str, port: u16, host: &str) -> IpPortItem {
    IpPortItem {
        ip: ip.to_string(),
        port,
        source_type: IpSourceType::Debug,
        host: host.to_string(),
        transport_protocol: Task::TRANSPORT_PROTOCOL_TCP,
        from_source: 0,
    }
}

/// The same, one item per port.
fn debug_items(ip: &str, host: &str, ports: &[u16]) -> Vec<IpPortItem> {
    ports
        .iter()
        .map(|&port| debug_item(ip, port, host))
        .collect()
}

/// `IPSourceTypeString[]` — how an [`IpSourceType`] is written into
/// [`NetSource::dump_table`].
fn source_name(source: IpSourceType) -> &'static str {
    match source {
        IpSourceType::Null => "NullIP",
        IpSourceType::Debug => "DebugIP",
        IpSourceType::Dns => "DNSIP",
        IpSourceType::NewDns => "NewDNSIP",
        IpSourceType::Proxy => "ProxyIP",
        IpSourceType::Backup => "BackupIP",
    }
}

/// `std::map<std::string, std::string>` — the `_extra_info` the C++ hands to
/// `OnNewDns` and gets back with the ips.
pub type ExtraInfo = BTreeMap<String, String>;

/// `DnsUtil::GetNewDNS()` — the dns the app answers (`OnNewDns`), which is
/// asked first. The `bool` is `_longlink_host`. Unset answers nothing.
pub type NewDns = dyn FnMut(&str, bool, &ExtraInfo) -> Vec<String> + Send;
/// `DnsUtil::GetDNS()` — the fallback, asked when the new dns answered
/// nothing. Unset answers nothing.
pub type Dns = dyn FnMut(&str) -> Vec<String> + Send;
/// `ActiveLogic::IsActive()` — whether the app is in the foreground; unset
/// answers `false`, the way `ActiveLogic` starts before the app says anything.
pub type IsActive = dyn FnMut() -> bool + Send;
/// `getNetInfo()` — one of the `kNoNet` / `kWifi` / `kMobile` / `kOtherNet`
/// values; unset answers [`NO_NET`], which is what makes a report do nothing.
pub type NetInfo = dyn FnMut() -> i32 + Send;

/// `__MakeIPPorts(...)` — what one host is asked for, which is the same three
/// things every call hands over: how many pairs the list may hold once the host
/// is done, whether this is the backup pass, and whether the list is for a long
/// link.
#[derive(Debug, Clone, Copy)]
struct Make {
    /// `_count`.
    count: usize,
    /// `_is_backup`.
    is_backup: bool,
    /// `_is_longlink`.
    is_longlink: bool,
}

/// `TimeoutSource` — where a quic timeout came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeoutSource {
    /// `kClientDefault` — nobody set it.
    #[default]
    ClientDefault = 0,
    /// `kServerDefault` — `SetDefaultQUIC*TimeoutMs` set it.
    ServerDefault = 1,
    /// `kCgiSpecial` — one cgi has one of its own.
    CgiSpecial = 2,
}

/// `LonglinkConfig` of `mars/stn/stn.h` — what a long link is built from.
///
/// `longlink_encoder` and `dns_func` are not here: the encoder is
/// [`crate::LongLinkEncoder`], which the caller owns, and `dns_func` is a
/// per-channel dns the port leaves to [`NetSource::set_new_dns`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LonglinkConfig {
    /// `name` — the channel id.
    pub name: String,
    /// `host_list` — empty means "the hosts the app set".
    pub host_list: Vec<String>,
    /// `is_keep_alive` — `false` leaves the reconnect to a task.
    pub is_keep_alive: bool,
    /// `group`.
    pub group: String,
    /// `isMain`.
    pub is_main: bool,
    /// `link_type` — one of the `Task::CHANNEL_*`, which is what decides which
    /// debug ip a long link is given.
    pub link_type: i32,
    /// `need_tls`.
    pub need_tls: bool,
}

impl LonglinkConfig {
    /// `LonglinkConfig(_name, _group, _isMain)` — the defaults of the C++
    /// constructor.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            host_list: Vec::new(),
            is_keep_alive: false,
            group: DEFAULT_LONGLINK_GROUP.to_string(),
            is_main: false,
            link_type: Task::CHANNEL_LONG,
            need_tls: true,
        }
    }

    /// `IsMain()`.
    pub fn is_main(&self) -> bool {
        self.is_main
    }
}

/// `NetSource`.
pub struct NetSource {
    /// `ipportstrategy_`.
    ipport_strategy: SimpleIpPortSort,
    /// `v4_timeout_`, in milliseconds.
    v4_timeout: u32,
    /// `v6_timeout_`, in milliseconds.
    v6_timeout: u32,

    /// `sg_longlink_hosts`.
    longlink_hosts: Vec<String>,
    /// `sg_longlink_ports`.
    longlink_ports: Vec<u16>,
    /// `sg_longlink_debugip`.
    longlink_debugip: String,

    /// `sg_minorlong_debugip`.
    minorlong_debugip: String,
    /// `sg_minorlong_port`.
    minorlong_port: u16,

    /// `sg_shortlink_port`.
    shortlink_port: u16,
    /// `sg_shortlink_debugip`.
    shortlink_debugip: String,
    /// `sg_host_backupips_mapping`.
    host_backup_ips: BTreeMap<String, Vec<String>>,
    /// `sg_lowpriority_longlink_ports`.
    low_priority_longlink_ports: Vec<u16>,

    /// `sg_host_debugip_mapping`.
    host_debugip: BTreeMap<String, String>,
    /// `sg_cgi_debug_mapping`.
    cgi_debug: BTreeMap<String, (String, u16)>,

    /// `sg_quic_reopen_tick` — when quic is let back in, as a tick count.
    quic_reopen_due: Option<u64>,
    /// `sg_quic_enabled` — off from `DisableQUIC` until the tick above comes.
    quic_enabled: bool,
    /// `quic_forbidden_` — off for good.
    quic_forbidden: bool,
    /// `sg_quic_default_rw_timeoutms`.
    quic_default_rw_timeoutms: u32,
    /// `sg_quic_default_timeout_source`.
    quic_default_timeout_source: TimeoutSource,
    /// `sg_cgi_quic_rw_timeoutms_mapping`.
    cgi_quic_rw_timeoutms: BTreeMap<String, u32>,
    /// `quic_default_conn_timeoutms_`.
    quic_default_conn_timeoutms: u32,
    /// `quic_default_connect_timeout_source_`.
    quic_default_connect_timeout_source: TimeoutSource,
    /// `cgi_quic_connect_timeoutms_mapping_`.
    cgi_quic_connect_timeoutms: BTreeMap<String, u32>,

    /// `sg_ipv6_enabled`.
    ipv6_enabled: bool,

    new_dns: Option<Box<NewDns>>,
    dns: Option<Box<Dns>>,
    is_active: Option<Box<IsActive>>,
    net_info: Option<Box<NetInfo>>,
    random: Box<Random>,
}

impl Default for NetSource {
    /// `NetSource(_active_logic, _context)`, at the tick count the clock gives.
    fn default() -> Self {
        Self::new()
    }
}

impl NetSource {
    /// `NetSource(_active_logic, _context)`.
    pub fn new() -> Self {
        Self::new_at(gettickcount())
    }

    /// The same, with the reading the C++'s `gettickcount()` would hand out:
    /// the seed of the `rand()` that shuffles the backup pairs.
    pub fn new_at(now: u64) -> Self {
        Self {
            ipport_strategy: SimpleIpPortSort::seeded(now),
            v4_timeout: 0,
            v6_timeout: 0,
            longlink_hosts: Vec::new(),
            longlink_ports: Vec::new(),
            longlink_debugip: String::new(),
            minorlong_debugip: String::new(),
            minorlong_port: 0,
            shortlink_port: 0,
            shortlink_debugip: String::new(),
            host_backup_ips: BTreeMap::new(),
            low_priority_longlink_ports: Vec::new(),
            host_debugip: BTreeMap::new(),
            cgi_debug: BTreeMap::new(),
            quic_reopen_due: None,
            quic_enabled: true,
            quic_forbidden: true,
            quic_default_rw_timeoutms: DEFAULT_QUIC_RW_TIMEOUT_MS,
            quic_default_timeout_source: TimeoutSource::ClientDefault,
            cgi_quic_rw_timeoutms: BTreeMap::new(),
            quic_default_conn_timeoutms: DEFAULT_QUIC_CONNECT_TIMEOUT_MS,
            quic_default_connect_timeout_source: TimeoutSource::ClientDefault,
            cgi_quic_connect_timeoutms: BTreeMap::new(),
            ipv6_enabled: true,
            new_dns: None,
            dns: None,
            is_active: None,
            net_info: None,
            random: Box::new(crate::xorshift(now)),
        }
    }

    /// `DnsUtil::GetNewDNS()` — the dns `StnManager::OnNewDns` answers.
    pub fn set_new_dns(
        &mut self,
        new_dns: impl FnMut(&str, bool, &ExtraInfo) -> Vec<String> + Send + 'static,
    ) {
        self.new_dns = Some(Box::new(new_dns));
    }

    /// `DnsUtil::GetDNS()`.
    pub fn set_dns(&mut self, dns: impl FnMut(&str) -> Vec<String> + Send + 'static) {
        self.dns = Some(Box::new(dns));
    }

    /// `ActiveLogic::IsActive()`.
    pub fn set_is_active(&mut self, is_active: impl FnMut() -> bool + Send + 'static) {
        self.is_active = Some(Box::new(is_active));
    }

    /// `getNetInfo()`.
    pub fn set_net_info(&mut self, net_info: impl FnMut() -> i32 + Send + 'static) {
        self.net_info = Some(Box::new(net_info));
    }

    /// `getCurrNetLabel` — which network the history is kept for, which is what
    /// makes a pair that failed on one network usable on another.
    pub fn set_net_label(&mut self, net_label: impl FnMut() -> Option<String> + Send + 'static) {
        self.ipport_strategy.set_net_label(net_label);
    }

    /// `rand()` — which backup pair comes first.
    ///
    /// The sort behind the table has one of its own
    /// ([`SimpleIpPortSort::set_random`]): the C++ gets both from one `rand()`
    /// for the whole process, and the order the two of them draw from it is not
    /// something a caller can see.
    pub fn set_random(&mut self, random: impl FnMut(usize) -> usize + Send + 'static) {
        self.random = Box::new(random);
    }

    /// `SetLongLink(_hosts, _ports, _debugip)` — an empty host list is an error
    /// in the C++ and is ignored here: what the app set last stays.
    pub fn set_longlink(&mut self, hosts: Vec<String>, ports: Vec<u16>, debugip: &str) {
        self.longlink_debugip = debugip.to_string();
        if !hosts.is_empty() {
            self.longlink_hosts = hosts;
        }
        self.longlink_ports = ports;
    }

    /// `SetShortlink(_port, _debugip)`.
    pub fn set_shortlink(&mut self, port: u16, debugip: &str) {
        self.shortlink_port = port;
        self.shortlink_debugip = debugip.to_string();
    }

    /// `SetMinorLongDebugIP(_ip, _port)`.
    pub fn set_minorlong_debug_ip(&mut self, ip: &str, port: u16) {
        self.minorlong_debugip = ip.to_string();
        self.minorlong_port = port;
    }

    /// `SetBackupIPs(_host, _ips)`.
    pub fn set_backup_ips(&mut self, host: &str, ips: Vec<String>) {
        self.host_backup_ips.insert(host.to_string(), ips);
    }

    /// `SetDebugIP(_host, _ip)` — an empty ip takes the host's debug ip away.
    pub fn set_debug_ip(&mut self, host: &str, ip: &str) {
        if ip.is_empty() {
            self.host_debugip.remove(host);
        } else {
            self.host_debugip.insert(host.to_string(), ip.to_string());
        }
    }

    /// `SetLowPriorityLonglinkPorts`.
    pub fn set_low_priority_longlink_ports(&mut self, ports: Vec<u16>) {
        self.low_priority_longlink_ports = ports;
    }

    /// `GetLongLinkDebugIP()`.
    pub fn longlink_debug_ip(&self) -> &str {
        &self.longlink_debugip
    }

    /// `GetShortLinkDebugIP()`.
    pub fn shortlink_debug_ip(&self) -> &str {
        &self.shortlink_debugip
    }

    /// `GetMinorLongLinkDebugIP()`.
    pub fn minorlong_debug_ip(&self) -> &str {
        &self.minorlong_debugip
    }

    /// `GetLongLinkHosts()`.
    pub fn longlink_hosts(&self) -> &[String] {
        &self.longlink_hosts
    }

    /// `GetLonglinkPorts(_ports)`.
    pub fn longlink_ports(&self) -> Vec<u16> {
        self.longlink_ports.clone()
    }

    /// `GetShortLinkPort()`.
    pub fn shortlink_port(&self) -> u16 {
        self.shortlink_port
    }

    /// `GetBackupIPs(_host, _iplist)`.
    pub fn backup_ips(&self, host: &str) -> Vec<String> {
        self.host_backup_ips.get(host).cloned().unwrap_or_default()
    }

    /// `SetCgiDebugIP(_cgi, _ip, _port)` — an empty cgi is ignored, an empty ip
    /// takes the cgi's debug pair away, and a port of zero is `80`.
    pub fn set_cgi_debug_ip(&mut self, cgi: &str, ip: &str, port: u16) {
        if cgi.is_empty() {
            return;
        }
        if ip.is_empty() {
            self.cgi_debug.remove(cgi);
            return;
        }
        let port = if port == 0 {
            CGI_DEBUG_DEFAULT_PORT
        } else {
            port
        };
        self.cgi_debug
            .insert(cgi.to_string(), (ip.to_string(), port));
    }

    /// `GetLongLinkItems(_config, _dns_util, _ipport_items, _extra_info)` —
    /// the pairs a long link is tried on, in order. Empty is the C++'s `false`:
    /// either no host was ever set or nothing resolved.
    pub fn get_longlink_items(&mut self, config: &LonglinkConfig) -> Vec<IpPortItem> {
        let now = gettickcount();
        let extra = ExtraInfo::new();
        self.get_longlink_items_at(now, config, &extra)
    }

    /// The same, with the reading the C++'s `gettickcount()` would hand out
    /// (it is what the ban list is compared against) and the `_extra_info` the
    /// dns is given.
    pub fn get_longlink_items_at(
        &mut self,
        now: u64,
        config: &LonglinkConfig,
        extra: &ExtraInfo,
    ) -> Vec<IpPortItem> {
        if let Some(items) = self.longlink_debug_ip_port(config) {
            return items;
        }

        let hosts: Vec<String> = if config.host_list.is_empty() {
            self.longlink_hosts.clone()
        } else {
            config.host_list.clone()
        };
        if hosts.is_empty() {
            return Vec::new();
        }

        self.get_ip_port_items_at(now, &hosts, true, extra)
    }

    /// `GetShortLinkItems(_hostlist, _ipport_items, _dns_util, _cgi,
    /// _extra_info)` — the pairs a short link is tried on. Empty is the C++'s
    /// `false`.
    pub fn get_shortlink_items(&mut self, host_list: &[String], cgi: &str) -> Vec<IpPortItem> {
        let now = gettickcount();
        let extra = ExtraInfo::new();
        self.get_shortlink_items_at(now, host_list, cgi, &extra)
    }

    /// The same, with the reading and the `_extra_info` handed in.
    pub fn get_shortlink_items_at(
        &mut self,
        now: u64,
        host_list: &[String],
        cgi: &str,
        extra: &ExtraInfo,
    ) -> Vec<IpPortItem> {
        if let Some(items) = self.shortlink_debug_ip_port(host_list, cgi) {
            return items;
        }
        if host_list.is_empty() {
            return Vec::new();
        }
        self.get_ip_port_items_at(now, host_list, false, extra)
    }

    /// `ReportLongIP(_is_success, _ip, _port)` — the tick count comes from the
    /// clock and the unix second from the host.
    pub fn report_long_ip(&mut self, now_secs: u64, is_success: bool, ip: &str, port: u16) {
        self.report_long_ip_at(gettickcount(), now_secs, is_success, ip, port);
    }

    /// The same, with both readings handed in. An empty ip, a port of zero, and
    /// no network are the three things that make the C++ drop the report.
    pub fn report_long_ip_at(
        &mut self,
        now: u64,
        now_secs: u64,
        is_success: bool,
        ip: &str,
        port: u16,
    ) {
        if ip.is_empty() || port == 0 {
            return;
        }
        if self.net_info() == NO_NET {
            return;
        }
        self.ipport_strategy
            .update_at(now, now_secs, ip, port, is_success);
    }

    /// `ReportShortIP(_is_success, _ip, _host, _port)` — the host is only in
    /// the C++'s log line, which is why the port does not use it.
    pub fn report_short_ip(
        &mut self,
        now_secs: u64,
        is_success: bool,
        ip: &str,
        _host: &str,
        port: u16,
    ) {
        self.report_short_ip_at(gettickcount(), now_secs, is_success, ip, _host, port);
    }

    /// The same, with both readings handed in. Only the empty ip and no network
    /// are dropped here — the C++ does not look at the port.
    pub fn report_short_ip_at(
        &mut self,
        now: u64,
        now_secs: u64,
        is_success: bool,
        ip: &str,
        _host: &str,
        port: u16,
    ) {
        if ip.is_empty() {
            return;
        }
        if self.net_info() == NO_NET {
            return;
        }
        self.ipport_strategy
            .update_at(now, now_secs, ip, port, is_success);
    }

    /// `RemoveLongBanIP(_ip)` — every port of the ip leaves the ban list, which
    /// is what a speed test that succeeded does.
    pub fn remove_long_ban_ip(&mut self, ip: &str) {
        self.ipport_strategy.remove_banned_list(ip);
    }

    /// `AddServerBan(_ip)` — the tick count comes from the clock.
    pub fn add_server_ban(&mut self, ip: &str) {
        self.add_server_ban_at(gettickcount(), ip);
    }

    /// The same, with the reading handed in.
    pub fn add_server_ban_at(&mut self, now: u64, ip: &str) {
        self.ipport_strategy.add_server_ban_at(now, ip);
    }

    /// `InitHistory2BannedList(_save)` — the history of the network the app is
    /// on becomes the ban list. The `_save` flag is the C++ writing its file,
    /// which the port does not do.
    pub fn init_history_to_banned_list(&mut self) {
        self.ipport_strategy.init_history_to_banned_list();
    }

    /// `ClearCache()` — the ban list rebuilt, and quic and ipv6 let back in.
    pub fn clear_cache(&mut self) {
        self.ipport_strategy.init_history_to_banned_list();
        self.quic_enabled = true;
        self.ipv6_enabled = true;
    }

    /// `DumpTable(_ipport_items)` — `ip:port:host:source`, `|` between the
    /// items, which is the C++'s `XMessage` line.
    pub fn dump_table(items: &[IpPortItem]) -> String {
        items
            .iter()
            .map(|item| {
                format!(
                    "{}{ITEM_DELIMITER}{}{ITEM_DELIMITER}{}{ITEM_DELIMITER}{}",
                    item.ip,
                    item.port,
                    item.host,
                    source_name(item.source_type)
                )
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    /// `DisableQUIC(seconds)` — the tick count comes from the clock.
    pub fn disable_quic(&mut self, seconds: i64) {
        self.disable_quic_at(gettickcount(), seconds);
    }

    /// The same, with the reading handed in: quic is off for `seconds` and comes
    /// back on its own the first time it is asked about afterwards, which is the
    /// C++'s `sg_quic_reopen_tick.gettickspan() >= 0`.
    pub fn disable_quic_at(&mut self, now: u64, seconds: i64) {
        self.quic_enabled = false;
        let millis = u64::try_from(seconds).unwrap_or(0).saturating_mul(1_000);
        self.quic_reopen_due = Some(now.saturating_add(millis));
    }

    /// `ForbidQUIC(forbid)`.
    pub fn forbid_quic(&mut self, forbid: bool) {
        self.quic_forbidden = forbid;
    }

    /// `CanUseQUIC()`.
    pub fn can_use_quic(&mut self) -> bool {
        self.can_use_quic_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn can_use_quic_at(&mut self, now: u64) -> bool {
        if self.quic_forbidden {
            return false;
        }
        if self.quic_enabled {
            return true;
        }
        if now >= self.quic_reopen_due.unwrap_or(0) {
            self.quic_enabled = true;
        }
        self.quic_enabled
    }

    /// `GetQUICRWTimeoutMs(_cgi, outsource)` — the timeout and where it came
    /// from.
    pub fn quic_rw_timeout_ms(&self, cgi: &str) -> (u32, TimeoutSource) {
        match self.cgi_quic_rw_timeoutms.get(cgi) {
            Some(ms) => (*ms, TimeoutSource::CgiSpecial),
            None => (
                self.quic_default_rw_timeoutms,
                self.quic_default_timeout_source,
            ),
        }
    }

    /// `SetQUICRWTimeoutMs(_cgi, ms)`.
    pub fn set_quic_rw_timeout_ms(&mut self, cgi: &str, ms: u32) {
        self.cgi_quic_rw_timeoutms.insert(cgi.to_string(), ms);
    }

    /// `SetDefaultQUICRWTimeoutMs(ms)` — and the source becomes
    /// [`TimeoutSource::ServerDefault`].
    pub fn set_default_quic_rw_timeout_ms(&mut self, ms: u32) {
        self.quic_default_rw_timeoutms = ms;
        self.quic_default_timeout_source = TimeoutSource::ServerDefault;
    }

    /// `GetQUICConnectTimeoutMs(_cgi, outsource)`.
    pub fn quic_connect_timeout_ms(&self, cgi: &str) -> (u32, TimeoutSource) {
        match self.cgi_quic_connect_timeoutms.get(cgi) {
            Some(ms) => (*ms, TimeoutSource::CgiSpecial),
            None => (
                self.quic_default_conn_timeoutms,
                self.quic_default_connect_timeout_source,
            ),
        }
    }

    /// `SetQUICConnectTimeoutMs(_cgi, ms)`.
    pub fn set_quic_connect_timeout_ms(&mut self, cgi: &str, ms: u32) {
        self.cgi_quic_connect_timeoutms.insert(cgi.to_string(), ms);
    }

    /// `SetDefaultQUICConnectTimeoutMs(ms)`.
    pub fn set_default_quic_connect_timeout_ms(&mut self, ms: u32) {
        self.quic_default_conn_timeoutms = ms;
        self.quic_default_connect_timeout_source = TimeoutSource::ServerDefault;
    }

    /// `DisableIPv6()`.
    pub fn disable_ipv6(&mut self) {
        self.ipv6_enabled = false;
    }

    /// `CanUseIPv6()`.
    pub fn can_use_ipv6(&self) -> bool {
        self.ipv6_enabled
    }

    /// `SetIpConnectTimeout(_v4_timeout, _v6_timeout)`.
    pub fn set_ip_connect_timeout(&mut self, v4_timeout: u32, v6_timeout: u32) {
        self.v4_timeout = v4_timeout;
        self.v6_timeout = v6_timeout;
    }

    /// `GetIpConnectTimeout()`.
    pub fn ip_connect_timeout(&self) -> (u32, u32) {
        (self.v4_timeout, self.v6_timeout)
    }

    /// `GetLongLinkSpeedTestIPs(_ip_vec)` — the C++ answers `true` and fills
    /// nothing in, which is an empty list here.
    pub fn longlink_speed_test_ips(&mut self) -> Vec<IpPortItem> {
        Vec::new()
    }

    /// `ReportLongLinkSpeedTestResult(_ip_vec)` — empty in the C++ too.
    pub fn report_longlink_speed_test_result(&mut self, _items: &[IpPortItem]) {}

    /// `ipportstrategy_` — the history and the ban list behind the sort.
    pub fn ipport_strategy(&self) -> &SimpleIpPortSort {
        &self.ipport_strategy
    }

    /// `__GetLonglinkDebugIPPort(_config, _ipport_items)` — [`None`] is the
    /// C++'s `false`, a link no debug ip was set for, and [`Some`] is its
    /// `true`, which it answers even when nothing was pushed: an app that set a
    /// debug ip but no ports gets a link with no pairs at all, not a dns
    /// lookup.
    ///
    /// The host debug ip of any of the hosts the app set wins, one item per
    /// long-link port. After that it is the link's own debug ip: the long-link
    /// one for `Task::CHANNEL_LONG` and the minor-long one for
    /// `Task::CHANNEL_MINOR_LONG`, with the first host of the list as the host.
    fn longlink_debug_ip_port(&self, config: &LonglinkConfig) -> Option<Vec<IpPortItem>> {
        for host in &self.longlink_hosts {
            if let Some(ip) = self.host_debugip.get(host) {
                return Some(debug_items(ip, host, &self.longlink_ports));
            }
        }

        // the C++ reads `front()` of a list it never checked: an app that set a
        // debug ip but no host gets the empty host here, not a crash
        let (ip, host) = match config.link_type {
            Task::CHANNEL_LONG if !self.longlink_debugip.is_empty() => (
                self.longlink_debugip.clone(),
                self.longlink_hosts.first().cloned().unwrap_or_default(),
            ),
            Task::CHANNEL_MINOR_LONG if !self.minorlong_debugip.is_empty() => (
                self.minorlong_debugip.clone(),
                config.host_list.first().cloned().unwrap_or_default(),
            ),
            _ => return None,
        };
        Some(debug_items(&ip, &host, &self.longlink_ports))
    }

    /// `__GetShortlinkDebugIPPort(_hostlist, _ipport_items, _cgi)`.
    ///
    /// A cgi with a debug pair wins over everything; then the host debug ip of
    /// any host in the list; then the short-link debug ip. [`None`] — the C++'s
    /// `!_ipport_items.empty()` with nothing pushed — is what lets the caller
    /// carry on to dns.
    fn shortlink_debug_ip_port(&self, host_list: &[String], cgi: &str) -> Option<Vec<IpPortItem>> {
        if let Some((ip, port)) = self.cgi_debug.get(cgi) {
            return Some(vec![debug_item(ip, *port, first_host(host_list))]);
        }

        for host in host_list {
            if let Some(ip) = self.host_debugip.get(host) {
                return Some(vec![debug_item(ip, self.shortlink_port, host)]);
            }
        }

        if self.shortlink_debugip.is_empty() {
            return None;
        }
        Some(vec![debug_item(
            &self.shortlink_debugip,
            self.shortlink_port,
            first_host(host_list),
        )])
    }

    /// `__GetIPPortItems(...)` — the pairs a host list is made of.
    ///
    /// In the foreground every host may add up to [`NUM_MAKE_COUNT`] pairs, and
    /// once exactly one host answered and it filled the list the count grows by
    /// one — the C++'s `merge_type_count` ladder, which is how a host list
    /// whose first host answered everything still gets a second kind of pair.
    /// In the background the C++ shares [`NUM_MAKE_COUNT`] `- 1` pairs out over
    /// the hosts and leaves the last one to the backup pass.
    fn get_ip_port_items_at(
        &mut self,
        now: u64,
        host_list: &[String],
        is_longlink: bool,
        extra: &ExtraInfo,
    ) -> Vec<IpPortItem> {
        let mut items = Vec::new();

        if self.is_active() {
            let mut merge_type_count = 0usize;
            let mut make_list_count = NUM_MAKE_COUNT;

            for is_backup in [false, true] {
                for host in host_list {
                    if merge_type_count == 1 && items.len() == NUM_MAKE_COUNT {
                        make_list_count = NUM_MAKE_COUNT + 1;
                    }
                    let make = Make {
                        count: make_list_count,
                        is_backup,
                        is_longlink,
                    };
                    let made = self.make_ip_ports_at(now, &items, host, extra, make);
                    // what the C++ counts is the length of the list after the
                    // call, not how many pairs the host added, so a host that
                    // added nothing still counts once anything is in there
                    if let Some(made) = made {
                        items.extend(made);
                        if !items.is_empty() {
                            merge_type_count += 1;
                        }
                    }
                }
            }
            return items;
        }

        // the C++ divides by `_hostlist.size()`, which its callers have checked
        // is not empty
        let host_count = host_list.len().max(1);
        let each = (NUM_MAKE_COUNT - 1) / host_count;
        let remainder = (NUM_MAKE_COUNT - 1) % host_count;
        let mut count = 0usize;

        for (index, host) in host_list.iter().enumerate() {
            if count >= NUM_MAKE_COUNT - 1 {
                break;
            }
            count += if index < remainder { each + 1 } else { each };
            let made = self.make_ip_ports_at(
                now,
                &items,
                host,
                extra,
                Make {
                    count,
                    is_backup: false,
                    is_longlink,
                },
            );
            items.extend(made.unwrap_or_default());
        }
        for host in host_list {
            if count >= NUM_MAKE_COUNT {
                break;
            }
            let made = self.make_ip_ports_at(
                now,
                &items,
                host,
                extra,
                Make {
                    count: NUM_MAKE_COUNT,
                    is_backup: true,
                    is_longlink,
                },
            );
            items.extend(made.unwrap_or_default());
        }
        items
    }

    /// `__MakeIPPorts(...)` — one host's worth of pairs.
    ///
    /// [`None`] is the C++'s `return 0`, a host no dns knows; [`Some`] is its
    /// `return _ip_items.size()`, which is what the caller counts — so a host
    /// whose dns answered but whose port list is empty hands back [`Some`] of
    /// nothing.
    fn make_ip_ports_at(
        &mut self,
        now: u64,
        so_far: &[IpPortItem],
        host: &str,
        extra: &ExtraInfo,
        make: Make,
    ) -> Option<Vec<IpPortItem>> {
        let Make {
            count,
            is_backup,
            is_longlink,
        } = make;
        let (mut ips, source, ports) = if is_backup {
            let mut ips = self.backup_ips(host);
            if ips.is_empty() {
                if let Some(dns) = self.dns.as_mut() {
                    ips = dns(host);
                }
                // the C++ keeps what the fallback answered as the backup ips of
                // the host, for the next time it is asked
                if !ips.is_empty() {
                    self.host_backup_ips.insert(host.to_string(), ips.clone());
                }
            }
            let ports = if !is_longlink {
                vec![self.shortlink_port]
            } else if self.low_priority_longlink_ports.is_empty() {
                self.longlink_ports.clone()
            } else {
                // a long link on its low-priority ports is what the backup pass
                // is for
                self.low_priority_longlink_ports.clone()
            };
            (ips, IpSourceType::Backup, ports)
        } else {
            let mut ips = match self.new_dns.as_mut() {
                Some(new_dns) => new_dns(host, is_longlink, extra),
                None => Vec::new(),
            };
            let source = if ips.is_empty() {
                if let Some(dns) = self.dns.as_mut() {
                    ips = dns(host);
                }
                IpSourceType::Dns
            } else {
                IpSourceType::NewDns
            };
            let ports = if is_longlink {
                self.longlink_ports.clone()
            } else {
                vec![self.shortlink_port]
            };
            (ips, source, ports)
        };

        if ips.is_empty() {
            return None;
        }

        if is_backup && !ports.is_empty() {
            let known: BTreeSet<&str> = so_far.iter().map(|item| item.ip.as_str()).collect();
            let ports_count = ports.len();
            // `_count - _ip_items.size()`: how many more pairs the caller wants,
            // which the C++ computes in `size_t` and wraps when the list is
            // already longer. The port clamps it, so the pairs the list has
            // already got are the ones dropped
            let mut required = count.saturating_sub(so_far.len());
            if required < ports_count {
                required += ports_count;
            }
            let mut room = ips.len() * ports_count;
            ips.retain(|ip| {
                if room <= required {
                    return true;
                }
                if known.contains(ip.as_str()) {
                    room -= ports_count;
                    return false;
                }
                true
            });
        }

        let mut made: Vec<IpPortItem> = ips
            .iter()
            .flat_map(|ip| {
                ports.iter().map(move |&port| IpPortItem {
                    ip: ip.clone(),
                    port,
                    source_type: source,
                    host: host.to_string(),
                    transport_protocol: Task::TRANSPORT_PROTOCOL_TCP,
                    from_source: 0,
                })
            })
            .collect();

        if is_backup {
            self.shuffle(&mut made);
            // the C++ resizes the whole list, of which these pairs are the tail
            made.truncate(count.saturating_sub(so_far.len()));
        } else {
            // `(int)(_count - len)` in the C++, which is negative when the list
            // is already longer than the count — and a negative `needcount`
            // keeps every pair
            let need = match count.checked_sub(so_far.len()) {
                Some(more) => i32::try_from(more).unwrap_or(i32::MAX),
                None => -1,
            };
            self.ipport_strategy
                .sort_and_filter_at(now, &mut made, need, true);
        }

        Some(made)
    }

    /// `random_shuffle(_ip_items.begin() + len, _ip_items.end())` — only the
    /// pairs one host just added move; what was in the list stays put.
    fn shuffle(&mut self, items: &mut [IpPortItem]) {
        for index in (1..items.len()).rev() {
            let picked = (self.random)(index + 1);
            items.swap(index, picked);
        }
    }

    /// `ActiveLogic::IsActive()`.
    fn is_active(&mut self) -> bool {
        match self.is_active.as_mut() {
            Some(is_active) => is_active(),
            None => false,
        }
    }

    /// `getNetInfo()`.
    fn net_info(&mut self) -> i32 {
        match self.net_info.as_mut() {
            Some(net_info) => net_info(),
            None => NO_NET,
        }
    }
}

impl std::fmt::Debug for NetSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetSource")
            .field("longlink_hosts", &self.longlink_hosts)
            .field("longlink_ports", &self.longlink_ports)
            .field("longlink_debugip", &self.longlink_debugip)
            .field("shortlink_port", &self.shortlink_port)
            .field("shortlink_debugip", &self.shortlink_debugip)
            .field("host_debugip", &self.host_debugip)
            .field("cgi_debug", &self.cgi_debug)
            .field("host_backup_ips", &self.host_backup_ips)
            .field("quic_enabled", &self.quic_enabled)
            .field("quic_forbidden", &self.quic_forbidden)
            .field("ipv6_enabled", &self.ipv6_enabled)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `NetSource` with two long-link hosts, two ports, a short link, and a
    /// dns that answers one ip per host. The app is in the foreground.
    fn a_source() -> NetSource {
        let mut source = NetSource::new_at(0);
        source.set_longlink(
            vec!["long.example".to_string(), "long2.example".to_string()],
            vec![80, 443],
            "",
        );
        source.set_shortlink(8080, "");
        source.set_is_active(|| true);
        source.set_net_info(|| 1);
        source.set_new_dns(|host, _is_longlink, _extra| match host {
            "long.example" => vec!["1.1.1.1".to_string()],
            "long2.example" => vec!["2.2.2.2".to_string()],
            _ => Vec::new(),
        });
        source.set_dns(|host| match host {
            "short.example" => vec!["3.3.3.3".to_string()],
            _ => Vec::new(),
        });
        source.set_random(|_bound| 0);
        source
    }

    fn ips(items: &[IpPortItem]) -> Vec<String> {
        items.iter().map(|item| item.ip.clone()).collect()
    }

    #[test]
    fn a_host_with_a_debug_ip_never_reaches_dns() {
        let mut source = a_source();
        source.set_debug_ip("long.example", "9.9.9.9");
        let items = source.get_longlink_items(&LonglinkConfig::new("main"));
        assert_eq!(
            items,
            vec![
                IpPortItem {
                    ip: "9.9.9.9".to_string(),
                    port: 80,
                    source_type: IpSourceType::Debug,
                    host: "long.example".to_string(),
                    transport_protocol: Task::TRANSPORT_PROTOCOL_TCP,
                    from_source: 0,
                },
                IpPortItem {
                    ip: "9.9.9.9".to_string(),
                    port: 443,
                    source_type: IpSourceType::Debug,
                    host: "long.example".to_string(),
                    transport_protocol: Task::TRANSPORT_PROTOCOL_TCP,
                    from_source: 0,
                },
            ],
            "one item per long-link port, and nothing else was asked"
        );

        // ... and taking it away again is what lets dns answer
        source.set_debug_ip("long.example", "");
        let items = source.get_longlink_items(&LonglinkConfig::new("main"));
        assert!(items.iter().all(|item| item.ip != "9.9.9.9"));
    }

    #[test]
    fn the_link_debug_ip_is_what_a_link_with_no_host_debug_ip_gets() {
        let mut source = a_source();
        source.set_longlink(vec!["long.example".to_string()], vec![80], "7.7.7.7");
        let items = source.get_longlink_items(&LonglinkConfig::new("main"));
        assert_eq!(ips(&items), vec!["7.7.7.7".to_string()]);
        assert_eq!(items[0].source_type, IpSourceType::Debug);

        // a minor long link has one of its own
        let mut source = a_source();
        source.set_minorlong_debug_ip("8.8.8.8", 0);
        let mut config = LonglinkConfig::new("minor");
        config.link_type = Task::CHANNEL_MINOR_LONG;
        config.host_list = vec!["minor.example".to_string()];
        let items = source.get_longlink_items(&config);
        assert_eq!(
            ips(&items),
            vec!["8.8.8.8".to_string(), "8.8.8.8".to_string()]
        );
        assert_eq!(items[0].host, "minor.example");
    }

    #[test]
    fn a_long_link_with_no_host_at_all_answers_nothing() {
        let mut source = NetSource::new_at(0);
        source.set_new_dns(|_, _, _| vec!["1.1.1.1".to_string()]);
        let items = source.get_longlink_items(&LonglinkConfig::new("main"));
        assert!(items.is_empty(), "no host, and nothing resolved");
    }

    #[test]
    fn the_hosts_of_the_config_win_over_the_ones_the_app_set() {
        let mut source = a_source();
        let mut config = LonglinkConfig::new("main");
        config.host_list = vec!["long2.example".to_string()];
        let items = source.get_longlink_items(&config);
        assert_eq!(
            ips(&items),
            vec!["2.2.2.2".to_string(), "2.2.2.2".to_string()]
        );
    }

    #[test]
    fn the_new_dns_is_asked_first_and_the_fallback_after_it() {
        let mut source = a_source();
        let items = source.get_longlink_items(&LonglinkConfig::new("main"));
        assert!(
            items
                .iter()
                .all(|item| item.source_type == IpSourceType::NewDns),
            "{:?}",
            items
        );

        // the new dns answering nothing is what reaches the fallback
        let mut source = a_source();
        source.set_new_dns(|_, _, _| Vec::new());
        source.set_dns(|host| match host {
            "long.example" => vec!["4.4.4.4".to_string()],
            _ => Vec::new(),
        });
        let items = source.get_longlink_items(&LonglinkConfig::new("main"));
        assert!(items.iter().all(|item| item.ip == "4.4.4.4"), "{:?}", items);
        assert_eq!(items.len(), 4, "two ports, twice: once from each pass");
        assert!(
            items
                .iter()
                .any(|item| item.source_type == IpSourceType::Dns),
            "the fallback filled the first pass in"
        );
        assert!(
            items
                .iter()
                .any(|item| item.source_type == IpSourceType::Backup),
            "and what it answered is the backup list of the host from then on"
        );
    }

    #[test]
    fn a_backup_pass_adds_the_pairs_the_dns_did_not() {
        let mut source = a_source();
        source.set_backup_ips("long.example", vec!["5.5.5.5".to_string()]);
        let items = source.get_longlink_items(&LonglinkConfig::new("main"));
        assert!(
            items
                .iter()
                .any(|item| item.ip == "5.5.5.5" && item.source_type == IpSourceType::Backup),
            "{:?}",
            items
        );
    }

    #[test]
    fn what_the_fallback_answered_becomes_the_backup_ips_of_the_host() {
        let mut source = a_source();
        source.set_new_dns(|_, _, _| Vec::new());
        assert!(source.backup_ips("short.example").is_empty());
        let items = source.get_shortlink_items(&["short.example".to_string()], "");
        assert!(!items.is_empty());
        assert_eq!(
            source.backup_ips("short.example"),
            vec!["3.3.3.3".to_string()]
        );
    }

    #[test]
    fn in_the_background_the_pairs_are_shared_out_over_the_hosts() {
        let mut source = a_source();
        source.set_is_active(|| false);
        // two hosts: `4 / 2` for each of them, and the fifth is left for the
        // backup pass, which a dns that knows no backup ips cannot fill
        let items = source.get_longlink_items(&LonglinkConfig::new("main"));
        assert_eq!(
            ips(&items),
            vec![
                "1.1.1.1".to_string(),
                "1.1.1.1".to_string(),
                "2.2.2.2".to_string(),
                "2.2.2.2".to_string()
            ]
        );
    }

    #[test]
    fn a_shortlink_debug_ip_with_no_host_still_answers_one_pair() {
        let mut source = a_source();
        source.set_shortlink(8080, "6.6.6.6");
        // an empty host list: the host of the one item is what `front()` would
        // have answered, which is nothing
        let items = source.get_shortlink_items(&[], "");
        assert_eq!(ips(&items), vec!["6.6.6.6".to_string()]);
        assert_eq!(items[0].host, "");

        // ... and one host gives the debug ip instead of the dns
        let items = source.get_shortlink_items(&["short.example".to_string()], "");
        assert_eq!(
            items,
            vec![IpPortItem {
                ip: "6.6.6.6".to_string(),
                port: 8080,
                source_type: IpSourceType::Debug,
                host: "short.example".to_string(),
                transport_protocol: Task::TRANSPORT_PROTOCOL_TCP,
                from_source: 0,
            }]
        );
    }

    #[test]
    fn a_cgi_with_a_debug_pair_beats_everything() {
        let mut source = a_source();
        source.set_debug_ip("short.example", "6.6.6.6");
        source.set_cgi_debug_ip("/cgi-bin/mm", "4.4.4.4", 0);
        let items = source.get_shortlink_items(&["short.example".to_string()], "/cgi-bin/mm");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].ip, "4.4.4.4");
        assert_eq!(
            items[0].port, CGI_DEBUG_DEFAULT_PORT,
            "a port of zero is 80"
        );

        // an empty ip takes it away, and the host debug ip is what answers then
        source.set_cgi_debug_ip("/cgi-bin/mm", "", 0);
        let items = source.get_shortlink_items(&["short.example".to_string()], "/cgi-bin/mm");
        assert_eq!(items[0].ip, "6.6.6.6");
        assert_eq!(items[0].port, 8080);
    }

    #[test]
    fn a_report_needs_an_ip_and_a_network() {
        let mut source = a_source();
        source.set_net_label(|| Some("wifi-home".to_string()));

        // no network: nothing is written down
        source.set_net_info(|| NO_NET);
        source.report_long_ip_at(0, 1_000, false, "1.1.1.1", 80);
        assert!(source.ipport_strategy().records().is_empty());

        // an empty ip and a port of zero are dropped before that
        source.set_net_info(|| 1);
        source.report_long_ip_at(0, 1_000, false, "", 80);
        source.report_long_ip_at(0, 1_000, false, "1.1.1.1", 0);
        assert!(source.ipport_strategy().records().is_empty());

        // ... and one that is not, is
        source.report_long_ip_at(0, 1_000, false, "1.1.1.1", 80);
        assert_eq!(source.ipport_strategy().records().len(), 1);

        // the short link does not look at the port, but does at the ip
        source.report_short_ip_at(0, 1_000, false, "", "short.example", 0);
        assert_eq!(source.ipport_strategy().records()[0].items.len(), 1);
        source.report_short_ip_at(0, 1_000, false, "9.9.9.9", "short.example", 0);
        assert_eq!(source.ipport_strategy().records()[0].items.len(), 2);
    }

    #[test]
    fn quic_is_forbidden_until_the_app_says_otherwise() {
        let mut source = a_source();
        assert!(!source.can_use_quic_at(0), "`quic_forbidden_` starts true");

        source.forbid_quic(false);
        assert!(source.can_use_quic_at(0));

        // off for a while, and back on the first question after it ran out
        source.disable_quic_at(1_000, 1);
        assert!(!source.can_use_quic_at(1_500));
        assert!(source.can_use_quic_at(2_000), "a second later it is back");

        // and `ClearCache` lets it back in too
        source.disable_quic_at(3_000, 60);
        source.clear_cache();
        assert!(source.can_use_quic_at(3_100));
    }

    #[test]
    fn the_timeouts_answer_where_they_came_from() {
        let mut source = a_source();
        assert_eq!(
            source.quic_rw_timeout_ms("/cgi-bin/mm"),
            (DEFAULT_QUIC_RW_TIMEOUT_MS, TimeoutSource::ClientDefault)
        );
        assert_eq!(
            source.quic_connect_timeout_ms("/cgi-bin/mm"),
            (
                DEFAULT_QUIC_CONNECT_TIMEOUT_MS,
                TimeoutSource::ClientDefault
            )
        );

        source.set_default_quic_rw_timeout_ms(9_000);
        source.set_default_quic_connect_timeout_ms(900);
        assert_eq!(
            source.quic_rw_timeout_ms("/cgi-bin/mm"),
            (9_000, TimeoutSource::ServerDefault)
        );
        assert_eq!(
            source.quic_connect_timeout_ms("/cgi-bin/mm"),
            (900, TimeoutSource::ServerDefault)
        );

        source.set_quic_rw_timeout_ms("/cgi-bin/mm", 1_000);
        source.set_quic_connect_timeout_ms("/cgi-bin/mm", 100);
        assert_eq!(
            source.quic_rw_timeout_ms("/cgi-bin/mm"),
            (1_000, TimeoutSource::CgiSpecial)
        );
        assert_eq!(
            source.quic_connect_timeout_ms("/cgi-bin/mm"),
            (100, TimeoutSource::CgiSpecial)
        );
    }

    #[test]
    fn the_table_is_dumped_the_way_the_c_plus_plus_writes_it() {
        let items = vec![
            IpPortItem {
                ip: "1.1.1.1".to_string(),
                port: 80,
                source_type: IpSourceType::NewDns,
                host: "long.example".to_string(),
                transport_protocol: Task::TRANSPORT_PROTOCOL_TCP,
                from_source: 0,
            },
            IpPortItem {
                ip: "5.5.5.5".to_string(),
                port: 443,
                source_type: IpSourceType::Backup,
                host: "long.example".to_string(),
                transport_protocol: Task::TRANSPORT_PROTOCOL_TCP,
                from_source: 0,
            },
        ];
        assert_eq!(
            NetSource::dump_table(&items),
            "1.1.1.1:80:long.example:NewDNSIP|5.5.5.5:443:long.example:BackupIP"
        );
        assert_eq!(NetSource::dump_table(&[]), "");
    }

    #[test]
    fn the_settings_are_answered_back() {
        let mut source = a_source();
        assert_eq!(source.longlink_hosts(), ["long.example", "long2.example"]);
        assert_eq!(source.longlink_ports(), vec![80, 443]);
        assert_eq!(source.shortlink_port(), 8080);
        assert_eq!(source.longlink_debug_ip(), "");
        source.set_longlink(vec!["other.example".to_string()], vec![5223], "1.2.3.4");
        assert_eq!(source.longlink_debug_ip(), "1.2.3.4");
        assert_eq!(source.longlink_ports(), vec![5223]);
        // an empty host list is an error in the C++ and is ignored
        source.set_longlink(Vec::new(), vec![80], "");
        assert_eq!(source.longlink_hosts(), ["other.example"]);

        source.set_minorlong_debug_ip("8.8.8.8", 443);
        assert_eq!(source.minorlong_debug_ip(), "8.8.8.8");
        source.set_shortlink(443, "6.6.6.6");
        assert_eq!(source.shortlink_debug_ip(), "6.6.6.6");

        source.set_ip_connect_timeout(3_000, 5_000);
        assert_eq!(source.ip_connect_timeout(), (3_000, 5_000));

        assert!(source.can_use_ipv6());
        source.disable_ipv6();
        assert!(!source.can_use_ipv6());

        // the two stubs of the C++
        let items = source.longlink_speed_test_ips();
        assert!(items.is_empty());
        source.report_longlink_speed_test_result(&items);

        source.add_server_ban_at(0, "1.1.1.1");
        assert!(source.ipport_strategy().is_server_banned_at(0, "1.1.1.1"));
        source.remove_long_ban_ip("1.1.1.1");
        assert!(source.ipport_strategy().ban_list().is_empty());

        source.init_history_to_banned_list();
        assert!(format!("{source:?}").contains("NetSource"));
    }
}
