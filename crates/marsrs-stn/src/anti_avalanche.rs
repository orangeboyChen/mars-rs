//! `mars/stn/src/anti_avalanche.h` — the two gates a task has to pass before it
//! goes on the air: the frequency limit always, the flow limit on mobile only.

use crate::flow_limit::FlowLimit;
use crate::frequency_limit::FrequencyLimit;
use crate::task::Task;
use marsrs_comm::tickcount::gettickcount;

/// `kFrequencyLimit` / `kFlowLimit` — which gate refused a task.
///
/// `as_check_type()` is the number `ReportTaskLimited` is given, which is the
/// enum of `mars/stn/src/anti_avalanche.h` and not the order of these
/// variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    /// `kFrequencyLimit`: the same body was sent too often.
    Frequency,
    /// `kFlowLimit`: the byte budget is used up.
    Flow,
}

impl LimitKind {
    /// `kFrequencyLimit` / `kFlowLimit` — what the app is told was weighed.
    pub fn as_check_type(self) -> i32 {
        match self {
            LimitKind::Frequency => 1,
            LimitKind::Flow => 2,
        }
    }
}

/// The two gates are clocked from [`gettickcount`];
/// [`AntiAvalanche::check_at`] takes the tick count explicitly so the tests
/// can drive them without sleeping.
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
    /// `Err` carries the number `ReportTaskLimited` is handed with the kind:
    /// how long ago the same body went out for [`LimitKind::Frequency`], and
    /// how many bytes were refused for [`LimitKind::Flow`] — the `span` and
    /// the `_len` the C++ passes by reference.
    ///
    /// `network_is_mobile` is the `comm::kMobile == comm::getNetInfo()` of the
    /// C++: the flow limit only applies to a mobile network.
    pub fn check(
        &mut self,
        task: &Task,
        body: &[u8],
        network_is_mobile: bool,
    ) -> Result<(), (LimitKind, u32)> {
        self.check_at(task, body, network_is_mobile, gettickcount())
    }

    /// `Check()` against an explicit tick count.
    pub fn check_at(
        &mut self,
        task: &Task,
        body: &[u8],
        network_is_mobile: bool,
        now: u64,
    ) -> Result<(), (LimitKind, u32)> {
        let (allowed, span) = self.frequency_limit.check_at(task, body, now);
        if !allowed {
            return Err((
                LimitKind::Frequency,
                u32::try_from(span).unwrap_or(u32::MAX),
            ));
        }
        if network_is_mobile && !self.flow_limit.check_at(task, body.len() as u64, now) {
            return Err((
                LimitKind::Flow,
                u32::try_from(body.len()).unwrap_or(u32::MAX),
            ));
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

    /// The frequency gate: both gates are private fields, so this is the only
    /// way in for a host that wants to read the table it keeps.
    pub fn frequency_limit(&mut self) -> &mut FrequencyLimit {
        &mut self.frequency_limit
    }

    /// The flow gate, for the same reason: a host reads the funnel through it
    /// to see what is charged to it, and how fast it drains.
    pub fn flow_limit(&mut self) -> &mut FlowLimit {
        &mut self.flow_limit
    }
}
