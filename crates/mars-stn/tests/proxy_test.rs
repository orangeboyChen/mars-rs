//! The proxy test, driven the way `mars/stn/src/proxy_test.cc` drives it: a
//! proxy the app set, the host a task would go to, and the answer the host's
//! socket gives the `GET` the test writes on it.
//!
//! The samples are the ones the C++ would see: an http proxy reached on its own
//! ip, one named by host and reached at what dns answered, a socks5 one the
//! test connects through, no proxy at all, and a host that has nothing but
//! hardcode ips to be reached on.

use std::sync::{Arc, Mutex};

use mars_comm::{LocalIpStack, ProxyInfo, ProxyType, SocketAddress};
use mars_stn::{OpBreaker, ProxyTest, SocketFd, SocketOperator, SocketProfile, Task, Verdict};

/// The four fields mars writes for itself, and the empty line that ends the
/// head: a `GET` has no `Content-Type` and no `Content-Length`.
const FIELDS: &str = "Accept: */*\r\n\
                      User-Agent: MicroMessenger Client\r\n\
                      Cache-Control: no-cache\r\n\
                      Connection: close\r\n";

/// What the host was asked for, and what it answers with.
#[derive(Clone, Default)]
struct Seen {
    addresses: Arc<Mutex<Vec<Vec<SocketAddress>>>>,
    proxies: Arc<Mutex<Vec<ProxyInfo>>>,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
    closed: Arc<Mutex<Vec<SocketFd>>>,
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
    breaker: Breaker,
}

impl Host {
    fn new(seen: Seen) -> Self {
        Self {
            seen,
            next: 3,
            breaker: Breaker,
        }
    }
}

impl SocketOperator for Host {
    fn connect(&mut self, addresses: &[SocketAddress], proxy: &ProxyInfo) -> SocketFd {
        self.seen.addresses.lock().unwrap().push(addresses.to_vec());
        self.seen.proxies.lock().unwrap().push(proxy.clone());
        let socket = SocketFd(self.next);
        self.next += 1;
        socket
    }

    fn send(&mut self, _socket: SocketFd, buffer: &[u8], timeout_ms: i32) -> Result<usize, i32> {
        // the C++ hands no timeout of its own, which is the socket's to pick
        assert_eq!(timeout_ms, -1);
        self.seen.sent.lock().unwrap().push(buffer.to_vec());
        Ok(buffer.len())
    }

    /// `Recv` — the reads of the answer are handed to the test as an argument,
    /// so the host is never asked for one; what it would have been asked for is
    /// [`mars_stn::BUFFER_SIZE`] bytes, waiting [`mars_stn::READ_TIMEOUT_MS`].
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
        Task::TRANSPORT_PROTOCOL_TCP
    }

    fn error_desc(&self, error_code: i32) -> String {
        format!("error {error_code}")
    }

    fn profile(&self) -> SocketProfile {
        SocketProfile::default()
    }

    fn breaker(&mut self) -> &mut dyn OpBreaker {
        &mut self.breaker
    }

    fn create_stream(&mut self, _socket: SocketFd) -> SocketFd {
        SocketFd::INVALID
    }

    fn set_ip_connection_timeout(&mut self, _v4_timeout_ms: u32, _v6_timeout_ms: u32) {}
}

/// The dns of the whole test: it knows the host a task goes to, and the host of
/// the proxy.
fn dns(host: &str) -> Vec<String> {
    match host {
        "short.weixin.qq.com" => vec!["183.3.226.35".to_string()],
        "proxy.example" => vec!["10.0.0.1".to_string()],
        _ => Vec::new(),
    }
}

/// A test on the host's sockets, wired to the dns above and to a log of what the
/// host was asked for.
fn test_of(seen: &Seen) -> ProxyTest {
    let mut test = ProxyTest::new();
    test.set_socket_operator(Host::new(seen.clone()));
    test.set_dns(dns);
    test.set_local_stack(|| LocalIpStack::IPv4);
    test
}

/// The reads of one answer, which is the whole of it.
fn answered(status: i32) -> Vec<Result<Vec<u8>, i32>> {
    vec![Ok(format!(
        "HTTP/1.1 {status} OK\r\nContent-Length: 0\r\n\r\n"
    )
    .into_bytes())]
}

#[test]
fn an_http_proxy_the_host_answers_two_hundred_on_is_available() {
    let seen = Seen::default();
    let mut test = test_of(&seen);
    let proxy = ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "", "");

    let verdict = test.test(
        &proxy,
        "short.weixin.qq.com",
        &[],
        answered(200).into_iter(),
    );
    assert_eq!(verdict, Verdict::Status(200));
    assert!(verdict.is_available());

    // the test went *to* the proxy, and asked it for the whole url
    let addresses = seen.addresses.lock().unwrap().clone();
    assert_eq!(addresses.len(), 1);
    assert_eq!(addresses[0][0].ip(), "10.0.0.1");
    assert_eq!(addresses[0][0].port(), 8080);
    let sent = seen.sent.lock().unwrap().clone();
    assert_eq!(
        String::from_utf8_lossy(&sent[0]),
        format!(
            "GET http://short.weixin.qq.com/ HTTP/1.1\r\n{FIELDS}Host: short.weixin.qq.com\r\n\r\n"
        )
    );
}

#[test]
fn a_proxy_the_host_answers_five_hundred_on_is_not_available() {
    let seen = Seen::default();
    let mut test = test_of(&seen);
    let proxy = ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "", "");

    assert!(!test.is_available(
        &proxy,
        "short.weixin.qq.com",
        &[],
        answered(500).into_iter()
    ));
    // the socket is closed whatever became of the answer
    assert_eq!(seen.closed.lock().unwrap().clone(), vec![SocketFd(3)]);
}

#[test]
fn a_socks5_proxy_is_one_the_test_connects_through() {
    let seen = Seen::default();
    let mut test = test_of(&seen);
    let proxy = ProxyInfo::new(ProxyType::Socks5, "", "10.0.0.1", 1080, "", "");

    let verdict = test.test(
        &proxy,
        "short.weixin.qq.com",
        &[],
        answered(302).into_iter(),
    );
    assert_eq!(verdict, Verdict::Status(302), "a redirect is available");

    // the host under test, on the test port, and the proxy alongside it
    let addresses = seen.addresses.lock().unwrap().clone();
    assert_eq!(addresses[0][0].ip(), "183.3.226.35");
    assert_eq!(addresses[0][0].port(), mars_stn::TEST_PORT);
    let proxies = seen.proxies.lock().unwrap().clone();
    assert_eq!(proxies[0], proxy);
    let sent = seen.sent.lock().unwrap().clone();
    assert!(
        String::from_utf8_lossy(&sent[0]).starts_with("GET / HTTP/1.1\r\n"),
        "a proxy that is not an http one is asked for the path"
    );
}

#[test]
fn a_proxy_named_by_host_is_reached_at_the_ip_dns_gave() {
    let seen = Seen::default();
    let mut test = test_of(&seen);
    let proxy = ProxyInfo::new(ProxyType::Http, "proxy.example", "", 8080, "", "");

    test.test(
        &proxy,
        "short.weixin.qq.com",
        &[],
        answered(200).into_iter(),
    );

    let addresses = seen.addresses.lock().unwrap().clone();
    assert_eq!(addresses[0][0].ip(), "10.0.0.1", "what dns answered");
    let proxies = seen.proxies.lock().unwrap().clone();
    assert_eq!(
        proxies[0],
        ProxyInfo {
            ip: "10.0.0.1".to_string(),
            ..proxy
        }
    );
}

#[test]
fn no_proxy_is_a_test_of_the_host_itself() {
    let seen = Seen::default();
    let mut test = test_of(&seen);

    let verdict = test.test(
        &ProxyInfo::none(),
        "short.weixin.qq.com",
        &[],
        answered(497).into_iter(),
    );
    assert_eq!(verdict, Verdict::Status(497));
    assert!(verdict.is_available(), "497 is one the C++ calls available");

    let addresses = seen.addresses.lock().unwrap().clone();
    assert_eq!(addresses[0][0].ip(), "183.3.226.35");
    assert_eq!(addresses[0][0].port(), mars_stn::TEST_PORT);
    let proxies = seen.proxies.lock().unwrap().clone();
    assert_eq!(proxies[0], ProxyInfo::none(), "no proxy to go through");
}

#[test]
fn a_host_with_only_hardcode_ips_is_reached_on_them() {
    let seen = Seen::default();
    let mut test = test_of(&seen);
    let hardcode = vec!["183.3.226.36".to_string()];

    let verdict = test.test(
        &ProxyInfo::none(),
        "unknown.example",
        &hardcode,
        answered(200).into_iter(),
    );
    assert_eq!(verdict, Verdict::Status(200));

    let addresses = seen.addresses.lock().unwrap().clone();
    assert_eq!(addresses[0][0].ip(), "183.3.226.36", "dns knew nothing");
}

#[test]
fn a_test_of_a_host_nothing_can_name_is_not_available() {
    let seen = Seen::default();
    let mut test = test_of(&seen);

    let verdict = test.test(
        &ProxyInfo::none(),
        "unknown.example",
        &[],
        answered(200).into_iter(),
    );
    assert_eq!(verdict, Verdict::Failed);
    assert!(
        seen.addresses.lock().unwrap().is_empty(),
        "no connect at all"
    );
    assert!(seen.sent.lock().unwrap().is_empty());
}

#[test]
fn an_http_proxy_with_an_account_is_logged_in_with() {
    let seen = Seen::default();
    let mut test = test_of(&seen);
    let proxy = ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "mars", "secret");

    test.test(
        &proxy,
        "short.weixin.qq.com",
        &[],
        answered(200).into_iter(),
    );

    let sent = seen.sent.lock().unwrap().clone();
    assert!(
        String::from_utf8_lossy(&sent[0])
            .contains("Proxy-Authorization: Basic bWFyczpzZWNyZXQ=\r\n"),
        "the account in base64"
    );
}
