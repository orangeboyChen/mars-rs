//! The failure paths of the compat library: a bad command line, a broken
//! `.xlog`, and a record the region is too small for.

use std::collections::HashMap;

use mars_compat::{count_blocks, decode_records, encode, read_records, Opts};

fn opts(pairs: &[(&str, &str)]) -> Opts {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<HashMap<_, _>>()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mars-compat-err-{name}-{}", std::process::id()));
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
    let records = dir.join("records.bin");
    // long enough that a header + tailer always fit in the encoded file
    let record: Vec<u8> = std::iter::repeat_n(b'x', 256)
        .chain(b"\n".iter().copied())
        .collect();
    std::fs::write(&records, &record).unwrap();
    let out = dir.join(name);
    encode(&opts(&[
        ("mode", "zlib"),
        ("sync", "1"),
        ("records", records.to_str().unwrap()),
        ("out", out.to_str().unwrap()),
    ]))
    .unwrap();
    std::fs::read(&out).unwrap()
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
    assert!(decode_records(&[], &key).unwrap_err().contains("no record"));
    // bad magic
    let error = decode_records(&[0u8; 128], &key).unwrap_err();
    assert!(error.contains("bad magic"), "{error}");

    // a real file cut short
    let mut bytes = encoded(&dir, "a.xlog");
    bytes.truncate(bytes.len() - 1);
    let error = decode_records(&bytes, &key).unwrap_err();
    assert!(error.contains("truncated"), "{error}");

    // a real file whose tailer was overwritten
    let mut bytes = encoded(&dir, "b.xlog");
    let last = bytes.len() - 1;
    bytes[last] = 0x7f;
    let error = decode_records(&bytes, &key).unwrap_err();
    assert!(error.contains("bad tailer"), "{error}");
    std::fs::remove_dir_all(&dir).ok();
}
