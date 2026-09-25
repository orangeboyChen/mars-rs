//! `mars/stn/task_profile.cc` — the three waits a task is given, as numbers.
//!
//! Upstream pins them in `mars/stn/test_cases/longlink_task_manager_test.cc`,
//! through `StartTask` on a long link it built to answer on cue; what those
//! cases are really asserting is `__FirstPkgTimeout`, `__ReadWriteTimeout` and
//! `TaskProfile::ComputeTaskTimeout`, which the short-link queue asks with the
//! same arguments. Here they are asked directly, over the table the C++ spreads
//! across its cases: what the network is, how long the request is, how many
//! tasks are already out, and what the app said about the server.
//!
//! Every number is the one the C++ computes, and so is every rounding: the
//! `1000 * _sendlen / rate` of the C++ is integer division, which is why a
//! 16-byte request buys one millisecond and not 1.3.

use mars_stn::dynamic_timeout::DynamicTimeoutStatus;
use mars_stn::task_profile::{compute_task_timeout, first_pkg_timeout, read_write_timeout};
use mars_stn::Task;

/// One row of the C++'s table: the arguments of `first_pkg_timeout`, and the
/// wait it answers with.
struct Wait {
    /// `task.server_process_cost` — what the app said the server needs.
    init: i64,
    /// How long the request is, in bytes.
    len: usize,
    /// How many tasks are already out on the link.
    count: i32,
    /// What the network has been like.
    status: DynamicTimeoutStatus,
    /// Whether the device is on a mobile network.
    mobile: bool,
    /// The wait, in milliseconds.
    expected: u64,
    /// What this row is.
    what: &'static str,
}

/// The table — wi-fi first, then the same questions on a mobile network, then
/// the two rows the app's own answer decides.
const WAITS: &[Wait] = &[
    // a request of no length at all on wi-fi: the base wait, nothing added
    Wait {
        init: 0,
        len: 0,
        count: 0,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: false,
        expected: 12_000,
        what: "the base wifi wait",
    },
    // 16 bytes: `1000 * 16 / 12288` is 1 ms, and the C++'s `test0` asks for
    // 12 000 because it rounds the fraction away — the port does not
    Wait {
        init: 0,
        len: 16,
        count: 0,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: false,
        expected: 12_001,
        what: "a 16-byte request is worth one millisecond",
    },
    // a megabyte: what it would buy is clipped to the maximum
    Wait {
        init: 0,
        len: 1 << 20,
        count: 0,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: false,
        expected: 22_000,
        what: "a megabyte on wifi is clipped to the wifi maximum",
    },
    Wait {
        init: 0,
        len: 1 << 20,
        count: 0,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: true,
        expected: 30_000,
        what: "a megabyte on a mobile network is clipped to the gprs maximum",
    },
    Wait {
        init: 0,
        len: 0,
        count: 0,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: true,
        expected: 15_000,
        what: "the base gprs wait",
    },
    // every task that is already out makes the wait longer — up to five of
    // them, which is the `std::min(_send_count, 5)` of the C++
    Wait {
        init: 0,
        len: 0,
        count: 1,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: false,
        expected: 13_500,
        what: "one task out on wifi",
    },
    Wait {
        init: 0,
        len: 0,
        count: 2,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: false,
        expected: 15_000,
        what: "two tasks out on wifi",
    },
    Wait {
        init: 0,
        len: 0,
        count: 3,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: false,
        expected: 16_500,
        what: "three tasks out on wifi",
    },
    Wait {
        init: 0,
        len: 0,
        count: 4,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: false,
        expected: 18_000,
        what: "four tasks out on wifi",
    },
    Wait {
        init: 0,
        len: 0,
        count: 5,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: false,
        expected: 19_500,
        what: "five tasks out on wifi",
    },
    Wait {
        init: 0,
        len: 0,
        count: 6,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: false,
        expected: 19_500,
        what: "six tasks out is five, on wifi",
    },
    Wait {
        init: 0,
        len: 0,
        count: 1,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: true,
        expected: 18_000,
        what: "one task out on a mobile network",
    },
    Wait {
        init: 0,
        len: 0,
        count: 6,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: true,
        expected: 30_000,
        what: "six tasks out is five, on a mobile network",
    },
    // a network that has been meeting its budgets is trusted with a shorter
    // wait, which is the one thing the dynamic timeout buys
    Wait {
        init: 0,
        len: 0,
        count: 0,
        status: DynamicTimeoutStatus::Excellent,
        mobile: false,
        expected: 7_000,
        what: "an excellent wifi network",
    },
    Wait {
        init: 0,
        len: 0,
        count: 0,
        status: DynamicTimeoutStatus::Excellent,
        mobile: true,
        expected: 10_000,
        what: "an excellent mobile network",
    },
    // ... unless the app said how long the server needs, which wins over the
    // network's own opinion and over the maximum
    Wait {
        init: 9_000,
        len: 0,
        count: 0,
        status: DynamicTimeoutStatus::Excellent,
        mobile: false,
        expected: 9_000,
        what: "what the app said beats an excellent network",
    },
    Wait {
        init: 9_000,
        len: 12_288,
        count: 0,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: false,
        expected: 10_000,
        what: "a second of transfer on wifi",
    },
    Wait {
        init: 9_000,
        len: 4_096,
        count: 0,
        status: DynamicTimeoutStatus::Evaluating,
        mobile: true,
        expected: 10_000,
        what: "a second of transfer on a mobile network",
    },
];

#[test]
fn the_first_pkg_wait_is_the_table_the_c_plus_plus_writes() {
    for row in WAITS {
        assert_eq!(
            first_pkg_timeout(row.init, row.len, row.count, row.status, row.mobile),
            row.expected,
            "{}",
            row.what
        );
    }
}

#[test]
fn the_read_write_wait_is_the_first_pkg_wait_plus_the_slowest_transfer() {
    // `1000 * kMaxRecvLen / rate`: the 64 KiB a read may bring in, at the
    // slowest rate the network is assumed to manage — 5 333 ms on wi-fi,
    // 16 000 ms on a mobile network
    assert_eq!(read_write_timeout(12_000, false), 17_333);
    assert_eq!(read_write_timeout(15_000, true), 31_000);
    assert_eq!(read_write_timeout(22_000, false), 27_333);
}

/// One row of `ComputeTaskTimeout`: the task, and the wait it is given.
struct Budget {
    task: Task,
    expected: u64,
    what: &'static str,
}

fn budget(expected: u64, what: &'static str, set: impl FnOnce(&mut Task)) -> Budget {
    let mut task = Task::new(1, 1);
    set(&mut task);
    Budget {
        task,
        expected,
        what,
    }
}

#[test]
fn the_whole_task_wait_grows_with_every_try_the_task_has_in_it() {
    let budgets = [
        // `(15s + 5s) * 1` — the wait itself, plus the margin, once
        budget(20_000, "a task with no retry in it", |_| {}),
        budget(40_000, "one retry", |task| task.retry_count = 1),
        budget(60_000, "two retries", |task| task.retry_count = 2),
        // `if (0 <= retry_count)`: a negative count is one try, not none
        budget(20_000, "a negative retry count", |task| {
            task.retry_count = -1
        }),
        // what the server said it needs is added to the wait itself
        budget(23_000, "a server that said it needs 3 s", |task| {
            task.server_process_cost = 3_000
        }),
        // a long-polling task is given its own wait and no more, whatever
        // the retries say
        budget(25_000, "a long-polling task", |task| {
            task.retry_count = 2;
            task.long_polling = true;
            task.long_polling_timeout = 20_000;
        }),
        // `total_timeout` is a ceiling: it cuts a wait down and never grows one
        budget(10_000, "a ceiling under the wait", |task| {
            task.retry_count = 2;
            task.total_timeout = 10_000;
        }),
        budget(60_000, "a ceiling over the wait", |task| {
            task.retry_count = 2;
            task.total_timeout = 100_000;
        }),
    ];

    for row in &budgets {
        assert_eq!(
            compute_task_timeout(&row.task),
            row.expected,
            "{}",
            row.what
        );
    }
}
