//! The short link, driven the way `mars/stn/src/shortlink.cc` drives it: a task
//! on `/cgi-bin/micromsg-bin/short`, the pairs `NetSource` makes out of the host
//! list dns answered, the connect that picks one of them, and the request the
//! run then writes on the socket.
//!
//! The samples are the ones the C++ would write: one task on its own host, one
//! through an http proxy it has to log in to, one on pairs newdns handed the
//! link, and one that takes the socket the pool kept from the task before it.

use std::sync::{Arc, Mutex};

use mars_comm::tickcount::gettickcount;
use mars_comm::{LocalIpStack, ProxyInfo, ProxyType, SocketAddress};
use mars_stn::short_link::{ConnectFail, ShortLink};
use mars_stn::{
    default_packer, pack, request_headers, request_url, CachedSocket, ConnectProfile, ErrCmdType,
    ExtraInfo, IpPortItem, IpSourceType, NetSource, OpBreaker, SocketFd, SocketOperator,
    SocketPool, SocketProfile, Task,
};

/// The reading the whole test runs on: the C++'s `gettickcount()`.
const NOW: u64 = 1000;

/// The five fields mars writes for itself, in its order, and the empty line that
/// ends the head.
const FIELDS: &str = "Accept: */*\r\n\
                      User-Agent: MicroMessenger Client\r\n\
                      Cache-Control: no-cache\r\n\
                      Content-Type: application/octet-stream\r\n\
                      Connection: close\r\n";

/// What the host was asked for, and what it answers with.
#[derive(Clone, Default)]
struct Seen {
    addresses: Arc<Mutex<Vec<Vec<SocketAddress>>>>,
    proxies: Arc<Mutex<Vec<ProxyInfo>>>,
    timeouts: Arc<Mutex<Vec<(u32, u32)>>>,
    reports: Arc<Mutex<Vec<Reported>>>,
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

/// A pipe that is never woken: the port has nothing blocking to give up.
#[derive(Default)]
struct Breaker;

impl OpBreaker for Breaker {
    fn is_break(&mut self) -> bool {
        false
    }

    fn break_(&mut self) -> bool {
        true
    }
}

/// The host's sockets: the numbers it hands out, and what it was asked to do
/// with them.
struct Host {
    seen: Seen,
    next: i64,
    profile: SocketProfile,
    protocol: i32,
    breaker: Breaker,
}

impl Host {
    fn new(seen: Seen) -> Self {
        Self {
            seen,
            next: 3,
            profile: SocketProfile::default(),
            protocol: Task::TRANSPORT_PROTOCOL_TCP,
            breaker: Breaker,
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

    fn send(&mut self, _socket: SocketFd, buffer: &[u8], _timeout_ms: i32) -> Result<usize, i32> {
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

/// The dns of the whole test: it knows the host of the task, and the host of the
/// proxy.
fn dns(host: &str) -> Vec<String> {
    match host {
        "short.weixin.qq.com" => vec!["183.3.226.35".to_string()],
        "proxy.example" => vec!["10.0.0.1".to_string()],
        _ => Vec::new(),
    }
}

/// The net source behind the link: the short-link port, and the backup pair the
/// app keeps for the host.
fn net_source() -> NetSource {
    let mut source = NetSource::new_at(NOW);
    source.set_shortlink(80, "");
    source.set_backup_ips("short.weixin.qq.com", vec!["183.3.226.36".to_string()]);
    source.set_dns(dns);
    source
}

/// A link on the host's sockets, wired to a net source and to a log of what the
/// host was asked for.
fn link(source: Arc<Mutex<NetSource>>, seen: &Seen) -> ShortLink {
    link_of(source, seen, false)
}

/// The same, one that may go through a proxy.
fn proxy_link(source: Arc<Mutex<NetSource>>, seen: &Seen) -> ShortLink {
    link_of(source, seen, true)
}

fn link_of(source: Arc<Mutex<NetSource>>, seen: &Seen, use_proxy: bool) -> ShortLink {
    let mut link = ShortLink::new(task(), use_proxy);
    link.set_socket_operator(Host::new(seen.clone()));
    link.set_shortlink_items(move |hosts, cgi| {
        let mut source = source.lock().unwrap();
        source.get_shortlink_items_at(NOW, hosts, cgi, &ExtraInfo::new())
    });
    link.set_shortlink_port(|| 80);
    link.set_dns(dns);
    link.set_ip_connect_timeout(|| (1500, 2500));
    link.set_local_ip_stack(|| LocalIpStack::IPv4);
    link.set_local_address(|_| Some(SocketAddress::new("192.168.1.2", 34567)));
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
    link
}

/// What the run writes on the socket: the url of the connect, the head of the
/// task, and the body.
fn request_of(profile: &ConnectProfile, task: &Task) -> Vec<u8> {
    let url = request_url(profile, &task.cgi);
    let headers = request_headers(profile, task);
    pack(&url, &headers, b"hello")
}

#[test]
fn a_task_goes_out_on_the_pair_dns_gave() {
    let seen = Seen::default();
    let mut link = link(Arc::new(Mutex::new(net_source())), &seen);
    let socket = link.connect_at(NOW).unwrap();

    assert!(socket.is_valid());
    let profile = link.profile();
    assert_eq!(profile.host, "short.weixin.qq.com");
    assert_eq!(profile.ip, "183.3.226.35");
    assert_eq!(profile.port, 80);
    assert_eq!(profile.ip_type, IpSourceType::Dns);
    assert_eq!(profile.ip_index, 0);
    assert_eq!(profile.local_ip, "192.168.1.2");
    assert_eq!(profile.local_port, 34567);
    assert_eq!(profile.connection_identify, "3@TCP");

    // and the request the run writes on it is the one the C++ writes
    assert_eq!(
        String::from_utf8_lossy(&request_of(profile, link.task())),
        format!(
            "POST /cgi-bin/micromsg-bin/short HTTP/1.1\r\n\
             {FIELDS}\
             Content-Length: 5\r\n\
             Host: short.weixin.qq.com\r\n\
             \r\n\
             hello"
        )
    );

    let addresses = seen.addresses.lock().unwrap();
    assert_eq!(
        addresses[0].len(),
        2,
        "the pair dns gave, and the backup pair the app keeps for the host"
    );
    assert_eq!(addresses[0][0].ip(), "183.3.226.35");
    assert_eq!(addresses[0][0].port(), 80);
    assert_eq!(addresses[0][1].ip(), "183.3.226.36");
    assert_eq!(seen.timeouts.lock().unwrap().as_slice(), &[(1500, 2500)]);
}

#[test]
fn a_task_that_goes_through_an_http_proxy_asks_for_the_whole_url() {
    let seen = Seen::default();
    let mut link = proxy_link(Arc::new(Mutex::new(net_source())), &seen);
    link.set_proxy(|_| ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "mars", "secret"));

    link.connect_at(NOW).unwrap();

    let profile = link.profile();
    assert_eq!(profile.ip_type, IpSourceType::Proxy);
    assert_eq!(profile.ip, "10.0.0.1");
    assert_eq!(
        profile.port, 80,
        "the short-link port of the pair the proxy was pushed as"
    );
    assert_eq!(
        String::from_utf8_lossy(&request_of(profile, link.task())),
        format!(
            "POST http://short.weixin.qq.com/cgi-bin/micromsg-bin/short HTTP/1.1\r\n\
             {FIELDS}\
             Content-Length: 5\r\n\
             Host: short.weixin.qq.com\r\n\
             Proxy-Authorization: Basic bWFyczpzZWNyZXQ=\r\n\
             \r\n\
             hello"
        ),
        "a proxy is asked for the whole url, and logged in to"
    );

    // the connect itself went to the proxy
    let addresses = seen.addresses.lock().unwrap();
    assert_eq!(addresses[0].len(), 1);
    assert_eq!(addresses[0][0].ip(), "10.0.0.1");
    assert_eq!(addresses[0][0].port(), 8080);
}

#[test]
fn a_proxy_named_by_host_is_reached_at_the_ip_dns_gave() {
    let seen = Seen::default();
    let mut link = proxy_link(Arc::new(Mutex::new(net_source())), &seen);
    link.set_proxy(|_| ProxyInfo::new(ProxyType::Http, "proxy.example", "", 8080, "", ""));

    link.connect_at(NOW).unwrap();
    assert_eq!(seen.addresses.lock().unwrap()[0][0].ip(), "10.0.0.1");
    assert_eq!(seen.proxies.lock().unwrap()[0].ip, "10.0.0.1");
}

#[test]
fn a_task_on_a_host_dns_could_not_name_is_no_socket_at_all() {
    let seen = Seen::default();
    // a dns that knows nothing, and no backup pair to fall back to
    let mut source = NetSource::new_at(NOW);
    source.set_shortlink(80, "");
    source.set_dns(|_| Vec::new());
    let mut link = link(Arc::new(Mutex::new(source)), &seen);

    assert_eq!(link.connect_at(NOW), Err(ConnectFail::NoAddress));
    assert_eq!(link.profile().disconn_errtype, ErrCmdType::Dns);
    assert!(
        seen.addresses.lock().unwrap().is_empty(),
        "nothing was connected"
    );
    assert!(
        seen.reports.lock().unwrap().is_empty(),
        "the C++ does not report this one"
    );
}

#[test]
fn a_connect_the_host_refused_says_a_socket_error_on_the_profile() {
    let seen = Seen::default();
    let mut link = link(Arc::new(Mutex::new(net_source())), &seen);
    link.set_socket_operator({
        let mut host = Host::new(seen.clone());
        host.profile.error_code = 110;
        host
    });

    assert_eq!(
        link.connect_at(NOW),
        Err(ConnectFail::Socket { error_code: 110 })
    );
    let profile = link.profile();
    assert_eq!(profile.disconn_errtype, ErrCmdType::Socket);
    assert_eq!(profile.conn_errcode, 110);
}

#[test]
fn the_pairs_newdns_handed_the_link_are_the_ones_it_goes_out_on() {
    let seen = Seen::default();
    let mut link = link(Arc::new(Mutex::new(net_source())), &seen);
    let mut item = IpPortItem::new("10.0.0.9", 443);
    item.source_type = IpSourceType::NewDns;
    item.host = "short.weixin.qq.com".to_string();
    link.set_connect_params(vec![item], 700, 900);

    link.connect_at(NOW).unwrap();

    let profile = link.profile();
    assert_eq!(profile.ip, "10.0.0.9");
    assert_eq!(profile.port, 443);
    assert_eq!(profile.ip_type, IpSourceType::NewDns);
    assert_eq!(
        seen.timeouts.lock().unwrap().as_slice(),
        &[(700, 900)],
        "the timeouts the pairs came with, not the net source's"
    );
    assert_eq!(
        seen.addresses.lock().unwrap()[0][0].ip(),
        "10.0.0.9",
        "dns was not asked at all"
    );
}

#[test]
fn a_task_that_asked_to_be_kept_takes_the_socket_the_pool_kept() {
    let seen = Seen::default();
    let source = Arc::new(Mutex::new(net_source()));
    // the pool the task before this one left its socket in
    let mut pool = SocketPool::new();
    pool.set_is_closed(|_| false);
    let mut item = IpPortItem::new("183.3.226.35", 80);
    item.source_type = IpSourceType::Dns;
    item.host = "short.weixin.qq.com".to_string();
    pool.add_cache(CachedSocket::new_at(NOW, item, SocketFd(11), 15));
    let pool = Arc::new(Mutex::new(pool));

    let mut task = task();
    task.headers
        .insert("Connection".to_string(), "Keep-Alive".to_string());
    let mut link = ShortLink::new(task, false);
    link.set_socket_operator(Host::new(seen.clone()));
    link.set_shortlink_items({
        let source = Arc::clone(&source);
        move |hosts, cgi| {
            let mut source = source.lock().unwrap();
            source.get_shortlink_items_at(NOW, hosts, cgi, &ExtraInfo::new())
        }
    });
    link.set_cache_socket(move |item| pool.lock().unwrap().get_socket_at(NOW, item));

    assert_eq!(link.connect_at(NOW), Ok(SocketFd(11)));
    let profile = link.profile();
    assert_eq!(profile.ip, "183.3.226.35");
    assert_eq!(profile.conn_rtt, 0);
    assert_eq!(profile.conn_cost, 0);
    assert_eq!(profile.socket_fd, SocketFd(11));
    assert!(profile.is_reused_fd);
    assert_eq!(profile.connection_identify, "11@TCP@REUSE");
    assert!(
        seen.addresses.lock().unwrap().is_empty(),
        "a reused socket is not connected"
    );

    // and the next task on it would write the same request
    assert!(request_of(profile, link.task()).ends_with(b"hello"));
}

#[test]
fn a_link_on_quic_marks_every_pair_quic() {
    let seen = Seen::default();
    let mut link = link(Arc::new(Mutex::new(net_source())), &seen);
    link.set_socket_operator({
        let mut host = Host::new(seen.clone());
        host.protocol = Task::TRANSPORT_PROTOCOL_QUIC;
        host
    });

    link.connect_at(NOW).unwrap();
    let profile = link.profile();
    assert_eq!(profile.transport_protocol, Task::TRANSPORT_PROTOCOL_QUIC);
    assert_eq!(
        profile.ip_items[0].transport_protocol,
        Task::TRANSPORT_PROTOCOL_QUIC
    );
}

#[test]
fn a_link_uses_the_clock_the_host_gives_it() {
    let seen = Seen::default();
    let mut link = link(Arc::new(Mutex::new(net_source())), &seen);
    link.connect().unwrap();
    assert!(
        link.profile().dns_time >= gettickcount(),
        "the reading of the clock, whenever that was"
    );
    assert_eq!(link.profile().start_connect_time, link.profile().dns_time);
}

#[test]
fn the_packer_the_app_replaced_is_the_one_the_run_writes_with() {
    let seen = Seen::default();
    let mut link = link(Arc::new(Mutex::new(net_source())), &seen);
    link.connect_at(NOW).unwrap();

    let headers = request_headers(link.profile(), link.task());
    assert_eq!(
        default_packer()("/cgi", &headers, b"hello"),
        pack("/cgi", &headers, b"hello")
    );
}
