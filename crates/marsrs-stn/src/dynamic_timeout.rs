//! `mars/stn/src/dynamic_timeout.h` — "is the network good right now?"
//!
//! Every finished task is classified (met its budget / finished normally /
//! failed) and the last ten of those are kept in a sliding window. Enough good
//! packages in a row make the status `Excellent`, too many failures make it
//! `Bad`; the C++ picks the first-package timeout of the next task from it.
//!
//! One of them answers for the whole process, and not one per queue: the
//! C++'s `NetCore` owns it and hands the *same* one to the short-link and the
//! long-link queue (`mars/stn/src/net_core.cc:85,217`), so a package that
//! went out on one of them is what the other computes its timeouts from. A
//! [`DynamicTimeout`] is therefore a handle, and a clone of one is another
//! handle on the same judgement rather than a copy of it.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::config::*;
use marsrs_comm::tickcount::gettickcount;

/// A lock another thread panicked in: the window is a handful of counters, so
/// the reading in it is taken as it is.
fn poisoned(poisoned: PoisonError<MutexGuard<'_, Window>>) -> MutexGuard<'_, Window> {
    poisoned.into_inner()
}

/// How good the network looks from the last ten packages: the C++ picks the
/// first-package timeout from it, and `Excellent` is what buys a shorter one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DynamicTimeoutStatus {
    /// `kEValuating` — not enough history yet.
    #[default]
    Evaluating = 1,
    /// `kExcellent` — the network consistently met its budgets.
    Excellent,
    /// `kBad` — too many of the last packages failed.
    Bad,
}

/// Which network a task ran on: the budgets a package is measured against are
/// tighter on Wi-Fi than on mobile, so the two are classified apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkKind {
    /// Wi-Fi: the tighter budgets.
    Wifi,
    /// Mobile: the looser budgets.
    Mobile,
}

/// The classification of one finished task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskTag {
    /// `kDynTimeTaskMeetExpectTag` — a small package within budget.
    MeetExpect = 1,
    /// `kDynTimeTaskMidPkgMeetExpectTag` — a middle package within budget.
    MidPkgMeetExpect = 2,
    /// `kDynTimeTaskBigPkgMeetExpectTag` — a big package within budget.
    BigPkgMeetExpect = 3,
    /// `kDynTimeTaskBiggerPkgMeetExpectTag` — anything bigger, within budget.
    BiggerPkgMeetExpect = 4,
    /// `KDynTimeTaskNormalTag` — finished, but slower than its budget.
    Normal = 0,
    /// `kDynTimeTaskFailedTag` — never finished at all.
    Failed = -1,
}

impl TaskTag {
    fn of(network: NetworkKind, total_size: u32, cost_time: u64) -> Self {
        if total_size == DYN_TIME_TASK_FAILED_PKG_LEN || cost_time == 0 {
            return Self::Failed;
        }
        let (small, middle, big, bigger) = match network {
            NetworkKind::Wifi => (
                DYN_TIME_SMALL_PACKAGE_WIFI_COSTTIME,
                DYN_TIME_MIDDLE_PACKAGE_WIFI_COSTTIME,
                DYN_TIME_BIG_PACKAGE_WIFI_COSTTIME,
                DYN_TIME_BIGGER_PACKAGE_WIFI_COSTTIME,
            ),
            NetworkKind::Mobile => (
                DYN_TIME_SMALL_PACKAGE_GPRS_COSTTIME,
                DYN_TIME_MIDDLE_PACKAGE_GPRS_COSTTIME,
                DYN_TIME_BIG_PACKAGE_GPRS_COSTTIME,
                DYN_TIME_BIGGER_PACKAGE_GPRS_COSTTIME,
            ),
        };
        if total_size < DYN_TIME_SMALL_PACKAGE_LEN {
            if cost_time <= small {
                return Self::MeetExpect;
            }
        } else if total_size <= DYN_TIME_MIDDLE_PACKAGE_LEN {
            if cost_time <= middle {
                return Self::MidPkgMeetExpect;
            }
        } else if total_size <= DYN_TIME_BIG_PACKAGE_LEN {
            if cost_time <= big {
                return Self::BigPkgMeetExpect;
            }
        } else if cost_time <= bigger {
            return Self::BiggerPkgMeetExpect;
        }
        Self::Normal
    }
}

/// [`DynamicTimeout::record_at`] takes the tick count explicitly so that the
/// five-minute expiry of the sliding window is testable without waiting;
/// [`DynamicTimeout::record`] is the `CgiTaskStatistic()` of the C++ against
/// [`gettickcount`].
#[derive(Debug, Clone, Default)]
pub struct DynamicTimeout {
    window: Arc<Mutex<Window>>,
}

/// What the C++'s `DynamicTimeout` holds, and the port's [`DynamicTimeout`]
/// shares: one window per process, and not one per queue.
#[derive(Debug, Clone)]
struct Window {
    status: DynamicTimeoutStatus,
    continuous_good_count: u32,
    latest_bigpkg_goodtime: u64,
    /// The last ten sends, `true` when one finished normally or better.
    history: [bool; 10],
    fncount_latest_modify_time: u64,
    pos: usize,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            status: DynamicTimeoutStatus::Evaluating,
            continuous_good_count: 0,
            latest_bigpkg_goodtime: 0,
            history: [true; 10],
            fncount_latest_modify_time: 0,
            pos: 0,
        }
    }
}

impl DynamicTimeout {
    /// `DynamicTimeout()` — a window of its own. [`NetCore`](crate::NetCore)
    /// makes one and hands it to both of its queues, which is what makes the
    /// network's judgement one the whole core shares.
    pub fn new() -> Self {
        Self::default()
    }

    /// `DynamicTimeout::CgiTaskStatistic(cgi, total_size, cost_time)`.
    pub fn record(&self, network: NetworkKind, total_size: u32, cost_time: u64) {
        self.record_at(network, total_size, cost_time, gettickcount())
    }

    /// `CgiTaskStatistic()` against an explicit tick count.
    pub fn record_at(&self, network: NetworkKind, total_size: u32, cost_time: u64, now: u64) {
        self.window
            .lock()
            .unwrap_or_else(poisoned)
            .switch(TaskTag::of(network, total_size, cost_time), now);
    }

    /// `DynamicTimeout::GetStatus()`.
    pub fn status(&self) -> DynamicTimeoutStatus {
        self.window.lock().unwrap_or_else(poisoned).status
    }

    /// How many packages in a row met their budget: one that only finished, or
    /// failed, puts it back to zero, and it only counts while the status is
    /// still `Evaluating`.
    pub fn continuous_good_count(&self) -> u32 {
        self.window
            .lock()
            .unwrap_or_else(poisoned)
            .continuous_good_count
    }

    /// How many of the last ten packages came in at all: six or fewer call the
    /// network bad, and more than that bring it back out of it.
    pub fn normal_count(&self) -> usize {
        self.window.lock().unwrap_or_else(poisoned).normal_count()
    }

    /// `DynamicTimeout::ResetStatus()`.
    pub fn reset(&self) {
        *self.window.lock().unwrap_or_else(poisoned) = Window::default();
    }
}

impl Window {
    fn normal_count(&self) -> usize {
        self.history.iter().filter(|ok| **ok).count()
    }

    fn switch(&mut self, tag: TaskTag, now: u64) {
        if self.fncount_latest_modify_time == 0
            || now.saturating_sub(self.fncount_latest_modify_time) > DYN_TIME_COUNT_EXPIRE_TIME
        {
            self.fncount_latest_modify_time = now;
            self.pos = 0;
            self.history = if self.status == DynamicTimeoutStatus::Bad {
                [false; 10]
            } else {
                [true; 10]
            };
        }
        self.pos = if self.pos + 1 >= self.history.len() {
            0
        } else {
            self.pos + 1
        };

        match tag {
            TaskTag::MidPkgMeetExpect
            | TaskTag::BigPkgMeetExpect
            | TaskTag::BiggerPkgMeetExpect => {
                if self.status == DynamicTimeoutStatus::Evaluating {
                    self.latest_bigpkg_goodtime = now;
                }
                if self.status == DynamicTimeoutStatus::Evaluating {
                    self.continuous_good_count += 1;
                }
                self.history[self.pos] = true;
            }
            TaskTag::MeetExpect => {
                if self.status == DynamicTimeoutStatus::Evaluating {
                    self.continuous_good_count += 1;
                }
                self.history[self.pos] = true;
            }
            TaskTag::Normal => {
                if self.status == DynamicTimeoutStatus::Evaluating {
                    self.continuous_good_count = 0;
                    self.latest_bigpkg_goodtime = 0;
                }
                self.history[self.pos] = true;
            }
            TaskTag::Failed => {
                self.continuous_good_count = 0;
                self.latest_bigpkg_goodtime = 0;
                self.history[self.pos] = false;
            }
        }

        let normal_count = self.normal_count();
        match self.status {
            DynamicTimeoutStatus::Evaluating => {
                if self.continuous_good_count >= DYN_TIME_MAX_CONTINUOUS_EXCELLENT_COUNT
                    && now.saturating_sub(self.latest_bigpkg_goodtime) <= DYN_TIME_COUNT_EXPIRE_TIME
                {
                    self.status = DynamicTimeoutStatus::Excellent;
                } else if normal_count <= DYN_TIME_MIN_NORMAL_PKG_COUNT {
                    self.status = DynamicTimeoutStatus::Bad;
                    self.fncount_latest_modify_time = 0;
                }
            }
            DynamicTimeoutStatus::Excellent => {
                if self.continuous_good_count == 0 && self.latest_bigpkg_goodtime == 0 {
                    self.status = DynamicTimeoutStatus::Evaluating;
                }
            }
            DynamicTimeoutStatus::Bad => {
                if normal_count > DYN_TIME_MIN_NORMAL_PKG_COUNT {
                    self.status = DynamicTimeoutStatus::Evaluating;
                    self.fncount_latest_modify_time = 0;
                }
            }
        }
    }
}
