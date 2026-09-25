//! `mars/stn/src/frequency_limit.h` — "the same body must not go out more than
//! `RECORD_INTERCEPT_COUNT` times".
//!
//! STN hashes the body (adler32) and counts how often each hash was sent. The
//! same body more than [`RECORD_INTERCEPT_COUNT`] times is treated as an
//! avalanche and refused; the table keeps at most [`MAX_RECORD_COUNT`] entries
//! and is swept once an hour, where entries that are still hot are kept (with a
//! reduced count) and cold ones are dropped.

use crate::task::Task;
use mars_comm::adler32::adler32;
use mars_comm::tickcount::gettickcount;

/// At most this many records are tracked.
pub const MAX_RECORD_COUNT: usize = 30;
/// Above this many sends of one body the task is refused.
pub const RECORD_INTERCEPT_COUNT: u32 = 105;
/// A hot record is kept with its count lowered to this.
pub const NOT_CLEAR_INTERCEPT_COUNT_RETRY: u32 = 99;
/// Records at or above this count are "hot".
pub const NOT_CLEAR_INTERCEPT_COUNT: u32 = 75;
/// "Hot" means: touched within the last ten minutes.
pub const NOT_CLEAR_INTERCEPT_INTERVAL: u64 = 10 * 60 * 1000;
/// How often the table is swept.
pub const RUN_CLEAR_RECORDS_INTERVAL: u64 = 60 * 60 * 1000;

/// `STAvalancheRecord`.
#[derive(Debug, Clone, Copy)]
pub struct AvalancheRecord {
    /// adler32 of the body.
    pub hash: u32,
    /// How often this body was sent.
    pub count: u32,
    /// When it was sent last.
    pub last_update: u64,
}

/// `FrequencyLimit`.
///
/// [`FrequencyLimit::check_at`] takes the tick count explicitly so that the
/// hourly sweep and the intercept count are testable without waiting an hour;
/// [`FrequencyLimit::check`] is the `Check()` of the C++ against
/// [`gettickcount`].
#[derive(Debug, Clone)]
pub struct FrequencyLimit {
    records: Vec<AvalancheRecord>,
    last_clear: u64,
}

impl Default for FrequencyLimit {
    fn default() -> Self {
        Self::new()
    }
}

impl FrequencyLimit {
    /// `FrequencyLimit()`.
    pub fn new() -> Self {
        Self::new_at(gettickcount())
    }

    /// `FrequencyLimit()` with the sweep clocked at `now`.
    pub fn new_at(now: u64) -> Self {
        Self {
            records: Vec::new(),
            last_clear: now,
        }
    }

    /// `FrequencyLimit::Check(task, buffer, len, span)`.
    ///
    /// Returns whether the task may go out, and how long ago the same body was
    /// sent last (`span`), which the caller reports.
    pub fn check(&mut self, task: &Task, body: &[u8]) -> (bool, u64) {
        self.check_at(task, body, gettickcount())
    }

    /// `Check()` against an explicit tick count.
    pub fn check_at(&mut self, task: &Task, body: &[u8], now: u64) -> (bool, u64) {
        if !task.limit_frequency {
            return (true, 0);
        }

        if now.saturating_sub(self.last_clear) >= RUN_CLEAR_RECORDS_INTERVAL {
            self.last_clear = now;
            self.clear_records(now);
        }

        let hash = adler32(body);
        match self.locate(hash) {
            Some(index) => {
                let span = now.saturating_sub(self.records[index].last_update);
                self.update(index, now);
                (self.records[index].count <= RECORD_INTERCEPT_COUNT, span)
            }
            None => {
                self.insert(hash, now);
                (true, 0)
            }
        }
    }

    /// When the table was swept last.
    pub fn last_clear(&self) -> u64 {
        self.last_clear
    }

    fn clear_records(&mut self, now: u64) {
        self.records.retain(|record| {
            let interval = now.saturating_sub(record.last_update);
            interval <= NOT_CLEAR_INTERCEPT_INTERVAL && NOT_CLEAR_INTERCEPT_COUNT <= record.count
        });
        for record in &mut self.records {
            if record.count > NOT_CLEAR_INTERCEPT_COUNT_RETRY {
                record.count = NOT_CLEAR_INTERCEPT_COUNT_RETRY;
            }
        }
    }

    fn locate(&self, hash: u32) -> Option<usize> {
        self.records.iter().rposition(|record| record.hash == hash)
    }

    fn insert(&mut self, hash: u32, now: u64) {
        // The C++ asserts and bails out here; the `Vec` cannot grow past the
        // cap because the branch below evicts as soon as the cap is reached.
        debug_assert!(self.records.len() <= MAX_RECORD_COUNT);
        if MAX_RECORD_COUNT == self.records.len() {
            // drop the record that was touched longest ago
            let oldest = self
                .records
                .iter()
                .enumerate()
                .min_by_key(|(_, record)| record.last_update)
                .map(|(index, _)| index)
                .unwrap_or(0);
            self.records.remove(oldest);
        }
        self.records.push(AvalancheRecord {
            hash,
            count: 1,
            last_update: now,
        });
    }

    fn update(&mut self, index: usize, now: u64) {
        self.records[index].count += 1;
        self.records[index].last_update = now;
    }

    /// The records currently tracked.
    pub fn records(&self) -> &[AvalancheRecord] {
        &self.records
    }
}
