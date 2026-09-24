//! `mars/comm/tickcount.h` + the `gettickcount()` of `mars/comm/time_utils.h`.
//!
//! `gettickcount()` is a **monotonic** millisecond counter: the C++ uses
//! `CLOCK_BOOTTIME` on Android and `mach_absolute_time` on Apple, so it is not
//! affected by wall-clock changes. `std::time::Instant` is the Rust equivalent.
//!
//! One difference is worth knowing: `CLOCK_BOOTTIME` keeps counting while the
//! device sleeps, while `Instant` on Linux is `CLOCK_MONOTONIC` and does not.
//! Code that measures durations across a suspend (which is what
//! `mars_comm::FrequencyLimit` guards against) should therefore treat a smaller reading
//! as "the clock was changed", exactly like the C++ does.

use std::sync::OnceLock;
use std::time::Instant;

/// The process start, the origin of [`gettickcount`].
static START: OnceLock<Instant> = OnceLock::new();

/// `::gettickcount()` — milliseconds since the process started.
///
/// The count starts at `1`: [`TickCount`] reserves `0` for "invalid", so the
/// very first reading — the one a freshly constructed `TickCount::now()`
/// takes — must not look like one.
pub fn gettickcount() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// `tickcountdiff_t` — a signed difference between two [`TickCount`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TickCountDiff(i64);

impl TickCountDiff {
    /// `tickcountdiff_t(diff)`.
    pub const fn new(diff: i64) -> Self {
        Self(diff)
    }

    /// The difference in milliseconds.
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl From<TickCountDiff> for i64 {
    fn from(diff: TickCountDiff) -> i64 {
        diff.0
    }
}

impl std::ops::AddAssign<i64> for TickCountDiff {
    fn add_assign(&mut self, rhs: i64) {
        self.0 += rhs;
    }
}

impl std::ops::SubAssign<i64> for TickCountDiff {
    fn sub_assign(&mut self, rhs: i64) {
        self.0 -= rhs;
    }
}

impl std::ops::MulAssign<i64> for TickCountDiff {
    fn mul_assign(&mut self, rhs: i64) {
        self.0 *= rhs;
    }
}

/// `tickcount_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TickCount(u64);

impl TickCount {
    /// A zero tick count, i.e. [`TickCount::is_valid`] is `false`.
    pub const fn invalid() -> Self {
        Self(0)
    }

    /// `tickcount_t(true)` — the current tick count.
    pub fn now() -> Self {
        Self(gettickcount())
    }

    /// `tickcount_t::get()`.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// `tickcount_t::gettickcount()` — refresh in place.
    pub fn refresh(&mut self) -> &mut Self {
        self.0 = gettickcount();
        self
    }

    /// `tickcount_t::gettickspan()` — how much time passed since `self`.
    pub fn tickspan(self) -> TickCountDiff {
        Self::now() - self
    }

    /// `tickcount_t::isValid()`.
    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }

    /// `tickcount_t::setInvalid()`.
    pub fn set_invalid(&mut self) {
        self.0 = 0;
    }
}

impl std::ops::Sub for TickCount {
    type Output = TickCountDiff;

    fn sub(self, rhs: Self) -> TickCountDiff {
        TickCountDiff(self.0.wrapping_sub(rhs.0) as i64)
    }
}

impl std::ops::Add<TickCountDiff> for TickCount {
    type Output = TickCount;

    fn add(self, rhs: TickCountDiff) -> TickCount {
        TickCount(self.0.wrapping_add(rhs.0 as u64))
    }
}

impl std::ops::Sub<TickCountDiff> for TickCount {
    type Output = TickCount;

    fn sub(self, rhs: TickCountDiff) -> TickCount {
        TickCount(self.0.wrapping_sub(rhs.0 as u64))
    }
}

impl std::ops::AddAssign<TickCountDiff> for TickCount {
    fn add_assign(&mut self, rhs: TickCountDiff) {
        self.0 = self.0.wrapping_add(rhs.0 as u64);
    }
}

impl std::ops::SubAssign<TickCountDiff> for TickCount {
    fn sub_assign(&mut self, rhs: TickCountDiff) {
        self.0 = self.0.wrapping_sub(rhs.0 as u64);
    }
}
