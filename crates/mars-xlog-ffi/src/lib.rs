//! `mars-xlog-ffi` — the C ABI seam of the Rust xlog port.
//!
//! This crate exists so the port can land **incrementally**: the existing C++,
//! JNI (`mars/xlog/jni/Java2C_Xlog.cc`) and ObjC layers keep their call sites
//! and simply link against `libmars_xlog_ffi.{a,so,dylib}` instead of
//! `libmarsxlog.a`. No big-bang rewrite required.
//!
//! The exported surface mirrors:
//!
//! | C symbol                            | C++ original                                        |
//! |-------------------------------------|-----------------------------------------------------|
//! | [`mars_xlog_open`]                  | `mars::xlog::appender_open(const XLogConfig&)`      |
//! | [`mars_xlog_write`]                 | `mars::xlog::XloggerWrite(...)`                     |
//! | [`mars_xlog_flush`]                 | `mars::xlog::appender_flush()`                      |
//! | [`mars_xlog_flush_sync`]            | `mars::xlog::appender_flush_sync()`                 |
//! | [`mars_xlog_close`]                 | `mars::xlog::appender_close()`                      |
//! | [`mars_xlog_set_level`]             | `xlogger_SetLevel()`                                |
//! | [`mars_xlog_set_console_log`]       | `mars::xlog::appender_set_console_log(bool)`        |
//! | [`mars_xlog_set_max_file_size`]     | `mars::xlog::appender_set_max_file_size(uint64_t)`  |
//! | [`mars_xlog_set_max_alive_duration`]| `mars::xlog::appender_set_max_alive_duration(long)` |
//! | [`mars_xlog_current_log_path`]      | `mars::xlog::appender_get_current_log_path(char*,unsigned)` |
//!
//! The matching C header lives next to this crate at
//! `crates/mars-xlog-ffi/include/mars_xlog.h`; `tests/header_sync.rs` keeps the
//! two in sync. See `README.md` for link instructions.
//!
//! # Safety contract
//!
//! This is the **only** crate in the workspace allowed to use `unsafe`, and it
//! is confined to reading/writing caller-owned C memory:
//!
//! * every entry point is wrapped in `guard` (`catch_unwind` +
//!   `AssertUnwindSafe`), so a panic never unwinds into C;
//! * every incoming pointer is null-checked, and invalid UTF-8 degrades to an
//!   empty string instead of panicking;
//! * every `unsafe` block carries a `// SAFETY:` note.
//!
//! The exported functions are safe `extern "C" fn`s (not `unsafe extern "C"`)
//! because the null checks make them total: calling them with garbage can never
//! cause UB inside Rust beyond what the caller already promised about the
//! pointer. The raw-pointer arguments themselves are only ever touched inside
//! the audited [`cstr`] helpers.

// `unsafe` is a deliberate, audited part of this crate (and only of this crate:
// every sibling crate is `#![deny(unsafe_code)]`).
#![allow(unsafe_code)]
// Any `unsafe fn` we add must document why it is sound, and unsafe operations
// inside `unsafe fn`s must still be explicit blocks.
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::missing_safety_doc)]
#![deny(rustdoc::broken_intra_doc_links)]

pub mod abi;
pub mod cstr;
pub mod error;
pub mod state;

pub use abi::{
    mars_xlog_close, mars_xlog_current_log_path, mars_xlog_flush, mars_xlog_flush_sync,
    mars_xlog_open, mars_xlog_set_console_log, mars_xlog_set_level,
    mars_xlog_set_max_alive_duration, mars_xlog_set_max_file_size, mars_xlog_write, MarsXLogConfig,
};
pub use error::{
    MARS_XLOG_ERR_APPENDER, MARS_XLOG_ERR_BAD_COMPRESS, MARS_XLOG_ERR_BAD_MODE,
    MARS_XLOG_ERR_EMPTY_LOG_DIR, MARS_XLOG_ERR_NO_PATH, MARS_XLOG_ERR_NO_SPACE,
    MARS_XLOG_ERR_NULL_CONFIG, MARS_XLOG_ERR_NULL_OUT, MARS_XLOG_ERR_PANIC, MARS_XLOG_OK,
};

use std::panic::{self, AssertUnwindSafe};

/// Runs `f` with a panic barrier around it.
///
/// This is the FFI equivalent of the `catch(...)` the C++ layers used to get
/// from their `extern "C"` shims: unwinding out of an `extern "C"` frame is
/// undefined behaviour, so every panic is turned into `fallback`.
///
/// `fallback` is the documented error code for the symbol (or `()` for the
/// `void` ones). Note that the *default* panic hook still prints the panic to
/// stderr, which is intentional: it is the only diagnostics channel a host
/// process (e.g. the JVM) gives us.
pub(crate) fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    // `AssertUnwindSafe` is sound here: the closures only touch interior-mutable
    // process state (atomics + the appender's own locks) and no panic can leave
    // a `catch_unwind` boundary with a broken invariant, because the panic is
    // caught and discarded before any C frame observes it.
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(_payload) => fallback,
    }
}
