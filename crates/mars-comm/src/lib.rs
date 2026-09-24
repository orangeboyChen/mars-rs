//! Common utilities of mars, ported from `mars/comm`.
//!
//! Only what the rest of the port needs is here, and the C++ semantics are kept
//! (including the quirks) rather than "modernised": `strutil` mirrors
//! `comm/strutil.h`, `tickcount` mirrors `comm/tickcount.h` +
//! `comm/time_utils.h`, `frequency_limit` mirrors `comm/comm_frequency_limit.h`
//! and `singleton` is the `OnceLock` equivalent of `comm/singleton.h`.
//!
//! `thread` mirrors `comm/thread/` on top of `std::thread`/`std::sync`, and
//! `message_queue` mirrors `comm/messagequeue/` and `alarm` the timer built on
//! top of it.
//!
//! `socket_address` mirrors `comm/socket/socket_address.cc` with the two
//! headers it needs: `ipv6_address` is `ipv6_address_utils.h` +
//! `nat64_prefix_util.h` and `local_ipstack` is `local_ipstack.h`.
//!
//! `proxy` mirrors the proxy half of `comm/comm_data.h`: `ProxyInfo` is the
//! value the C++ hands a connect, and `ProxyType` is how the proxy is talked
//! to.
//!
//! The `wstring` overloads of the C++ are not ported: every caller in this
//! repository works on UTF-8 `String`/`&str`.

/// The NAT64 prefix of [`ipv6_address`] is one value for the whole process, so
/// the unit tests that move it need **one** lock for the crate, not one per
/// module: `ipv6_address`'s tests and `socket_address`'s tests run in the same
/// binary, and a prefix one of them learned would otherwise be the prefix the
/// other one reads.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub mod alarm;
pub mod frequency_limit;
pub mod ipv6_address;
pub mod local_ipstack;
pub mod message_queue;
pub mod proxy;
pub mod singleton;
pub mod socket_address;
pub mod strutil;
pub mod thread;
pub mod tickcount;

pub use frequency_limit::FrequencyLimit;
pub use local_ipstack::LocalIpStack;
pub use proxy::{ProxyInfo, ProxyType};
pub use socket_address::SocketAddress;
pub use tickcount::{TickCount, TickCountDiff};
