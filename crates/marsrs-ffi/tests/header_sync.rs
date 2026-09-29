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

/// Whether `header` declares `symbol` — the whole name, and in a line that is
/// not a comment.
///
/// A plain `contains` took the `mars_xlog_current_log_path` of
/// `mars_xlog_current_log_path_instance` for it, so a symbol the header never
/// declared passed as long as a longer one began with it; and a mention in a
/// doc comment is not a declaration either, which is what the comment lines
/// are skipped for.
fn declares(header: &str, symbol: &str) -> bool {
    let needle = format!(" {symbol}(");
    header.lines().any(|line| {
        let trimmed = line.trim_start();
        !trimmed.starts_with('*')
            && !trimmed.starts_with("/*")
            && !trimmed.starts_with("//")
            && line.contains(&needle)
    })
}

/// Whether `header` declares the type `ty`: the `} MarsXLogConfig;` a struct
/// or an enum ends with, or the `(*MarsXLogConfig)` of a function pointer. A
/// whole name either way, which a plain `contains` did not ask for.
fn declares_type(header: &str, ty: &str) -> bool {
    header.contains(&format!("}} {ty};")) || header.contains(&format!("(*{ty})"))
}

/// Whether `header` declares the field `field`, `;` and all: the space in front
/// of it is what keeps `name;` from matching `channel_name;`.
fn declares_field(header: &str, field: &str) -> bool {
    header.contains(&format!(" {field}"))
}

/// The guard rail is not itself vacuous: `mars_xlog_current_log_path` is the
/// beginning of `mars_xlog_current_log_path_instance`, and is not declared by
/// a header that declares the longer one.
#[test]
fn a_name_that_starts_another_is_not_a_declaration() {
    assert!(declares(
        "int mars_xlog_current_log_path_instance(long long instance, char* out, unsigned int len);",
        "mars_xlog_current_log_path_instance"
    ));
    assert!(!declares(
        "int mars_xlog_current_log_path_instance(long long instance, char* out, unsigned int len);",
        "mars_xlog_current_log_path"
    ));
}

#[test]
fn header_declares_every_exported_symbol() {
    let header = header();
    for symbol in [
        "mars_xlog_assert",
        "mars_xlog_request_flush_all",
        "mars_xlog_flush_now_all",
        "mars_xlog_request_flush_instance",
        "mars_xlog_flush_now_instance",
        "mars_xlog_set_level_instance",
        "mars_xlog_set_console_fun",
        "mars_xlog_set_console_log_instance",
        "mars_xlog_set_max_file_size_instance",
        "mars_xlog_set_max_alive_duration_instance",
        "mars_xlog_set_mode_instance",
        "mars_xlog_current_log_path",
        "mars_xlog_current_log_path_instance",
        "mars_xlog_current_log_cache_path",
        "mars_xlog_oneshot_flush",
        "mars_xlog_make_logfile_name",
        "mars_xlog_getfilepath_from_timespan",
        // The six instance symbols, which the list above used to leave out: a
        // rename of one of them shipped with a header that still named the old
        // one, and nothing failed.
        "mars_xlog_new_instance",
        "mars_xlog_get_instance",
        "mars_xlog_release_instance",
        "mars_xlog_write_instance",
        "mars_xlog_is_enabled_for",
        "mars_xlog_get_level",
    ] {
        assert!(
            declares(&header, symbol),
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
        assert!(
            declares_type(&header, ty),
            "include/mars_xlog.h is missing `{ty}`"
        );
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
            declares_field(&header, field),
            "include/mars_xlog.h is missing the `{field}` field"
        );
    }
    // The config must be a plain C struct, i.e. `#[repr(C)]` on the Rust side.
    assert!(
        header.contains("typedef struct"),
        "MarsXLogConfig must be a C struct"
    );
}

/// `mars_xlog_oneshot_flush` answers a `TFileIOAction`, so the header's
/// `MARS_XLOG_ACTION_*` values are part of the contract too.
#[test]
fn action_codes_match_the_header_defines() {
    use marsrs_appender::FileIoAction;

    let header = header();
    for (name, value) in [
        ("MARS_XLOG_ACTION_NONE", FileIoAction::None),
        ("MARS_XLOG_ACTION_SUCCESS", FileIoAction::Success),
        ("MARS_XLOG_ACTION_UNNECESSARY", FileIoAction::Unnecessary),
        ("MARS_XLOG_ACTION_OPEN_FAILED", FileIoAction::OpenFailed),
        ("MARS_XLOG_ACTION_READ_FAILED", FileIoAction::ReadFailed),
        ("MARS_XLOG_ACTION_WRITE_FAILED", FileIoAction::WriteFailed),
        ("MARS_XLOG_ACTION_CLOSE_FAILED", FileIoAction::CloseFailed),
        ("MARS_XLOG_ACTION_REMOVE_FAILED", FileIoAction::RemoveFailed),
    ] {
        let needle = format!("{name} {value}", value = value as i32);
        assert!(
            header.contains(&needle),
            "include/mars_xlog.h out of sync: expected `{name}` = {}",
            value as i32
        );
    }
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
