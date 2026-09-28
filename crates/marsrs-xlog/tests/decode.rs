//! The reader against a file that is not whole.
//!
//! Damage in the middle of a `.xlog` is what an operator actually meets: a
//! block a killed process never finished, a byte that went wrong on its way to
//! wherever the file was copied. `decode_log_file.c` walks past it —
//! `getLogStartPos` finds where the log starts again and `decodeBuffer` marks
//! the span it skipped — so the records behind the damage are still in the
//! output. These are the cases that check that this reader does the same,
//! built out of records by hand because a file that decodes to a known text is
//! easier to assert on than one the appender wrote.

use marsrs_crypt::{magic, CLIENT_PUBKEY_LEN};

/// One sync record, `header + body + tailer`: the body is the log text itself,
/// so what these files say is what a decoder of them has to answer.
fn sync_record(text: &[u8]) -> Vec<u8> {
    record(
        magic::SYNC_NOCRYPT_ZLIB_START,
        &[0; CLIENT_PUBKEY_LEN],
        text,
    )
}

/// One record with `magic` in front of it and `pubkey` in its header.
fn record(magic_start: u8, pubkey: &[u8; CLIENT_PUBKEY_LEN], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(magic_start);
    out.extend_from_slice(&1u16.to_le_bytes()); // seq
    out.push(0); // begin hour
    out.push(0); // end hour
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(pubkey);
    out.extend_from_slice(body);
    out.push(magic::END);
    out
}

/// A whole record, then one whose magic is no longer a magic, then one more:
/// what is behind the damage is the point of the walk.
///
/// The offset the record starts at and how many bytes the walk has to skip are
/// two different numbers, because the three records do not hold texts of the
/// same length.
fn damaged_file() -> (Vec<u8>, usize, usize) {
    let first = sync_record(b"first\n");
    let damaged = first.len();
    let skipped = sync_record(b"second\n").len();

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&first);
    bytes.extend_from_slice(&sync_record(b"second\n"));
    bytes.extend_from_slice(&sync_record(b"third\n"));

    // The magic of the second record, which is the only byte of it that has to
    // be wrong for a reader that walks one record at a time to stop there.
    bytes[damaged] = 0xff;
    (bytes, damaged, skipped)
}

#[test]
fn the_records_behind_the_damage_are_decoded_too() {
    let (bytes, _, _) = damaged_file();
    let plain = marsrs_xlog::decode_records(&bytes, None).expect("the walk went on");
    let text = String::from_utf8_lossy(&plain);

    assert!(text.contains("first\n"), "{text}");
    assert!(text.contains("third\n"), "{text}");
    // The damaged record's own text is not in the output — it is not in the
    // file either — and the span is named instead.
    assert!(!text.contains("second\n"), "{text}");
    assert!(
        text.contains("[F]decode_log_file.py decode error len="),
        "the skipped span is not marked: {text}"
    );
}

#[test]
fn the_marker_names_how_much_was_skipped() {
    let (bytes, damaged, skipped) = damaged_file();
    let plain = marsrs_xlog::decode_records(&bytes, None).expect("the walk went on");
    let text = String::from_utf8_lossy(&plain);

    // From the byte the walk stopped at to the byte the next record starts at,
    // which is the whole of the record the file lost.
    assert!(damaged < bytes.len(), "the fixture is not damaged");
    assert!(
        text.contains(&format!(
            "[F]decode_log_file.py decode error len={skipped}\n"
        )),
        "the marker does not name the span: {text}"
    );
}

/// A key the point is not on: `uECC_shared_secret` answers 0 for it, and
/// `decodeBuffer` puts its marker where the record's text would have been
/// rather than ending the walk.
#[test]
fn a_record_no_key_can_be_derived_for_leaves_its_marker_behind() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&sync_record(b"before\n"));
    // A public key that is 64 of the same byte is not a point on the curve.
    bytes.extend_from_slice(&record(
        magic::ASYNC_ZLIB_START,
        &[0x11; CLIENT_PUBKEY_LEN],
        b"\x00",
    ));
    bytes.extend_from_slice(&sync_record(b"after\n"));

    let plain = marsrs_xlog::decode_records(&bytes, Some(&[0x11; 32])).expect("the walk went on");
    let text = String::from_utf8_lossy(&plain);

    assert!(text.contains("before\n"), "{text}");
    assert!(text.contains("Get ECDH key error"), "{text}");
    assert!(text.contains("after\n"), "{text}");
}

/// Nothing behind the damage is a record either, which is the one case there
/// is nothing to go on to: the reason is what the caller gets.
#[test]
fn a_file_with_no_record_in_it_says_so() {
    let bytes = vec![0xff; 4 * 1024];
    let err = marsrs_xlog::decode_records(&bytes, None).expect_err("there is no record here");

    assert!(err.recovered.is_empty(), "nothing was recovered: {err:?}");
    assert!(err.reason.contains("bad magic"), "{err:?}");
}

/// A record whose body will not inflate is a record that is there: the marker
/// stands in for its text and the walk goes on at the one behind it.
#[test]
fn a_body_that_will_not_inflate_leaves_its_marker_behind() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&sync_record(b"before\n"));
    // A zstd body that is not a zstd frame at all.
    bytes.extend_from_slice(&record(
        magic::ASYNC_NOCRYPT_ZSTD_START,
        &[0; CLIENT_PUBKEY_LEN],
        b"not a zstd frame",
    ));
    bytes.extend_from_slice(&sync_record(b"after\n"));

    let plain = marsrs_xlog::decode_records(&bytes, None).expect("the walk went on");
    let text = String::from_utf8_lossy(&plain);

    assert!(text.contains("before\n"), "{text}");
    assert!(text.contains("zstd decompress error"), "{text}");
    assert!(text.contains("after\n"), "{text}");
}
