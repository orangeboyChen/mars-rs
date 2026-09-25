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
//! Everything that needs the app callbacks (`net_core`, `longlink_task_manager`,
//! `shortlink`, `stn_logic`) comes later.

/// The `static`s of this crate are one value for the whole process —
/// `sg_client_version` in [`longlink`] and `outer_setted_heart_` in
/// [`smart_heartbeat`] — so the unit tests that move them need **one** lock for
/// the crate, not one per module.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub mod anti_avalanche;
pub mod config;
pub mod dynamic_timeout;
pub mod flow_limit;
pub mod frequency_limit;
pub mod longlink;
pub mod smart_heartbeat;
pub mod task;
pub mod task_profile;
pub mod weak_network;

pub use anti_avalanche::{AntiAvalanche, LimitKind};
pub use dynamic_timeout::{DynamicTimeout, DynamicTimeoutStatus};
pub use flow_limit::FlowLimit;
pub use frequency_limit::FrequencyLimit;
pub use longlink::{LongLinkEncoder, Unpacked};
pub use smart_heartbeat::{
    NetHeartbeatInfo, SmartHeartBeatAction, SmartHeartBeatType, SmartHeartbeat,
};
pub use task::{HostRedirectType, Task};
pub use task_profile::{ErrCmdType, TaskFailStep, TaskOutcome};
pub use weak_network::{ReportWeak, WeakKey, WeakNetworkLogic};
