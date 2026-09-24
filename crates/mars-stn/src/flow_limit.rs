//! `mars/stn/src/flow_limit.h` — the funnel that caps how many bytes
//! `limit_flow` tasks may put on the air.
//!
//! A funnel of [`MAX_VOL`] bytes drains at [`ACTIVE_SPEED`] while the app is
//! foregrounded and at [`INACTIVE_SPEED`] in the background; a task whose body
//! does not fit is refused. The drain is recomputed on every call, from whole
//! seconds since the last one.

use crate::config::{ACTIVE_SPEED, INACTIVE_MIN_VOL, INACTIVE_SPEED, MAX_VOL};
use crate::task::Task;
use mars_comm::tickcount::gettickcount;

/// `FlowLimit`.
///
/// The `_at` variants take the tick count explicitly so that the drain — which
/// the C++ computes from whole seconds of wall clock — is testable without
/// sleeping; the plain ones are the C++ entry points against [`gettickcount`].
#[derive(Debug, Clone, Copy)]
pub struct FlowLimit {
    funnel_speed: u64,
    cur_funnel_vol: u64,
    time_lastflow_computer: u64,
}

impl FlowLimit {
    /// `FlowLimit(isactive)`.
    pub fn new(is_active: bool) -> Self {
        Self::new_at(is_active, gettickcount())
    }

    /// `FlowLimit(isactive)` with the funnel clocked at `now`.
    pub fn new_at(is_active: bool, now: u64) -> Self {
        Self {
            funnel_speed: if is_active {
                ACTIVE_SPEED
            } else {
                INACTIVE_SPEED
            },
            cur_funnel_vol: 0,
            time_lastflow_computer: now,
        }
    }

    /// `FlowLimit::Check(task, buffer, len)` — `false` when the task is refused.
    ///
    /// `len` is the size of the body, which is all the C++ looks at.
    pub fn check(&mut self, task: &Task, len: u64) -> bool {
        self.check_at(task, len, gettickcount())
    }

    /// `Check()` against an explicit tick count.
    pub fn check_at(&mut self, task: &Task, len: u64, now: u64) -> bool {
        if !task.limit_flow {
            return true;
        }
        self.flash_cur_vol(now);
        if self.cur_funnel_vol + len > MAX_VOL {
            return false;
        }
        self.cur_funnel_vol += len;
        true
    }

    /// `FlowLimit::Active(isactive)` — switching to the background also caps the
    /// volume that is still charged to the funnel.
    pub fn set_active(&mut self, is_active: bool) {
        self.set_active_at(is_active, gettickcount())
    }

    /// `Active(isactive)` against an explicit tick count.
    pub fn set_active_at(&mut self, is_active: bool, now: u64) {
        self.flash_cur_vol(now);
        if !is_active && self.cur_funnel_vol > INACTIVE_MIN_VOL {
            self.cur_funnel_vol = INACTIVE_MIN_VOL;
        }
        self.funnel_speed = if is_active {
            ACTIVE_SPEED
        } else {
            INACTIVE_SPEED
        };
    }

    /// Bytes currently charged to the funnel.
    pub fn current_volume(&self) -> u64 {
        self.cur_funnel_vol
    }

    /// Bytes per second the funnel drains at right now.
    pub fn funnel_speed(&self) -> u64 {
        self.funnel_speed
    }

    fn flash_cur_vol(&mut self, now: u64) {
        let interval = now.saturating_sub(self.time_lastflow_computer) / 1000;
        if interval == 0 {
            return;
        }
        let drained = interval * self.funnel_speed;
        self.cur_funnel_vol = self.cur_funnel_vol.saturating_sub(drained);
        self.time_lastflow_computer = now;
    }
}
