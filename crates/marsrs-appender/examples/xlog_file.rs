//! Writes one record per line of a file through the real appender, into a log
//! directory, and prints the `.xlog` it left — the Rust half of the end-to-end
//! cross-read test in `scripts/compat/cross.sh`.
//!
//! ```text
//! cargo run -p marsrs-appender --example xlog_file -- \
//!     [--mode=zlib|zstd] [--sync=1|0] <logdir> <records> [pubkey]
//! ```
//!
//! `--mode` and `--sync` are the two switches the C++ `XLogConfig` carries, so
//! the appender matrix in `scripts/compat/cross.sh` builds the same shape on
//! both sides: without them every row would open the appender this file's
//! defaults name, and the rows that ask for zstd or async would only re-run
//! sync/zlib.
//!
//! The file that comes out is what an app would ship to a server: one
//! `Xlog` of the port's own over its mmap'd cache file, with a
//! `<prefix>_YYYYMMDD.xlog` at the other end. `scripts/compat/cross.sh` hands it
//! to upstream's own decoder.

use std::path::PathBuf;

use marsrs_appender::{AppenderMode, CompressMode, LogLevel, XLogConfig, Xlog};

fn main() {
    // The flags first: `--mode` picks `CompressMode`, `--sync` `AppenderMode`.
    // Anything that is not a flag is positional, in the documented order.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut mode = "zlib";
    let mut sync = "1";
    let mut positional: Vec<&str> = Vec::new();
    for arg in &args {
        if let Some(value) = arg.strip_prefix("--mode=") {
            mode = value;
        } else if let Some(value) = arg.strip_prefix("--sync=") {
            sync = value;
        } else {
            positional.push(arg);
        }
    }

    let mut positional = positional.into_iter();
    let (Some(dir), Some(records_path)) = (positional.next(), positional.next()) else {
        eprintln!("usage: xlog_file [--mode=zlib|zstd] [--sync=1|0] <logdir> <records> [pubkey]");
        std::process::exit(2);
    };
    // Empty is "no server key", i.e. the no-crypt magics.
    let pub_key = positional.next().unwrap_or_default().to_owned();

    let compress_mode = match mode {
        "zlib" => CompressMode::Zlib,
        "zstd" => CompressMode::Zstd,
        other => {
            eprintln!("--mode must be zlib or zstd, got `{other}`");
            std::process::exit(2);
        }
    };
    let appender_mode = match sync {
        "1" | "true" => AppenderMode::Sync,
        "0" | "false" => AppenderMode::Async,
        other => {
            eprintln!("--sync must be 0 or 1, got `{other}`");
            std::process::exit(2);
        }
    };

    let text = match std::fs::read(records_path) {
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
        mode: appender_mode,
        logdir: PathBuf::from(dir),
        nameprefix: "cross".to_owned(),
        pub_key,
        compress_mode,
        compress_level: 6,
        cachedir: None,
        cache_days: 0,
    };
    let xlog = Xlog::open(config, LogLevel::Info).expect("Xlog::open");

    // One `Xlog`, and not the process-wide appender: an appender of an app's
    // own is what the port opens now, so what the two halves of `cross.sh`
    // compare is what one appender of each writes.
    for record in &records {
        xlog.i("cross", record);
    }

    xlog.flush_now();

    let mut found = 0;
    for entry in std::fs::read_dir(dir).expect("read_dir") {
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
