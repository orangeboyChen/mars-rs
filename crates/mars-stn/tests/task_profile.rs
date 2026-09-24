//! `mars/stn/task_profile.h` — `GetFailStep()`, over the samples the C++
//! answers with.
//!
//! The step is decided by the error first and by how far the task got second,
//! and the value is what the weak-network report keys off.

use mars_stn::task_profile::{ErrCmdType, TaskFailStep, TaskOutcome};

/// The step of a task that is still the one `TaskProfile`'s constructor built:
/// no error, no ip, no package.
#[test]
fn a_task_that_was_never_run_has_no_fail_step() {
    let outcome = TaskOutcome::new(1_000, 2_000);
    assert_eq!(outcome.err_type, ErrCmdType::Ok);
    assert_eq!(outcome.err_code, 0);
    assert_eq!(outcome.fail_step(), TaskFailStep::Succ);
    assert_eq!(outcome.cost(), 1_000);
}

#[test]
fn the_error_type_decides_the_step() {
    for (err_type, expected) in [
        (ErrCmdType::Dns, TaskFailStep::Dns),
        (ErrCmdType::EnDecode, TaskFailStep::Decode),
        (ErrCmdType::Socket, TaskFailStep::PkgPkg),
        (ErrCmdType::Http, TaskFailStep::PkgPkg),
        (ErrCmdType::NetMsgXp, TaskFailStep::PkgPkg),
        (ErrCmdType::Server, TaskFailStep::Server),
    ] {
        // a task that got an ip and a package, so only the error is left
        let outcome = TaskOutcome {
            err_type,
            ip_index: 0,
            last_receive_pkg_time: 1_500,
            ..TaskOutcome::new(1_000, 2_000)
        };
        assert_eq!(outcome.fail_step(), expected, "{err_type:?}");
    }
}

#[test]
fn how_far_the_task_got_decides_the_step() {
    // no ip at all: the connect never happened
    let no_ip = TaskOutcome {
        err_type: ErrCmdType::Socket,
        ..TaskOutcome::new(1_000, 2_000)
    };
    assert_eq!(no_ip.fail_step(), TaskFailStep::Connect);

    // an ip but no package: the first package never came
    let no_pkg = TaskOutcome {
        err_type: ErrCmdType::Socket,
        ip_index: 0,
        ..TaskOutcome::new(1_000, 2_000)
    };
    assert_eq!(no_pkg.fail_step(), TaskFailStep::FirstPkg);
}

#[test]
fn a_local_timeout_is_a_timeout_whatever_the_type_says() {
    let outcome = TaskOutcome {
        err_type: ErrCmdType::Local,
        err_code: mars_stn::task_profile::LOCAL_TASK_TIMEOUT,
        ip_index: 0,
        last_receive_pkg_time: 1_500,
        ..TaskOutcome::new(1_000, 2_000)
    };
    assert_eq!(outcome.fail_step(), TaskFailStep::Timeout);
}

#[test]
fn a_task_that_the_server_answered_with_a_code_failed_at_the_server() {
    let outcome = TaskOutcome {
        err_code: -100,
        ip_index: 0,
        last_receive_pkg_time: 1_500,
        ..TaskOutcome::new(1_000, 2_000)
    };
    assert_eq!(
        outcome.fail_step(),
        TaskFailStep::Server,
        "kEctOK but a code"
    );
}

#[test]
fn anything_else_is_the_other_step() {
    for err_type in [ErrCmdType::False, ErrCmdType::Dial, ErrCmdType::Canceld] {
        let outcome = TaskOutcome {
            err_type,
            err_code: mars_stn::task_profile::LOCAL_CANCEL,
            ip_index: 0,
            last_receive_pkg_time: 1_500,
            ..TaskOutcome::new(1_000, 2_000)
        };
        assert_eq!(outcome.fail_step(), TaskFailStep::Other, "{err_type:?}");
    }
}

#[test]
fn the_discriminants_are_the_ones_the_report_adds_to_its_offset() {
    assert_eq!(TaskFailStep::Succ as i32, 0);
    assert_eq!(TaskFailStep::Dns as i32, 1);
    assert_eq!(TaskFailStep::Connect as i32, 2);
    assert_eq!(TaskFailStep::FirstPkg as i32, 3);
    assert_eq!(TaskFailStep::PkgPkg as i32, 4);
    assert_eq!(TaskFailStep::Decode as i32, 5);
    assert_eq!(TaskFailStep::Other as i32, 6);
    assert_eq!(TaskFailStep::Timeout as i32, 7);
    assert_eq!(TaskFailStep::Server as i32, 8);

    assert_eq!(ErrCmdType::Ok as i32, 0);
    assert_eq!(ErrCmdType::Canceld as i32, 10);
    assert_eq!(mars_stn::task_profile::LOCAL_TASK_TIMEOUT, -1);
    assert_eq!(mars_stn::task_profile::LONG_FIRST_PKG_TIMEOUT, -500);
}
