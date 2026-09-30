//! Two processes, one prefix, one log file: the case the cache slots exist
//! for, and the one the port's own module doc promises it gets right where
//! the C++ does not.
//!
//! What is under test is that every record of every writer reaches the log.
//! That is what stops being true the moment two writers are handed the same
//! cache file — and the moment `sys::lock_excludes` answers `false` because a
//! peer happened to be holding the log lock, that is exactly what happens:
//! `claim_cache_slot` falls back to the C++'s single fixed `<prefix>.mmap3`,
//! which two writers then write through with their own idea of its length.
//!
//! `tests/cross_writer.rs` used to pin this and went when the process-wide
//! appender did, so it is back — over handles, and with the two writers in
//! two processes rather than two threads, because a shared cache region is a
//! cross-process hazard and not a threading one.

use std::path::PathBuf;
use std::process::Command;

use marsrs_appender::{AppenderMode, LogLevel, XLogConfig, Xlog};

/// Set in the children, never in the parent: what turns the test below into
/// the writer it is asked to be instead of the process that spawns two.
const ROLE: &str = "MARSRS_CROSS_PROCESS_ROLE";

/// Where the two writers open their appenders, and where the parent reads the
/// log back.
const DIR: &str = "MARSRS_CROSS_PROCESS_DIR";

/// How many records each writer writes.
const RECORDS: usize = 200;

#[test]
fn two_processes_keep_every_record() {
    if let Ok(role) = std::env::var(ROLE) {
        return write_as(
            &role,
            &std::env::var(DIR).expect("the child was given no directory"),
        );
    }

    let dir = std::env::temp_dir().join(format!("marsrs-cross-process-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the shared directory");

    // Files for the sweep at `open` to walk, so the section a writer holds the
    // log lock for is long enough for the other one to open inside it: the
    // question this test asks is only asked at that moment, and on an empty
    // directory the window is too short to hit.
    for day in 0usize..400 {
        let month = day / 31 + 1;
        let in_month = day % 31 + 1;
        std::fs::write(
            dir.join(format!("shared_2025{month:02}{in_month:02}.xlog")),
            b"",
        )
        .expect("a file for the sweep");
    }

    let exe = std::env::current_exe().expect("the test binary's own path");
    let mut children: Vec<_> = ["a", "b"]
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

/// One writer: waits for the signal, opens the shared prefix, writes, drains,
/// closes. Sync mode, because a record handed to the file in the call it was
/// written in is the one this test can read back out of the file.
fn write_as(role: &str, dir: &str) {
    let dir = PathBuf::from(dir);
    while !dir.join("go").exists() {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

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
    xlog.close();
}

/// Every `.xlog` of the directory as one string. Sync mode stores the payload
/// verbatim, so a record is found the way the appender's own tests find one.
fn log_text(dir: &std::path::Path) -> String {
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
