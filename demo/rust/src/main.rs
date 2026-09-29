//! The Rust demo of mars-rs.
//!
//! One program that does what an app does with the logger, in the order an app
//! does it:
//!
//! 1. describe where the files go and how they are written — [`XLogConfig`];
//! 2. open the appender — [`appender_open`];
//! 3. write one record at every level — [`appender_write`];
//! 4. drain it to disk — [`appender_flush_sync`];
//! 5. read the file back — [`decode_log_file`];
//! 6. close it — [`appender_close`].
//!
//! Run it with `cargo run` from this directory, or `cargo run -- <dir>` to
//! choose where the `.xlog` files land. They are the same files the C++
//! implementation writes, so `xlog decode` of the port's own CLI — the binary
//! of `marsrs-xlog`, `cargo install marsrs-xlog` — reads them too.
//!
//! Two things this program deliberately does *not* show:
//!
//! - **An appender per component.** [`marsrs_xlog::appender_open_instance`]
//!   opens a second appender with a directory, a prefix and a level of its own,
//!   which is how a component gets a file nobody else's records land in. This
//!   demo is one appender, because one is what an app that only logs needs.
//! - **Encryption.** `XLogConfig::pub_key` takes the 128 hex characters of a
//!   public key, and an appender opened with one writes records only the
//!   matching private key decrypts. The key pair is `xlog keygen`, and the
//!   reader's side of it is the second argument of `decode_log_file`.

use std::error::Error;
use std::path::PathBuf;

use marsrs_xlog::{
    appender_close, appender_flush_sync, appender_get_current_log_path,
    appender_getfilepath_from_timespan, appender_open, appender_set_console_log,
    appender_set_max_alive_duration, appender_set_max_file_size, appender_write, decode_log_file,
    is_enabled_for, set_level, AppenderMode, CompressMode, LogLevel, XLogConfig, XLoggerInfo,
    DEFAULT_HANDLE,
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
    // One appender for the process. `appender_open` is the one call with an
    // error to answer: it fails when the directory cannot be made, and — unlike
    // the C++ port, which returns quietly — when an appender is already open,
    // so a program that re-opens closes first.
    appender_open(config)?;

    // The level a record has to reach to be written at all. There is no level
    // in `XLogConfig`: the level belongs to the logger, and `DEFAULT_HANDLE` is
    // the process-wide one `appender_open` opened. Verbose, so that all six
    // records of `RECORDS` survive; an app in the field sets `Info`.
    set_level(DEFAULT_HANDLE, LogLevel::Verbose);
    // Mirror every record to stderr as well. Off in an app that ships, and on
    // here so that a run shows the records twice: once on the terminal, once
    // in the file it reads back at the end.
    appender_set_console_log(true);
    // Close a file at 8 MiB and drop one at ten days. Both are 0 by default,
    // which is not the same 0 twice: a maximum size of 0 never splits a file,
    // and a lifetime of 0 is the C++'s own ten days.
    appender_set_max_file_size(8 * 1024 * 1024);
    appender_set_max_alive_duration(10 * 24 * 3600);

    // -- 3. write ------------------------------------------------------------
    //
    // `XLoggerInfo` is what lands in the record beside the message: the level,
    // the tag, and where the call came from. `appender_write` takes `None` for
    // it and fills in the pid, the tid and the clock from the OS, which is what
    // an app that does not care about the source line writes.
    //
    // `file!`, `line!` and the function name are compile-time constants of the
    // line the macro sits on, which is why every record below carries the same
    // one: a real caller writes them from inside the `log!` macro it wraps the
    // port in, where `line!` is the caller's line and not the wrapper's.
    for (level, tag, message) in RECORDS {
        let written = appender_write(
            Some(&XLoggerInfo {
                level,
                tag: Some(tag.into()),
                filename: Some(file!().into()),
                func_name: Some("main".into()),
                line: line!() as i32,
                ..Default::default()
            }),
            message,
        );
        println!("wrote {:?} [{}] {}", level, tag, message);
        debug_assert!(written, "a write after a successful open is accepted");
    }

    // A record that is expensive to build is worth asking about first: the
    // level gate is the only thing standing between a dropped record and the
    // `String` its caller paid for.
    if is_enabled_for(DEFAULT_HANDLE, LogLevel::Debug) {
        let summary = format!("{} records written", RECORDS.len());
        appender_write(None, &summary);
    }

    // -- 4. flush ------------------------------------------------------------
    //
    // `appender_flush` only signals the writer thread and returns; this one
    // waits, so every record above is on disk before the next line runs. An app
    // calls it before it reads the files, uploads them, or exits.
    appender_flush_sync();

    // -- 5. read back --------------------------------------------------------
    //
    // The directory the appender is writing into. It is a directory and not a
    // file, because that is what the C++ function it ports answers
    // (`XloggerAppender::GetCurrentLogPath` hands back `sg_logdir`).
    if let Some(dir) = appender_get_current_log_path() {
        println!("writing into:  {}", dir.display());
    }

    // Today's files, asked of the port rather than rebuilt from the date: the
    // name is `<prefix>_<YYYYMMDD>.xlog`, and a file that was split for size
    // has a second one beside it, which is why this answers a list.
    // `timespan` is days ago — `0` is today.
    let today = appender_getfilepath_from_timespan(0, PREFIX, &logdir);
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
    // the last records it wrote.
    appender_close();

    Ok(())
}
