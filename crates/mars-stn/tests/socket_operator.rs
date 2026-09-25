//! What `TcpSocketOperator` decides and what it calls a socket, through the
//! public API — and one host operator, so the [`SocketOperator`] the port leaves
//! to the host is answered by something.

use mars_comm::{ProxyInfo, ProxyType, SocketAddress};
use mars_stn::{
    contain_ipv6, is_impatient, tcp_identify, OpBreaker, SocketFd, SocketOperator, SocketProfile,
};

/// A host's `OPBreaker`: a flag that is woken once.
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

/// A host's `TcpSocketOperator`: the sockets are numbers it hands out, and what
/// it was asked to do is what it keeps.
struct Host {
    next: i64,
    opened: Vec<SocketFd>,
    sent: Vec<Vec<u8>>,
    closed: Vec<SocketFd>,
    breaker: Breaker,
    profile: SocketProfile,
    timeouts: (u32, u32),
}

impl Host {
    fn new() -> Self {
        Self {
            next: 3,
            opened: Vec::new(),
            sent: Vec::new(),
            closed: Vec::new(),
            breaker: Breaker { broken: false },
            profile: SocketProfile::default(),
            timeouts: (0, 0),
        }
    }
}

impl SocketOperator for Host {
    fn connect(&mut self, _addresses: &[SocketAddress], _proxy: &ProxyInfo) -> SocketFd {
        let socket = SocketFd(self.next);
        self.next += 1;
        self.opened.push(socket);
        socket
    }

    fn send(&mut self, _socket: SocketFd, buffer: &[u8], _timeout_ms: i32) -> Result<usize, i32> {
        self.sent.push(buffer.to_vec());
        Ok(buffer.len())
    }

    fn recv(
        &mut self,
        _socket: SocketFd,
        _max_size: usize,
        _timeout_ms: i32,
        _wait_full_size: bool,
    ) -> Result<Vec<u8>, i32> {
        Ok(b"answer".to_vec())
    }

    fn close(&mut self, socket: SocketFd) {
        self.closed.push(socket);
    }

    fn identify(&self, socket: SocketFd) -> String {
        tcp_identify(socket)
    }

    fn protocol(&self) -> i32 {
        mars_stn::Task::TRANSPORT_PROTOCOL_TCP
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

    fn set_ip_connection_timeout(&mut self, v4_timeout_ms: u32, v6_timeout_ms: u32) {
        self.timeouts = (v4_timeout_ms, v6_timeout_ms);
    }
}

#[test]
fn a_socket_is_invalid_until_the_host_hands_one_out() {
    assert!(!SocketFd::default().is_valid());
    assert!(!SocketFd::INVALID.is_valid());
    assert!(SocketFd(0).is_valid());
    assert_eq!(SocketFd::default(), SocketFd::INVALID);
}

#[test]
fn the_host_opens_sends_and_closes_through_the_operator() {
    let mut host = Host::new();

    let socket = host.connect(&[SocketAddress::new("1.1.1.1", 80)], &ProxyInfo::none());
    assert!(socket.is_valid());
    assert_eq!(host.send(socket, b"hello", -1), Ok(5));
    assert_eq!(
        host.recv(socket, 1024, 1_000, false),
        Ok(b"answer".to_vec())
    );
    assert_eq!(host.protocol(), mars_stn::Task::TRANSPORT_PROTOCOL_TCP);
    assert_eq!(host.identify(socket), "3@TCP");

    host.close(socket);
    assert_eq!(host.closed, vec![socket]);
    assert_eq!(host.profile(), SocketProfile::default());
}

#[test]
fn a_breaker_is_woken_once_and_says_so() {
    let mut host = Host::new();
    assert!(!host.breaker().is_break());

    assert!(host.breaker().break_());
    assert!(host.breaker().is_break());
}

#[test]
fn the_timeouts_the_host_sets_are_what_a_connect_decides_with() {
    let mut host = Host::new();
    host.set_ip_connection_timeout(1_000, 2_000);
    assert_eq!(host.timeouts, (1_000, 2_000));
}

#[test]
fn only_a_v6_address_makes_a_connect_race_two_addresses() {
    let v4 = [SocketAddress::new("1.1.1.1", 80)];
    let v6 = [SocketAddress::new("::1", 80)];

    assert!(!contain_ipv6(&v4));
    assert!(contain_ipv6(&v6));

    // `ConnectImpatient` with two timeouts: a v6 address and both of them set
    assert!(is_impatient(&v6, 1_000, 2_000));
    assert!(!is_impatient(&v6, 0, 2_000));
    assert!(!is_impatient(&v6, 1_000, 0));
    assert!(!is_impatient(&v4, 1_000, 2_000));
}

#[test]
fn a_stream_is_made_on_a_socket_the_host_already_has() {
    let mut host = Host::new();
    let socket = host.connect(&[SocketAddress::new("::1", 80)], &ProxyInfo::none());
    assert_eq!(host.create_stream(socket), SocketFd(socket.0 + 100));
}

#[test]
fn the_proxy_a_connect_goes_through_is_the_one_the_host_is_given() {
    let mut host = Host::new();
    let proxy = ProxyInfo::new(ProxyType::Socks5, "", "10.0.0.1", 1080, "u", "p");
    assert!(proxy.is_valid());

    let socket = host.connect(&[SocketAddress::new("1.1.1.1", 80)], &proxy);
    assert_eq!(host.opened, vec![socket]);
}
