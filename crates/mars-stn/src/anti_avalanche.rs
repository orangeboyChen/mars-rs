//! `mars/stn/src/anti_avalanche.h` — the two gates a task has to pass before it
//! goes on the air: the frequency limit always, the flow limit on mobile only.

use crate::flow_limit::FlowLimit;
use crate::frequency_limit::FrequencyLimit;
use crate::task::Task;
use mars_comm::tickcount::gettickcount;

/// `kFrequencyLimit` / `kFlowLimit` — which gate refused a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    /// `kFrequencyLimit`: the same body was sent too often.
    Frequency,
    /// `kFlowLimit`: the byte budget is used up.
    Flow,
}

/// `AntiAvalanche`.
///
/// The two gates are clocked from [`gettickcount`]; [`AntiAvalanche::check_at`]
/// takes the tick count explicitly so the tests can drive them.
#[derive(Debug, Clone)]
pub struct AntiAvalanche {
    frequency_limit: FrequencyLimit,
    flow_limit: FlowLimit,
}

impl AntiAvalanche {
    /// `AntiAvalanche(context, isactive)`.
    pub fn new(is_active: bool) -> Self {
        Self {
            frequency_limit: FrequencyLimit::new(),
            flow_limit: FlowLimit::new(is_active),
        }
    }

    /// `AntiAvalanche(context, isactive)` with both gates clocked at `now`.
    pub fn new_at(is_active: bool, now: u64) -> Self {
        Self {
            frequency_limit: FrequencyLimit::new_at(now),
            flow_limit: FlowLimit::new_at(is_active, now),
        }
    }

    /// `AntiAvalanche::Check(task, buffer, len)`.
    ///
    /// `network_is_mobile` is the `comm::kMobile == comm::getNetInfo()` of the
    /// C++: the flow limit only applies to a mobile network.
    pub fn check(
        &mut self,
        task: &Task,
        body: &[u8],
        network_is_mobile: bool,
    ) -> Result<(), LimitKind> {
        self.check_at(task, body, network_is_mobile, gettickcount())
    }

    /// `Check()` against an explicit tick count.
    pub fn check_at(
        &mut self,
        task: &Task,
        body: &[u8],
        network_is_mobile: bool,
        now: u64,
    ) -> Result<(), LimitKind> {
        let (allowed, _span) = self.frequency_limit.check_at(task, body, now);
        if !allowed {
            return Err(LimitKind::Frequency);
        }
        if network_is_mobile && !self.flow_limit.check_at(task, body.len() as u64, now) {
            return Err(LimitKind::Flow);
        }
        Ok(())
    }

    /// `AntiAvalanche::OnSignalActive(isactive)`.
    pub fn on_signal_active(&mut self, is_active: bool) {
        self.set_active_at(is_active, gettickcount())
    }

    /// `OnSignalActive(isactive)` against an explicit tick count.
    pub fn set_active_at(&mut self, is_active: bool, now: u64) {
        self.flow_limit.set_active_at(is_active, now);
    }

    /// The frequency gate, for callers that report its `span`.
    pub fn frequency_limit(&mut self) -> &mut FrequencyLimit {
        &mut self.frequency_limit
    }

    /// The flow gate.
    pub fn flow_limit(&mut self) -> &mut FlowLimit {
        &mut self.flow_limit
    }
}
