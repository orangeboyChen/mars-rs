//! `mars/stn/src/task_intercept.cc` — the answer of a task the app took over.
//!
//! A task the app answers itself instead of letting STN send it is written
//! down here with its answer, so that the next attempt at the same task can
//! hand that answer back instead of going out again. An answer is only good
//! for a minute ([`INTERCEPT_TIMEOUT`]) and is forgotten the moment it is
//! asked for and found to be too old.
//!
//! The C++ keeps one `TaskInterceptInfo` per task name, and its
//! `GetInterceptTaskInfo` answers `false` on every path — including the one
//! where it hands the data out — so the port answers the question the name
//! asks instead: [`TaskIntercept::intercept_task_info_at`] is [`Some`] with
//! the data when the answer is still good and [`None`] when it is not.

use std::collections::HashMap;

use mars_comm::tickcount::gettickcount;

/// How long an intercepted answer is good for, in milliseconds.
pub const INTERCEPT_TIMEOUT: u64 = 60 * 1000;

/// `struct TaskInterceptInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskInterceptInfo {
    /// `name`
    pub name: String,
    /// `intercept_time`
    pub intercept_time: u64,
    /// `data` — the answer the app gave.
    pub data: String,
}

/// `TaskIntercept`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskIntercept {
    /// `intercept_tasks_`
    intercept_tasks: HashMap<String, TaskInterceptInfo>,
}

impl TaskIntercept {
    /// `TaskIntercept()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `AddInterceptTask(_name, _data)` — the tick count comes from the clock.
    pub fn add_intercept_task(&mut self, name: impl Into<String>, data: impl Into<String>) {
        self.add_intercept_task_at(gettickcount(), name, data)
    }

    /// The same, with the reading handed in: a task without a name is not
    /// written down at all, which is the one thing the C++ refuses.
    pub fn add_intercept_task_at(
        &mut self,
        now: u64,
        name: impl Into<String>,
        data: impl Into<String>,
    ) {
        let name = name.into();
        if name.is_empty() {
            return;
        }
        self.intercept_tasks.insert(
            name.clone(),
            TaskInterceptInfo {
                name,
                intercept_time: now,
                data: data.into(),
            },
        );
    }

    /// `GetInterceptTaskInfo(_name, _last_data)` — the answer the app gave, or
    /// [`None`] when there is none, or when it is older than
    /// [`INTERCEPT_TIMEOUT`] and is forgotten on the way out.
    pub fn intercept_task_info(&mut self, name: &str) -> Option<String> {
        self.intercept_task_info_at(gettickcount(), name)
    }

    /// The same, with the reading handed in.
    pub fn intercept_task_info_at(&mut self, now: u64, name: &str) -> Option<String> {
        let info = self.intercept_tasks.get(name)?;
        if now.saturating_sub(info.intercept_time) > INTERCEPT_TIMEOUT {
            self.intercept_tasks.remove(name);
            return None;
        }
        Some(info.data.clone())
    }

    /// How many answers are being kept.
    pub fn len(&self) -> usize {
        self.intercept_tasks.len()
    }

    /// Whether there is nothing to hand back.
    pub fn is_empty(&self) -> bool {
        self.intercept_tasks.is_empty()
    }

    /// Forget every answer.
    pub fn clear(&mut self) {
        self.intercept_tasks.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_comes_back_while_it_is_fresh() {
        let mut intercept = TaskIntercept::new();
        intercept.add_intercept_task_at(1_000, "task", "the answer");
        assert_eq!(intercept.len(), 1);

        assert_eq!(
            intercept.intercept_task_info_at(1_000, "task"),
            Some("the answer".to_string())
        );
        // ... and right up to the minute
        assert_eq!(
            intercept.intercept_task_info_at(1_000 + INTERCEPT_TIMEOUT, "task"),
            Some("the answer".to_string())
        );
    }

    #[test]
    fn an_answer_that_is_too_old_is_forgotten() {
        let mut intercept = TaskIntercept::new();
        intercept.add_intercept_task_at(1_000, "task", "the answer");

        assert_eq!(
            intercept.intercept_task_info_at(1_000 + INTERCEPT_TIMEOUT + 1, "task"),
            None
        );
        assert!(intercept.is_empty(), "it was erased on the way out");
    }

    #[test]
    fn a_task_without_a_name_is_not_written_down() {
        let mut intercept = TaskIntercept::new();
        intercept.add_intercept_task_at(0, "", "the answer");
        assert!(intercept.is_empty());
        assert_eq!(intercept.intercept_task_info_at(0, ""), None);
    }

    #[test]
    fn a_second_answer_for_the_same_task_replaces_the_first() {
        let mut intercept = TaskIntercept::new();
        intercept.add_intercept_task_at(0, "task", "the first");
        intercept.add_intercept_task_at(1_000, "task", "the second");
        assert_eq!(intercept.len(), 1);
        assert_eq!(
            intercept.intercept_task_info_at(1_000, "task"),
            Some("the second".to_string())
        );

        intercept.clear();
        assert!(intercept.is_empty());
    }

    #[test]
    fn the_methods_that_take_no_reading_ask_the_clock_themselves() {
        let mut intercept = TaskIntercept::default();
        intercept.add_intercept_task("task", "the answer");
        assert_eq!(
            intercept.intercept_task_info("task"),
            Some("the answer".to_string())
        );
        assert_eq!(intercept.intercept_task_info("other"), None);
        assert!(format!("{intercept:?}").contains("TaskIntercept"));
    }
}
