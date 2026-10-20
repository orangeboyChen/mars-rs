//! Keeps `include/mars_xlog.h` and `src/abi.rs` in sync.
//!
//! The header is hand-maintained (there is no cbindgen step), so this test is
//! the guard rail: the two must export and declare the same set of
//! `mars_xlog_*` symbols, the types and the config fields must appear in the
//! header, and the `MARS_XLOG_ERR_*` values must match `src/error.rs`.

use std::collections::BTreeSet;
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

fn abi() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/abi.rs");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Every `mars_xlog_*` name in `text`: the identifier a `mars_xlog_` starts,
/// whether a return type, a `(` or a `;` sits in front of it.
fn names_in(text: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let bytes = text.as_bytes();
    for (at, _) in text.match_indices("mars_xlog_") {
        let rest = &bytes[at..];
        let end = rest
            .iter()
            .position(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
            .unwrap_or(rest.len());
        names.insert(text[at..at + end].to_owned());
    }
    names
}

/// Every name a `#[no_mangle] pub extern "C" fn mars_xlog_*` in `src/abi.rs`
/// exports. An empty parse is a failure and not a pass: a rename of the
/// pattern, or a file that moved, must not read as "nothing to check".
fn exported() -> BTreeSet<String> {
    let names: BTreeSet<String> = abi()
        .lines()
        .filter(|line| line.contains("extern \"C\" fn "))
        .flat_map(names_in)
        .collect();
    assert!(
        !names.is_empty(),
        "no `mars_xlog_*` export found in src/abi.rs"
    );
    names
}

/// Every `mars_xlog_*` name the header declares, wherever the declaration
/// begins — a return type on the line in front of it, and a multi-line
/// parameter list behind it.
fn declared() -> BTreeSet<String> {
    let names = names_in(&header());
    assert!(
        !names.is_empty(),
        "no `mars_xlog_*` declaration found in include/mars_xlog.h"
    );
    names
}

/// The one direction that used to be checked and the one that was not: an
/// export no declaration matches is a symbol a C caller cannot see, and a
/// declaration no export matches is a link error waiting for the first app
/// that calls it.
#[test]
fn the_header_and_the_abi_name_the_same_symbols() {
    let exported = exported();
    let declared = declared();

    let missing: Vec<&String> = exported.difference(&declared).collect();
    let orphaned: Vec<&String> = declared.difference(&exported).collect();
    assert!(
        missing.is_empty(),
        "include/mars_xlog.h does not declare {}",
        missing
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert!(
        orphaned.is_empty(),
        "include/mars_xlog.h declares {} which src/abi.rs does not export",
        orphaned
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
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

/// The number the C ABI is: the one `demo/c/README.md`, the platform READMEs
/// and `scripts/build_xcframework.sh` write down, and the one that rotted into
/// a 23 that no symbol of the header was ever part of. A symbol that lands is
/// a number none of them says any more, so this is the test that says which of
/// the two is wrong.
#[test]
fn the_abi_is_the_seventeen_symbols_the_prose_counts() {
    let declared = declared();
    assert_eq!(
        declared.len(),
        17,
        "include/mars_xlog.h declares {count} `mars_xlog_*` symbols, and 17 is what \
         demo/c/README.md, the platform READMEs and scripts/build_xcframework.sh say: \
         {declared:?}",
        count = declared.len()
    );
}
