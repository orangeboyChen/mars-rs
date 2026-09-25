//! `socket_address`, through the public api.
//!
//! The samples are what `mars/comm/socket/socket_address.cc` answers for the
//! same addresses: the url an IPv6 address gets brackets around, the `ip()` of
//! a v4-mapped or NAT64 address is the embedded IPv4 address, and the stack
//! decides which of the three a v4 address is turned into.

use std::sync::{Mutex, OnceLock};

use mars_comm::ipv6_address::{in6_set_addr_nat64, in6_set_addr_v4mapped, set_nat64_prefix};
use mars_comm::{LocalIpStack, SocketAddress};

/// The NAT64 prefix is process-wide, so the tests that set it take it in turn.
///
/// `cargo test` runs this binary apart from the crate's unit tests, so the
/// prefix it sets cannot reach them — but every test in here shares the one
/// process, and therefore this one lock.
fn prefixes() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn the_url_carries_the_port_and_brackets_the_ipv6_address() {
    assert_eq!(SocketAddress::new("1.2.3.4", 8080).url(), "1.2.3.4:8080");
    assert_eq!(
        SocketAddress::new("2001:db8::1", 443).url(),
        "[2001:db8::1]:443"
    );
    assert_eq!(SocketAddress::new("::1", 80).url(), "[::1]:80");
    // and an ip that is neither is empty rather than "not an ip:80"
    assert_eq!(SocketAddress::new("not an ip", 80).url(), "");
    assert!(!SocketAddress::new("not an ip", 80).valid());
}

#[test]
fn the_ip_that_a_caller_would_connect_to() {
    let mapped = SocketAddress::from_v6(in6_set_addr_v4mapped([10, 0, 0, 1]), 80);
    assert_eq!(mapped.ip(), "10.0.0.1");
    assert_eq!(mapped.ipv6(), "::ffff:10.0.0.1");

    let nat64 = SocketAddress::from_v6(in6_set_addr_nat64([192, 0, 2, 1]), 80);
    assert_eq!(nat64.ip(), "192.0.2.1");
    assert_eq!(nat64.ipv6(), "64:ff9b::192.0.2.1");

    // and a real v6 address has nothing to strip
    assert_eq!(SocketAddress::new("2001:db8::1", 443).ip(), "2001:db8::1");
}

#[test]
fn what_a_client_may_connect_to() {
    for (ip, port, allow_loopback, expected) in [
        ("1.2.3.4", 80, false, true),
        // no port
        ("1.2.3.4", 0, false, false),
        // 0.0.0.0 / 255.255.255.255 / 127.0.0.1
        ("0.0.0.0", 80, false, false),
        ("255.255.255.255", 80, false, false),
        ("127.0.0.1", 80, false, false),
        ("127.0.0.1", 80, true, true),
    ] {
        assert_eq!(
            SocketAddress::new(ip, port).valid_server_address(allow_loopback, false),
            expected,
            "{ip}:{port} with allow_loopback={allow_loopback}"
        );
    }
    // ... and an ip that did not parse is not a server either
    assert!(!SocketAddress::new("not an ip", 80).valid_server_address(true, true));
}

#[test]
fn the_stack_decides_what_a_v4_address_becomes() {
    let stacks = [
        (LocalIpStack::None, "::ffff:8.8.8.8"),
        (LocalIpStack::IPv4, "8.8.8.8"),
        (LocalIpStack::IPv6, "64:ff9b::8.8.8.8"),
        (LocalIpStack::Dual, "::ffff:8.8.8.8"),
    ];
    let guard = prefixes();
    set_nat64_prefix(mars_comm::ipv6_address::NAT64_PREFIX);
    for (stack, expected) in stacks {
        let mut addr = SocketAddress::new("8.8.8.8", 53);
        addr.v4_to_v6_address(stack);
        assert_eq!(addr.ipv6(), expected, "on {}", stack.name());
        assert_eq!(addr.ip(), "8.8.8.8", "on {}", stack.name());
        assert_eq!(addr.port(), 53);
    }
    drop(guard);
}

#[test]
fn a_nat64_address_takes_the_prefix_the_network_handed_out() {
    let guard = prefixes();
    let mut learned = mars_comm::ipv6_address::NAT64_PREFIX;
    learned[0] = 0x20;
    learned[1] = 0x01;
    set_nat64_prefix(learned);

    // the well-known prefix is what an address starts with, and the fix moves
    // it under the one the network handed out
    let mut addr = SocketAddress::from_v6(in6_set_addr_nat64([8, 8, 8, 8]), 53);
    assert!(addr.fix_current_nat64_addr(LocalIpStack::IPv6), "ipv6-only");
    assert_eq!(addr.ipv6(), "2001:ff9b::808:808");
    assert_eq!(addr.url(), "[2001:ff9b::808:808]:53");
    assert!(addr.is_v6());

    // on any other stack nothing is rewritten, so the address stays as it is
    let mut dual = SocketAddress::from_v6(in6_set_addr_nat64([8, 8, 8, 8]), 53);
    assert!(!dual.fix_current_nat64_addr(LocalIpStack::Dual));
    assert_eq!(dual.ipv6(), "64:ff9b::8.8.8.8");

    set_nat64_prefix(mars_comm::ipv6_address::NAT64_PREFIX);
    drop(guard);
}

#[test]
fn the_family_of_an_address() {
    let v4 = SocketAddress::new("1.2.3.4", 80);
    assert!(v4.is_v4() && !v4.is_v6() && !v4.is_v4mapped_address());
    assert_eq!(v4.address_length(), 16);

    let mut mapped = SocketAddress::new("1.2.3.4", 80);
    mapped.v4_to_v4mapped_address();
    assert!(mapped.is_v4mapped_address());
    assert!(mapped.is_v4() && !mapped.is_v6());
    assert_eq!(mapped.address_length(), 28);

    let v6 = SocketAddress::new("2001:db8::1", 80);
    assert!(v6.is_v6() && !v6.is_v4());
    assert_eq!(v6.address_length(), 28);

    // an address that did not parse is none of them
    let unspec = SocketAddress::unspecified();
    assert!(!unspec.is_v4() && !unspec.is_v6());
    assert_eq!(unspec.address_length(), 0);
}

#[test]
fn equality_is_the_ip_and_the_port() {
    assert_eq!(
        SocketAddress::new("1.2.3.4", 80),
        SocketAddress::new("1.2.3.4", 80)
    );
    assert_ne!(
        SocketAddress::new("1.2.3.4", 80),
        SocketAddress::new("1.2.3.4", 81)
    );

    // the mapped form of an address is that address: `operator==` is
    // `is_ipport_equal`, and `ip()` strips the prefix
    let mut mapped = SocketAddress::new("1.2.3.4", 80);
    mapped.v4_to_v4mapped_address();
    assert!(mapped.is_ipport_equal(&SocketAddress::new("1.2.3.4", 80)));
    assert_eq!(mapped, SocketAddress::new("1.2.3.4", 80));
    assert_ne!(mapped, SocketAddress::new("1.2.3.4", 81));
}
