//! Two processes, one prefix, one log file: the case the cache slots exist
//! for, and the one the port's own module doc promises it gets right where
//! the C++ does not.
//!
//! What is under test is that two live writers of one prefix are given two
//! cache files, and that every record of both reaches the log. The first is
//! the property that breaks: `sys::lock_excludes` answering `false` makes
//! `claim_cache_slot` fall back to the C++'s single fixed `<prefix>.mmap3`,
//! which both writers then write through with their own idea of its length —
//! records lost, and a log that no longer frames end to end.
//!
//! The count is taken while both writers are still open, and that is the
//! whole trick: a slot is released when its writer closes, so two writers
//! that open one after the other claim the same file quite legitimately.
//!
//! `tests/cross_writer.rs` used to cover this and went when the process-wide
//! appender did, so it is back — over handles, and with the writers in two
//! processes rather than two threads, because a shared cache region is a
//! cross-process hazard and not a threading one.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use marsrs_appender::{AppenderMode, LogLevel, XLogConfig, Xlog};

/// Set in the children, never in the parent: what turns the test below into
/// the writer it is asked to be instead of the process that spawns two.
const ROLE: &str = "MARSRS_CROSS_PROCESS_ROLE";

/// Where the two writers open their appenders, and where the parent reads the
/// log back.
const DIR: &str = "MARSRS_CROSS_PROCESS_DIR";

/// How many records each writer writes.
const RECORDS: usize = 200;

/// How long the parent waits for a child to say it is open. A child that
/// never answers cannot hang the suite: a test that can hang is worse than
/// one that fails.
const WAIT: Duration = Duration::from_secs(60);

#[test]
fn two_processes_keep_every_record() {
    if let Ok(role) = std::env::var(ROLE) {
        let dir = std::env::var(DIR).expect("the child was given no directory");
        return write_as(&role, PathBuf::from(dir));
    }

    let dir = std::env::temp_dir().join(format!("marsrs-cross-process-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the shared directory");

    let exe = std::env::current_exe().expect("the test binary's own path");
    let mut children: Vec<Child> = ["a", "b"]
        .iter()
        .map(|role| {
            Command::new(&exe)
                .args([
                    "--exact",
                    "two_processes_keep_every_record",
                    "--test-threads=1",
                ])
                .env(ROLE, role)
                .env(DIR, &dir)
                .spawn()
                .expect("a child process")
        })
        .collect();

    // Both open at once, which is the only moment the question is asked: a
    // writer that opens while the other is not inside a locked section gets
    // the right answer by luck and not by anything this test can rely on.
    std::fs::write(dir.join("go"), b"").expect("the start signal");

    let result = std::panic::catch_unwind(|| {
        wait_for(&dir, "open");
        count(&dir)
    });
    // Whatever the answer was, the children are told to stop before the
    // failure travels: a child left waiting for a signal that never comes is
    // a process the suite leaves behind.
    std::fs::write(dir.join("stop"), b"").expect("the stop signal");
    if let Err(payload) = result {
        for child in children.iter_mut() {
            let _ = child.wait();
        }
        std::panic::resume_unwind(payload);
    }
    wait_for(&dir, "done");
    for child in children.iter_mut() {
        let status = child.wait().expect("a child to finish");
        assert!(status.success(), "a child writer failed: {status}");
    }

    let text = log_text(&dir);
    for role in ["a", "b"] {
        for index in 0..RECORDS {
            let needle = format!("{role}-{index:04}");
            assert!(
                text.contains(&needle),
                "'{needle}' is not in the log the two writers share — {role} lost {} of \
                 {RECORDS} records",
                text.matches(&format!("{role}-")).count()
            );
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// The pin: two live writers, two cache files. One means the two were handed
/// the same slot, which is the C++'s bug and not this port's.
fn count(dir: &Path) {
    let slots: Vec<_> = std::fs::read_dir(dir)
        .expect("the shared directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".mmap3"))
        .collect();
    assert_eq!(
        slots.len(),
        2,
        "two live writers of one prefix claimed {slots:?} — one cache file between them \
         means `lock_excludes` read a peer's lock as a filesystem that cannot lock"
    );
}

/// Waits until every role has written its `<what>-<role>` marker. Panics if
/// they never do, and never waits longer than [`WAIT`].
fn wait_for(dir: &Path, what: &str) {
    let started = Instant::now();
    for role in ["a", "b"] {
        let marker = dir.join(format!("{what}-{role}"));
        while !marker.exists() {
            assert!(
                started.elapsed() < WAIT,
                "the {role} writer never said `{what}`: {}",
                dir.display()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

/// One writer: waits for the signal, opens the shared prefix, writes, drains,
/// says it is open, and stays open until it is told to stop.
///
/// Sync mode, because a record handed to the file in the call it was written
/// in is the one this test can read back out of the file. The cache file is
/// claimed at `open` whatever the mode is, which is why the parent can count
/// it here.
fn write_as(role: &str, dir: PathBuf) {
    let go = dir.join("go");
    let started = Instant::now();
    while !go.exists() && started.elapsed() < WAIT {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(go.exists(), "the parent never said `go`: {}", dir.display());

    let config = XLogConfig {
        mode: AppenderMode::Sync,
        logdir: dir.clone(),
        nameprefix: "shared".to_owned(),
        pub_key: String::new(),
        compress_mode: marsrs_buffer::CompressMode::Zlib,
        compress_level: 6,
        cachedir: None,
        cache_days: 0,
    };
    let xlog = Xlog::open(config, LogLevel::Info).expect("the shared appender");
    for index in 0..RECORDS {
        assert!(
            xlog.i("cross", &format!("{role}-{index:04}")),
            "the record {role}-{index:04} was not written"
        );
    }
    xlog.flush_now();
    std::fs::write(dir.join(format!("open-{role}")), b"").expect("the marker");

    // Held open while the parent counts the cache files: a writer that closed
    // already released its slot, and the count would mean nothing.
    //
    // Bounded, because the parent may never reach the signal — the count is
    // the assertion, and a failing one leaves no `stop` behind. A child that
    // waits for it without a bound outlives the suite.
    let stop = dir.join("stop");
    let started = Instant::now();
    while !stop.exists() && started.elapsed() < WAIT {
        std::thread::sleep(Duration::from_millis(2));
    }
    xlog.close();
    std::fs::write(dir.join(format!("done-{role}")), b"").expect("the marker");
}

/// Every `.xlog` of the directory as one string. Sync mode stores the payload
/// verbatim, so a record is found the way the appender's own tests find one.
fn log_text(dir: &Path) -> String {
    let mut text = String::new();
    for entry in std::fs::read_dir(dir).expect("the shared directory") {
        let path = entry.expect("a directory entry").path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("xlog") {
            text.push_str(&String::from_utf8_lossy(
                &std::fs::read(&path).expect("the log file"),
            ));
        }
    }
    text
}
