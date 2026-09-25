//! `mars/stn/task_profile.h` — the outcome of a finished task.
//!
//! The C++ `TaskProfile` is the whole record of a task that is running: the
//! [`crate::Task`] it came from, the prepare/transfer/connect profiles, the
//! history of its retries, and the fields `stn_logic` fills in as it goes.
//! What is here is what the code that reads a *finished* task needs: the two
//! error fields, the two times, and the two numbers of the transfer profile
//! that `TaskProfile::GetFailStep()` looks at. The rest of the struct comes
//! with the code that runs the tasks.
//!
//! `ErrCmdType` is the one of `mars/stn/stn.h`. Its companion is not an enum
//! there either: `err_code` is an `int` that carries either a server code or
//! one of the `kEctLocal*` values, so those are constants here.

/// `ErrCmdType` of `mars/stn/stn.h` — how a task failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrCmdType {
    /// `kEctOK`
    #[default]
    Ok = 0,
    /// `kEctFalse`
    False = 1,
    /// `kEctDial`
    Dial = 2,
    /// `kEctDns`
    Dns = 3,
    /// `kEctSocket`
    Socket = 4,
    /// `kEctHttp`
    Http = 5,
    /// `kEctNetMsgXP`
    NetMsgXp = 6,
    /// `kEctEnDecode`
    EnDecode = 7,
    /// `kEctServer`
    Server = 8,
    /// `kEctLocal`
    Local = 9,
    /// `kEctCanceld`
    Canceld = 10,
}

/// `kEctLocalTaskTimeout` — what `TaskProfile::err_code` carries when the task
/// timed out locally.
pub const LOCAL_TASK_TIMEOUT: i32 = -1;
/// `kEctLocalTaskRetry`.
pub const LOCAL_TASK_RETRY: i32 = -2;
/// `kEctLocalStartTaskFail`.
pub const LOCAL_START_TASK_FAIL: i32 = -3;
/// `kEctLocalAntiAvalanche`.
pub const LOCAL_ANTI_AVALANCHE: i32 = -4;
/// `kEctLocalChannelSelect`.
pub const LOCAL_CHANNEL_SELECT: i32 = -5;
/// `kEctLocalNoNet`.
pub const LOCAL_NO_NET: i32 = -6;
/// `kEctLocalCancel`.
pub const LOCAL_CANCEL: i32 = -7;
/// `kEctLocalClear`.
pub const LOCAL_CLEAR: i32 = -8;
/// `kEctLocalReset`.
pub const LOCAL_RESET: i32 = -9;
/// `kEctLocalTaskParam`.
pub const LOCAL_TASK_PARAM: i32 = -12;
/// `kEctLocalCgiFrequcencyLimit`.
pub const LOCAL_CGI_FREQUENCY_LIMIT: i32 = -13;
/// `kEctLocalChannelID`.
pub const LOCAL_CHANNEL_ID: i32 = -14;
/// `kEctLocalLongLinkReleased`.
pub const LOCAL_LONG_LINK_RELEASED: i32 = -15;
/// `kEctLocalLongLinkUnAvailable`.
pub const LOCAL_LONG_LINK_UNAVAILABLE: i32 = -16;
/// `kEctLongFirstPkgTimeout`.
pub const LONG_FIRST_PKG_TIMEOUT: i32 = -500;
/// `kEctLongPkgPkgTimeout`.
pub const LONG_PKG_PKG_TIMEOUT: i32 = -501;

/// `TaskFailStep` — "do not insert or delete": the C++ turns the value into a
/// report key by adding it to an offset, so the discriminants are part of the
/// contract with the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskFailStep {
    /// `kStepSucc`
    Succ = 0,
    /// `kStepDns`
    Dns = 1,
    /// `kStepConnect`
    Connect = 2,
    /// `kStepFirstPkg`
    FirstPkg = 3,
    /// `kStepPkgPkg`
    PkgPkg = 4,
    /// `kStepDecode`
    Decode = 5,
    /// `kStepOther`
    Other = 6,
    /// `kStepTimeout`
    Timeout = 7,
    /// `kStepServer`
    Server = 8,
}

/// What a finished task reports.
///
/// `start_task_time` and `end_task_time` are `::gettickcount()` readings; the
/// other four fields are the ones `TaskProfile::GetFailStep()` and
/// `WeakNetworkLogic::OnTaskEvent()` read, with the defaults the C++
/// constructor gives them (`ip_index` is `-1`, `last_receive_pkg_time` is `0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskOutcome {
    /// `err_type`
    pub err_type: ErrCmdType,
    /// `err_code` — 0, a server code, or one of the `LOCAL_*` constants.
    pub err_code: i32,
    /// `transfer_profile.connect_profile.ip_index` — `-1` when the task never
    /// got an ip to connect to.
    pub ip_index: i32,
    /// `transfer_profile.last_receive_pkg_time` — `0` when no package ever
    /// arrived.
    pub last_receive_pkg_time: u64,
    /// `start_task_time`
    pub start_task_time: u64,
    /// `end_task_time` — 0 while the task has not finished.
    pub end_task_time: u64,
}

impl TaskOutcome {
    /// A task that succeeded: `err_type` is `kEctOK` and `err_code` is 0.
    pub fn new(start_task_time: u64, end_task_time: u64) -> Self {
        Self {
            err_type: ErrCmdType::Ok,
            err_code: 0,
            ip_index: -1,
            last_receive_pkg_time: 0,
            start_task_time,
            end_task_time,
        }
    }

    /// How the task failed, by the same rules as the C++: `err_type` first,
    /// then how far the task got.
    pub fn fail_step(&self) -> TaskFailStep {
        if self.err_type == ErrCmdType::Ok && self.err_code == 0 {
            return TaskFailStep::Succ;
        }
        if self.err_type == ErrCmdType::Dns {
            return TaskFailStep::Dns;
        }
        if self.ip_index == -1 {
            return TaskFailStep::Connect;
        }
        if self.last_receive_pkg_time == 0 {
            return TaskFailStep::FirstPkg;
        }
        if self.err_type == ErrCmdType::EnDecode {
            return TaskFailStep::Decode;
        }
        if self.err_type == ErrCmdType::Socket
            || self.err_type == ErrCmdType::Http
            || self.err_type == ErrCmdType::NetMsgXp
        {
            return TaskFailStep::PkgPkg;
        }
        if self.err_code == LOCAL_TASK_TIMEOUT {
            return TaskFailStep::Timeout;
        }
        if self.err_type == ErrCmdType::Server
            || (self.err_type == ErrCmdType::Ok && self.err_code != 0)
        {
            return TaskFailStep::Server;
        }
        TaskFailStep::Other
    }

    /// `end_task_time - start_task_time` — what the C++ compares against
    /// `GOOD_TASK_SPAN` and `WEAK_TASK_SPAN`.
    pub fn cost(&self) -> u64 {
        self.end_task_time.saturating_sub(self.start_task_time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_that_succeeded_has_no_fail_step() {
        let outcome = TaskOutcome::new(100, 700);
        assert_eq!(outcome.err_type, ErrCmdType::Ok);
        assert_eq!(outcome.fail_step(), TaskFailStep::Succ);
        assert_eq!(outcome.cost(), 600);
        // and it never got an ip, which is what the C++ starts from
        assert_eq!(outcome.ip_index, -1);
        assert_eq!(outcome.last_receive_pkg_time, 0);
    }

    #[test]
    fn the_fail_step_is_decided_by_the_error_first() {
        let dns = TaskOutcome {
            err_type: ErrCmdType::Dns,
            ip_index: 2,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(dns.fail_step(), TaskFailStep::Dns, "before the ip check");

        let decode = TaskOutcome {
            err_type: ErrCmdType::EnDecode,
            ip_index: 2,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(decode.fail_step(), TaskFailStep::Decode);
    }

    #[test]
    fn the_fail_step_is_decided_by_how_far_the_task_got() {
        let no_ip = TaskOutcome {
            err_type: ErrCmdType::Socket,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(no_ip.fail_step(), TaskFailStep::Connect);

        let no_pkg = TaskOutcome {
            err_type: ErrCmdType::Socket,
            ip_index: 0,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(no_pkg.fail_step(), TaskFailStep::FirstPkg);

        let pkg_pkg = TaskOutcome {
            err_type: ErrCmdType::Socket,
            ip_index: 0,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(pkg_pkg.fail_step(), TaskFailStep::PkgPkg);
    }

    #[test]
    fn a_local_timeout_is_a_timeout_and_a_server_code_is_the_server() {
        let timeout = TaskOutcome {
            err_type: ErrCmdType::Local,
            err_code: LOCAL_TASK_TIMEOUT,
            ip_index: 0,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(timeout.fail_step(), TaskFailStep::Timeout);

        let server = TaskOutcome {
            err_type: ErrCmdType::Server,
            ip_index: 0,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(server.fail_step(), TaskFailStep::Server);

        // a success with a server code is the server's fault too
        let code = TaskOutcome {
            err_code: -100,
            ip_index: 0,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(code.fail_step(), TaskFailStep::Server);

        // ... and anything else is `kStepOther`
        let other = TaskOutcome {
            err_type: ErrCmdType::Canceld,
            err_code: LOCAL_CANCEL,
            ip_index: 0,
            last_receive_pkg_time: 300,
            ..TaskOutcome::new(0, 0)
        };
        assert_eq!(other.fail_step(), TaskFailStep::Other);
    }
}
