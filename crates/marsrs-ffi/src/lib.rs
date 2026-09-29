//! `marsrs-ffi` — the C ABI seam of the Rust xlog port.
//!
//! This crate exists so the port can land **incrementally**: the existing C++,
//! JNI (`mars/xlog/jni/Java2C_Xlog.cc`) and ObjC layers keep their call sites
//! and simply link against `libmars_ffi.{a,so,dylib}` instead of
//! `libmarsxlog.a`. No big-bang rewrite required.
//!
//! The exported surface mirrors:
//!
//! | C symbol                                | C++ original                                        |
//! |-----------------------------------------|-----------------------------------------------------|
//! | [`abi::mars_xlog_new_instance`]               | `mars::xlog::NewXloggerInstance(config, level)`     |
//! | [`mars_xlog_write_instance`]             | `mars::xlog::XloggerWrite(...)`, `xlogger_Write`    |
//! | [`abi::mars_xlog_release_instance`]           | `mars::xlog::ReleaseXloggerInstance(name_prefix)`   |
//! | [`mars_xlog_set_level_instance`]        | `xlogger_SetLevel()`, `SetLevel` on an instance      |
//! | [`mars_xlog_request_flush_instance`]     | `mars::xlog::appender_flush()`                      |
//! | [`mars_xlog_flush_now_instance`]        | `mars::xlog::appender_flush_sync()`                 |
//! | [`mars_xlog_set_console_log_instance`]  | `mars::xlog::appender_set_console_log(bool)`        |
//! | [`mars_xlog_set_max_file_size_instance`]| `mars::xlog::appender_set_max_file_size(uint64_t)`  |
//! | [`mars_xlog_set_max_alive_duration_instance`] | `mars::xlog::appender_set_max_alive_duration(long)` |
//! | [`mars_xlog_current_log_path`]          | `mars::xlog::appender_get_current_log_path(char*,unsigned)` |
//!
//! Every one of the instance symbols takes a handle, and `0` is the process-wide
//! appender: the C++ it mirrors has a free function for that logger and a
//! handle-taking one for the rest, and this seam has one spelling instead — the
//! same thing the platforms do, where the handle is a field of the object an
//! app holds. What a C caller has no way to do is *install* that appender:
//! `mars_xlog_new_instance` is how an app gets a logger, and the process-wide
//! one is the plumbing the JNI bridge sets up from Rust.
//!
//! The matching C header lives next to this crate at
//! `crates/marsrs-ffi/include/mars_xlog.h` (`include/mars_sdt.h` for the `sdt`
//! feature, `include/mars_stn.h` for the `stn` one); `tests/header_sync.rs`
//! keeps them in sync. See `README.md` for link instructions.
//!
//! # Safety contract
//!
//! This is the crate the workspace's `unsafe` lives in, and it is confined to
//! reading/writing caller-owned C memory:
//!
//! * every entry point is wrapped in `guard` (`catch_unwind` +
//!   `AssertUnwindSafe`), so a panic never unwinds into C;
//! * every incoming pointer is null-checked, and invalid UTF-8 degrades to an
//!   empty string instead of panicking;
//! * every `unsafe` block carries a `// SAFETY:` note.
//!
//! An entry point that takes a caller's pointer is an `unsafe extern "C" fn`,
//! and one that takes none is a safe one: the null checks make both total —
//! calling them with garbage cannot cause UB inside Rust beyond what the
//! caller already promised — but a pointer is a promise C has to keep, and
//! `unsafe` is the signature that says so where the call is written. The
//! raw-pointer arguments themselves are only ever touched inside the audited
//! [`cstr`] helpers, every `unsafe` block carries a `// SAFETY:` note, and
//! every `unsafe fn` carries the `# Safety` this crate denies going without.

// `unsafe` is a deliberate, audited part of this crate: this is the boundary a
// caller's raw pointers are read across. The appender allows it too, beside
// the mapping that cannot be written without it; the rest of the tree denies
// it outright.
#![allow(unsafe_code)]
// Any `unsafe fn` we add must document why it is sound, and unsafe operations
// inside `unsafe fn`s must still be explicit blocks.
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::missing_safety_doc)]
#![deny(rustdoc::broken_intra_doc_links)]

// The xlog half of the port, behind the feature the Apple artifact is built
// with. `cstr` stays unconditional: it is the pointer handling every future
// entry point needs, and it carries no symbols of its own.
#[cfg(feature = "xlog")]
pub mod abi;
pub mod cstr;
#[cfg(feature = "xlog")]
pub mod error;
// The diagnosis, behind its own feature: `mars_sdt_*`, which is the seam
// `mars/sdt/jni/*_Java2C.cc` has — and the one an app that only logs does not
// need in its binary.
#[cfg(feature = "sdt")]
pub mod sdt;
#[cfg(feature = "xlog")]
pub mod state;
// The task pipeline, behind its own feature: `mars_stn_*`, which is the seam
// `mars/stn/jni/*_Java2C.cc` has — and the one an app that only logs does not
// need in its binary.
#[cfg(feature = "stn")]
pub mod stn;

// The Rust-side mirror of the C surface: the symbols an instance is addressed
// through, plus the handful that are the process-wide appender's own
// (`assert`, the two paths and the recovery helpers) — those have no instance
// spelling, there being nothing to pick between.
//
// What is *not* here is the process-wide lifecycle: `mars_xlog_open`,
// `mars_xlog_write` and `mars_xlog_close` went when the C ABI took them out.
// An app holds an instance now, the way it does in Rust and in Kotlin, and
// the process-wide appender is the plumbing the JNI bridge installs from
// Rust — not a thing a C caller is offered a handle for.
#[cfg(feature = "xlog")]
pub use abi::{
    mars_xlog_assert, mars_xlog_current_log_path, mars_xlog_flush_now_all,
    mars_xlog_flush_now_instance, mars_xlog_request_flush_all, mars_xlog_request_flush_instance,
    mars_xlog_set_console_fun, mars_xlog_set_console_log_instance, mars_xlog_set_level_instance,
    mars_xlog_set_max_alive_duration_instance, mars_xlog_set_max_file_size_instance,
    mars_xlog_set_mode_instance, mars_xlog_write_instance, MarsXLogConfig, MarsXLogConsoleFun,
};
#[cfg(feature = "xlog")]
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
///
/// Ungated: the diagnosis' and the task pipeline's entry points cross it too,
/// and both are built without `xlog` for the Apple net framework.
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
