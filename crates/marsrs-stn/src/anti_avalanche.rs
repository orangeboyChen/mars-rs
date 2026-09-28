//! `mars/stn/src/anti_avalanche.h` — the two gates a task has to pass before it
//! goes on the air: the frequency limit always, the flow limit on mobile only.
//!
//! A task a gate refused is not one the app never hears of again:
//! `mars/stn/src/anti_avalanche.cc` reports it (`ReportTaskLimited`, `:47` for
//! the frequency limit and `:52` for the flow one), and the app answers with
//! the limit it would rather be held to. [`AntiAvalanche::set_on_limited`] is
//! what the gate asks here.

use std::sync::{Arc, Mutex, PoisonError};

use crate::flow_limit::FlowLimit;
use crate::frequency_limit::FrequencyLimit;
use crate::task::Task;
use marsrs_comm::tickcount::gettickcount;

/// `kFrequencyLimit` / `kFlowLimit` — which gate refused a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    /// `kFrequencyLimit`: the same body was sent too often.
    Frequency,
    /// `kFlowLimit`: the byte budget is used up.
    Flow,
}

impl LimitKind {
    /// `kFrequencyLimit` / `kFlowLimit` as the `_check_type` the C++ reports a
    /// task with: `1` and `2`, which is how `mars/stn/src/anti_avalanche.h`
    /// declares them.
    pub fn check_type(self) -> i32 {
        match self {
            Self::Frequency => 1,
            Self::Flow => 2,
        }
    }
}

/// `ReportTaskLimited` — the app a refused task is reported to, and the limit
/// it answers with.
///
/// `_param` goes in as the measure the gate refused the task by: how long ago
/// the same body went out last for [`LimitKind::Frequency`], the length of the
/// body for [`LimitKind::Flow`]. The answer comes back out of the same value,
/// which is how an app shrinks a limit it is being held to — the C++'s own
/// callers hand `Check` a temporary `_len`, so what the app answered never
/// reaches the verdict: the task stays refused.
pub type OnLimited = dyn FnMut(LimitKind, &Task, u32) -> u32 + Send;

/// The two gates are clocked from [`gettickcount`];
/// [`AntiAvalanche::check_at`] takes the tick count explicitly so the tests
/// can drive them without sleeping.
#[derive(Clone)]
pub struct AntiAvalanche {
    frequency_limit: FrequencyLimit,
    flow_limit: FlowLimit,
    on_limited: Option<Arc<Mutex<OnLimited>>>,
}

impl std::fmt::Debug for AntiAvalanche {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AntiAvalanche")
            .field("frequency_limit", &self.frequency_limit)
            .field("flow_limit", &self.flow_limit)
            .field("on_limited", &self.on_limited.is_some())
            .finish()
    }
}

impl AntiAvalanche {
    /// `AntiAvalanche(context, isactive)`.
    pub fn new(is_active: bool) -> Self {
        Self {
            frequency_limit: FrequencyLimit::new(),
            flow_limit: FlowLimit::new(is_active),
            on_limited: None,
        }
    }

    /// `AntiAvalanche(context, isactive)` with both gates clocked at `now`.
    pub fn new_at(is_active: bool, now: u64) -> Self {
        Self {
            frequency_limit: FrequencyLimit::new_at(now),
            flow_limit: FlowLimit::new_at(is_active, now),
            on_limited: None,
        }
    }

    /// `AntiAvalanche::Check(task, buffer, len)`.
    ///
    /// `network_is_mobile` is the `comm::kMobile == comm::getNetInfo()` of the
    /// C++: the flow limit only applies to a mobile network.
    ///
    /// A task a gate refused is reported to the app first, and what the app
    /// answered is left the way the C++ leaves it: the verdict is `Err` either
    /// way.
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
        let (allowed, span) = self.frequency_limit.check_at(task, body, now);
        if !allowed {
            self.report_limited(LimitKind::Frequency, task, param_of(span));
            return Err(LimitKind::Frequency);
        }
        if network_is_mobile && !self.flow_limit.check_at(task, body.len() as u64, now) {
            self.report_limited(LimitKind::Flow, task, param_of(body.len() as u64));
            return Err(LimitKind::Flow);
        }
        Ok(())
    }

    /// `ReportTaskLimited` — the app a refused task is reported to.
    ///
    /// The C++'s gate reaches the `StnManager` from the boot context it was
    /// made with; this one is a value with no context in it, so the app is the
    /// hook it was handed.
    pub fn set_on_limited(
        &mut self,
        on_limited: impl FnMut(LimitKind, &Task, u32) -> u32 + Send + 'static,
    ) {
        self.on_limited = Some(Arc::new(Mutex::new(on_limited)));
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

    /// The report itself, which a gate with no app in it does not make: the
    /// C++ asserts its callback is there, where a port that has not been wired
    /// yet has nobody to ask.
    fn report_limited(&mut self, kind: LimitKind, task: &Task, param: u32) {
        if let Some(on_limited) = self.on_limited.as_ref() {
            let _ = on_limited.lock().unwrap_or_else(poisoned)(kind, task, param);
        }
    }
}

/// `_param` — the `unsigned int` the C++ reports a task with. A measure that
/// does not fit one is capped rather than wrapped, which is the only reading
/// of it an app can act on.
fn param_of(measure: u64) -> u32 {
    u32::try_from(measure).unwrap_or(u32::MAX)
}

/// The gate, without letting a panic in one thread take every thread with it.
fn poisoned<T>(poisoned: PoisonError<T>) -> T {
    poisoned.into_inner()
}
