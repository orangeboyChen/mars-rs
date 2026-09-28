//! `marsrs` — the Rust port of [Tencent's
//! mars](https://github.com/Tencent/mars): the xlog appender, the STN task
//! pipeline and the SDT network diagnosis.
//!
//! The C++ project publishes two artifacts, `mars-core` (everything) and
//! `mars-xlog` (the logger alone), and this port publishes the same pair
//! under names of its own: this crate is the whole port, and
//! [`marsrs-xlog`](https://crates.io/crates/marsrs-xlog) is the logger
//! alone. An app that only logs takes that one and carries none of STN's or
//! SDT's bytes; this one re-exports it as [`xlog`].
//!
//! * [`xlog`] — the logger: open, write, flush, close. Ports `mars/xlog`.
//! * [`stn`] — the task pipeline: anti-avalanche, dynamic timeout. Ports
//!   `mars/stn`.
//! * [`sdt`] — the diagnosis: ping, DNS, TCP and HTTP. Ports `mars/sdt`.
//! * [`comm`] — what the two above are written over. Ports `mars/comm`.
//! * [`bytes`] — `AutoBuffer` and `PtrBuffer`. Ports
//!   `mars/comm/autobuffer.h`.
//!
//! Four of the five are optional crates of this workspace behind a feature
//! of their own, so `--no-default-features --features xlog` is the build an
//! app that only logs gets. [`bytes`] is `marsrs_core`, and it is the one
//! that is not: the formatter of [`xlog`] and the profile of [`sdt`] are
//! written in `AutoBuffer`, so there is no build of this crate that does
//! without it.
//!
//! ```no_run
//! use marsrs::xlog::{appender_close, appender_flush_sync, appender_open, appender_write, XLogConfig};
//!
//! let mut config = XLogConfig::default();
//! config.logdir = std::path::PathBuf::from("/tmp/mars-log");
//! appender_open(config).unwrap();
//! appender_write(None, "hello from mars");
//! appender_flush_sync();
//! appender_close();
//! ```

#[cfg(feature = "comm")]
pub use marsrs_comm as comm;
pub use marsrs_core as bytes;
#[cfg(feature = "sdt")]
pub use marsrs_sdt as sdt;
#[cfg(feature = "stn")]
pub use marsrs_stn as stn;
#[cfg(feature = "xlog")]
pub use marsrs_xlog as xlog;

#[cfg(test)]
mod tests {
    #[test]
    fn every_module_is_reachable_through_the_facade() {
        // One name per re-export, so that a rename in a crate behind this
        // one breaks this test rather than a caller's build. The four
        // optional ones are named behind the feature that carries them,
        // because a build that leaves a module out is a build this test
        // has to pass in too.
        assert!(std::any::type_name::<crate::bytes::AutoBuffer>().ends_with("AutoBuffer"));
        #[cfg(feature = "comm")]
        assert!(std::any::type_name::<crate::comm::LocalIpStack>().ends_with("LocalIpStack"));
        #[cfg(feature = "sdt")]
        assert!(std::any::type_name::<crate::sdt::SdtLogic>().ends_with("SdtLogic"));
        #[cfg(feature = "stn")]
        assert!(std::any::type_name::<crate::stn::AntiAvalanche>().ends_with("AntiAvalanche"));
        #[cfg(feature = "xlog")]
        assert!(std::any::type_name::<crate::xlog::XLogConfig>().ends_with("XLogConfig"));
    }
}
