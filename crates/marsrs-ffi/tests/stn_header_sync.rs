//! Keeps `include/mars_stn.h` and `src/stn.rs` in sync.
//!
//! The header is hand-maintained (there is no cbindgen step), so this test is
//! the guard rail: every `mars_stn_*` symbol and every type it declares has to
//! appear in the header, and the `MARS_STN_*` values have to match.

#![cfg(feature = "stn")]

use std::fs;

use mars_ffi::stn::{
    MARS_STN_ERR_NO_DUE, MARS_STN_ERR_NULL_CONFIG, MARS_STN_ERR_NULL_TASK, MARS_STN_ERR_PANIC,
    MARS_STN_ERR_REFUSED, MARS_STN_OK,
};

fn header() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("include/mars_stn.h");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Whether `header` declares `symbol` — the whole name, and in a line that is
/// not a comment.
///
/// A plain `contains` took the `mars_stn_reset` of
/// `mars_stn_reset_and_init_encoder_version` for it, so a symbol the header
/// never declared passed as long as a longer one began with it; and a mention
/// in a doc comment is not a declaration either, which is what the comment
/// lines are skipped for.
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

/// Whether `header` declares the type `ty`: the `} MarsStnAnswer;` a struct or
/// an enum ends with, or the `(*MarsStnAsk)` of a function pointer. A whole
/// name either way, which a plain `contains` did not ask for: `MarsStnQuestion`
/// passed on the strength of `MarsStnQuestionKind` being there.
fn declares_type(header: &str, ty: &str) -> bool {
    header.contains(&format!("}} {ty};")) || header.contains(&format!("(*{ty})"))
}

/// Whether `header` declares the field `field`, `;` and all: the space in front
/// of it is what keeps `name;` from matching `channel_name;`.
fn declares_field(header: &str, field: &str) -> bool {
    header.contains(&format!(" {field}"))
}

/// The guard rail is not itself vacuous: `mars_stn_reset` is the beginning of
/// `mars_stn_reset_and_init_encoder_version`, and is not declared by a header
/// that declares the longer one — nor `MarsStnQuestion` by one that declares
/// `MarsStnQuestionKind`.
#[test]
fn a_name_that_starts_another_is_not_a_declaration() {
    assert!(declares(
        "void mars_stn_reset_and_init_encoder_version(int version, const char* name);",
        "mars_stn_reset_and_init_encoder_version"
    ));
    assert!(!declares(
        "void mars_stn_reset_and_init_encoder_version(int version, const char* name);",
        "mars_stn_reset"
    ));
    assert!(declares_type(
        "} MarsStnQuestionKind;",
        "MarsStnQuestionKind"
    ));
    assert!(!declares_type("} MarsStnQuestionKind;", "MarsStnQuestion"));
}

#[test]
fn header_declares_every_exported_symbol() {
    let header = header();
    for symbol in [
        "mars_stn_set_app",
        "mars_stn_reset",
        "mars_stn_reset_and_init_encoder_version",
        "mars_stn_set_longlink_svr_addr",
        "mars_stn_set_shortlink_svr_addr",
        "mars_stn_set_debug_ip",
        "mars_stn_set_backup_ips",
        "mars_stn_start_task",
        "mars_stn_stop_task",
        "mars_stn_has_task",
        "mars_stn_redo_tasks",
        "mars_stn_touch_tasks",
        "mars_stn_clear_tasks",
        "mars_stn_run_pending",
        "mars_stn_due_time",
        "mars_stn_makesure_longlink_connected",
        "mars_stn_makesure_longlink_connected_ext",
        "mars_stn_longlink_is_connected",
        "mars_stn_longlink_is_connected_ext",
        "mars_stn_create_longlink",
        "mars_stn_destroy_longlink",
        "mars_stn_mark_main_longlink",
        "mars_stn_disable_longlink",
        "mars_stn_noop_task_id",
        "mars_stn_set_signalling_strategy",
        "mars_stn_keep_signalling",
        "mars_stn_stop_signalling",
        "mars_stn_set_client_version",
        "mars_stn_gen_task_id",
        "mars_stn_gen_sequence_id",
        "mars_stn_trig_nooping",
        "mars_stn_on_foreground",
        "mars_stn_on_network_change",
    ] {
        assert!(
            declares(&header, symbol),
            "include/mars_stn.h is missing `{symbol}`"
        );
    }
}

#[test]
fn header_declares_the_types_and_their_fields() {
    let header = header();
    for ty in [
        "MarsStnQuestionKind",
        "MarsStnAnswerKind",
        "MarsStnHeader",
        "MarsStnStrings",
        "MarsStnLonglinkConfig",
        "MarsStnTask",
        "MarsStnCgiProfile",
        "MarsStnDnsProfile",
        "MarsStnQuestion",
        "MarsStnAnswer",
        "MarsStnAsk",
    ] {
        assert!(
            declares_type(&header, ty),
            "include/mars_stn.h is missing `{ty}`"
        );
    }
    // The app, the task and what a run leaves behind.
    for field in [
        "name;",
        "value;",
        "items;",
        "count;",
        "taskid;",
        "cmdid;",
        "channel_id;",
        "channel_select;",
        "transport_protocol;",
        "cgi;",
        "send_only;",
        "need_authed;",
        "limit_flow;",
        "limit_frequency;",
        "network_status_sensitive;",
        "channel_strategy;",
        "priority;",
        "retry_count;",
        "server_process_cost;",
        "total_timeout;",
        "long_polling;",
        "long_polling_timeout;",
        "report_arg;",
        "channel_name;",
        "group_name;",
        "user_id;",
        "protocol;",
        "headers;",
        "header_count;",
        "shortlink_host_list;",
        "shortlink_fallback_hostlist;",
        "longlink_host_list;",
        "minorlong_host_list;",
        "quic_host_list;",
        "max_minorlinks;",
        "function;",
        "cgi_prefix;",
        "redirect_type;",
        "client_sequence_id;",
        "start_time;",
        "start_connect_time;",
        "connect_successful_time;",
        "start_send_packet_time;",
        "send_packet_finished_time;",
        "start_read_packet_time;",
        "read_packet_finished_time;",
        "start_encode_packet_time;",
        "encode_packet_finished_time;",
        "start_decode_packet_time;",
        "decode_packet_finished_time;",
        "channel_type;",
        "rtt;",
        "nettype;",
        "end_time;",
        "err_type;",
        "err_code;",
        "dnstype;",
        "is_keep_alive;",
        "is_main;",
        "link_type;",
        "need_tls;",
        "group;",
    ] {
        assert!(
            declares_field(&header, field),
            "include/mars_stn.h is missing the `{field}` field"
        );
    }
    // The question and the answer.
    for field in [
        "kind;",
        "host;",
        "user_id;",
        "channel_id;",
        "ip;",
        "port;",
        "send;",
        "recv;",
        "sequence;",
        "body;",
        "body_count;",
        "hash;",
        "hash_count;",
        "profile;",
        "profile_json;",
        "dns;",
        "net_status_all;",
        "net_status_longlink;",
        "link_status;",
        "longlink_host;",
        "check_type;",
        "task;",
        "yes;",
        "ips;",
        "ip_count;",
        "bytes;",
        "byte_count;",
        "error_code;",
        "handle;",
        "mode;",
        "cmdid;",
        "limit;",
    ] {
        assert!(
            declares_field(&header, field),
            "include/mars_stn.h is missing the `{field}` field"
        );
    }
    // Every type a caller fills in or reads has to be a plain C struct, i.e.
    // `#[repr(C)]` on the Rust side.
    assert_eq!(
        header.matches("typedef struct").count(),
        8,
        "MarsStnHeader, MarsStnStrings, MarsStnLonglinkConfig, MarsStnTask, MarsStnCgiProfile, \
         MarsStnDnsProfile, MarsStnQuestion and MarsStnAnswer must be C structs"
    );
}

#[test]
fn error_codes_match_the_header_defines() {
    let header = header();
    for (name, value) in [
        ("MARS_STN_OK", MARS_STN_OK),
        ("MARS_STN_ERR_PANIC", MARS_STN_ERR_PANIC),
        ("MARS_STN_ERR_NULL_TASK", MARS_STN_ERR_NULL_TASK),
        ("MARS_STN_ERR_REFUSED", MARS_STN_ERR_REFUSED),
        ("MARS_STN_ERR_NO_DUE", MARS_STN_ERR_NO_DUE as i32),
        ("MARS_STN_ERR_NULL_CONFIG", MARS_STN_ERR_NULL_CONFIG),
    ] {
        let needle = format!("{name} (-{n})", n = value.abs());
        let zero = format!("{name} 0");
        assert!(
            header.contains(&needle) || header.contains(&zero),
            "include/mars_stn.h out of sync: expected `{name}` = {value}"
        );
    }
}
