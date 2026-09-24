//! STN — the task pipeline of mars — ported from `mars/stn`.
//!
//! The first slice is the part that has no sockets and no callbacks: the task
//! model and the three policies that decide whether a task may go out at all
//! (anti-avalanche: frequency, flow, and the dynamic timeout status). They are
//! pure logic over [`mars_comm::tickcount::gettickcount`], which is what makes
//! them testable here.
//!
//! Everything that needs a network or the app callbacks (`net_core`, `longlink`,
//! `shortlink`, `stn_logic`) comes later.

pub mod anti_avalanche;
pub mod config;
pub mod dynamic_timeout;
pub mod flow_limit;
pub mod frequency_limit;
pub mod task;

pub use anti_avalanche::{AntiAvalanche, LimitKind};
pub use dynamic_timeout::{DynamicTimeout, DynamicTimeoutStatus};
pub use flow_limit::FlowLimit;
pub use frequency_limit::FrequencyLimit;
pub use task::{HostRedirectType, Task};
