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
pub fn nat64_prefix() -> Nat64Prefix {
    *prefix()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `set_nat64_prefix` — what the host learned, or [`Nat64Prefix::well_known`]
/// to go back to the well-known one.
pub fn set_nat64_prefix(prefix: Nat64Prefix) {
    *self::prefix()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = prefix;
}

fn prefix() -> &'static Mutex<Nat64Prefix> {
    static PREFIX: OnceLock<Mutex<Nat64Prefix>> = OnceLock::new();
    PREFIX.get_or_init(|| Mutex::new(Nat64Prefix::well_known()))
}

/// `in6addr_nat64_init` and the length of the prefix the network handed out.
///
/// RFC 6052 lets a NAT64 prefix be 96, 64, 56, 48, 40 or 32 bits long, and
/// which one it is decides *where* in the address the IPv4 address sits
/// (`ReplaceNat64WithV4IP`, `nat64_prefix_util.cc:162`): only `Pref64::/96`
/// puts it in the last four bytes, and every shorter one leaves the byte at 8
/// zero. The C++ reads the length off the address the resolver synthesised, as
/// a count of trailing zero bytes; the port asks for it, because a host that
/// learned a bare prefix has no synthesised address to read it from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nat64Prefix {
    /// The first twelve bytes of it; the bits after `len` are zero.
    pub bytes: [u8; 12],
    /// How many bits of `bytes` are the prefix. A length outside the six of
    /// RFC 6052 is read as 96, which is the C++'s own answer for one it did
    /// not expect.
    pub len: u32,
}

impl Nat64Prefix {
    /// `in6addr_nat64_init`, `64:ff9b::/96` — the well-known prefix of RFC
    /// 6052, and what stands in until the host sets what its network handed
    /// out.
    pub fn well_known() -> Self {
        Self {
            bytes: NAT64_PREFIX,
            len: 96,
        }
    }

    /// The offset the IPv4 address is written at: `ReplaceNat64WithV4IP`'s
    /// switch (`nat64_prefix_util.cc:162`), by the length of the prefix. The
    /// three lengths in between put it around byte 8, which is the zero `u`
    /// octet RFC 6052 leaves there for everything but `/96` and `/32`.
    fn v4_offset(&self) -> usize {
        match self.len {
            96 => 12,
            64 => 9,
            56 => 7,
            48 => 6,
            40 => 5,
            32 => 4,
            // the C++'s `default`: it asserts, and goes on with 12
            _ => 12,
        }
    }
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
/// first four bytes are the well-known prefix and the eight after them go
/// unread. An address upstream classifies as NAT64 is classified as one here,
/// whatever sits between the prefix and the embedded address.
///
/// A prefix the network handed out is *not* recognised, which is why
/// [`SocketAddress::fix_current_nat64_addr`] has to be called on the
/// addresses that came from a NAT64 network.
///
/// [`SocketAddress::fix_current_nat64_addr`]: crate::socket_address::SocketAddress::fix_current_nat64_addr
pub fn in6_is_addr_nat64(v6: &[u8; 16]) -> bool {
    v6[..4] == NAT64_PREFIX[..4]
}

/// `ConvertV4toNat64V6(_v4_addr, _v6_addr)` — `None` when the stack is not
/// IPv6-only, which is what the C++ returns `false` for, and the v6 address
/// otherwise: the prefix the host learned with the v4 address embedded in it
/// (RFC 6052).
///
/// The v4 goes where [`Nat64Prefix::len`] says it goes and not always in the
/// last four bytes, which is what `ReplaceNat64WithV4IP` does with the address
/// the resolver synthesised (`nat64_prefix_util.cc:162`). Only the bits of the
/// prefix are copied, so the byte at 8 is the zero `u` octet the three middle
/// lengths leave there.
pub fn convert_v4_to_nat64_v6(v4: [u8; 4], stack: LocalIpStack) -> Option<[u8; 16]> {
    if stack != LocalIpStack::IPv6 {
        return None;
    }
    let prefix = nat64_prefix();
    let mut v6 = [0u8; 16];
    let prefix_bytes = (prefix.len / 8).min(12) as usize;
    v6[..prefix_bytes].copy_from_slice(&prefix.bytes[..prefix_bytes]);
    match prefix.len {
        56 => {
            v6[7] = v4[0];
            v6[9..12].copy_from_slice(&v4[1..]);
        }
        48 => {
            v6[6..8].copy_from_slice(&v4[..2]);
            v6[9..11].copy_from_slice(&v4[2..]);
        }
        40 => {
            v6[5..8].copy_from_slice(&v4[..3]);
            v6[9] = v4[3];
        }
        _ => {
            let at = prefix.v4_offset();
            v6[at..at + 4].copy_from_slice(&v4);
        }
    }
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
    }

    #[test]
    fn the_nat64_test_reads_the_first_four_bytes_only() {
        // `s6_addr32[0] == htonl(0x0064ff9b)`: the eight bytes between the
        // prefix and the embedded address are not part of the test, so an
        // address that padded them is a NAT64 one all the same.
        let mut padded = in6_set_addr_nat64([1, 1, 1, 1]);
        padded[11] = 1;
        assert!(in6_is_addr_nat64(&padded));
        padded[4] = 0x20;
        assert!(in6_is_addr_nat64(&padded));
        assert_eq!(embedded_v4(&padded), [1, 1, 1, 1]);
    }

    #[test]
    fn the_conversion_needs_an_ipv6_only_network() {
        let guard = prefixes();
        set_nat64_prefix(Nat64Prefix::well_known());

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
        set_nat64_prefix(Nat64Prefix {
            bytes: learned,
            len: 96,
        });
        let converted = convert_v4_to_nat64_v6([8, 8, 8, 8], LocalIpStack::IPv6).unwrap();
        assert_eq!(&converted[..12], &learned);
        assert_eq!(&converted[12..], &[8, 8, 8, 8]);
        assert!(!in6_is_addr_nat64(&converted), "not the well-known prefix");

        set_nat64_prefix(Nat64Prefix::well_known());
        drop(guard);
    }

    #[test]
    fn a_prefix_shorter_than_96_moves_the_v4_address_out_of_the_tail() {
        // `ReplaceNat64WithV4IP` (`nat64_prefix_util.cc:162`): a /96 is the
        // only length that puts the whole v4 in the last four bytes — a /64
        // starts it one byte in, and the three in between split it around the
        // zero `u` octet at byte 8.
        let guard = prefixes();
        let v4 = [8, 8, 4, 4];

        set_nat64_prefix(Nat64Prefix {
            bytes: NAT64_PREFIX,
            len: 64,
        });
        let converted = convert_v4_to_nat64_v6(v4, LocalIpStack::IPv6).unwrap();
        assert_eq!(&converted[..8], &NAT64_PREFIX[..8], "eight bytes of /64");
        assert_eq!(converted[8], 0, "the `u` octet");
        assert_eq!(&converted[9..13], &v4);
        assert_eq!(&converted[13..], &[0, 0, 0]);

        set_nat64_prefix(Nat64Prefix {
            bytes: NAT64_PREFIX,
            len: 56,
        });
        let converted = convert_v4_to_nat64_v6(v4, LocalIpStack::IPv6).unwrap();
        assert_eq!(&converted[..7], &NAT64_PREFIX[..7], "seven bytes of /56");
        assert_eq!(converted[7], v4[0]);
        assert_eq!(converted[8], 0);
        assert_eq!(&converted[9..12], &v4[1..]);

        set_nat64_prefix(Nat64Prefix {
            bytes: NAT64_PREFIX,
            len: 48,
        });
        let converted = convert_v4_to_nat64_v6(v4, LocalIpStack::IPv6).unwrap();
        assert_eq!(&converted[..6], &NAT64_PREFIX[..6], "six bytes of /48");
        assert_eq!(&converted[6..8], &v4[..2]);
        assert_eq!(converted[8], 0);
        assert_eq!(&converted[9..11], &v4[2..]);

        set_nat64_prefix(Nat64Prefix {
            bytes: NAT64_PREFIX,
            len: 40,
        });
        let converted = convert_v4_to_nat64_v6(v4, LocalIpStack::IPv6).unwrap();
        assert_eq!(&converted[..5], &NAT64_PREFIX[..5], "five bytes of /40");
        assert_eq!(&converted[5..8], &v4[..3]);
        assert_eq!(converted[8], 0);
        assert_eq!(converted[9], v4[3]);

        set_nat64_prefix(Nat64Prefix {
            bytes: NAT64_PREFIX,
            len: 32,
        });
        let converted = convert_v4_to_nat64_v6(v4, LocalIpStack::IPv6).unwrap();
        assert_eq!(&converted[..4], &NAT64_PREFIX[..4], "four bytes of /32");
        assert_eq!(&converted[4..8], &v4);

        set_nat64_prefix(Nat64Prefix::well_known());
        drop(guard);
    }

    #[test]
    fn a_length_rfc_6052_does_not_have_is_read_as_96() {
        // the C++'s `default`, which asserts and goes on with the tail
        let guard = prefixes();
        set_nat64_prefix(Nat64Prefix {
            bytes: NAT64_PREFIX,
            len: 72,
        });
        assert_eq!(
            convert_v4_to_nat64_v6([8, 8, 4, 4], LocalIpStack::IPv6),
            Some(in6_set_addr_nat64([8, 8, 4, 4]))
        );
        set_nat64_prefix(Nat64Prefix::well_known());
        drop(guard);
    }
}
