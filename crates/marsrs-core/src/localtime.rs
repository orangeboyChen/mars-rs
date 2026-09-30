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
        // The cache is only allowed to answer what `chrono` would have, so it
        // is asked twice: the first call is a miss that fills the cache, and
        // it is the second — the one that came out of it — that has to agree
        // with the hour `chrono` works out for the same second.
        for secs in [0i64, 1_700_000_000, 1_700_000_000 + 13 * 3600] {
            let _miss = local_time(secs);
            let cached = local_time(secs);
            // what `chrono` says of the same second, asked here and not
            // through `lookup`, which is the function under test
            let Some(local) = Local.timestamp_opt(secs, 0).single() else {
                panic!("{secs} is a second `chrono` cannot name");
            };
            assert_eq!(cached.hour, local.hour() as u8, "{secs} in this zone");
            assert_eq!(cached.date, (local.year(), local.month(), local.day()));
        }
    }

    #[test]
    fn the_hour_of_now_is_the_hour_of_the_second_it_was_read_at() {
        // `local_hour()` is the hour of the second `now_secs()` read, so a
        // bound `chrono` guarantees anyway — `hour() < 24` — is not what
        // pins it: what does is that it is the hour of one of the seconds
        // the two calls may have fallen in, in *this* zone and not in UTC.
        let before = now_secs();
        let hour = local_hour();
        let after = now_secs();

        let mut hours = Vec::new();
        for secs in before..=after {
            if let Some(local) = Local.timestamp_opt(secs, 0).single() {
                hours.push(local.hour() as u8);
            }
        }
        assert!(
            hours.contains(&hour),
            "{hour} is the hour of no second between {before} and {after}: {hours:?}"
        );
    }

    #[test]
    fn now_secs_is_the_second_the_clock_is_at() {
        // Asked of the clock this function reads and not of a date in the
        // past: an assertion that `now` is later than some second a build
        // happened on is one the machine's own clock answers, whichever
        // number this function returns.
        let before = seconds_of(SystemTime::now());
        let secs = now_secs();
        let after = seconds_of(SystemTime::now());
        assert!(
            (before..=after).contains(&secs),
            "{secs} is not a second between {before} and {after}"
        );
    }

    /// The seconds of a [`SystemTime`], the way [`now_secs`] reads them.
    fn seconds_of(time: SystemTime) -> i64 {
        time.duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0)
    }
}
