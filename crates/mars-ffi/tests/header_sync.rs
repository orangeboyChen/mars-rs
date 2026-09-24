//! Keeps `include/mars_xlog.h` and `src/abi.rs` in sync.
//!
//! The header is hand-maintained (there is no cbindgen step), so this test is
//! the guard rail: every exported symbol and both enums must appear in the
//! header, and the `MARS_XLOG_ERR_*` values must match `src/error.rs`.

use std::fs;

use mars_ffi::{
    MARS_XLOG_ERR_APPENDER, MARS_XLOG_ERR_BAD_COMPRESS, MARS_XLOG_ERR_BAD_MODE,
    MARS_XLOG_ERR_EMPTY_LOG_DIR, MARS_XLOG_ERR_NO_PATH, MARS_XLOG_ERR_NO_SPACE,
    MARS_XLOG_ERR_NULL_CONFIG, MARS_XLOG_ERR_NULL_OUT, MARS_XLOG_ERR_PANIC, MARS_XLOG_OK,
};

fn header() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("include/mars_xlog.h");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

#[test]
fn header_declares_every_exported_symbol() {
    let header = header();
    for symbol in [
        "mars_xlog_open",
        "mars_xlog_write",
        "mars_xlog_flush",
        "mars_xlog_flush_sync",
        "mars_xlog_close",
        "mars_xlog_set_level",
        "mars_xlog_set_console_log",
        "mars_xlog_set_max_file_size",
        "mars_xlog_set_max_alive_duration",
        "mars_xlog_current_log_path",
    ] {
        assert!(
            header.contains(symbol),
            "include/mars_xlog.h is missing `{symbol}`"
        );
    }
}

#[test]
fn header_declares_the_types_and_config_fields() {
    let header = header();
    for ty in [
        "MarsAppenderMode",
        "MarsCompressMode",
        "MarsLogLevel",
        "MarsXLogConfig",
    ] {
        assert!(header.contains(ty), "include/mars_xlog.h is missing `{ty}`");
    }
    for field in [
        "mode;",
        "log_dir;",
        "name_prefix;",
        "pub_key;",
        "compress_mode;",
        "compress_level;",
        "cache_dir;",
        "cache_days;",
    ] {
        assert!(
            header.contains(field),
            "include/mars_xlog.h is missing the `{field}` field"
        );
    }
    // The config must be a plain C struct, i.e. `#[repr(C)]` on the Rust side.
    assert!(
        header.contains("typedef struct"),
        "MarsXLogConfig must be a C struct"
    );
}

#[test]
fn error_codes_match_the_header_defines() {
    let header = header();
    for (name, value) in [
        ("MARS_XLOG_OK", MARS_XLOG_OK),
        ("MARS_XLOG_ERR_NULL_CONFIG", MARS_XLOG_ERR_NULL_CONFIG),
        ("MARS_XLOG_ERR_BAD_MODE", MARS_XLOG_ERR_BAD_MODE),
        ("MARS_XLOG_ERR_BAD_COMPRESS", MARS_XLOG_ERR_BAD_COMPRESS),
        ("MARS_XLOG_ERR_EMPTY_LOG_DIR", MARS_XLOG_ERR_EMPTY_LOG_DIR),
        ("MARS_XLOG_ERR_APPENDER", MARS_XLOG_ERR_APPENDER),
        ("MARS_XLOG_ERR_NULL_OUT", MARS_XLOG_ERR_NULL_OUT),
        ("MARS_XLOG_ERR_NO_SPACE", MARS_XLOG_ERR_NO_SPACE),
        ("MARS_XLOG_ERR_NO_PATH", MARS_XLOG_ERR_NO_PATH),
        ("MARS_XLOG_ERR_PANIC", MARS_XLOG_ERR_PANIC),
    ] {
        let needle = format!("{name} (-{n})", n = value.abs());
        let zero = format!("{name} 0");
        assert!(
            header.contains(&needle) || header.contains(&zero),
            "include/mars_xlog.h out of sync: expected `{name}` = {value}"
        );
    }
}
