//! STN — the task pipeline of mars — ported from `mars/stn`.
//!
//! The first slice is the part that has no sockets and no callbacks: the task
//! model and the three policies that decide whether a task may go out at all
//! (anti-avalanche: frequency, flow, and the dynamic timeout status). They are
//! pure logic over [`mars_comm::tickcount::gettickcount`], which is what makes
//! them testable here.
//!
//! The second slice is the long link: its wire format ([`longlink`]) and the
//! heartbeat interval that keeps it alive ([`smart_heartbeat`]). Neither needs
//! a socket either — the packer is bytes in, bytes out, and the heartbeat
//! takes the answers as arguments — but the long link that *uses* them does,
//! so what is here is the format and the computation, not the connection.
//!
//! The third slice is what a task that ran leaves behind ([`task_profile`]) and
//! the logic that calls the network weak ([`weak_network`]): the error a task
//! failed with and how far it got decide whether the network is weak, and the
//! report the C++ hands to the app is a `(key, value)` pair the port hands to a
//! callback instead.
//!
//! The fourth slice is what keeps the long link trusted and the mapping alive:
//! the identify check the app answers before the connection is used
//! ([`longlink_identify_checker`]) and the signalling that keeps a mapping up
//! while the app waits ([`signalling_keeper`]).
//!
//! The fifth slice is the task that outlived the link it was started on
//! ([`zombie_task_manager`]): it is saved instead of failed, and started again
//! when the link comes back or when its periodic check decides it waited long
//! enough.
//!
//! The sixth slice is which ip/port pair a task is tried on
//! ([`simple_ipport_sort`]): one bit per attempt decides whether a pair is used
//! at all and in which order, and the history behind it is what survives
//! between runs — the port has no filesystem, so the host loads and saves it.
//!
//! The seventh slice is how soon the long link is tried again
//! ([`longlink_connect_monitor`]): an interval out of a table, and a ladder it
//! walks while the app is in the background.
//!
//! Everything that needs the app callbacks (`net_core`, `longlink_task_manager`,
//! `shortlink`, `stn_logic`) comes later.

/// The `static`s of this crate are one value for the whole process —
/// `sg_client_version` in [`longlink`], `outer_setted_heart_` in
/// [`smart_heartbeat`], and `g_period` / `g_keepTime` in
/// [`signalling_keeper`] — so the unit tests that move them need **one** lock
/// for the crate, not one per module.
/// `rand()` — the port's own, so a test does not have to care which numbers the
/// platform would have handed out.
///
/// The modules that need one take it as a [`FnMut`] a host can replace, and
/// this is what they start with: a xorshift over `seed`.
pub(crate) fn xorshift(seed: u64) -> impl FnMut(usize) -> usize + Send + 'static {
    let mut state = if seed == 0 {
        0x9E37_79B9_7F4A_7C15
    } else {
        seed
    };
    move |bound: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        if bound == 0 {
            0
        } else {
            (state % bound as u64) as usize
        }
    }
}

#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `rand()` — what the platform hands out in the C++: a number in `0..bound`.
///
/// Both [`simple_ipport_sort`] and [`longlink_connect_monitor`] take one as a
/// callback, which is what makes the shuffle and the jitter a test can pin
/// down.
pub type Random = dyn FnMut(usize) -> usize + Send;

pub mod anti_avalanche;
pub mod config;
pub mod dynamic_timeout;
pub mod flow_limit;
pub mod frequency_limit;
pub mod longlink;
pub mod longlink_connect_monitor;
pub mod longlink_identify_checker;
pub mod signalling_keeper;
pub mod simple_ipport_sort;
pub mod smart_heartbeat;
pub mod task;
pub mod task_profile;
pub mod weak_network;
pub mod zombie_task_manager;

pub use anti_avalanche::{AntiAvalanche, LimitKind};
pub use dynamic_timeout::{DynamicTimeout, DynamicTimeoutStatus};
pub use flow_limit::FlowLimit;
pub use frequency_limit::FrequencyLimit;
pub use longlink::{LongLinkEncoder, Unpacked};
pub use longlink_connect_monitor::{
    ActiveState, ConnectType, LongLinkConnectMonitor, LongLinkStatus, INACTIVE_BUFFER,
    INTERNAL_MAX_INDEX, INTERVALS, NO_ACCOUNT_INFO_INACTIVE_INTERVAL, NO_ACCOUNT_INFO_SALT_RATE,
    NO_ACCOUNT_INFO_SALT_RISE, NO_NET_SALT_RATE, NO_NET_SALT_RISE, RECONNECT_INTERVAL,
    START_CHECK_PERIOD, TIME_CHECK_PERIOD, UP_OR_DOWN_THRESHOLD, WAKE_ALARM_INTERVAL,
};
pub use longlink_identify_checker::{IdentifyMode, LongLinkIdentifyChecker};
pub use signalling_keeper::{SignallingKeeper, DEFAULT_KEEP_TIME, DEFAULT_PERIOD};
pub use simple_ipport_sort::{
    BanItem, IpPortItem, IpSourceType, Record, RecordItem, SimpleIpPortSort, BAN_FAIL_COUNT,
    BAN_TIME, FAIL_UPDATE_INTERVAL, MAX_BAN_TIME, RECORD_TIMEOUT, SERVER_BAN_TIME,
    SUCCESS_UPDATE_INTERVAL,
};
pub use smart_heartbeat::{
    NetHeartbeatInfo, SmartHeartBeatAction, SmartHeartBeatType, SmartHeartbeat,
};
pub use task::{HostRedirectType, Task};
pub use task_profile::{ErrCmdType, TaskFailHandleType, TaskFailStep, TaskOutcome};
pub use weak_network::{ReportWeak, WeakKey, WeakNetworkLogic};
pub use zombie_task_manager::ZombieTaskManager;
