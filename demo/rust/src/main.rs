//! The Rust demo of mars-rs.
//!
//! One program that does what an app does with the logger, in the order an app
//! does it:
//!
//! 1. describe where the files go and how they are written — [`XLogConfig`];
//! 2. open the appender — [`Xlog::open`];
//! 3. write one record at every level — [`Xlog::log`];
//! 4. drain it to disk — [`Xlog::flush_now`];
//! 5. read the file back — [`decode_log_file`];
//! 6. close it — [`Xlog::close`].
//!
//! Run it with `cargo run` from this directory, or `cargo run -- <dir>` to
//! choose where the `.xlog` files land. They are the same files the C++
//! implementation writes, so `xlog decode` of the port's own CLI — the binary
//! of `marsrs-xlog`, `cargo install marsrs-xlog` — reads them too.
//!
//! Two things this program deliberately does *not* show:
//!
//! - **A second writer over one prefix.** [`Xlog::open_unregistered`] opens an
//!   appender no prefix is registered for, which is what a second copy of the
//!   library in one process needs — the JNI and the Kotlin side both opening,
//!   say. This demo is one appender, because one is what an app that only logs
//!   needs.
//! - **Encryption.** `XLogConfig::pub_key` takes the 128 hex characters of a
//!   public key, and an appender opened with one writes records only the
//!   matching private key decrypts. The key pair is `xlog keygen`, and the
//!   reader's side of it is the second argument of `decode_log_file`.

use std::error::Error;
use std::path::PathBuf;

use marsrs_xlog::{
    decode_log_file, AppenderMode, CompressMode, LogLevel, XLogConfig, Xlog,
};

/// The prefix every file of this demo is named after. A file is
/// `<prefix>_<YYYYMMDD>.xlog`, so the prefix is what an app uses to tell its
/// own files from another component's in a shared directory.
const PREFIX: &str = "marsrs";

/// One record per level, in the order the levels get stricter. `fatal` is the
/// last one an app writes before it gives up, and `none` is missing on purpose:
/// it is the level that drops every record, and a program that sets it writes
/// nothing to read back.
const RECORDS: [(LogLevel, &str, &str); 6] = [
    (LogLevel::Verbose, "trace", "the finest record there is"),
    (
        LogLevel::Debug,
        "net",
        "resolved 3 addresses for example.com",
    ),
    (LogLevel::Info, "startup", "cold start in 412 ms"),
    (LogLevel::Warn, "net", "retrying after 1204 ms"),
    (LogLevel::Error, "login", "login failed: token expired"),
    (LogLevel::Fatal, "login", "giving up after 3 attempts"),
];

fn main() -> Result<(), Box<dyn Error>> {
    // Where the files go. `./log` beside the checkout by default, and whatever
    // the caller passed when there was one — a demo that writes into a
    // directory it was handed is a demo that can be aimed at a real app's
    // directory without being edited.
    let logdir = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("log"), PathBuf::from);

    // -- 1. the configuration ------------------------------------------------
    //
    // `..Default::default()` and not a full literal: eight fields, and the six
    // this demo leaves alone are the six whose defaults an app takes — async
    // writes, zlib, no cache directory, no expiry. `logdir` and `nameprefix`
    // are the two with no default worth having.
    let config = XLogConfig {
        mode: AppenderMode::Async,
        logdir: logdir.clone(),
        nameprefix: PREFIX.to_owned(),
        compress_mode: CompressMode::Zlib,
        ..Default::default()
    };
    println!("log directory: {}", logdir.display());

    // -- 2. open -------------------------------------------------------------
    //
    // One `Xlog` for the process, and it is the appender an app writes through
    // on every platform of the port — `Xlog.open(config)` in Kotlin, in Dart
    // and in TypeScript, and `Xlog::open` here. `open` is the one call with an
    // error to answer: it fails when the directory cannot be made, when the
    // prefix is empty, and — unlike the C++ port, which returns quietly — when
    // the appender refused the config, so a program that re-opens closes first.
    //
    // The level is the second argument and not a field of the config, which is
    // the one place this is not Kotlin's shape: `XLogConfig` is the C++'s, and
    // the eight fields it has are the eight the C++ gives it — a ninth would be
    // a field no other spelling of the config carries. Verbose, so that all six
    // records of `RECORDS` survive; an app in the field opens with `Info`.
    let xlog = Xlog::open(config, LogLevel::Verbose)?;

    // Mirror every record to stderr as well. Off in an app that ships, and on
    // here so that a run shows the records twice: once on the terminal, once
    // in the file it reads back at the end.
    xlog.set_console_log_enabled(true);
    // Close a file at 8 MiB and drop one at ten days. Both are 0 by default,
    // which is not the same 0 twice: a maximum size of 0 never splits a file,
    // and a lifetime of 0 is the C++'s own ten days.
    xlog.set_max_file_size_bytes(8 * 1024 * 1024);
    xlog.set_max_alive_time_seconds(10 * 24 * 3600);

    // -- 3. write ------------------------------------------------------------
    //
    // A write is a level, a tag and a message: [`Xlog::v`] through
    // [`Xlog::f`] are the six levels as six one-letter methods, and
    // [`Xlog::log`] takes the level an app named itself. The file, the function
    // and the line of the record are the port's to fill in, so a record says
    // where it was written without the caller naming it.

    for (level, tag, message) in RECORDS {
        let written = xlog.log(level, tag, message);
        println!("wrote {:?} [{}] {}", level, tag, message);
        debug_assert!(written, "a write after a successful open is accepted");
    }

    // A record that is expensive to build is worth asking about first:
    // [`Xlog::is_loggable`] is the level gate, and it is the only thing
    // standing between a dropped record and the `String` its caller paid for.
    if xlog.is_loggable(LogLevel::Debug) {
        let summary = format!("{} records written", RECORDS.len());
        xlog.d("demo", &summary);
    }

    // -- 4. flush ------------------------------------------------------------
    //
    // [`Xlog::request_flush`] asks the writer thread for the drain and returns
    // at once, and nothing ever answers when it is over; [`Xlog::flush`] hands
    // the drain to a thread of its own and is `Ready` once the records are on
    // the disk. This one drains on the calling thread, so every record above is
    // on disk before the next line runs. An app calls it before it reads the
    // files, uploads them, or exits.
    xlog.flush_now();

    // -- 5. read back --------------------------------------------------------
    //
    // The directory the appender is writing into. It is a directory and not a
    // file, because that is what the C++ function it ports answers
    // (`XloggerAppender::GetCurrentLogPath` hands back `sg_logdir`).
    if let Some(dir) = xlog.current_log_path() {
        println!("writing into:  {}", dir.display());
    }

    // Today's files, asked of the port rather than rebuilt from the date: the
    // name is `<prefix>_<YYYYMMDD>.xlog`, and a file that was split for size
    // has a second one beside it, which is why this answers a list.
    // `timespan` is days ago — `0` is today.
    let today = xlog.log_files(0);
    let path = today.first().ok_or("no log file was written today")?;
    println!("log file:      {}", path.display());

    // `decode_log_file` is the reader of the same crate: it walks the framing
    // the appender wrote and hands back the records as text, which is what the
    // C++ project's own decoder prints. `None` is the private key, and `None`
    // is right because this appender was opened without a public one.
    let plain = decode_log_file(path, None)?;
    println!("--- what is in the file ---");
    println!("{}", String::from_utf8_lossy(&plain));

    // -- 6. close ------------------------------------------------------------
    //
    // Drains what is left and closes the appender. An app that skips it loses
    // whatever the writer thread still held, which with an async appender is
    // the last records it wrote — though an `Xlog` that is simply dropped
    // closes itself, so a program that lets it go out of scope loses nothing.
    xlog.close();

    Ok(())
}
