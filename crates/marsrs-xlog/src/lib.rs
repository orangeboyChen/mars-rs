//! `marsrs-xlog` — the xlog half of the port of [Tencent's
//! mars](https://github.com/Tencent/mars), and the pair of
//! [`marsrs`](https://crates.io/crates/marsrs).
//!
//! The C++ project publishes two artifacts: `mars-core`, which is everything,
//! and `mars-xlog`, which is the logger and nothing else. This port publishes
//! the same pair under names of its own, and this crate is the second of
//! them. An app that only logs takes this and gets no bytes of STN or SDT; an
//! app that wants the whole port takes [`marsrs`](https://crates.io/crates/marsrs)
//! instead, which re-exports this crate as its `xlog` module.
//!
//! | what                       | where                                                |
//! |----------------------------|------------------------------------------------------|
//! | open, write, flush, close  | [`appender_open`], [`appender_write`], [`appender_flush_sync`], [`appender_close`] |
//! | one logger per prefix      | [`appender_open_instance`] and the `*_instance` functions |
//! | the configuration          | [`XLogConfig`], [`AppenderMode`], [`LogLevel`]        |
//! | reading a `.xlog` back     | [`decode_log_file`], [`LogBuffer`], [`get_period_logs`] |
//! | the byte primitives        | [`bytes`]                                            |
//!
//! The files it writes are the C++ implementation's: same magic number, same
//! TEA/`zlib`/`zstd` framing, so a `.xlog` written here is read by upstream's
//! `decode_mars_log_file.py` and one written there is read by [`LogBuffer`].
//!
//! Reading one back is [`decode_log_file`], and the CLI is the same reader with
//! a command line in front of it — `cargo install marsrs-xlog` puts `xlog` on
//! the `$PATH`, and a release carries it as an archive of its own:
//!
//! ```text
//! xlog decode --privkey=<hex> marsrs_20260927.xlog --out=marsrs.plain
//! xlog encode --pubkey=<hex> records.txt --out=marsrs_20260927.xlog
//! ```
//!
//! ```no_run
//! use marsrs_xlog::{appender_close, appender_flush_sync, appender_open, appender_write, XLogConfig};
//!
//! let mut config = XLogConfig::default();
//! config.logdir = std::path::PathBuf::from("/tmp/mars-log");
//! appender_open(config).unwrap();
//! appender_write(None, "hello from mars");
//! appender_flush_sync();
//! appender_close();
//! ```

pub use marsrs_appender::*;
pub use marsrs_buffer::{get_period_logs, get_period_logs_with_timeout_ms, LogBuffer};
/// The byte primitives the port is written over: `AutoBuffer`, `PtrBuffer` and
/// the little-endian helpers. They are public here because [`log_formater`] and
/// [`LogBuffer`] take them, not because a caller is expected to build one.
pub use marsrs_core as bytes;

/// Reading a `.xlog` back: the port of upstream's `decode_log_file.c`.
pub mod decode;

pub use decode::{decode_log_file, decode_records};

#[cfg(test)]
mod tests {
    use super::{bytes, LogBuffer, XLogConfig};

    #[test]
    fn the_xlog_surface_is_reachable_through_the_facade() {
        // One name per re-export, so that a rename in a crate behind this one
        // breaks this test rather than a caller's build.
        assert!(std::any::type_name::<XLogConfig>().ends_with("XLogConfig"));
        assert!(std::any::type_name::<LogBuffer>().ends_with("LogBuffer"));
        // `PtrBuffer<'a>` keeps its lifetime in its name, so it is matched on
        // and not compared whole.
        assert!(std::any::type_name::<bytes::PtrBuffer<'static>>().contains("PtrBuffer"));
    }
}
