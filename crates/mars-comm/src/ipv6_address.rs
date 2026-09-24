//! `mars/comm/socket/ipv6_address_utils.h` and
//! `mars/comm/socket/nat64_prefix_util.h` — the `IN6_*` macros and the NAT64
//! prefix.
//!
//! The macros of the C++ work on a `struct in6_addr`; here that is a
//! `[u8; 16]`, and the twelve-byte prefixes are their own constants rather
//! than a global `in6_addr` with the last four bytes zeroed.
//!
//! `GetNetworkNat64Prefix()` asks the system — it resolves `ipv4only.arpa`
//! (or `192.0.2.1`) on a network that is IPv6-only, which is why the header
//! warns that it may block. There is nothing to port there: the prefix the
//! host learned is handed in with [`set_nat64_prefix`], and
//! [`convert_v4_to_nat64_v6`] refuses, exactly like the C++, when the stack
//! is not IPv6-only.

use std::sync::{Mutex, OnceLock};

use crate::local_ipstack::LocalIpStack;

/// `in6addr_v4mapped_init` — the first twelve bytes of `::ffff:0:0/96`.
pub const V4_MAPPED_PREFIX: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff];
/// `in6addr_nat64_init` — the first twelve bytes of the well-known
/// `64:ff9b::/96` (RFC 6052).
pub const NAT64_PREFIX: [u8; 12] = [0x00, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0];

/// `kWellKnownNat64Prefix` — the text the C++ puts in front of the embedded
/// IPv4 address, `::` included.
pub const WELL_KNOWN_NAT64_PREFIX_TEXT: &str = "64:ff9b::";

/// The prefix `GetNetworkNat64Prefix()` last came back with. The C++ asks the
/// platform every time; here the host sets what it learned, and the
/// well-known prefix stands in until it does.
pub fn nat64_prefix() -> [u8; 12] {
    *prefix()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `set_nat64_prefix` — what the host learned, or [`NAT64_PREFIX`] to go back
/// to the well-known one.
pub fn set_nat64_prefix(prefix: [u8; 12]) {
    *self::prefix()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = prefix;
}

fn prefix() -> &'static Mutex<[u8; 12]> {
    static PREFIX: OnceLock<Mutex<[u8; 12]>> = OnceLock::new();
    PREFIX.get_or_init(|| Mutex::new(NAT64_PREFIX))
}

/// `IN6_SET_ADDR_V4MAPPED(a6, a4)` — the last four bytes are the v4 address.
pub fn in6_set_addr_v4mapped(v4: [u8; 4]) -> [u8; 16] {
    let mut v6 = [0u8; 16];
    v6[..12].copy_from_slice(&V4_MAPPED_PREFIX);
    v6[12..].copy_from_slice(&v4);
    v6
}

/// `IN6_SET_ADDR_NAT64(a6, a4)` — the macro always uses
/// `in6addr_nat64_init`, so the well-known prefix it is, whatever the
/// network's own is.
pub fn in6_set_addr_nat64(v4: [u8; 4]) -> [u8; 16] {
    let mut v6 = [0u8; 16];
    v6[..12].copy_from_slice(&NAT64_PREFIX);
    v6[12..].copy_from_slice(&v4);
    v6
}

/// The embedded IPv4 address of a v4-mapped or NAT64 address: what
/// `ipv6_address_utils.h` picks out as `s6_addr32[3]`.
pub fn embedded_v4(v6: &[u8; 16]) -> [u8; 4] {
    let mut v4 = [0u8; 4];
    v4.copy_from_slice(&v6[12..]);
    v4
}

/// `IN6_IS_ADDR_V4MAPPED(a6)`.
pub fn in6_is_addr_v4mapped(v6: &[u8; 16]) -> bool {
    v6[..12] == V4_MAPPED_PREFIX
}

/// `IN6_IS_ADDR_NAT64(a6)` — `s6_addr32[0] == htonl(0x0064ff9b)`, so the
/// first four bytes are the well-known prefix and the eight after them are
/// zero. A prefix the network handed out is *not* recognised here, which is
/// why [`SocketAddress::fix_current_nat64_addr`] has to be called on the
/// addresses that came from a NAT64 network.
///
/// [`SocketAddress::fix_current_nat64_addr`]: crate::socket_address::SocketAddress::fix_current_nat64_addr
pub fn in6_is_addr_nat64(v6: &[u8; 16]) -> bool {
    v6[..12] == NAT64_PREFIX
}

/// `ConvertV4toNat64V6(_v4_addr, _v6_addr)` — `None` when the stack is not
/// IPv6-only, which is what the C++ returns `false` for, and the v6 address
/// otherwise: the prefix the host learned with the v4 address embedded in it
/// (RFC 6052).
pub fn convert_v4_to_nat64_v6(v4: [u8; 4], stack: LocalIpStack) -> Option<[u8; 16]> {
    if stack != LocalIpStack::IPv6 {
        return None;
    }
    let mut v6 = [0u8; 16];
    v6[..12].copy_from_slice(&nat64_prefix());
    v6[12..].copy_from_slice(&v4);
    Some(v6)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The prefix is process-wide, so the tests that set it take the crate's
    /// turn: [`crate::test_lock`].
    fn prefixes() -> std::sync::MutexGuard<'static, ()> {
        crate::test_lock()
    }

    #[test]
    fn a_v4_address_is_mapped_into_the_last_four_bytes() {
        let v6 = in6_set_addr_v4mapped([1, 2, 3, 4]);
        assert_eq!(&v6[..12], &V4_MAPPED_PREFIX);
        assert_eq!(&v6[12..], &[1, 2, 3, 4]);
        assert!(in6_is_addr_v4mapped(&v6));
        assert!(
            !in6_is_addr_nat64(&v6),
            "a mapped address is not a nat64 one"
        );
        assert_eq!(embedded_v4(&v6), [1, 2, 3, 4]);
    }

    #[test]
    fn a_nat64_address_carries_the_well_known_prefix() {
        let v6 = in6_set_addr_nat64([192, 0, 2, 1]);
        assert_eq!(&v6[..12], &NAT64_PREFIX);
        assert!(in6_is_addr_nat64(&v6));
        assert!(!in6_is_addr_v4mapped(&v6));
        assert_eq!(embedded_v4(&v6), [192, 0, 2, 1]);
    }

    #[test]
    fn only_the_well_known_prefix_is_recognised() {
        let mut other = in6_set_addr_nat64([1, 1, 1, 1]);
        other[0] = 0x20;
        assert!(!in6_is_addr_nat64(&other));

        // and the eight bytes between the prefix and the v4 address have to
        // be zero for `IN6_IS_ADDR_NAT64`
        let mut padded = in6_set_addr_nat64([1, 1, 1, 1]);
        padded[11] = 1;
        assert!(!in6_is_addr_nat64(&padded));
    }

    #[test]
    fn the_conversion_needs_an_ipv6_only_network() {
        let guard = prefixes();
        set_nat64_prefix(NAT64_PREFIX);

        assert_eq!(
            convert_v4_to_nat64_v6([8, 8, 8, 8], LocalIpStack::IPv4),
            None,
            "the C++ refuses a stack that is not ipv6-only"
        );
        assert_eq!(
            convert_v4_to_nat64_v6([8, 8, 8, 8], LocalIpStack::IPv6),
            Some(in6_set_addr_nat64([8, 8, 8, 8]))
        );

        // a prefix the network handed out is the one that is used
        let mut learned = NAT64_PREFIX;
        learned[0] = 0x20;
        learned[1] = 0x01;
        set_nat64_prefix(learned);
        let converted = convert_v4_to_nat64_v6([8, 8, 8, 8], LocalIpStack::IPv6).unwrap();
        assert_eq!(&converted[..12], &learned);
        assert_eq!(&converted[12..], &[8, 8, 8, 8]);
        assert!(!in6_is_addr_nat64(&converted), "not the well-known prefix");

        set_nat64_prefix(NAT64_PREFIX);
        drop(guard);
    }
}
