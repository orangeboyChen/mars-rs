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
