//! `mars/stn/src/longlink_identify_checker.cc`, through the public api.
//!
//! The samples are what the C++ answers for the same calls: `kCheckNow` is the
//! only mode that hands a buffer out, the hash the app filled outlives the
//! buffer that never went out, and a response only counts when the task id is
//! the one `SetID` was given and the app takes it.

use std::sync::{Arc, Mutex};

use mars_stn::{IdentifyMode, LongLinkIdentifyChecker};

/// `Task::kMinorLonglinkCmdMask`, which the public api carries on `Task`.
const MINOR_LONGLINK_CMD_MASK: u32 = mars_stn::Task::MINOR_LONGLINK_CMD_MASK;

/// The app: it fills the buffer and the hash, and answers `kCheckNow`.
fn app_that_checks_now() -> LongLinkIdentifyChecker {
    let mut checker = LongLinkIdentifyChecker::new("default", false);
    checker.set_check_buffer(|_channel, buffer, hash, cmdid| {
        buffer.extend_from_slice(b"identify");
        hash.extend_from_slice(b"hash");
        *cmdid = 17;
        IdentifyMode::CheckNow
    });
    checker
}

#[test]
fn only_a_check_that_is_made_now_hands_a_buffer_out() {
    let mut never = LongLinkIdentifyChecker::new("default", false);
    never.set_check_buffer(|_channel, buffer, _hash, _cmdid| {
        buffer.extend_from_slice(b"identify");
        IdentifyMode::CheckNever
    });
    assert!(never.get_identify_buffer().is_none());
    assert!(never.has_checked(), "kCheckNever marks it checked");

    let mut next = LongLinkIdentifyChecker::new("default", false);
    next.set_check_buffer(|_channel, buffer, _hash, _cmdid| {
        buffer.extend_from_slice(b"identify");
        IdentifyMode::CheckNext
    });
    assert!(next.get_identify_buffer().is_none());
    assert!(!next.has_checked(), "kCheckNext asks again");

    let mut now = app_that_checks_now();
    assert_eq!(now.get_identify_buffer(), Some((b"identify".to_vec(), 17)));
    assert_eq!(now.cmd_id(), 17);
}

#[test]
fn a_minor_long_link_hands_its_mask_out_first() {
    let seen = Arc::new(Mutex::new(0u32));
    let recording = Arc::clone(&seen);
    let mut checker = LongLinkIdentifyChecker::new("minor", true);
    checker.set_check_buffer(move |_channel, _buffer, _hash, cmdid| {
        *recording.lock().unwrap_or_else(|p| p.into_inner()) = *cmdid;
        IdentifyMode::CheckNow
    });

    let (_buffer, cmdid) = checker.get_identify_buffer().unwrap();
    assert_eq!(cmdid, MINOR_LONGLINK_CMD_MASK);
    assert_eq!(
        *seen.lock().unwrap_or_else(|p| p.into_inner()),
        MINOR_LONGLINK_CMD_MASK,
        "the app is handed the mask, and an app that writes one of its own drops it"
    );
}

#[test]
fn the_hash_is_what_the_response_is_judged_against() {
    let mut checker = app_that_checks_now();
    let _ = checker.get_identify_buffer();
    checker.set_id(42);

    let seen = Arc::new(Mutex::new(Vec::new()));
    let recording = Arc::clone(&seen);
    checker.set_on_response(move |channel, response, hash| {
        recording.lock().unwrap_or_else(|p| p.into_inner()).push((
            channel.to_owned(),
            response.to_vec(),
            hash.to_vec(),
        ));
        true
    });

    assert!(checker.on_identify_resp(b"resp"));
    assert!(checker.has_checked());
    assert_eq!(checker.taskid(), 0, "the response clears the task id");
    assert_eq!(
        *seen.lock().unwrap_or_else(|p| p.into_inner()),
        vec![("default".to_owned(), b"resp".to_vec(), b"hash".to_vec())]
    );

    // and a second one is not asked for any more
    assert!(checker.get_identify_buffer().is_none());
}

#[test]
fn the_response_has_to_be_the_one_that_was_asked_for() {
    let mut checker = app_that_checks_now();
    let _ = checker.get_identify_buffer();
    checker.set_id(42);
    checker.set_on_response(|_channel, _response, _hash| true);

    // `kLongLinkIdentifyCheckerTaskID` would be what `SetID` is called with
    assert!(checker.is_identify_resp(42));
    assert!(!checker.is_identify_resp(43));
    assert!(!checker.is_identify_resp(0), "a task id of 0 is no answer");

    // the app's verdict is what decides, and it clears the task id either way
    let mut refused = app_that_checks_now();
    refused.set_on_response(|_channel, _response, _hash| false);
    refused.set_id(42);
    assert!(!refused.on_identify_resp(b"resp"));
    assert!(!refused.has_checked());
    assert_eq!(refused.taskid(), 0);
}

#[test]
fn reset_lets_a_new_connection_ask_again() {
    let mut checker = LongLinkIdentifyChecker::new("default", false);
    checker.set_check_buffer(|_channel, _buffer, _hash, _cmdid| IdentifyMode::CheckNever);
    assert!(checker.get_identify_buffer().is_none());
    assert!(checker.has_checked());

    checker.reset();
    assert!(!checker.has_checked());
    assert_eq!(checker.cmd_id(), 0);
    assert_eq!(checker.taskid(), 0);
    assert_eq!(checker.hash_code(), b"");
}

#[test]
fn without_the_app_nothing_is_checked() {
    let mut checker = LongLinkIdentifyChecker::default();
    assert_eq!(checker.channel_id(), "");
    assert!(checker.get_identify_buffer().is_none());
    assert!(!checker.has_checked());
    assert!(!checker.on_identify_resp(b"resp"));
    assert!(!checker.has_checked());
    assert!(format!("{checker:?}").contains("LongLinkIdentifyChecker"));
}
