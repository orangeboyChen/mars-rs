//! Local time, cached for the second it was asked for.
//!
//! A single record asks the platform for the local time three times: once for
//! the timestamp in its header, once for the calendar day the log file belongs
//! to and once for the hour the crypt header is stamped with. Upstream asks per
//! record too — `formater.cc` for every header, `XloggerAppender::__OpenLogFile`
//! when it compares the open file's day with today's,
//! `LogCrypt::SetHeaderInfo` for the begin/end hour — and so did this port, at
//! ~90 ns the question, because `chrono::Local` is a `tzset` + `localtime_r`
//! pair.
//!
//! The answer only changes when the second does, so [`local_time`] keeps the
//! last one per thread and re-asks once a second instead of once a record. The
//! cache key *is* the second, so a cached answer is exactly the answer
//! `localtime` would have given for that second — nothing is approximated, and
//! a clock that is read twice in the same second cannot disagree with itself.

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Local, TimeZone, Timelike};

/// One second of local time: the UTC offset, the hour and the calendar day.
///
/// Everything a record needs from `localtime`, in one snapshot, so that the
/// header, the day roll-over check and the crypt hour share a single lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    /// The `tv_sec` this snapshot was taken for — also its cache key.
    pub secs: i64,
    /// `tm_gmtoff`: seconds east of UTC.
    pub offset: i32,
    /// Local hour, `0..=23`.
    pub hour: u8,
    /// Local calendar day, `(year, month, day)`.
    pub date: (i32, u32, u32),
}

impl LocalTime {
    /// What a second the platform cannot represent looks like: the epoch, in
    /// UTC. `localtime` answers `NULL` for it and the C++ prints an empty
    /// timestamp, so the port keeps the same sentinel the callers already had.
    fn unknown(secs: i64) -> Self {
        Self {
            secs,
            offset: 0,
            hour: 0,
            date: (1970, 1, 1),
        }
    }
}

thread_local! {
    /// The last [`LocalTime`] this thread asked for.
    static CACHED: Cell<Option<LocalTime>> = const { Cell::new(None) };
}

/// Local time for `secs`, cached for that second on this thread.
pub fn local_time(secs: i64) -> LocalTime {
    CACHED.with(|cell| {
        if let Some(cached) = cell.get() {
            if cached.secs == secs {
                return cached;
            }
        }

        let fresh = lookup(secs);
        cell.set(Some(fresh));
        fresh
    })
}

/// The one `localtime` in a second: a UTC instant has exactly one local time,
/// so `secs` alone decides the offset, the hour and the day — the sub-second
/// part of the timestamp changes none of them.
fn lookup(secs: i64) -> LocalTime {
    let Some(local) = Local.timestamp_opt(secs, 0).single() else {
        return LocalTime::unknown(secs);
    };

    let date = local.date_naive();
    LocalTime {
        secs,
        offset: local.offset().local_minus_utc(),
        hour: local.hour() as u8,
        date: (date.year(), date.month(), date.day()),
    }
}

/// Wall-clock seconds since the epoch (`time(nullptr)` / `gettimeofday`).
pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// The current local hour `0..=23` — the begin/end hour of the xlog crypt
/// header, which is stamped into every record and every drained batch.
pub fn local_hour() -> u8 {
    local_time(now_secs()).hour
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_second_is_the_same_snapshot() {
        let a = local_time(1_700_000_000);
        let b = local_time(1_700_000_000);
        assert_eq!(a, b);
        assert_eq!(a.secs, 1_700_000_000);
    }

    #[test]
    fn a_second_the_platform_cannot_lose_is_the_epoch() {
        // `i64::MAX` is outside what `chrono` can turn into a `DateTime`.
        let unknown = local_time(i64::MAX);
        assert_eq!(unknown.offset, 0);
        assert_eq!(unknown.date, (1970, 1, 1));
    }

    #[test]
    fn the_hour_agrees_with_chrono() {
        // The cache is only allowed to answer what `chrono` would have.
        for secs in [0i64, 1_700_000_000, 1_700_000_000 + 13 * 3600] {
            let cached = local_time(secs);
            let fresh = lookup(secs);
            assert_eq!(cached, fresh);
            assert_eq!(cached.hour, fresh.hour);
        }
    }

    #[test]
    fn local_hour_is_an_hour() {
        assert!(local_hour() < 24);
    }

    #[test]
    fn now_secs_is_after_the_build() {
        assert!(now_secs() > 1_600_000_000);
    }
}
