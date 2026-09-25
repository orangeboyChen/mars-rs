//! `mars/sdt` — the network diagnosis component, ported from
//! `mars/sdt/constants.h`, `mars/sdt/sdt.h`, `mars/sdt/netchecker_profile.h`,
//! `mars/sdt/src/sdt_core.cc` and `mars/sdt/sdt_logic.cc`.
//!
//! SDT ("smart diagnosis tool") answers "why can this phone not reach the
//! server?" by running ping, DNS, TCP and HTTP checks against the hosts STN
//! uses. The checks themselves are sockets, which this crate does not own: what
//! is here is the vocabulary (the profiles and the constants), the plan the mode
//! turns into, the run loop, and — in [`checkimpl`] and [`activecheck`] — the
//! four probes and the four checks around them, so a host only has to answer
//! the probes with a network of its own. [`trafficmonitor`] is the budget a run
//! of probes is given: how much traffic a diagnosis may cost the app.

pub mod activecheck;
pub mod checkimpl;
pub mod constants;
pub mod netchecker_profile;
pub mod sdt;
pub mod sdt_core;
pub mod sdt_logic;
pub mod trafficmonitor;

pub use constants::*;
pub use netchecker_profile::{CheckRequestProfile, CheckResultProfile};
pub use sdt::{
    Callback, CheckErrCode, CheckIPPort, CheckIPPorts, CheckStatus, CollectingCallback,
    NetCheckStatus, NetCheckType, TcpErrCode,
};
pub use sdt_core::SdtCore;
pub use sdt_logic::SdtLogic;
pub use trafficmonitor::{NetCheckTrafficMonitor, DEFAULT_WIFI_DATA_THRESHOLD};
