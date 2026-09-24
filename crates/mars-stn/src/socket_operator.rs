//! `mars/stn/src/socket_operator.h` and `tcp_socket_operator.cc` — what a link
//! does with a socket.
//!
//! The C++ has an abstract `SocketOperator` and one implementation per
//! transport, `TcpSocketOperator`, which is the one that talks to the platform:
//! `socket()`, `connect()`, `send()`, `recv()`, `close()`. None of that belongs
//! in this crate, so the port keeps the shape and hands the doing to the host:
//! [`SocketOperator`] is a trait the host implements, and a socket is an opaque
//! [`SocketFd`] it hands back — not a file descriptor the port would have to
//! know how to read.
//!
//! Two things in `tcp_socket_operator.cc` are not platform calls but decisions,
//! and they are two functions here: [`contain_ipv6`] asks whether any of the
//! addresses a connect was given is a v6 one, and [`is_impatient`] is what
//! `TcpSocketOperator::Connect` decides with it. [`tcp_identify`] is the string
//! the C++ writes for a socket in its logs.
//!
//! `TcpSocketOperator::ErrorDesc` is the platform's `strerror`, which is not
//! ported: what a platform calls its errors is the host's to say.

use mars_comm::{ProxyInfo, SocketAddress};

/// `SOCKET` — what the platform hands out for a socket.
///
/// The C++'s is an `int` on unix and a `HANDLE` on windows, and it is compared
/// against `INVALID_SOCKET` everywhere, which is why there is one here rather
/// than a bare number: [`SocketFd::INVALID`] is what a connect that failed
/// answers, and there is no other number that means anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SocketFd(pub i64);

impl SocketFd {
    /// `INVALID_SOCKET`.
    pub const INVALID: Self = Self(-1);

    /// `INVALID_SOCKET != _sock` — whether this is one the platform gave out.
    pub fn is_valid(self) -> bool {
        self != Self::INVALID
    }
}

impl Default for SocketFd {
    /// A socket nobody got: `0` is a descriptor the platform can hand out, so
    /// the default is the one that means "none".
    fn default() -> Self {
        Self::INVALID
    }
}

/// `SocketProfile` — what one connect left behind for the caller to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SocketProfile {
    /// `rtt` — how long the address that won took to answer.
    pub rtt: u32,
    /// `index` — which of the addresses the connect was given won.
    pub index: i32,
    /// `errorCode` — why the connect failed, in the platform's words.
    pub error_code: i32,
    /// `totalCost` — how long the whole connect took.
    pub total_cost: u32,
    /// `is0rtt` — whether the connect was a 0-rtt one.
    pub is_0rtt: i32,
}

/// `OPBreaker` — the pipe the C++ wakes to get a blocking call to give up.
pub trait OpBreaker {
    /// `IsBreak()` — whether it was woken.
    fn is_break(&mut self) -> bool;
    /// `Break()` — wake it.
    fn break_(&mut self) -> bool;
}

/// `SocketOperator` — everything a link asks of a socket.
///
/// What the C++ answers through an `int& _errcode` is a `Result` here: `Err`
/// carries the error code it would have written, and the platform's description
/// of one is [`SocketOperator::error_desc`].
///
/// It is `Send`, which is what lets a link be shared: the long link is one
/// value the link's own run, its monitor, its timer check and its signalling
/// keeper all hold (see [`crate::longlink_metadata`]), and they ask it for
/// things through callbacks that have to be `Send` too.
pub trait SocketOperator: Send {
    /// `Connect(_vecaddr, _proxy_type, _proxy_addr, _proxy_username,
    /// _proxy_pwd)` — a socket on one of the addresses, or
    /// [`SocketFd::INVALID`].
    fn connect(&mut self, addresses: &[SocketAddress], proxy: &ProxyInfo) -> SocketFd;

    /// `Send(_sock, _buffer, _len, _errcode, _timeout)` — how many bytes went
    /// out, or the error code. A timeout of `-1`, the C++'s default, is a
    /// timeout the host picks.
    fn send(&mut self, socket: SocketFd, buffer: &[u8], timeout_ms: i32) -> Result<usize, i32>;

    /// `Recv(_sock, _buffer, _max_size, _errcode, _timeout,
    /// _wait_full_size)` — the bytes that came in, or the error code.
    fn recv(
        &mut self,
        socket: SocketFd,
        max_size: usize,
        timeout_ms: i32,
        wait_full_size: bool,
    ) -> Result<Vec<u8>, i32>;

    /// `Close(_sock)`.
    fn close(&mut self, socket: SocketFd);

    /// `Identify(_sock)` — what the socket is called in a log.
    fn identify(&self, socket: SocketFd) -> String;

    /// `Protocol()` — one of the `Task::kTransportProtocol*` values.
    fn protocol(&self) -> i32;

    /// `ErrorDesc(_errcode)` — the platform's description of an error code.
    fn error_desc(&self, error_code: i32) -> String;

    /// `Profile()` — what the last connect left behind.
    fn profile(&self) -> SocketProfile;

    /// `Breaker()` — the pipe a blocking call of this operator listens to.
    fn breaker(&mut self) -> &mut dyn OpBreaker;

    /// `CreateStream(_sock)` — a quic stream on a socket that is already open,
    /// or [`SocketFd::INVALID`].
    fn create_stream(&mut self, socket: SocketFd) -> SocketFd;

    /// `SetIpConnectionTimeout(_v4_timeout, _v6_timeout)` — what
    /// [`is_impatient`] then decides with.
    fn set_ip_connection_timeout(&mut self, v4_timeout_ms: u32, v6_timeout_ms: u32);
}

/// `ContainIPv6(_vecaddr)` — whether any of the addresses is a v6 one, which is
/// what makes a connect race a v4 and a v6 address against each other.
pub fn contain_ipv6(addresses: &[SocketAddress]) -> bool {
    addresses.iter().any(SocketAddress::is_v6)
}

/// Whether `TcpSocketOperator::Connect` builds the connect that tries two
/// addresses at once (`ConnectImpatient` with a v4 and a v6 timeout): it does
/// when there is a v6 address to try *and* the host gave a timeout for both.
pub fn is_impatient(addresses: &[SocketAddress], v4_timeout_ms: u32, v6_timeout_ms: u32) -> bool {
    contain_ipv6(addresses) && v4_timeout_ms > 0 && v6_timeout_ms > 0
}

/// `TcpSocketOperator::Identify(_sock)` — `"%d@TCP"`.
pub fn tcp_identify(socket: SocketFd) -> String {
    format!("{}@TCP", socket.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(ip: &str) -> SocketAddress {
        SocketAddress::new(ip, 80)
    }

    #[test]
    fn a_socket_is_invalid_until_the_platform_hands_one_out() {
        assert!(!SocketFd::default().is_valid());
        assert!(!SocketFd::INVALID.is_valid());
        assert!(SocketFd(3).is_valid());
        assert_eq!(SocketFd::default(), SocketFd::INVALID);
    }

    #[test]
    fn only_a_v6_address_makes_a_list_contain_one() {
        assert!(!contain_ipv6(&[]));
        assert!(!contain_ipv6(&[v4("1.1.1.1"), v4("2.2.2.2")]));
        assert!(contain_ipv6(&[
            v4("1.1.1.1"),
            SocketAddress::new("::1", 80)
        ]));
    }

    #[test]
    fn a_connect_is_impatient_only_with_a_v6_address_and_both_timeouts() {
        let v6 = [SocketAddress::new("::1", 80)];
        let v4s = [v4("1.1.1.1")];

        assert!(is_impatient(&v6, 1000, 1000));
        // one of the two timeouts missing is a connect that tries one address
        assert!(!is_impatient(&v6, 0, 1000));
        assert!(!is_impatient(&v6, 1000, 0));
        assert!(!is_impatient(&v4s, 1000, 1000));
    }

    #[test]
    fn a_socket_is_identified_by_the_number_the_platform_gave_it() {
        assert_eq!(tcp_identify(SocketFd(7)), "7@TCP");
    }
}
