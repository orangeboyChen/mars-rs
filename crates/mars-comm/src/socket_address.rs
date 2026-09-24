//! `mars/comm/socket/socket_address.cc` — an ip and a port.
//!
//! The C++ keeps a `sockaddr_storage` and, next to it, the two `char[]` it
//! fills from it: `ip_` (the text, and for a NAT64 address the well-known
//! prefix in front of the embedded IPv4 address) and `url_`
//! (`ip:port`, `[ip]:port` for IPv6). The port keeps an address family and the
//! raw bytes of the address, and fills the same two strings, which is what
//! every caller of `socket_address` actually reads; the `sockaddr` itself is
//! only ever handed to the platform's socket calls.
//!
//! Two members have no counterpart here:
//!
//! * `getsockname`/`getpeername` need a socket;
//! * `local_ipstack_detect()` needs the platform's interfaces.
//!
//! So the stack the host detected is an argument of the two methods that need
//! it ([`SocketAddress::v4_to_v6_address`] and
//! [`SocketAddress::fix_current_nat64_addr`]) instead of a global the code
//! asks for behind the caller's back.

use std::net::{Ipv4Addr, Ipv6Addr};

use crate::ipv6_address::{
    convert_v4_to_nat64_v6, embedded_v4, in6_is_addr_nat64, in6_is_addr_v4mapped,
    in6_set_addr_nat64, in6_set_addr_v4mapped, WELL_KNOWN_NAT64_PREFIX_TEXT,
};
use crate::local_ipstack::LocalIpStack;

/// `INADDR_ANY`.
const INADDR_ANY: u32 = 0x0000_0000;
/// `INADDR_BROADCAST` — 255.255.255.255, which is `INADDR_NONE` too.
const INADDR_BROADCAST: u32 = 0xffff_ffff;
/// `INADDR_LOOPBACK` — 127.0.0.1, `htonl`ed.
const INADDR_LOOPBACK: u32 = 0x7f00_0001;

/// The `::ffff:` the C++ looks for with `strncasecmp` before it decides an
/// address is not a NAT64 one.
const V4_MAPPED_PREFIX_TEXT: &str = "::ffff:";

/// What the C++ `sockaddr_storage` holds: an address is either family, or
/// `AF_UNSPEC` — what is left when the ip did not parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Address {
    /// `sockaddr_in`.
    V4([u8; 4], u16),
    /// `sockaddr_in6`.
    V6([u8; 16], u16),
    /// `AF_UNSPEC`.
    Unspec,
}

/// `socket_address`.
#[derive(Debug, Clone)]
pub struct SocketAddress {
    addr: Address,
    /// `ip_` — the text of the address, NAT64 prefix included when there is
    /// one. [`SocketAddress::ip`] is what strips it again.
    ip: String,
    /// `url_` — `ip:port`, or `[ip]:port` for IPv6.
    url: String,
}

impl SocketAddress {
    /// `socket_address(const char* _ip, uint16_t _port)` — `AF_UNSPEC` when
    /// neither `inet_pton(AF_INET)` nor `inet_pton(AF_INET6)` takes the ip.
    pub fn new(ip: &str, port: u16) -> Self {
        if let Ok(v4) = ip.parse::<Ipv4Addr>() {
            Self::from_v4(v4.octets(), port)
        } else if let Ok(v6) = ip.parse::<Ipv6Addr>() {
            Self::from_v6(v6.octets(), port)
        } else {
            Self::unspecified()
        }
    }

    /// `socket_address(const sockaddr_in&)` — or `const struct in_addr&`,
    /// which is the same thing with the port left at zero.
    pub fn from_v4(v4: [u8; 4], port: u16) -> Self {
        Self::init(Address::V4(v4, port))
    }

    /// `socket_address(const sockaddr_in6&)`.
    pub fn from_v6(v6: [u8; 16], port: u16) -> Self {
        Self::init(Address::V6(v6, port))
    }

    /// What the C++ leaves for an address that is neither: `AF_UNSPEC`, an
    /// empty ip and an empty url.
    pub fn unspecified() -> Self {
        SocketAddress {
            addr: Address::Unspec,
            ip: String::new(),
            url: String::new(),
        }
    }

    /// `__init` — fills `ip_` and `url_` from the address.
    fn init(addr: Address) -> Self {
        let ip = match addr {
            Address::V4(v4, _) => Ipv4Addr::from(v4).to_string(),
            Address::V6(v6, _) if in6_is_addr_nat64(&v6) => {
                // The C++ copies `kWellKnownNat64Prefix` in front and then
                // `inet_ntop`s into `ip_ + 9`, which on the Apple platforms
                // writes the embedded IPv4 address there (their `inet_ntop`
                // renders a NAT64 address that way). The port writes the
                // embedded address directly, which is what the code intends
                // and what `ip()` assumes when it strips the prefix again.
                format!(
                    "{}{}",
                    WELL_KNOWN_NAT64_PREFIX_TEXT,
                    Ipv4Addr::from(embedded_v4(&v6))
                )
            }
            Address::V6(v6, _) => Ipv6Addr::from(v6).to_string(),
            Address::Unspec => String::new(),
        };
        let url = match addr {
            Address::Unspec => String::new(),
            Address::V4(..) => format!("{ip}:{}", addr.port()),
            Address::V6(..) => format!("[{ip}]:{}", addr.port()),
        };
        SocketAddress { addr, ip, url }
    }

    /// `ip()` — the text a caller would connect to: for a v4-mapped or NAT64
    /// address that is the embedded IPv4 address, because the C++ skips the
    /// `::ffff:` / `64:ff9b::` in front of it.
    pub fn ip(&self) -> &str {
        match self.addr {
            Address::V4(..) => &self.ip,
            Address::V6(..) => {
                // `strncasecmp`: the prefixes are ascii, so the byte offsets
                // of the lowered copy are the ones of `ip_` too.
                let lower = self.ip.to_ascii_lowercase();
                if lower.starts_with(V4_MAPPED_PREFIX_TEXT) {
                    &self.ip[V4_MAPPED_PREFIX_TEXT.len()..]
                } else if lower.starts_with(WELL_KNOWN_NAT64_PREFIX_TEXT) {
                    &self.ip[WELL_KNOWN_NAT64_PREFIX_TEXT.len()..]
                } else {
                    &self.ip
                }
            }
            Address::Unspec => "",
        }
    }

    /// `ipv6()` — `ip_` as it was written, prefix and all.
    pub fn ipv6(&self) -> &str {
        &self.ip
    }

    /// `url()`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// `port()`.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// `valid()`.
    pub fn valid(&self) -> bool {
        !matches!(self.addr, Address::Unspec)
    }

    /// `valid_server_address(_allowloopback, _ignore_port)` — an address a
    /// client may connect to: a port (unless the port is ignored), and not
    /// `INADDR_ANY`, not the broadcast address and not the loopback address
    /// (unless loopback is allowed).
    ///
    /// A v6 address that is neither v4-mapped nor NAT64 is taken as it is —
    /// the C++ answers `true` for it with a `// TODO`.
    pub fn valid_server_address(&self, allow_loopback: bool, ignore_port: bool) -> bool {
        let host = match self.addr {
            Address::V4(v4, port) => {
                if !ignore_port && port == 0 {
                    return false;
                }
                u32::from_be_bytes(v4)
            }
            Address::V6(v6, port) if in6_is_addr_v4mapped(&v6) => {
                if !ignore_port && port == 0 {
                    return false;
                }
                u32::from_be_bytes(embedded_v4(&v6))
            }
            Address::V6(..) => return true,
            Address::Unspec => return false,
        };
        host != INADDR_ANY
            && host != INADDR_BROADCAST
            && (allow_loopback || host != INADDR_LOOPBACK)
    }

    /// `valid_broadcast_address()` — the broadcast address *and* a port.
    pub fn valid_broadcast_address(&self) -> bool {
        match self.addr {
            Address::V4(v4, port) => port != 0 && u32::from_be_bytes(v4) == INADDR_BROADCAST,
            _ => false,
        }
    }

    /// `valid_loopback_ip()` — only `127.0.0.1`, and only as an IPv4 address.
    pub fn valid_loopback_ip(&self) -> bool {
        match self.addr {
            Address::V4(v4, _) => u32::from_be_bytes(v4) == INADDR_LOOPBACK,
            _ => false,
        }
    }

    /// `valid_broadcast_ip()` — `255.255.255.255`, without the port.
    pub fn valid_broadcast_ip(&self) -> bool {
        match self.addr {
            Address::V4(v4, _) => u32::from_be_bytes(v4) == INADDR_BROADCAST,
            _ => false,
        }
    }

    /// `isv4mapped_address()`.
    pub fn is_v4mapped_address(&self) -> bool {
        match self.addr {
            Address::V6(v6, _) => in6_is_addr_v4mapped(&v6),
            _ => false,
        }
    }

    /// `isv6()` — a v6 address that is not a v4-mapped one.
    pub fn is_v6(&self) -> bool {
        matches!(self.addr, Address::V6(..)) && !self.is_v4mapped_address()
    }

    /// `isv4()` — either an IPv4 address or a v4-mapped one.
    pub fn is_v4(&self) -> bool {
        matches!(self.addr, Address::V4(..)) || self.is_v4mapped_address()
    }

    /// `v4tov4mapped_address()` — an address that is already v6 is left alone.
    pub fn v4_to_v4mapped_address(&mut self) -> &mut Self {
        if let Address::V4(v4, port) = self.addr {
            *self = Self::init(Address::V6(in6_set_addr_v4mapped(v4), port));
        }
        self
    }

    /// `v4tonat64_address()` — the address the C++ builds is the well-known
    /// `64:ff9b::/96` one, but `address_fix()` runs behind it and replaces it
    /// with the prefix the network actually uses when the stack is IPv6-only.
    pub fn v4_to_nat64_address(&mut self, stack: LocalIpStack) -> &mut Self {
        if self.is_v6() {
            return self;
        }
        if let Address::V4(v4, port) = self.addr {
            *self = Self::init(Address::V6(in6_set_addr_nat64(v4), port));
        }
        self.fix_current_nat64_addr(stack);
        self
    }

    /// `v4tov6_address(stack)` — NAT64 on an IPv6-only network, the address
    /// itself on an IPv4-only one and v4-mapped otherwise, which is the
    /// POSIX branch of the C++ (the Windows one maps nothing).
    pub fn v4_to_v6_address(&mut self, stack: LocalIpStack) -> &mut Self {
        match stack {
            LocalIpStack::IPv6 => self.v4_to_nat64_address(stack),
            LocalIpStack::IPv4 => self,
            LocalIpStack::None | LocalIpStack::Dual => self.v4_to_v4mapped_address(),
        }
    }

    /// `fix_current_nat64_addr()` — `true` when the address was rewritten.
    ///
    /// A v4-mapped address is skipped (`strncasecmp("::FFFF:", ip_, 7)`), and
    /// so is anything that is not v6 at all; what is left gets the embedded
    /// IPv4 address moved under the prefix the network handed out, which is
    /// [`convert_v4_to_nat64_v6`] and therefore IPv6-only.
    pub fn fix_current_nat64_addr(&mut self, stack: LocalIpStack) -> bool {
        let Address::V6(v6, port) = self.addr else {
            return false;
        };
        if self
            .ip
            .to_ascii_lowercase()
            .starts_with(V4_MAPPED_PREFIX_TEXT)
        {
            return false;
        }
        let Some(converted) = convert_v4_to_nat64_v6(embedded_v4(&v6), stack) else {
            return false;
        };
        let text = Ipv6Addr::from(converted).to_string();
        let ip = if text.starts_with(WELL_KNOWN_NAT64_PREFIX_TEXT) {
            format!(
                "{}{}",
                WELL_KNOWN_NAT64_PREFIX_TEXT,
                Ipv4Addr::from(embedded_v4(&converted))
            )
        } else {
            text
        };
        *self = SocketAddress {
            addr: Address::V6(converted, port),
            url: format!("[{ip}]:{port}"),
            ip,
        };
        true
    }

    /// `is_ipport_equal(_sa)` — the ip and the port, which is what
    /// `operator==` compares.
    pub fn is_ipport_equal(&self, other: &Self) -> bool {
        self.ip() == other.ip() && self.port() == other.port()
    }

    /// `address_length()` — `sizeof(sockaddr_in)` or `sizeof(sockaddr_in6)`,
    /// and nothing at all for an address that did not parse.
    pub fn address_length(&self) -> usize {
        match self.addr {
            Address::V4(..) => 16,
            Address::V6(..) => 28,
            Address::Unspec => 0,
        }
    }
}

impl PartialEq for SocketAddress {
    /// `operator==` — which is `is_ipport_equal`, so a v4-mapped address
    /// equals the v4 address it carries.
    fn eq(&self, other: &Self) -> bool {
        self.is_ipport_equal(other)
    }
}

impl Eq for SocketAddress {}

impl Address {
    fn port(self) -> u16 {
        match self {
            Self::V4(_, port) | Self::V6(_, port) => port,
            Self::Unspec => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipv6_address::{in6_set_addr_nat64, set_nat64_prefix, NAT64_PREFIX};
    use std::sync::{Mutex, OnceLock};

    /// The NAT64 prefix is process-wide, so the tests that set it take it in
    /// turn.
    fn prefixes() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn an_ip_that_is_neither_is_unspecified() {
        let addr = SocketAddress::new("not an ip", 80);
        assert!(!addr.valid());
        assert_eq!(addr.ip(), "");
        assert_eq!(addr.url(), "");
        assert_eq!(addr.port(), 0);
        assert_eq!(addr.address_length(), 0);
    }

    #[test]
    fn a_v4_address_keeps_its_text_and_its_url() {
        let addr = SocketAddress::new("1.2.3.4", 8080);
        assert_eq!(addr.ip(), "1.2.3.4");
        assert_eq!(addr.ipv6(), "1.2.3.4");
        assert_eq!(addr.url(), "1.2.3.4:8080");
        assert_eq!(addr.port(), 8080);
        assert_eq!(addr.address_length(), 16);
        assert!(addr.is_v4());
        assert!(!addr.is_v6());
        assert!(!addr.is_v4mapped_address());
    }

    #[test]
    fn a_v6_address_is_bracketed_in_its_url() {
        let addr = SocketAddress::new("2001:db8::1", 443);
        assert_eq!(addr.ip(), "2001:db8::1");
        assert_eq!(addr.url(), "[2001:db8::1]:443");
        assert_eq!(addr.address_length(), 28);
        assert!(addr.is_v6());
        assert!(!addr.is_v4());
    }

    #[test]
    fn a_v4_mapped_address_reports_the_v4_address() {
        let addr = SocketAddress::from_v6(in6_set_addr_v4mapped([10, 0, 0, 1]), 80);
        assert!(addr.is_v4mapped_address());
        assert!(addr.is_v4(), "isv4() takes the mapped ones too");
        assert!(!addr.is_v6(), "... and isv6() does not");
        assert_eq!(addr.ip(), "10.0.0.1");
        assert_eq!(addr.ipv6(), "::ffff:10.0.0.1");
        assert_eq!(addr.url(), "[::ffff:10.0.0.1]:80");
    }

    #[test]
    fn a_nat64_address_reports_the_embedded_v4_address() {
        let addr = SocketAddress::from_v6(in6_set_addr_nat64([192, 0, 2, 1]), 80);
        assert_eq!(addr.ip(), "192.0.2.1", "the prefix is stripped");
        assert_eq!(addr.ipv6(), "64:ff9b::192.0.2.1", "... but ip_ keeps it");
        assert_eq!(addr.url(), "[64:ff9b::192.0.2.1]:80");
        assert!(addr.is_v6());
        assert!(!addr.is_v4());
    }

    #[test]
    fn what_a_client_may_connect_to() {
        assert!(SocketAddress::new("1.2.3.4", 80).valid_server_address(false, false));
        // no port
        assert!(!SocketAddress::new("1.2.3.4", 0).valid_server_address(false, false));
        assert!(SocketAddress::new("1.2.3.4", 0).valid_server_address(false, true));
        // 0.0.0.0, 255.255.255.255 and 127.0.0.1
        assert!(!SocketAddress::new("0.0.0.0", 80).valid_server_address(false, false));
        assert!(!SocketAddress::new("255.255.255.255", 80).valid_server_address(false, false));
        assert!(!SocketAddress::new("127.0.0.1", 80).valid_server_address(false, false));
        assert!(SocketAddress::new("127.0.0.1", 80).valid_server_address(true, false));
        // a v4-mapped address is judged by the v4 address it carries
        assert!(
            !SocketAddress::from_v6(in6_set_addr_v4mapped([127, 0, 0, 1]), 80)
                .valid_server_address(false, false)
        );
        // ... and a real v6 one is taken as it comes
        assert!(SocketAddress::new("2001:db8::1", 0).valid_server_address(false, false));
        assert!(!SocketAddress::unspecified().valid_server_address(false, false));
    }

    #[test]
    fn the_loopback_and_the_broadcast_addresses() {
        let loopback = SocketAddress::new("127.0.0.1", 80);
        assert!(loopback.valid_loopback_ip());
        assert!(!loopback.valid_broadcast_ip());
        assert!(!loopback.valid_broadcast_address(), "no broadcast port");

        let broadcast = SocketAddress::new("255.255.255.255", 7);
        assert!(broadcast.valid_broadcast_ip());
        assert!(broadcast.valid_broadcast_address());
        assert!(!broadcast.valid_loopback_ip());

        // none of it applies to v6
        assert!(!SocketAddress::new("::1", 80).valid_loopback_ip());
        assert!(!SocketAddress::new("2001:db8::1", 80).valid_broadcast_address());
    }

    #[test]
    fn mapping_a_v4_address_leaves_the_ip_alone() {
        let mut addr = SocketAddress::new("1.2.3.4", 80);
        addr.v4_to_v4mapped_address();
        assert_eq!(addr.ip(), "1.2.3.4");
        assert!(addr.is_v4mapped_address());

        // twice changes nothing
        addr.v4_to_v4mapped_address();
        assert_eq!(addr.ip(), "1.2.3.4");
        assert!(addr.is_v4mapped_address());
    }

    #[test]
    fn the_stack_decides_what_a_v4_address_becomes() {
        let mut v4 = SocketAddress::new("8.8.8.8", 53);
        v4.v4_to_v6_address(LocalIpStack::IPv4);
        assert_eq!(v4.ip(), "8.8.8.8");
        assert!(!v4.is_v6());

        let mut mapped = SocketAddress::new("8.8.8.8", 53);
        mapped.v4_to_v6_address(LocalIpStack::Dual);
        assert!(mapped.is_v4mapped_address());
        assert_eq!(mapped.ip(), "8.8.8.8");
    }

    #[test]
    fn a_nat64_address_is_fixed_to_the_prefix_of_the_network() {
        let guard = prefixes();
        set_nat64_prefix(NAT64_PREFIX);

        // on a network that is not ipv6-only nothing is converted, so the
        // well-known prefix is what stays
        let mut addr = SocketAddress::new("8.8.8.8", 53);
        addr.v4_to_v6_address(LocalIpStack::IPv6);
        assert_eq!(addr.ipv6(), "64:ff9b::8.8.8.8");
        assert_eq!(addr.ip(), "8.8.8.8");

        // ... and on one that is, with a prefix of its own
        let mut learned = NAT64_PREFIX;
        learned[0] = 0x20;
        learned[1] = 0x01;
        set_nat64_prefix(learned);
        let mut addr = SocketAddress::new("8.8.8.8", 53);
        addr.v4_to_v6_address(LocalIpStack::IPv6);
        assert_eq!(addr.ip(), "2001:ff9b::808:808", "no prefix to strip");
        assert!(addr.is_v6());

        // a v4-mapped address is never touched
        let mut mapped = SocketAddress::new("8.8.8.8", 53);
        mapped.v4_to_v4mapped_address();
        assert!(!mapped.fix_current_nat64_addr(LocalIpStack::IPv6));
        assert_eq!(mapped.ip(), "8.8.8.8");

        set_nat64_prefix(NAT64_PREFIX);
        drop(guard);
    }

    #[test]
    fn two_addresses_are_equal_by_ip_and_port() {
        assert_eq!(
            SocketAddress::new("1.2.3.4", 80),
            SocketAddress::new("1.2.3.4", 80)
        );
        assert_ne!(
            SocketAddress::new("1.2.3.4", 80),
            SocketAddress::new("1.2.3.4", 81)
        );
        // and the mapped form is the same address
        let mut mapped = SocketAddress::new("1.2.3.4", 80);
        mapped.v4_to_v4mapped_address();
        assert!(mapped.is_ipport_equal(&SocketAddress::new("1.2.3.4", 80)));
        assert_eq!(mapped, SocketAddress::new("1.2.3.4", 80));
    }
}
