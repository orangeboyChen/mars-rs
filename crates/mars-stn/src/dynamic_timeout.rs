//! `mars/stn/src/dynamic_timeout.h` — "is the network good right now?"
//!
//! Every finished task is classified (met its budget / finished normally /
//! failed) and the last ten of those are kept in a sliding window. Enough good
//! packages in a row make the status `Excellent`, too many failures make it
//! `Bad`; STN uses the status to pick the first-package timeout.

use crate::config::*;
use mars_comm::tickcount::gettickcount;

/// `DynamicTimeoutStatus`.
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

/// Which network the task ran on, i.e. the `kMobile == getNetInfo()` of the C++.
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
    /// `kDynTimeTaskMidPkgMeetExpectTag`.
    MidPkgMeetExpect = 2,
    /// `kDynTimeTaskBigPkgMeetExpectTag`.
    BigPkgMeetExpect = 3,
    /// `kDynTimeTaskBiggerPkgMeetExpectTag`.
    BiggerPkgMeetExpect = 4,
    /// `KDynTimeTaskNormalTag` — finished, but slower than its budget.
    Normal = 0,
    /// `kDynTimeTaskFailedTag`.
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

/// `DynamicTimeout`.
///
/// [`DynamicTimeout::record_at`] takes the tick count explicitly so that the
/// five-minute expiry of the sliding window is testable without waiting;
/// [`DynamicTimeout::record`] is the `CgiTaskStatistic()` of the C++ against
/// [`gettickcount`].
#[derive(Debug, Clone)]
pub struct DynamicTimeout {
    status: DynamicTimeoutStatus,
    continuous_good_count: u32,
    latest_bigpkg_goodtime: u64,
    /// The last ten sends, `true` when one finished normally or better.
    history: [bool; 10],
    fncount_latest_modify_time: u64,
    pos: usize,
}

impl Default for DynamicTimeout {
    fn default() -> Self {
        Self::new()
    }
}

impl DynamicTimeout {
    /// `DynamicTimeout()`.
    pub fn new() -> Self {
        Self {
            status: DynamicTimeoutStatus::Evaluating,
            continuous_good_count: 0,
            latest_bigpkg_goodtime: 0,
            history: [true; 10],
            fncount_latest_modify_time: 0,
            pos: 0,
        }
    }

    /// `DynamicTimeout::CgiTaskStatistic(cgi, total_size, cost_time)`.
    pub fn record(&mut self, network: NetworkKind, total_size: u32, cost_time: u64) {
        self.record_at(network, total_size, cost_time, gettickcount())
    }

    /// `CgiTaskStatistic()` against an explicit tick count.
    pub fn record_at(&mut self, network: NetworkKind, total_size: u32, cost_time: u64, now: u64) {
        self.status_switch(TaskTag::of(network, total_size, cost_time), now);
    }

    /// `DynamicTimeout::GetStatus()`.
    pub fn status(&self) -> DynamicTimeoutStatus {
        self.status
    }

    /// How many packages in a row met their budget.
    pub fn continuous_good_count(&self) -> u32 {
        self.continuous_good_count
    }

    /// How many of the last ten packages finished normally or better.
    pub fn normal_count(&self) -> usize {
        self.history.iter().filter(|ok| **ok).count()
    }

    /// `DynamicTimeout::ResetStatus()`.
    pub fn reset(&mut self) {
        self.status = DynamicTimeoutStatus::Evaluating;
        self.continuous_good_count = 0;
        self.latest_bigpkg_goodtime = 0;
        self.history = [true; 10];
        self.fncount_latest_modify_time = 0;
        self.pos = 0;
    }

    fn status_switch(&mut self, tag: TaskTag, now: u64) {
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

        let normal_count = self.history.iter().filter(|ok| **ok).count();
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
