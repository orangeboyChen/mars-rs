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
//! The `wstring` overloads of the C++ are not ported: every caller in this
//! repository works on UTF-8 `String`/`&str`.

pub mod alarm;
pub mod frequency_limit;
pub mod ipv6_address;
pub mod local_ipstack;
pub mod message_queue;
pub mod singleton;
pub mod socket_address;
pub mod strutil;
pub mod thread;
pub mod tickcount;

pub use frequency_limit::FrequencyLimit;
pub use local_ipstack::LocalIpStack;
pub use socket_address::SocketAddress;
pub use tickcount::{TickCount, TickCountDiff};
