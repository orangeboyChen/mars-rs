//! Keeps `include/mars_stn.h` and `src/stn.rs` in sync.
//!
//! The header is hand-maintained (there is no cbindgen step), so this test is
//! the guard rail: every `mars_stn_*` symbol and every type it declares has to
//! appear in the header, and the `MARS_STN_*` values have to match.

#![cfg(feature = "stn")]

use std::fs;

use mars_ffi::stn::{
    MARS_STN_ERR_NULL_TASK, MARS_STN_ERR_PANIC, MARS_STN_ERR_REFUSED, MARS_STN_OK,
};

fn header() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("include/mars_stn.h");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
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
        "mars_stn_makesure_longlink_connected",
        "mars_stn_set_signalling_strategy",
        "mars_stn_keep_signalling",
        "mars_stn_stop_signalling",
        "mars_stn_set_client_version",
        "mars_stn_gen_task_id",
        "mars_stn_gen_sequence_id",
        "mars_stn_trig_nooping",
    ] {
        assert!(
            header.contains(symbol),
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
        "MarsStnTask",
        "MarsStnCgiProfile",
        "MarsStnDnsProfile",
        "MarsStnQuestion",
        "MarsStnAnswer",
        "MarsStnAsk",
    ] {
        assert!(header.contains(ty), "include/mars_stn.h is missing `{ty}`");
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
        "channel_type;",
        "rtt;",
        "nettype;",
        "end_time;",
        "err_type;",
        "err_code;",
        "dnstype;",
    ] {
        assert!(
            header.contains(field),
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
            header.contains(field),
            "include/mars_stn.h is missing the `{field}` field"
        );
    }
    // Every type a caller fills in or reads has to be a plain C struct, i.e.
    // `#[repr(C)]` on the Rust side.
    assert_eq!(
        header.matches("typedef struct").count(),
        7,
        "MarsStnHeader, MarsStnStrings, MarsStnTask, MarsStnCgiProfile, MarsStnDnsProfile, \
         MarsStnQuestion and MarsStnAnswer must be C structs"
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
    ] {
        let needle = format!("{name} (-{n})", n = value.abs());
        let zero = format!("{name} 0");
        assert!(
            header.contains(&needle) || header.contains(&zero),
            "include/mars_stn.h out of sync: expected `{name}` = {value}"
        );
    }
}
