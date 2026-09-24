//! Common utilities of mars, ported from `mars/comm`.
//!
//! Only what the rest of the port needs is here, and the C++ semantics are kept
//! (including the quirks) rather than "modernised": `strutil` mirrors
//! `comm/strutil.h`, `tickcount` mirrors `comm/tickcount.h` +
//! `comm/time_utils.h`, `frequency_limit` mirrors `comm/comm_frequency_limit.h`
//! and `singleton` is the `OnceLock` equivalent of `comm/singleton.h`.
//!
//! `thread` mirrors `comm/thread/` on top of `std::thread`/`std::sync`.
//!
//! The `wstring` overloads of the C++ are not ported: every caller in this
//! repository works on UTF-8 `String`/`&str`.

pub mod frequency_limit;
pub mod singleton;
pub mod strutil;
pub mod thread;
pub mod tickcount;

pub use frequency_limit::FrequencyLimit;
pub use tickcount::{TickCount, TickCountDiff};
