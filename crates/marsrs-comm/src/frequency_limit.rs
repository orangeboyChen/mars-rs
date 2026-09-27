//! `mars/comm/comm_frequency_limit.h` — "at most `count` calls per
//! `time_span`".

use std::collections::VecDeque;

/// `CommFrequencyLimit`.
///
/// `check()` returns `true` while no more than `count` calls are on the books
/// within the last `time_span` milliseconds, and `false` once the budget is
/// used up — the C++'s test is `touch_times_.size() <= count_`, so `count + 1`
/// calls go through per span. [`FrequencyLimit::new`] still asserts
/// `count > 0` the way the C++ does, so the smallest limit there is lets two
/// calls through, and a caller that needs one has to count them itself.
///
/// The C++ amends its history when the clock jumps backwards (a user changing
/// the device time) instead of blocking forever; the port does the same, and
/// [`FrequencyLimit::check_at`] exists so that behaviour is testable.
#[derive(Debug, Clone)]
pub struct FrequencyLimit {
    count: usize,
    time_span: u64,
    touch_times: VecDeque<u64>,
}

impl FrequencyLimit {
    /// `CommFrequencyLimit(count, time_span)`.
    ///
    /// Both arguments have to be greater than zero: the C++ asserts on it and a
    /// zero `count` would reject every call.
    pub fn new(count: usize, time_span: u64) -> Self {
        assert!(count > 0, "CommFrequencyLimit: count must be > 0");
        assert!(time_span > 0, "CommFrequencyLimit: time_span must be > 0");
        Self {
            count,
            time_span,
            touch_times: VecDeque::new(),
        }
    }

    /// `CommFrequencyLimit::Check()` against [`crate::tickcount::gettickcount`].
    pub fn check(&mut self) -> bool {
        self.check_at(crate::tickcount::gettickcount())
    }

    /// `Check()` against an explicit tick count.
    pub fn check_at(&mut self, now: u64) -> bool {
        // The user changed the clock: repair the history instead of blocking.
        if let Some(oldest) = self.touch_times.front() {
            if now < *oldest {
                let len = self.touch_times.len();
                self.touch_times.clear();
                for _ in 0..len {
                    self.touch_times.push_back(now.saturating_sub(1));
                }
            }
        }

        if self.touch_times.len() <= self.count {
            self.touch_times.push_back(now);
            return true;
        }

        let oldest = self.touch_times.front().copied().unwrap_or(now);
        if now.saturating_sub(oldest) <= self.time_span {
            return false;
        }

        self.del_older_touch_time(now);
        self.touch_times.push_back(now);
        true
    }

    /// How many calls are recorded right now.
    pub fn len(&self) -> usize {
        self.touch_times.len()
    }

    /// Whether the history was rewritten after the clock jumped backwards. Only
    /// used by the tests: the C++ logs a warning and carries on.
    pub fn touch_times_are_amended(&self) -> bool {
        let mut iter = self.touch_times.iter();
        let first = iter.next();
        self.touch_times.len() > 1 && iter.all(|t| Some(t) == first)
    }

    /// Whether no call is recorded.
    pub fn is_empty(&self) -> bool {
        self.touch_times.is_empty()
    }

    fn del_older_touch_time(&mut self, now: u64) {
        while let Some(oldest) = self.touch_times.front().copied() {
            if now.saturating_sub(oldest) > self.time_span {
                self.touch_times.pop_front();
            } else {
                break;
            }
        }
    }
}
