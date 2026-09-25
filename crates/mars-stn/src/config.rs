//! `mars/stn/config.h` — the tunables of the task pipeline.

/// Packages up to this size have to answer within the "small package" budget.
pub const DYN_TIME_SMALL_PACKAGE_LEN: u32 = 3 * 1024;
/// ... the "middle package" budget.
pub const DYN_TIME_MIDDLE_PACKAGE_LEN: u32 = 10 * 1024;
/// ... the "big package" budget; anything larger uses the "bigger" one.
pub const DYN_TIME_BIG_PACKAGE_LEN: u32 = 30 * 1024;

/// Wi-Fi budgets, in milliseconds.
pub const DYN_TIME_SMALL_PACKAGE_WIFI_COSTTIME: u64 = 500;
pub const DYN_TIME_MIDDLE_PACKAGE_WIFI_COSTTIME: u64 = 2 * 1000;
pub const DYN_TIME_BIG_PACKAGE_WIFI_COSTTIME: u64 = 4 * 1000;
pub const DYN_TIME_BIGGER_PACKAGE_WIFI_COSTTIME: u64 = 6 * 1000;

/// Mobile budgets, in milliseconds.
pub const DYN_TIME_SMALL_PACKAGE_GPRS_COSTTIME: u64 = 1000;
pub const DYN_TIME_MIDDLE_PACKAGE_GPRS_COSTTIME: u64 = 3 * 1000;
pub const DYN_TIME_BIG_PACKAGE_GPRS_COSTTIME: u64 = 5 * 1000;
pub const DYN_TIME_BIGGER_PACKAGE_GPRS_COSTTIME: u64 = 7 * 1000;

/// How many packages in a row have to meet their budget before the network is
/// called excellent.
pub const DYN_TIME_MAX_CONTINUOUS_EXCELLENT_COUNT: u32 = 10;
/// Below this many normal packages out of the last ten the network is bad.
pub const DYN_TIME_MIN_NORMAL_PKG_COUNT: usize = 6;
/// The sliding window of the "last N packages" bookkeeping, in milliseconds.
pub const DYN_TIME_COUNT_EXPIRE_TIME: u64 = 5 * 60 * 1000;
/// The size a failed task is reported with.
pub const DYN_TIME_TASK_FAILED_PKG_LEN: u32 = 0xffff_ffff;

/// `FlowLimit`: bytes per second the funnel drains while the app is in the
/// background, and while it is foregrounded.
pub const INACTIVE_SPEED: u64 = 20 * 1024 * 1024 / 3600;
pub const ACTIVE_SPEED: u64 = 80 * 1024 * 1024 / 3600;
/// The volume kept when the app goes to the background.
pub const INACTIVE_MIN_VOL: u64 = 60 * 1024 * 1024;
/// The funnel capacity: above this a `limit_flow` task is refused.
pub const MAX_VOL: u64 = 80 * 1024 * 1024;

/// Heartbeat range, in milliseconds: `MinHeartInterval` (3.5 minutes) and
/// `MaxHeartInterval` (10 minutes).
pub const MIN_HEART_INTERVAL: u32 = 3 * 60 * 1000 + 30 * 1000;
pub const MAX_HEART_INTERVAL: u32 = 10 * 60 * 1000;

/// `HeartStep` — how far the smart heartbeat pushes the interval up while it is
/// still looking for the largest one that keeps the TCP alive.
pub const HEART_STEP: u32 = 60 * 1000;
/// `SuccessStep` — the margin it keeps below `MAX_HEART_INTERVAL`, so the value
/// it settles on is `curHeart - SuccessStep`… which the C++ spells as
/// "`MaxHeartInterval - SuccessStep`" and uses as both ceiling and target.
pub const SUCCESS_STEP: u32 = 20 * 1000;

/// `MaxHeartFailCount` — failures on the current interval before the value is
/// given up on.
pub const MAX_HEART_FAIL_COUNT: u32 = 2;
/// `BaseSuccCount` — successes on one interval before it tries a bigger one.
pub const BASE_SUCC_COUNT: u32 = 5;
/// `NetStableTestCount` — heartbeats at `MIN_HEART_INTERVAL` before the network
/// is considered stable enough to start computing.
pub const NET_STABLE_TEST_COUNT: u32 = 3;

/// One week, in seconds: how old a settled interval has to be before the smart
/// heartbeat probes a bigger one.
pub const ONE_DAY_SECONDS: i64 = 24 * 60 * 60;
pub const PROBE_BIGGER_HEART_AGE: i64 = 7 * ONE_DAY_SECONDS;
/// `MAX_INI_SECTIONS` — how many networks `Heartbeat.ini` remembers.
pub const MAX_INI_SECTIONS: usize = 20;
/// How far the heartbeat has to have drifted from its interval, in
/// milliseconds, for the network to look like it is dozing (MIUI aligns alarms
/// to five-minute marks, which is what this is for).
pub const DOZE_JUDGE_WINDOW: i64 = 20 * 1000;
