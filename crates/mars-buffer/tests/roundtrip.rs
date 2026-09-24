//! End-to-end tests for the public `mars-buffer` API.
//!
//! They exercise the real production shape of the data: records written into a
//! 4 KiB "mmap" region, flushed into a `.xlog` file, and finally scanned back
//! with [`get_period_logs`].

use std::path::Path;

use mars_buffer::{get_period_logs, get_period_logs_with_timeout_ms, CompressMode, LogBuffer};
use mars_core::{local_hour, AutoBuffer};
use mars_crypt::{magic, LogCrypt, HEADER_LEN, TAILER_LEN};

/// Size of the fake mmap'd cache region.
const REGION_LEN: usize = 4 * 1024;

/// Writes `records` into a region, flushes and returns the file bytes.
fn async_flush(mode: CompressMode, records: &[&str]) -> Vec<u8> {
    let mut region = vec![0u8; REGION_LEN];
    let mut buf = LogBuffer::new(true, None, mode, 6);

    for record in records {
        assert!(buf.write(&mut region, record.as_bytes()));
    }
    assert!(buf.len() > HEADER_LEN);
    assert!(buf.len() + TAILER_LEN <= REGION_LEN);

    let mut out = AutoBuffer::new();
    let n = buf.flush(&mut region, &mut out);
    assert_eq!(n, out.len());
    assert_eq!(buf.len(), 0);
    assert!(region.iter().all(|&b| b == 0), "region must be cleared");

    let bytes = out.as_slice().to_vec();
    assert_eq!(bytes[n - 1], magic::END);
    bytes
}

/// Writes `records` with the sync path into one file image.
fn sync_flush(mode: CompressMode, records: &[&str]) -> Vec<u8> {
    let mut buf = LogBuffer::new(true, None, mode, 6);
    let mut bytes = Vec::new();
    for record in records {
        let mut out = AutoBuffer::new();
        assert!(buf.write_sync(record.as_bytes(), &mut out));
        bytes.extend_from_slice(out.as_slice());
    }
    bytes
}

fn scan(bytes: &[u8], name: &str) -> (u64, u64) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).expect("write");
    scan_file(&path, bytes.len() as u64)
}

fn scan_file(path: &Path, file_size: u64) -> (u64, u64) {
    let (begin, end) = get_period_logs(path, 0, 24).expect("scan");
    assert!(end > begin);
    assert!(end <= file_size);
    (begin, end)
}

#[test]
fn zlib_async_region_flushes_into_a_scannable_file() {
    let bytes = async_flush(CompressMode::Zlib, &["first\n", "second\n", "third\n"]);
    assert_eq!(bytes[0], magic::ASYNC_NOCRYPT_ZLIB_START);
    assert_eq!(
        LogCrypt::get_log_len(&bytes) as usize,
        bytes.len() - HEADER_LEN - TAILER_LEN
    );

    let (begin, end) = scan(&bytes, "zlib.xlog");
    assert_eq!(begin, 0);
    assert_eq!(end, bytes.len() as u64);
}

#[test]
fn zstd_async_region_flushes_into_a_scannable_file() {
    let bytes = async_flush(CompressMode::Zstd, &["first\n", "second\n", "third\n"]);
    assert_eq!(bytes[0], magic::ASYNC_NOCRYPT_ZSTD_START);

    let (begin, end) = scan(&bytes, "zstd.xlog");
    assert_eq!(begin, 0);
    assert_eq!(end, bytes.len() as u64);
}

#[test]
fn sync_records_round_trip_for_both_modes() {
    for (mode, expected_magic) in [
        (CompressMode::Zlib, magic::SYNC_NOCRYPT_ZLIB_START),
        (CompressMode::Zstd, magic::SYNC_NOCRYPT_ZSTD_START),
    ] {
        let bytes = sync_flush(mode, &["one\n", "two\n"]);
        assert_eq!(bytes[0], expected_magic);
        // Two complete records: header + body + tailer each.
        assert_eq!(bytes.len(), 2 * (HEADER_LEN + 4 + TAILER_LEN));
        assert_eq!(bytes[bytes.len() - 1], magic::END);

        let (begin, end) = scan(&bytes, "sync.xlog");
        assert_eq!(begin, 0);
        assert_eq!(end, bytes.len() as u64);

        // The record hour comes from the header, so an hour-wide window works.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sync_hour.xlog");
        std::fs::write(&path, &bytes).expect("write");
        let hour = local_hour() as i32;
        let (begin, end) = get_period_logs(&path, hour, hour + 1).expect("scan");
        assert_eq!(begin, 0);
        assert_eq!(end, bytes.len() as u64);
    }
}

#[test]
fn scanned_range_covers_the_whole_file_after_a_resync() {
    let mut bytes = vec![0xCDu8; 5];
    bytes.extend_from_slice(&sync_flush(CompressMode::Zlib, &["one\n", "two\n"]));

    let (begin, end) = scan(&bytes, "resync.xlog");
    assert_eq!(begin, 5, "the garbage prefix must be skipped");
    assert_eq!(end, bytes.len() as u64);
}

#[test]
fn timeout_wrapper_matches_the_direct_scan() {
    let bytes = sync_flush(CompressMode::Zlib, &["one\n"]);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("timeout.xlog");
    std::fs::write(&path, &bytes).expect("write");

    let direct = get_period_logs(&path, 0, 24).expect("scan");
    let wrapped = get_period_logs_with_timeout_ms(&path, 0, 24, 5_000).expect("scan");
    assert_eq!(direct, wrapped);
    assert_eq!(wrapped, (0, bytes.len() as u64));
}

#[test]
fn attach_recovers_a_region_written_by_another_instance() {
    let mut region = vec![0u8; REGION_LEN];
    {
        let mut writer = LogBuffer::new(false, None, CompressMode::Zlib, 6);
        assert!(writer.write(&mut region, b"survives a restart\n"));
    }
    let written = region[..].to_vec();

    // A "new process" maps the same region and must recover the length.
    let mut reader = LogBuffer::new(false, None, CompressMode::Zlib, 6);
    reader.attach(&mut region);
    assert_eq!(reader.len(), HEADER_LEN + 19);
    assert_eq!(region, written, "attach must not modify the region");

    let mut out = AutoBuffer::new();
    let n = reader.flush(&mut region, &mut out);
    assert_eq!(n, HEADER_LEN + 19 + TAILER_LEN);
    assert_eq!(
        &out.as_slice()[HEADER_LEN..HEADER_LEN + 19],
        b"survives a restart\n"
    );
}
