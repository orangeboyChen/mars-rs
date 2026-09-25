//! `mars/comm/adler32.c` — the Adler-32 checksum, the Mark Adler reference
//! implementation in the arithmetic of `unsigned long`.
//!
//! [`adler32`] is the seed-0 call STN keys its avalanche table on
//! (`mars-stn`'s `FrequencyLimit`, and this crate's own `frequency_limit`); [`adler32_seeded`]
//! is the hash continued over more bytes, which is what
//! [`basepacker`](crate::basepacker) does with the checksum of a package it
//! has already started.
//!
//! The unrolled `NMAX` blocks of the C are one modulo per block and nothing
//! else: a byte at a time with a modulo after each one is the same sum, which
//! is what these two do, and neither can overflow the way the C would on a
//! 32-bit `unsigned long`. Not ported: `adler32_combine`, which no caller in
//! mars has.

/// `BASE` — the largest prime smaller than 65536, which the sums are taken
/// modulo.
const BASE: u32 = 65521;

/// `::adler32(0, buffer, len)`, the hash STN keys its avalanche table on.
///
/// `mars/comm/adler32.c` is the public-domain Mark Adler reference
/// implementation; this is the same arithmetic. The seed is the `0` STN passes
/// rather than the `1` zlib starts from, so an empty body hashes to `0` and
/// `"mars"` hashes one lower than the value everyone quotes: what matters for
/// the table — and for the C++ it shares the table with — is that the two
/// sides agree, not that they agree with zlib.
pub fn adler32(data: &[u8]) -> u32 {
    adler32_seeded(0, data)
}

/// `::adler32(adler, buffer, len)` — the checksum of `data` read on from the
/// sums a previous call left in `seed`, the way
/// [`basepacker::packer_pack`](crate::basepacker::packer_pack) hashes the body
/// of a package after it has hashed its URL.
///
/// The seed is the two 16-bit sums of an earlier call: `seed & 0xffff` is `a`,
/// `seed >> 16` is `b`. A seed of `0` is [`adler32`].
pub fn adler32_seeded(seed: u32, data: &[u8]) -> u32 {
    let mut a = seed & 0xffff;
    let mut b = (seed >> 16) & 0xffff;

    for byte in data {
        a = (a + u32::from(*byte)) % BASE;
        b = (b + a) % BASE;
    }

    (b << 16) | a
}
