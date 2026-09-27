//! Writes one record per line of a file through the real appender, into a log
//! directory, and prints the `.xlog` it left — the Rust half of the end-to-end
//! cross-read test in `scripts/compat/cross.sh`.
//!
//! ```text
//! cargo run -p mars-appender --example xlog_file -- <logdir> <records> [pubkey]
//! ```
//!
//! The file that comes out is what an app would ship to a server: the
//! process-wide `XloggerAppender` over its mmap'd cache file, with a
//! `<prefix>_YYYYMMDD.xlog` at the other end. `scripts/compat/cross.sh` hands it
//! to upstream's own decoder.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mars_appender::{
    appender_close, appender_flush_sync, appender_open, appender_write, AppenderMode, CompressMode,
    LogLevel, XLogConfig, XLoggerInfo,
};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(dir), Some(records_path)) = (args.next(), args.next()) else {
        eprintln!("usage: xlog_file <logdir> <records> [pubkey]");
        std::process::exit(2);
    };
    // Empty is "no server key", i.e. the no-crypt magics.
    let pub_key = args.next().unwrap_or_default();

    let text = match std::fs::read(&records_path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(err) => {
            eprintln!("read {records_path}: {err}");
            std::process::exit(1);
        }
    };
    let records: Vec<&str> = text.lines().collect();
    if records.is_empty() {
        eprintln!("{records_path} holds no records");
        std::process::exit(1);
    }

    let config = XLogConfig {
        mode: AppenderMode::Sync,
        logdir: PathBuf::from(&dir),
        nameprefix: "cross".to_owned(),
        pub_key,
        compress_mode: CompressMode::Zlib,
        compress_level: 6,
        cachedir: None,
        cache_days: 0,
    };
    appender_open(config).expect("appender_open");

    for (index, record) in records.iter().enumerate() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        let info = XLoggerInfo {
            level: LogLevel::Info,
            tag: Some("cross".into()),
            filename: Some("xlog_file.rs".into()),
            func_name: Some("main".into()),
            line: i32::try_from(index).unwrap_or(0),
            timeval: (
                i64::try_from(now.as_secs()).unwrap_or(0),
                i64::from(now.subsec_micros()),
            ),
            ..Default::default()
        };
        appender_write(Some(&info), record);
    }

    appender_flush_sync();
    appender_close();

    let mut found = 0;
    for entry in std::fs::read_dir(&dir).expect("read_dir") {
        let path = entry.expect("read_dir entry").path();
        if path.extension().is_some_and(|ext| ext == "xlog") {
            println!("{}", path.display());
            found += 1;
        }
    }
    if found == 0 {
        eprintln!("the appender left no .xlog in {dir}");
        std::process::exit(1);
    }
}
