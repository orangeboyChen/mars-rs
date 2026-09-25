//! `mars/comm/adler32.c` — the Adler-32 checksum.

use mars_comm::adler32::{adler32, adler32_seeded};

#[test]
fn adler32_matches_the_reference_implementation() {
    // `mars/comm/adler32.c` is the Mark Adler reference implementation, which
    // is what zlib ships; these are its sums started from the seed STN passes
    // (0, not zlib's 1), so every value is one lower than the one everyone
    // quotes.
    assert_eq!(adler32(b""), 0x0000_0000);
    assert_eq!(adler32(b"a"), 0x0061_0061);
    assert_eq!(adler32(b"abc"), 0x024a_0126);
    assert_eq!(adler32(b"Wikipedia"), 0x11dd_0397);
    assert_eq!(
        adler32(b"The quick brown fox jumps over the lazy dog"),
        0x5bb1_0fd9
    );
    assert_eq!(adler32(&(0..=255u8).collect::<Vec<u8>>()), 0xacf6_7f80);
    assert_eq!(adler32(&b"mars".repeat(100)), 0x164b_a9ec);

    // the hash the table keys on, and the two ends of the sum it is built from
    let a = 0u32;
    assert_eq!(adler32(b""), a);
    assert_ne!(adler32(b"a"), adler32(b"b"));
}

#[test]
fn a_seed_of_nothing_is_the_hash_of_the_bytes() {
    // `adler32(0, ...)`: the two sums start at 0
    assert_eq!(adler32_seeded(0, b"mars"), adler32(b"mars"));
    assert_eq!(adler32_seeded(0, b""), 0);
}

#[test]
fn a_hash_read_on_is_the_hash_of_the_bytes_in_one_go() {
    // `adler32(adler32(0, url), body)` is `adler32(0, url ++ body)`, which is
    // what the C++ relies on when it hashes the body of a package after its URL
    let whole = b"the url and the body";
    let (url, body) = whole.split_at(7);

    assert_eq!(adler32_seeded(adler32(url), body), adler32(whole));
    assert_eq!(adler32(whole), 0x4b62_0736);
    assert_ne!(adler32_seeded(adler32(url), body), adler32(body));

    // a seed that is not the sums of the bytes in front of these
    assert_ne!(adler32_seeded(1, body), adler32(body));
}
