//! The failure paths of the compat library: a bad command line, a broken
//! `.xlog`, and a record the region is too small for.

use std::collections::HashMap;

use marsrs_compat::{count_blocks, decode_records, encode, read_records, Opts};

fn opts(pairs: &[(&str, &str)]) -> Opts {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<HashMap<_, _>>()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("marsrs-compat-err-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn encode_refuses_a_broken_command_line() {
    // missing --mode
    let error = encode(&opts(&[("records", "/dev/null")])).unwrap_err();
    assert!(error.contains("mode"), "{error}");
    // unknown mode
    let error = encode(&opts(&[("mode", "lz4"), ("records", "/dev/null")])).unwrap_err();
    assert!(error.contains("zlib or zstd"), "{error}");
    // --compress has to be 0 or 1
    let error = encode(&opts(&[
        ("mode", "zlib"),
        ("compress", "maybe"),
        ("records", "/dev/null"),
    ]))
    .unwrap_err();
    assert!(error.contains("must be 0 or 1"), "{error}");
    // --region has to be a number
    let error = encode(&opts(&[
        ("mode", "zlib"),
        ("region", "huge"),
        ("records", "/dev/null"),
    ]))
    .unwrap_err();
    assert!(error.contains("region"), "{error}");
}

#[test]
fn read_records_refuses_an_empty_file() {
    let dir = scratch("empty");
    let path = dir.join("records.bin");
    std::fs::write(&path, b"").unwrap();
    let error = read_records(path.to_str().unwrap()).unwrap_err();
    assert!(error.contains("no records"), "{error}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn encode_reports_a_record_that_does_not_fit() {
    let dir = scratch("toobig");
    let records = dir.join("records.bin");
    // pseudorandom so that compression cannot save it
    let payload: Vec<u8> = (0..4096).map(|i| (i * 31 + 7) as u8).collect();
    std::fs::write(&records, &payload).unwrap();
    let out = dir.join("out.xlog");

    let error = encode(&opts(&[
        ("mode", "zlib"),
        ("sync", "0"),
        // smaller than the record
        ("region", "64"),
        ("records", records.to_str().unwrap()),
        ("out", out.to_str().unwrap()),
    ]))
    .unwrap_err();
    assert!(error.contains("did not fit"), "{error}");
    std::fs::remove_dir_all(&dir).ok();
}

/// Encodes one record so the damage tests start from a real `.xlog`.
fn encoded(dir: &std::path::Path, name: &str) -> Vec<u8> {
    encoded_blocks(dir, name, 1)
}

#[test]
fn count_blocks_reports_every_kind_of_damage() {
    let dir = scratch("blocks");

    // bad magic
    let error = count_blocks(&[0u8; 128]).unwrap_err();
    assert!(error.contains("bad magic"), "{error}");

    // a real file whose length field claims more than the file holds
    let mut bytes = encoded(&dir, "a.xlog");
    bytes[5..9].copy_from_slice(&0x00ff_ffffu32.to_le_bytes());
    let error = count_blocks(&bytes).unwrap_err();
    assert!(error.contains("truncated"), "{error}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn decode_records_reports_every_kind_of_damage() {
    let dir = scratch("decode");
    let key = [0u8; 32];

    // nothing at all
    let error = decode_records(&[], Some(&key)).unwrap_err().reason;
    assert!(error.contains("no record"), "{error}");
    // bad magic
    let error = decode_records(&[0u8; 128], Some(&key)).unwrap_err().reason;
    assert!(error.contains("bad magic"), "{error}");

    // a real file cut short
    let mut bytes = encoded(&dir, "a.xlog");
    bytes.truncate(bytes.len() - 1);
    let error = decode_records(&bytes, Some(&key)).unwrap_err().reason;
    assert!(error.contains("truncated"), "{error}");

    // a real file whose tailer was overwritten
    let mut bytes = encoded(&dir, "b.xlog");
    let last = bytes.len() - 1;
    bytes[last] = 0x7f;
    let error = decode_records(&bytes, Some(&key)).unwrap_err().reason;
    assert!(error.contains("bad tailer"), "{error}");
    std::fs::remove_dir_all(&dir).ok();
}

/// `count` copies of one record, encoded in sync mode: a file of `count` blocks
/// of the same size, so where a block starts is arithmetic and not a guess.
///
/// 64 bytes and not 256, because what the walk makes of a broken block depends
/// on the bytes of its own header: `getLogStartPos` accepts any byte from
/// `MAGIC_CRYPT_START` to the last magic as a record start, and a length of 256
/// is `00 01 00 00` — so the walk resyncs into the block's own length field and
/// reports where it comes out, and not the damage it started from. `64` is
/// `40 00 00 00`, and no byte of the block behind that header is one either.
fn encoded_blocks(dir: &std::path::Path, name: &str, count: usize) -> Vec<u8> {
    let records = dir.join("records.bin");
    // long enough that a header + tailer always fit in the encoded file
    let record: Vec<u8> = std::iter::repeat_n(b'x', 64)
        .chain(b"\n".iter().copied())
        .collect();
    std::fs::write(&records, record.repeat(count)).unwrap();
    let out = dir.join(name);
    encode(&opts(&[
        ("mode", "zlib"),
        ("sync", "1"),
        ("records", records.to_str().unwrap()),
        ("out", out.to_str().unwrap()),
    ]))
    .unwrap();
    let mut bytes = std::fs::read(&out).unwrap();

    // Both hours of every header are written over with 0. An hour of 1 to 13 is
    // a byte `getLogStartPos` takes for the start of a record, so a file the
    // encoder wrote at such an hour carries a resync point in its own header:
    // the walk lands there and reports where it came out, and not the damage
    // these tests put in — which damage a run sees hung on the clock. `0` is no
    // magic of any kind, and it is the hour `normalize_for_compare` writes over
    // both of them with already.
    let stride = bytes.len() / count;
    for block in (0..bytes.len()).step_by(stride) {
        bytes[block + 3] = 0;
        bytes[block + 4] = 0;
    }
    bytes
}

#[test]
fn a_file_cut_short_keeps_the_records_before_the_cut() {
    let dir = scratch("cut-short");
    let key = [0u8; 32];

    let bytes = encoded_blocks(&dir, "a.xlog", 3);
    let stride = bytes.len() / 3;
    // Inside the third block, past its header: what a write that was cut off
    // halfway leaves in the file, and far enough into it that the header is
    // whole — a block cut off before its length would not be a block at all.
    let mut cut = bytes;
    cut.truncate(2 * stride + 100);

    // The damage is reported, and the two whole blocks before it are handed
    // back with it: `parseFile` writes the output it has either way, and the
    // records in front of a broken one are not less true for what follows.
    let error = decode_records(&cut, Some(&key)).expect_err("a file with a broken tail decoded");
    assert!(error.reason.contains("truncated"), "{}", error.reason);
    // Two of the three records, and not a byte of the third: `read_records`
    // hands the encoder one line per record, newline excluded, so that line is
    // what one whole block decodes to.
    let records = std::fs::read(dir.join("records.bin")).expect("read the records");
    let line: Vec<u8> = records[..records.len() / 3]
        .iter()
        .copied()
        .filter(|byte| *byte != b'\n')
        .collect();
    assert_eq!(error.recovered, line.repeat(2));
    std::fs::remove_dir_all(&dir).ok();
}
