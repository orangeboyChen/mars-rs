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
    // Every sync record the writer puts out is numbered 0 and every async one
    // is numbered by a counter that skips 0, so 1 is the first number of most
    // files: what these fixtures are built to look like.
    record_with_seq(magic_start, pubkey, 1, body)
}

/// One record numbered `seq`: the hole `decodeBuffer` marks is a hole in these
/// numbers, and not in the bytes that carry them.
fn record_with_seq(
    magic_start: u8,
    pubkey: &[u8; CLIENT_PUBKEY_LEN],
    seq: u16,
    body: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(magic_start);
    out.extend_from_slice(&seq.to_le_bytes()); // seq
    out.push(0); // begin hour
    out.push(0); // end hour
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(pubkey);
    out.extend_from_slice(body);
    out.push(magic::END);
    out
}

/// One record of the shortest kind there is — `magic`, a four-byte length, the
/// text XORed with the key the two of them make, and the tailer: six bytes, and
/// a file that ends in one is a file with a record in it however short the file
/// is. `0x01` is `MAGIC_CRYPT_START`, the oldest start `decode_log_file.c`
/// reads.
fn legacy_record(text: &[u8]) -> Vec<u8> {
    // `BASE_KEY ^ (0xff & length) ^ magic`, the cipher of a record whose header
    // carries no client public key.
    let key = 0xcc ^ (text.len() as u8) ^ 0x01;
    let mut out = vec![0x01];
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend(text.iter().map(|byte| byte ^ key));
    out.push(magic::END);
    out
}

/// An async record for `text`, compressed the way the appender compresses it.
fn async_record(seq: u16, text: &[u8]) -> Vec<u8> {
    use std::io::Write;

    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 0).expect("the encoder starts");
    encoder.write_all(text).expect("the text compresses");
    let body = encoder.finish().expect("the frame ends");
    record_with_seq(
        magic::ASYNC_NOCRYPT_ZSTD_START,
        &[0; CLIENT_PUBKEY_LEN],
        seq,
        &body,
    )
}

/// An async file of three records, numbered 1, 2 and 4: the third is the one
/// that says the file lost a record in the middle of it.
fn file_with_a_hole() -> Vec<u8> {
    let mut bytes = Vec::new();
    for (seq, text) in [(1, "a\n"), (2, "b\n"), (4, "c\n")] {
        bytes.extend_from_slice(&async_record(seq, text.as_bytes()));
    }
    bytes
}

/// A block the file lost is a hole in the sequence numbers of the records
/// either side of it, and that is all it is: the bytes of the record that went
/// are gone too, so nothing else in the file says it was ever there.
#[test]
fn a_sequence_that_leaves_a_hole_is_marked_before_the_record_behind_it() {
    let plain = marsrs_xlog::decode_records(&file_with_a_hole(), None).expect("the walk went on");
    let text = String::from_utf8_lossy(&plain);

    assert_eq!(
        text,
        "a\nb\n[F]decode_log_file.py log seq:3-3 is missing\nc\n"
    );
}

/// A hole of more than one record names both ends of it.
#[test]
fn the_marker_names_the_first_and_the_last_sequence_lost() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&async_record(1, b"a\n"));
    bytes.extend_from_slice(&async_record(5, b"b\n"));

    let plain = marsrs_xlog::decode_records(&bytes, None).expect("the walk went on");
    let text = String::from_utf8_lossy(&plain);

    assert_eq!(text, "a\n[F]decode_log_file.py log seq:2-4 is missing\nb\n");
}

/// A file whose records follow one another is a whole file, however it opens:
/// what the C exempts `0` and `1` for, so that neither a sync record nor the
/// first record of a file is read as a hole.
#[test]
fn a_sequence_with_no_hole_in_it_is_not_marked() {
    let mut bytes = Vec::new();
    for (seq, text) in [(1, "a\n"), (2, "b\n"), (3, "c\n")] {
        bytes.extend_from_slice(&async_record(seq, text.as_bytes()));
    }
    // A sync record is numbered 0 — the writer fills no sequence in for it —
    // so it neither counts as a hole nor moves the numbering on: the async
    // record behind it follows the one in front of it.
    bytes.extend_from_slice(&record_with_seq(
        magic::SYNC_NOCRYPT_ZLIB_START,
        &[0; CLIENT_PUBKEY_LEN],
        0,
        b"d\n",
    ));
    bytes.extend_from_slice(&async_record(4, b"e\n"));

    let plain = marsrs_xlog::decode_records(&bytes, None).expect("the walk went on");
    let text = String::from_utf8_lossy(&plain);

    assert_eq!(text, "a\nb\nc\nd\ne\n");
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

/// A private key no secret can be derived from is the record's failure and not
/// the file's: `uECC_shared_secret` answers 0 for it — the same answer a client
/// key that is not a point gets — and `decodeBuffer` goes on at the record
/// behind it either way. One such record used to end the walk, and the days of
/// log standing behind it went with it.
#[test]
fn a_private_key_no_secret_comes_from_leaves_its_marker_behind() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&sync_record(b"before\n"));
    // The header's key never gets as far as being checked: the private key is
    // the first thing the ECDH is asked about, and it is the one refused.
    bytes.extend_from_slice(&record(
        magic::ASYNC_ZLIB_START,
        &[0x11; CLIENT_PUBKEY_LEN],
        b"\x00",
    ));
    bytes.extend_from_slice(&sync_record(b"after\n"));

    // A scalar of zero is not a secp256k1 private key. Reaching this case at
    // all takes an unusual caller: a key of zeroes, or one past the order of
    // the curve, and a file that mixes encrypted records with plain ones.
    let plain = marsrs_xlog::decode_records(&bytes, Some(&[0; 32])).expect("the walk went on");
    let text = String::from_utf8_lossy(&plain);

    assert!(text.contains("before\n"), "{text}");
    // The same marker a client key that is not a point gets, naming this time
    // which of the two keys it was.
    assert!(text.contains("Get ECDH key error (private key:"), "{text}");
    assert!(text.contains("after\n"), "{text}");
}

/// A file made of nothing but a record whose body will not inflate still holds
/// a record: what the walk answers is that record's marker, and not an error
/// saying no record was found.
#[test]
fn a_file_of_one_unreadable_record_is_that_marker() {
    let bytes = record(
        magic::ASYNC_NOCRYPT_ZSTD_START,
        &[0; CLIENT_PUBKEY_LEN],
        b"not a zstd frame",
    );

    let plain = marsrs_xlog::decode_records(&bytes, None).expect("a record was found");
    let text = String::from_utf8_lossy(&plain);

    assert!(text.contains("zstd decompress error"), "{text}");
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

/// A record the walk ends on still counts as read far enough to number: the C
/// reads the sequence and writes the hole in front of it before it tries the
/// body, so a record no key was given for is not a reason to lose the hole.
#[test]
fn the_record_that_ends_the_walk_is_numbered_too() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&async_record(1, b"a\n"));
    // Numbered 3, so that 2 is the hole between the two of them, and encrypted:
    // what the walk is given no private key for, and so what ends it.
    bytes.extend_from_slice(&record_with_seq(
        magic::ASYNC_ZLIB_START,
        &[0; CLIENT_PUBKEY_LEN],
        3,
        b"\x00",
    ));

    let err = marsrs_xlog::decode_records(&bytes, None).expect_err("no key was given");
    let text = String::from_utf8_lossy(&err.recovered);

    assert!(err.reason.contains("no private key"), "{err:?}");
    assert!(text.contains("a\n"), "{text}");
    assert!(text.contains("log seq:2-2 is missing"), "{text}");
}

/// A file too short to hold a modern record can still hold an old one, and a
/// byte that is no magic is not the end of it: six bytes is all the walk has
/// to leave room for when it does not know what the byte in front of it is.
#[test]
fn a_legacy_record_behind_junk_is_read_though_the_file_is_short() {
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend_from_slice(&legacy_record(b"x"));

    let plain = marsrs_xlog::decode_records(&bytes, None).expect("a record was found");
    let text = String::from_utf8_lossy(&plain);

    assert!(
        text.contains("[F]decode_log_file.py decode error len=2\n"),
        "{text}"
    );
    assert!(text.ends_with('x'), "{text}");
}

/// The two reasons a walk of a real `.xlog` can end on — a record the file ends
/// in the middle of, and a tailer that is no longer the end — asserted against
/// a record built here and not against one the appender wrote: a header carries
/// the hour it was written at, and an hour of 1 to 13 is a byte `getLogStartPos`
/// takes for the start of a record, so what a file of the appender's own reports
/// hangs on the clock. The oldest shape holds a magic and a length and nothing
/// else, so nothing in it moves.
#[test]
fn a_record_the_file_ends_in_the_middle_of_says_where_it_was_cut() {
    let mut bytes = legacy_record(b"hello");
    bytes.truncate(bytes.len() - 2);

    let err = marsrs_xlog::decode_records(&bytes, None).expect_err("the record is not whole");

    assert!(err.reason.contains("truncated"), "{err:?}");
    assert!(err.recovered.is_empty(), "{:?}", err.recovered);
}

#[test]
fn a_tailer_that_is_no_longer_the_end_of_the_record_says_so() {
    let mut bytes = legacy_record(b"hello");
    let last = bytes.len() - 1;
    bytes[last] = 0x7f;

    let err = marsrs_xlog::decode_records(&bytes, None).expect_err("the tailer is gone");

    assert!(err.reason.contains("bad tailer"), "{err:?}");
    assert!(err.recovered.is_empty(), "{:?}", err.recovered);
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

/// Damage all through a file — what a `.xlog` copied off a device by halves
/// looks like — is a resync after every span, and a resync is a scan of
/// everything behind the damage. Five hundred spans is what this asks the walk
/// for, which is the shape a resync that starts over at the damage each time
/// costs a file's length per span.
///
/// What is asserted is that the answer is unchanged by the bound: every record
/// behind every span is still read, and every span is still marked with its
/// length.
#[test]
fn damage_all_through_a_file_is_walked_in_one_pass() {
    /// Bytes no record of any shape can start in: `0xff` is no magic of any of
    /// the thirteen.
    const JUNK: [u8; 64] = [0xff; 64];

    let mut bytes = Vec::new();
    let mut expected = String::new();
    for index in 0..512 {
        bytes.extend_from_slice(&JUNK);
        let text = format!("record {index}\n");
        bytes.extend_from_slice(&sync_record(text.as_bytes()));
        expected.push_str(&format!("{DAMAGE_MARKER}{}\n", JUNK.len()));
        expected.push_str(&text);
    }

    let plain = marsrs_xlog::decode_records(&bytes, None).expect("the walk went on");
    let text = String::from_utf8_lossy(&plain);

    assert_eq!(text, expected);
}

/// `decodeBuffer`'s marker for a span it skipped, which the test above spells
/// out itself: the string a tool grep's the decoded text for.
const DAMAGE_MARKER: &str = "[F]decode_log_file.py decode error len=";
