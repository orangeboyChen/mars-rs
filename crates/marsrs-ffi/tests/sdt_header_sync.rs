//! Keeps `include/mars_sdt.h` and `src/sdt.rs` in sync.
//!
//! The header is hand-maintained (there is no cbindgen step), so this test is
//! the guard rail: every `mars_sdt_*` symbol and every type it declares has to
//! appear in the header, and the `MARS_SDT_*` values have to match.

#![cfg(feature = "sdt")]

use std::fs;

use mars_ffi::sdt::{
    MARS_SDT_ERR_BAD_ARG, MARS_SDT_ERR_BUSY, MARS_SDT_ERR_NO_CHECK, MARS_SDT_ERR_NO_PROBE,
    MARS_SDT_ERR_NO_SPACE, MARS_SDT_ERR_NULL_OUT, MARS_SDT_ERR_PANIC, MARS_SDT_OK,
};

fn header() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("include/mars_sdt.h");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Whether `header` declares `symbol` — the whole name, and in a line that is
/// not a comment.
///
/// A plain `contains` took the `mars_sdt_http_netcheck_cgi` of
/// `mars_sdt_set_http_netcheck_cgi` for it, so a symbol the header never
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

/// Whether `header` declares the type `ty`: the `} MarsSdtAnswer;` a struct or
/// an enum ends with, or the `(*MarsSdtProbe)` of a function pointer. A whole
/// name either way, which a plain `contains` did not ask for.
fn declares_type(header: &str, ty: &str) -> bool {
    header.contains(&format!("}} {ty};")) || header.contains(&format!("(*{ty})"))
}

/// Whether `header` declares the field `field`, `;` and all: the space in front
/// of it is what keeps `name;` from matching `channel_name;`.
fn declares_field(header: &str, field: &str) -> bool {
    header.contains(&format!(" {field}"))
}

/// The guard rail is not itself vacuous: `mars_sdt_http_netcheck_cgi` is the
/// beginning of `mars_sdt_set_http_netcheck_cgi`, and is not declared by a
/// header that declares the longer one.
#[test]
fn a_name_that_starts_another_is_not_a_declaration() {
    assert!(declares(
        "void mars_sdt_set_http_netcheck_cgi(const char* cgi);",
        "mars_sdt_set_http_netcheck_cgi"
    ));
    assert!(!declares(
        "void mars_sdt_set_http_netcheck_cgi(const char* cgi);",
        "mars_sdt_http_netcheck_cgi"
    ));
}

#[test]
fn header_declares_every_exported_symbol() {
    let header = header();
    for symbol in [
        "mars_sdt_reset",
        "mars_sdt_set_http_netcheck_cgi",
        "mars_sdt_http_netcheck_cgi",
        "mars_sdt_start_active_check",
        "mars_sdt_cancel_active_check",
        "mars_sdt_is_checking",
        "mars_sdt_plan",
        "mars_sdt_run_checks",
        "mars_sdt_take_report",
    ] {
        assert!(
            declares(&header, symbol),
            "include/mars_sdt.h is missing `{symbol}`"
        );
    }
}

#[test]
fn header_declares_the_types_and_their_fields() {
    let header = header();
    for ty in [
        "MarsSdtKind",
        "MarsSdtCheck",
        "MarsSdtQuery",
        "MarsSdtAnswer",
        "MarsSdtIpPort",
        "MarsSdtHosts",
        "MarsSdtProbe",
    ] {
        assert!(
            declares_type(&header, ty),
            "include/mars_sdt.h is missing `{ty}`"
        );
    }
    // What a probe is asked, and what it answers.
    for field in [
        "host;",
        "port;",
        "timeout;",
        "error_code;",
        "rtt;",
        "ips;",
        "ip_count;",
        "sent;",
        "received;",
        "is_noop_resp;",
        "status_code;",
        "loss_rate;",
        "avgrtt;",
    ] {
        assert!(
            declares_field(&header, field),
            "include/mars_sdt.h is missing the `{field}` field"
        );
    }
    // The hosts a diagnosis is started with.
    for field in ["name;", "ports;", "port_count;"] {
        assert!(
            declares_field(&header, field),
            "include/mars_sdt.h is missing the `{field}` field"
        );
    }
    // Both structs have to be plain C ones, i.e. `#[repr(C)]` on the Rust side.
    assert_eq!(
        header.matches("typedef struct").count(),
        4,
        "MarsSdtQuery, MarsSdtAnswer, MarsSdtIpPort and MarsSdtHosts must be C structs"
    );
}

#[test]
fn error_codes_match_the_header_defines() {
    let header = header();
    for (name, value) in [
        ("MARS_SDT_OK", MARS_SDT_OK),
        ("MARS_SDT_ERR_PANIC", MARS_SDT_ERR_PANIC),
        ("MARS_SDT_ERR_NULL_OUT", MARS_SDT_ERR_NULL_OUT),
        ("MARS_SDT_ERR_NO_SPACE", MARS_SDT_ERR_NO_SPACE),
        ("MARS_SDT_ERR_NO_PROBE", MARS_SDT_ERR_NO_PROBE),
        ("MARS_SDT_ERR_BUSY", MARS_SDT_ERR_BUSY),
        ("MARS_SDT_ERR_NO_CHECK", MARS_SDT_ERR_NO_CHECK),
        ("MARS_SDT_ERR_BAD_ARG", MARS_SDT_ERR_BAD_ARG),
    ] {
        let needle = format!("{name} (-{n})", n = value.abs());
        let zero = format!("{name} 0");
        assert!(
            header.contains(&needle) || header.contains(&zero),
            "include/mars_sdt.h out of sync: expected `{name}` = {value}"
        );
    }
}
