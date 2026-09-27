//! Integration tests for the two ways one prefix ends up with more than one
//! writer.
//!
//! * **Two instances in one process** — a React Native module that linked its
//!   own copy of the crate next to the Kotlin one, or the JNI and the Kotlin
//!   side both opening. From the crate's point of view these are two
//!   `XloggerAppender`s over the same directory and prefix, which is what
//!   [`mars_appender::appender_open_instance`] twice gives.
//! * **Two processes** — an Android app with a `:push` process, or a host app
//!   and its extension. The parent test re-executes this very binary to get a
//!   real second process.
//!
//! What has to hold in both is that the log is **complete**: every record
//! every writer wrote, in a file that still decodes end to end. The C++ shares
//! one `<prefix>.mmap3` between them, so each flush writes what the other
//! buffered and then clears it; the port gives each writer a cache file of its
//! own and serialises the steps that move more than one file.

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use mars_appender::{
    appender_close, appender_close_instance, appender_flush_instance, appender_oneshot_flush,
    appender_open, appender_open_instance, appender_write, appender_write_instance, AppenderMode,
    FileIoAction, LogLevel, XLogConfig, XLoggerInfo,
};
use mars_buffer::{CompressMode, LogBuffer};
use mars_crypt::{magic, LogCrypt, HEADER_LEN, TAILER_LEN};

/// How many records each writer writes: enough to pass the 4 KiB flush
/// threshold several times over, so the two writers really do interleave.
const RECORDS: usize = 256;

/// The directory the parent hands the process it spawns. Set for that process
/// only, which is what makes [`peer_process_writer`] the no-op a plain
/// `cargo test` run of this binary needs it to be.
const PEER_DIR: &str = "MARS_XLOG_PEER_DIR";

fn config(dir: &Path) -> XLogConfig {
    XLogConfig {
        // Async, which is what xlog defaults to and the only mode that keeps
        // records in the cache file at all — the file the two writers used to
        // share. Sync hands every record straight to the log file.
        mode: AppenderMode::Async,
        logdir: dir.to_path_buf(),
        nameprefix: "Mars".to_owned(),
        pub_key: String::new(),
        compress_mode: CompressMode::Zlib,
        compress_level: 6,
        cachedir: None,
        cache_days: 0,
    }
}

fn info(level: LogLevel) -> XLoggerInfo<'static> {
    XLoggerInfo {
        level,
        tag: Some("cross".into()),
        filename: Some("cross_writer.rs".into()),
        ..Default::default()
    }
}

/// Splits a `.xlog` file into `[header][payload][tailer]` records, asserting
/// the framing of every one of them — which is the completeness claim: a file
/// two writers interleaved is still a sequence of whole records.
fn records(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + HEADER_LEN + TAILER_LEN <= bytes.len() {
        assert!(
            magic::magic_start_is_valid(bytes[pos]),
            "bad magic {:#04x} at {pos}",
            bytes[pos]
        );
        let len = LogCrypt::get_log_len(&bytes[pos..]) as usize;
        let end = pos + HEADER_LEN + len;
        assert!(end + TAILER_LEN <= bytes.len(), "truncated record at {pos}");
        assert_eq!(bytes[end], magic::END, "bad tailer at {end}");
        out.push(&bytes[pos + HEADER_LEN..end]);
        pos = end + TAILER_LEN;
    }
    assert_eq!(pos, bytes.len(), "{} trailing bytes", bytes.len() - pos);
    out
}

/// Raw payloads plus, when a payload is a DEFLATE stream, its inflated form.
fn payload_text(bytes: &[u8]) -> String {
    let mut text = String::new();
    for body in records(bytes) {
        text.push_str(&String::from_utf8_lossy(body));
        let mut out = Vec::new();
        let mut decoder = flate2::bufread::DeflateDecoder::new(std::io::Cursor::new(body));
        // A `Z_SYNC_FLUSH` stream is never terminated, so `read_to_end`
        // reports `UnexpectedEof` after emitting everything it could.
        let _ = decoder.read_to_end(&mut out);
        if !out.is_empty() {
            text.push('\n');
            text.push_str(&String::from_utf8_lossy(&out));
        }
    }
    text
}

fn today_log_file(dir: &Path) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d");
    dir.join(format!("Mars_{stamp}.xlog"))
}

/// Waits for `file` to appear, so the two writers really run at the same time
/// rather than one after the other.
fn wait_for(path: &Path, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!("timed out waiting for {what}: {}", path.display());
}

/// Two `XloggerAppender`s over one directory and prefix — the JNI and the
/// Kotlin side, or a React Native module with its own copy of the crate.
///
/// Each must claim a cache file of its own, and every record of both must reach
/// the log.
#[test]
fn two_instances_of_one_prefix_keep_every_record() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("log");
    let cfg = config(&log);

    let native = appender_open_instance(cfg.clone()).unwrap();
    let kotlin = appender_open_instance(cfg.clone()).unwrap();
    assert_ne!(native, kotlin);
    // A cache file each: the shared `<prefix>.mmap3` is the bug being fixed.
    assert!(log.join("Mars.mmap3").exists());
    assert!(
        log.join("Mars_1.mmap3").exists(),
        "the second instance mmapped the first one's cache file"
    );

    for i in 0..RECORDS {
        assert!(appender_write_instance(
            native,
            Some(&info(LogLevel::Info)),
            &format!("native-{i:04}")
        ));
        assert!(appender_write_instance(
            kotlin,
            Some(&info(LogLevel::Info)),
            &format!("kotlin-{i:04}")
        ));
    }
    appender_flush_instance(native, true);
    appender_flush_instance(kotlin, true);
    appender_close_instance(native);
    appender_close_instance(kotlin);

    let text = payload_text(&fs::read(today_log_file(&log)).unwrap());
    for i in 0..RECORDS {
        assert!(text.contains(&format!("native-{i:04}")), "{text}");
        assert!(text.contains(&format!("kotlin-{i:04}")), "{text}");
    }
}

/// Two writers of one prefix and one log directory, each with a cache
/// directory of its own.
///
/// What the two share is the log file, so the lock that serialises them has to
/// be the log's: next to each writer's own cache it would be a different file
/// for each of them, and would exclude nobody — a direct flush could then land
/// between the chunks of the other's cache-file move.
#[test]
fn two_cache_directories_share_the_log_directories_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("log");
    let mut cfg = config(&log);
    cfg.cachedir = Some(tmp.path().join("cache-a"));

    let first = appender_open_instance(cfg.clone()).unwrap();
    cfg.cachedir = Some(tmp.path().join("cache-b"));
    let second = appender_open_instance(cfg).unwrap();

    let entries: Vec<_> = fs::read_dir(&log)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        log.join("Mars.lock").exists(),
        "the shared-log lock is not in the log directory: {entries:?}"
    );

    for i in 0..RECORDS {
        assert!(appender_write_instance(
            first,
            Some(&info(LogLevel::Info)),
            &format!("cache-a-{i:04}")
        ));
        assert!(appender_write_instance(
            second,
            Some(&info(LogLevel::Info)),
            &format!("cache-b-{i:04}")
        ));
    }
    appender_flush_instance(first, true);
    appender_flush_instance(second, true);
    appender_close_instance(first);
    appender_close_instance(second);

    let text = payload_text(&fs::read(today_log_file(&log)).unwrap());
    for i in 0..RECORDS {
        assert!(text.contains(&format!("cache-a-{i:04}")), "{text}");
        assert!(text.contains(&format!("cache-b-{i:04}")), "{text}");
    }
}

/// One slot that recovers and one that cannot be read: the failure is what the
/// caller has to see, or a half-recovered cache looks like a clean one.
#[test]
fn a_slot_that_does_not_recover_is_reported_with_the_one_that_did() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("log");
    let cfg = config(&log);
    fs::create_dir_all(&log).unwrap();

    // Slot 0, the way a process that died leaves it: one record in a whole
    // region, and a lock nobody holds.
    let recovered = log.join("Mars.mmap3");
    let mut region = vec![0u8; 150 * 1024];
    let mut buffer = LogBuffer::new(true, None, CompressMode::Zlib, 6);
    buffer.attach(&mut region);
    assert!(buffer.write(&mut region, b"the slot that recovers"));
    fs::write(&recovered, &region).unwrap();

    // Slot 2, unreadable: shorter than a region, which is what a truncated or
    // half-written cache file looks like.
    let unreadable = log.join("Mars_2.mmap3");
    fs::write(&unreadable, b"not a whole cache file").unwrap();

    assert_eq!(
        appender_oneshot_flush(&cfg),
        FileIoAction::ReadFailed,
        "the first slot's success must not hide the second's failure"
    );
    assert!(!recovered.exists(), "the slot that recovered must be gone");
    assert!(unreadable.exists(), "the slot that failed must be kept");

    let text = payload_text(&fs::read(today_log_file(&log)).unwrap());
    assert!(text.contains("the slot that recovers"), "{text}");
}

/// The body [`two_processes_keep_every_record`] runs in a second process: open
/// the same prefix in the same directory and write.
///
/// It is a `#[test]` because re-executing this binary and filtering is the only
/// way to get a real second process; without [`PEER_DIR`] it does nothing.
#[test]
fn peer_process_writer() {
    let Ok(dir) = env::var(PEER_DIR) else {
        return;
    };
    let dir = PathBuf::from(dir);
    let log = dir.join("log");

    appender_open(config(&log)).unwrap();
    // Tell the parent the appender is open, then wait for it to catch up, so
    // the two write sets overlap instead of one finishing first.
    fs::write(dir.join("ready"), b"1").unwrap();
    wait_for(&dir.join("go"), "the parent to start writing");

    for i in 0..RECORDS {
        assert!(appender_write(
            Some(&info(LogLevel::Info)),
            &format!("peer-{i:04}")
        ));
    }
    appender_close();
}

/// Two processes writing one prefix: an app with a `:push` process, or a host
/// and its extension.
#[test]
fn two_processes_keep_every_record() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("log");

    let mut peer = Command::new(env::current_exe().unwrap())
        .args(["--exact", "peer_process_writer"])
        .env(PEER_DIR, tmp.path())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawning the peer process");

    wait_for(&tmp.path().join("ready"), "the peer to open its appender");
    appender_open(config(&log)).unwrap();
    fs::write(tmp.path().join("go"), b"1").unwrap();

    for i in 0..RECORDS {
        assert!(appender_write(
            Some(&info(LogLevel::Info)),
            &format!("local-{i:04}")
        ));
    }
    appender_close();

    let status = peer.wait().expect("waiting for the peer process");
    assert!(status.success(), "the peer process failed: {status}");

    let text = payload_text(&fs::read(today_log_file(&log)).unwrap());
    for i in 0..RECORDS {
        assert!(text.contains(&format!("peer-{i:04}")), "{text}");
        assert!(text.contains(&format!("local-{i:04}")), "{text}");
    }
}

/// One-shot recovery must drain the cache file a process that died left behind
/// and leave the one a live writer is using alone: reading and unlinking that
/// one loses everything the writer buffers afterwards.
#[test]
fn one_shot_flush_leaves_a_live_cache_slot_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("log");
    let cfg = config(&log);

    let live = appender_open_instance(cfg.clone()).unwrap();
    let live_slot = log.join("Mars.mmap3");
    assert!(live_slot.exists(), "the instance claimed no cache file");

    // Nothing of a *live* writer's is there to drain.
    assert_eq!(appender_oneshot_flush(&cfg), FileIoAction::Unnecessary);

    // A cache file the way a process that was killed leaves it: a full region,
    // one record in it, and no lock anybody holds.
    let dead_slot = log.join("Mars_1.mmap3");
    let mut region = vec![0u8; 150 * 1024];
    let mut buffer = LogBuffer::new(true, None, CompressMode::Zlib, 6);
    buffer.attach(&mut region);
    assert!(buffer.write(&mut region, b"left behind by a dead process"));
    fs::write(&dead_slot, &region).unwrap();

    assert_eq!(appender_oneshot_flush(&cfg), FileIoAction::Success);
    assert!(
        !dead_slot.exists(),
        "the dead slot must be drained and gone"
    );
    assert!(live_slot.exists(), "the live slot must survive");

    let text = payload_text(&fs::read(today_log_file(&log)).unwrap());
    assert!(text.contains("left behind by a dead process"), "{text}");
    appender_close_instance(live);
}
