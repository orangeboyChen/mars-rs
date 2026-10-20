//! The reader against a file an older appender wrote.
//!
//! `LogMagicNum` in `mars/xlog/crypt/log_magic_num.h` knows eight record
//! starts, `0x06` to `0x0D`, and that is what an appender writes. What
//! `decode_log_file.c` *reads* is thirteen: the five from `0x01` to `0x05`
//! predate the client public key, so their header is 5 or 9 bytes instead of
//! 73, there is no secret to agree, and the text is XORed with a byte made of
//! the magic rather than TEA-encrypted. A log file is older than the app that
//! reads it more often than not, so these are the cases that check the reader
//! takes the five in — built by hand, because no writer in this tree makes
//! them.

use marsrs_crypt::magic;

/// One record of the oldest shape: `1 + 4` bytes of header, which is a magic
/// and the length and nothing else.
fn old_record(magic_start: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![magic_start];
    bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
    bytes.extend_from_slice(body);
    bytes.push(magic::END);
    bytes
}

/// One record of the shape that came next: `1 + 2 + 1 + 1 + 4`, which is the
/// magic, a sequence and the two hours behind it. Still no public key.
fn new_record(magic_start: u8, seq: u16, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![magic_start];
    bytes.extend_from_slice(&seq.to_le_bytes());
    bytes.push(0); // begin hour
    bytes.push(0); // end hour
    bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
    bytes.extend_from_slice(body);
    bytes.push(magic::END);
    bytes
}

/// `BASE_KEY ^ (0xff & length) ^ magic` — the key of a record whose header
/// carries no sequence, which is to say the two oldest shapes.
fn old_key(magic_start: u8, length: usize) -> u8 {
    0xcc ^ (length as u8) ^ magic_start
}

/// `BASE_KEY ^ (0xff & seq) ^ magic` — the key of one that carries a sequence.
fn seq_key(magic_start: u8, seq: u16) -> u8 {
    0xcc ^ (seq as u8) ^ magic_start
}

fn xor(bytes: &[u8], key: u8) -> Vec<u8> {
    bytes.iter().map(|byte| byte ^ key).collect()
}

/// The raw DEFLATE stream of `text`, the way `LogZlibBuffer` leaves one: no
/// zlib header, and a `Z_SYNC_FLUSH` behind it instead of a stream end.
fn deflate(text: &[u8]) -> Vec<u8> {
    use flate2::{Compress, Compression, FlushCompress};

    let mut encoder = Compress::new(Compression::default(), false);
    let mut out = vec![0u8; text.len() + text.len() / 128 + 64];
    let before = encoder.total_out();
    encoder
        .compress(text, &mut out, FlushCompress::Sync)
        .expect("the text compresses");
    let written = (encoder.total_out() - before) as usize;
    assert_eq!(
        encoder.total_in() as usize,
        text.len(),
        "the whole text went into the stream"
    );
    out.truncate(written);
    out
}

/// A record of the oldest shape holds the log text itself, XORed with a byte
/// of its own length and nothing more: `decodeBuffer` ends in its `else` for
/// it, which is the XOR and no decompress behind it.
#[test]
fn the_oldest_record_is_xored_and_nothing_more() {
    let text = b"hello\n";
    let key = old_key(0x01, text.len());
    let bytes = old_record(0x01, &xor(text, key));

    // No private key: a file written before the public key arrived has no ECDH
    // in it to undo, so there is nothing for the reader to be given.
    let plain = marsrs_xlog::decode_records(&bytes, None).expect("the record decoded");

    assert_eq!(plain, text);
}

/// One of the three that are deflated: the XOR comes off first, and what is
/// under it is a stream and not the text.
#[test]
fn a_deflated_old_record_is_inflated_after_the_xor() {
    let text = b"hello\n";
    let body = deflate(text);
    let key = old_key(0x02, body.len());
    let bytes = old_record(0x02, &xor(&body, key));

    let plain = marsrs_xlog::decode_records(&bytes, None).expect("the record decoded");

    assert_eq!(plain, text);
}

/// A record numbered before the public key arrived is numbered like any other:
/// the sequence is what the key is made of, and the hole in front of it is one
/// the reader marks.
#[test]
fn a_record_numbered_before_the_public_key_names_the_hole_in_front_of_it() {
    let mut bytes = Vec::new();
    for (seq, text) in [(1, b"a\n"), (3, b"b\n")] {
        bytes.extend_from_slice(&new_record(0x03, seq, &xor(text, seq_key(0x03, seq))));
    }

    let plain = marsrs_xlog::decode_records(&bytes, None).expect("the walk went on");
    let text = String::from_utf8_lossy(&plain);

    assert_eq!(text, "a\n[F]decode_log_file.py log seq:2-2 is missing\nb\n");
}

/// `NEW_MAGIC_COMPRESS_CRYPT_START1` files one chunk per log line: a
/// `uint16_t` length and its bytes, and the chunk lengths are not part of what
/// the key goes over nor of what is inflated.
#[test]
fn the_chunks_of_a_chunked_record_are_put_back_to_back_first() {
    let text = b"a\nb\nc\n";
    // One stream split over three chunks, so that no chunk carries a whole
    // line and reassembly is the only way to read it.
    let stream = xor(&deflate(text), seq_key(0x05, 7));
    let mut body = Vec::new();
    for chunk in stream.chunks(3) {
        body.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        body.extend_from_slice(chunk);
    }
    let bytes = new_record(0x05, 7, &body);

    let plain = marsrs_xlog::decode_records(&bytes, None).expect("the record decoded");

    assert_eq!(plain, text);
}

/// A chunk whose length runs past the body ends the chunk walk: the C
/// `memcpy`s those bytes out of the buffer all the same and lands in the same
/// place, because its `readPos += singleLogLen + 2` leaves the loop either way.
#[test]
fn a_chunk_that_runs_past_the_body_ends_the_chunk_walk() {
    let mut body = Vec::new();
    body.extend_from_slice(&(2u16).to_le_bytes());
    body.extend_from_slice(b"ab");
    // A second chunk asking for more than the body holds.
    body.extend_from_slice(&(64u16).to_le_bytes());

    let mut bytes = new_record(0x05, 0, &body);
    bytes.extend_from_slice(&sync_record(b"after\n"));

    let plain = marsrs_xlog::decode_records(&bytes, None).expect("the walk went on");
    let text = String::from_utf8_lossy(&plain);

    // The chunk that runs past the body is the last one read, so what is left
    // of the record is not a stream and it contributes no text — and the record
    // behind it is what says the walk went on, which is the point of it. The C
    // exits the whole process on a stream that will not inflate; this does not.
    // "ab" is not in it: the record the walk gave up on contributes no text
    // at all, and not the half of a chunk it read.
    assert_eq!(text, "after\n", "{text}");
}

/// One sync record of the shape an appender writes now, for a file that mixes
/// the two eras the way a file written across an upgrade does.
fn sync_record(text: &[u8]) -> Vec<u8> {
    use marsrs_crypt::CLIENT_PUBKEY_LEN;

    let mut bytes = vec![magic::SYNC_NOCRYPT_ZLIB_START];
    bytes.extend_from_slice(&0u16.to_le_bytes()); // seq
    bytes.push(0); // begin hour
    bytes.push(0); // end hour
    bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&[0; CLIENT_PUBKEY_LEN]);
    bytes.extend_from_slice(text);
    bytes.push(magic::END);
    bytes
}

/// The scan that finds the log again after damage is over the same thirteen
/// magics as the walk, so a span of damage in front of an old record ends at
/// it and not at the end of the file.
#[test]
fn the_walk_goes_on_at_an_old_record_behind_the_damage() {
    let mut bytes = sync_record(b"first\n");

    // A span no record can be read out of, long enough that the walk cannot
    // take it for the tail of the file.
    bytes.extend_from_slice(&[0xff; 80]);

    let text = b"second\n";
    bytes.extend_from_slice(&old_record(0x01, &xor(text, old_key(0x01, text.len()))));

    let plain = marsrs_xlog::decode_records(&bytes, None).expect("the walk went on");
    let decoded = String::from_utf8_lossy(&plain);

    assert!(decoded.contains("first\n"), "{decoded}");
    assert!(decoded.contains("second\n"), "{decoded}");
    assert!(
        decoded.contains("[F]decode_log_file.py decode error len="),
        "the skipped span is not marked: {decoded}"
    );
}
