//! Integration test: drives the whole public surface of `mars-crypt`
//! from *outside* the crate, so an accidental visibility change shows up here.

use mars_core::AutoBuffer;
use mars_crypt::magic;
use mars_crypt::{LogCrypt, CLIENT_PUBKEY_LEN, HEADER_LEN, TAILER_LEN, TEA_BLOCK_LEN};

/// One decoded record: `(magic_start, seq, (begin_hour, end_hour), len, body)`.
type Decoded<'a> = (u8, u16, (u8, u8), u32, &'a [u8]);

/// Rebuilds the buffer layer's "decode" step: header + payload + tailer.
fn decode(bytes: &[u8]) -> Option<Decoded<'_>> {
    if bytes.len() < HEADER_LEN + TAILER_LEN {
        return None;
    }
    let magic_start = bytes[0];
    if !magic::magic_start_is_valid(magic_start) {
        return None;
    }
    let len = LogCrypt::get_log_len(bytes);
    let body_end = HEADER_LEN + len as usize;
    if bytes.len() < body_end + TAILER_LEN {
        return None;
    }
    Some((
        magic_start,
        u16::from_le_bytes([bytes[1], bytes[2]]),
        (bytes[3], bytes[4]),
        len,
        &bytes[HEADER_LEN..body_end],
    ))
}

#[test]
fn public_constants_match_the_contract() {
    assert_eq!(HEADER_LEN, 73);
    assert_eq!(TAILER_LEN, 1);
    assert_eq!(CLIENT_PUBKEY_LEN, 64);
    assert_eq!(TEA_BLOCK_LEN, 8);
}

#[test]
fn sync_record_round_trips_through_the_public_api() {
    let mut crypt = LogCrypt::new(None);
    assert!(!crypt.is_crypt());
    assert_eq!(crypt.header_len(), HEADER_LEN);
    assert_eq!(crypt.tailer_len(), TAILER_LEN);

    let payload = b"integration test payload";
    let mut out = AutoBuffer::new();
    let total = crypt.crypt_sync_log(
        payload,
        &mut out,
        magic::SYNC_NOCRYPT_ZLIB_START,
        magic::END,
    );

    assert_eq!(total, HEADER_LEN + TAILER_LEN + payload.len());

    let bytes = out.as_slice();
    let (magic_start, seq, hours, len, body) = decode(bytes).expect("record must decode");
    assert_eq!(magic_start, magic::SYNC_NOCRYPT_ZLIB_START);
    assert_eq!(seq, 0);
    assert_eq!(hours, LogCrypt::get_log_hour(bytes).unwrap());
    assert_eq!(len, payload.len() as u32);
    assert_eq!(body, payload);
    assert_eq!(bytes[HEADER_LEN + payload.len()], magic::END);

    // `fix()` agrees with the decoder and restores the sequence number.
    assert_eq!(crypt.fix(bytes), Some(payload.len() as u32));
}

#[test]
fn async_record_round_trips_through_the_public_api() {
    let crypt = LogCrypt::new(None);
    let payload: Vec<u8> = (0..41u8).collect();

    let mut out = Vec::new();
    let mut remain_nocrypt_len = 0usize;
    crypt.crypt_async_log(&payload, &mut out, &mut remain_nocrypt_len);

    assert_eq!(out, payload);
    assert_eq!(remain_nocrypt_len, 0);
    assert_eq!(out.len() % TEA_BLOCK_LEN, payload.len() % TEA_BLOCK_LEN);
}

#[test]
fn statics_work_on_a_standalone_region() {
    let mut region = vec![0u8; HEADER_LEN];
    region[0] = magic::ASYNC_ZSTD_START;
    region[3] = 5;
    region[4] = 6;

    assert_eq!(LogCrypt::get_log_hour(&region), Some((5, 6)));

    LogCrypt::update_log_len(&mut region, 7);
    assert_eq!(LogCrypt::get_log_len(&region), 7);
    LogCrypt::update_log_len(&mut region, 3);
    assert_eq!(LogCrypt::get_log_len(&region), 10);

    LogCrypt::update_log_hour(&mut region);
    let (begin, end) = LogCrypt::get_log_hour(&region).unwrap();
    assert_eq!(begin, 5);
    assert!(end < 24);

    let mut tailer = vec![0xFFu8; TAILER_LEN];
    LogCrypt::set_tailer_info(&mut tailer, magic::END);
    assert_eq!(tailer, vec![magic::END; TAILER_LEN]);
}

#[test]
fn invalid_pubkeys_keep_the_nocrypt_path() {
    for pubkey in [None, Some(""), Some("not-hex"), Some("ab")] {
        assert!(!LogCrypt::new(pubkey).is_crypt());
    }
}
