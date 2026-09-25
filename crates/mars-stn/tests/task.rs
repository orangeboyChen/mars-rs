//! `mars/stn/stn.h` — the `Task` the caller hands to STN.
//!
//! The port keeps the field names, the constants and the defaults of the C++
//! constructor, so most of this file is a transcription of `Task::Task()` and
//! of the `static const int`s of `stn.h`.

use mars_stn::{HostRedirectType, Task};

#[test]
fn the_constructor_fills_in_the_cpp_defaults() {
    let task = Task::new(7, 9);
    assert_eq!(task.taskid, 7);
    assert_eq!(task.cmdid, 9);
    assert_eq!(task.channel_select, Task::CHANNEL_BOTH);
    assert_eq!(task.transport_protocol, Task::TRANSPORT_PROTOCOL_DEFAULT);
    assert!(task.cgi.is_empty());

    assert!(!task.send_only);
    assert!(task.need_authed);
    assert!(task.limit_flow);
    assert!(task.limit_frequency);
    assert!(!task.network_status_sensitive);
    assert_eq!(task.channel_strategy, Task::CHANNEL_NORMAL_STRATEGY);
    assert_eq!(task.priority, Task::TASK_PRIORITY_NORMAL);
    // `-1` is "the caller did not say": both `Task::Task()` and the Java
    // `Task` give it that, and `NetCore` reads `DEF_TASK_RETRY_COUNT` for it.
    // `0` would be "do not retry".
    assert_eq!(task.retry_count, -1);
    assert_eq!(task.server_process_cost, 0);
    assert_eq!(task.total_timeout, 0);
    assert!(!task.long_polling);
    assert_eq!(task.long_polling_timeout, 0);

    assert!(task.report_arg.is_empty());
    assert!(task.channel_name.is_empty());
    assert!(task.group_name.is_empty());
    assert!(task.user_id.is_empty());
    assert_eq!(task.protocol, 0);
    assert!(task.headers.is_empty());
    assert!(task.shortlink_host_list.is_empty());
    assert!(task.shortlink_fallback_hostlist.is_empty());
    assert!(task.longlink_host_list.is_empty());
    assert!(task.minorlong_host_list.is_empty());
    assert!(task.quic_host_list.is_empty());
    assert_eq!(task.max_minorlinks, 0);
    assert!(task.function.is_empty());
    assert!(task.cgi_prefix.is_empty());
    assert_eq!(task.redirect_type, HostRedirectType::None);
    assert_eq!(task.client_sequence_id, 0);
}

#[test]
fn the_channel_constants_match_stn_h() {
    assert_eq!(Task::CHANNEL_SHORT, 0x1);
    assert_eq!(Task::CHANNEL_LONG, 0x2);
    assert_eq!(Task::CHANNEL_BOTH, 0x3);
    assert_eq!(Task::CHANNEL_MINOR_LONG, 0x4);
    assert_eq!(Task::CHANNEL_NORMAL, 0x5);
    assert_eq!(Task::CHANNEL_ALL, 0x7);

    assert_eq!(Task::CHANNEL_NORMAL_STRATEGY, 0);
    assert_eq!(Task::CHANNEL_FAST_STRATEGY, 1);
    assert_eq!(Task::CHANNEL_DISASTER_RECOVERY_STRATEGY, 2);
}

#[test]
fn the_protocol_and_priority_constants_match_stn_h() {
    assert_eq!(Task::TRANSPORT_PROTOCOL_DEFAULT, 0);
    assert_eq!(Task::TRANSPORT_PROTOCOL_TCP, 1);
    assert_eq!(Task::TRANSPORT_PROTOCOL_QUIC, 2);
    assert_eq!(Task::TRANSPORT_PROTOCOL_MIXED, 3);

    assert_eq!(Task::TASK_PRIORITY_HIGHEST, 0);
    assert_eq!(Task::TASK_PRIORITY_0, 0);
    assert_eq!(Task::TASK_PRIORITY_1, 1);
    assert_eq!(Task::TASK_PRIORITY_2, 2);
    assert_eq!(Task::TASK_PRIORITY_3, 3);
    assert_eq!(Task::TASK_PRIORITY_NORMAL, 3);
    assert_eq!(Task::TASK_PRIORITY_4, 4);
    assert_eq!(Task::TASK_PRIORITY_5, 5);
    assert_eq!(Task::TASK_PRIORITY_LOWEST, 5);
}

#[test]
fn the_reserved_task_ids_match_stn_h() {
    assert_eq!(Task::INVALID_TASK_ID, 0);
    assert_eq!(Task::NOOP_TASK_ID, 0xffff_ffff);
    assert_eq!(Task::LONG_LINK_IDENTIFY_CHECKER_TASK_ID, 0xffff_fffe);
    assert_eq!(Task::SIGNALLING_KEEPER_TASK_ID, 0xffff_fffd);
    assert_eq!(Task::MINOR_LONGLINK_CMD_MASK, 0xff00_0000);
}

#[test]
fn a_short_link_task_is_built_up_from_the_defaults() {
    let mut task = Task::new(1, 2);
    task.cgi = "/cgi-bin/micromsg-bin/newsync".to_owned();
    task.cgi_prefix = "/cgi-bin/micromsg-bin".to_owned();
    task.channel_select = Task::CHANNEL_SHORT;
    task.transport_protocol = Task::TRANSPORT_PROTOCOL_TCP;
    task.send_only = true;
    task.retry_count = 3;
    task.total_timeout = 15 * 1000;
    task.headers.insert(
        "Content-Type".to_owned(),
        "application/octet-stream".to_owned(),
    );
    task.headers.insert("Accept".to_owned(), "*/*".to_owned());
    task.shortlink_host_list = vec!["short.weixin.qq.com".to_owned()];
    task.shortlink_fallback_hostlist = vec!["short2.weixin.qq.com".to_owned()];
    task.redirect_type = HostRedirectType::HttpToHttps;

    // `BTreeMap`, like the C++ `std::map`: the headers come out sorted by key
    let keys: Vec<&String> = task.headers.keys().collect();
    assert_eq!(keys, vec!["Accept", "Content-Type"]);

    let clone = task.clone();
    assert_eq!(clone, task);
    assert_ne!(clone, Task::new(1, 2));
    assert_eq!(HostRedirectType::default(), HostRedirectType::None);
}
