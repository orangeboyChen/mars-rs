//! Return codes shared with `include/mars_xlog.h`.
//!
//! The C++ originals returned `void` for most of these calls and `bool` for
//! `appender_get_current_log_path`; the C ABI needs a real error channel so a
//! host process can tell "not opened" from "success". The `#define`s in
//! `mars_xlog.h` repeat these values verbatim, and `tests/header_sync.rs`
//! asserts the pair stays in sync.

/// `mars_xlog_open` / `mars_xlog_current_log_path` succeeded.
pub const MARS_XLOG_OK: i32 = 0;
/// `config` was null.
pub const MARS_XLOG_ERR_NULL_CONFIG: i32 = -1;
/// `config->mode` was not a [`crate::abi::MarsAppenderMode`] value.
pub const MARS_XLOG_ERR_BAD_MODE: i32 = -2;
/// `config->compress_mode` was not a [`crate::abi::MarsCompressMode`] value.
pub const MARS_XLOG_ERR_BAD_COMPRESS: i32 = -3;
/// `config->log_dir` was null or empty — the appender has nowhere to write.
pub const MARS_XLOG_ERR_EMPTY_LOG_DIR: i32 = -4;
/// The Rust appender rejected the config (`AppenderError`).
pub const MARS_XLOG_ERR_APPENDER: i32 = -5;
/// `out` was null in `mars_xlog_current_log_path`.
pub const MARS_XLOG_ERR_NULL_OUT: i32 = -6;
/// The output buffer is too small to hold the path plus its NUL terminator.
pub const MARS_XLOG_ERR_NO_SPACE: i32 = -7;
/// No log file is currently open, so there is no path to report.
pub const MARS_XLOG_ERR_NO_PATH: i32 = -8;
/// A Rust panic was caught at the FFI boundary.
pub const MARS_XLOG_ERR_PANIC: i32 = -99;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_negative() {
        let all = [
            MARS_XLOG_ERR_NULL_CONFIG,
            MARS_XLOG_ERR_BAD_MODE,
            MARS_XLOG_ERR_BAD_COMPRESS,
            MARS_XLOG_ERR_EMPTY_LOG_DIR,
            MARS_XLOG_ERR_APPENDER,
            MARS_XLOG_ERR_NULL_OUT,
            MARS_XLOG_ERR_NO_SPACE,
            MARS_XLOG_ERR_NO_PATH,
            MARS_XLOG_ERR_PANIC,
        ];
        assert_eq!(MARS_XLOG_OK, 0);
        for (i, code) in all.iter().enumerate() {
            assert!(*code < 0, "{code} must be negative");
            for other in &all[i + 1..] {
                assert_ne!(*code, *other, "duplicate error code {code}");
            }
        }
    }
}
