//! `mars/stn/src/task_intercept.cc` — the answer of a task the app took over.
//!
//! A task the app answers itself instead of letting STN send it is written
//! down here with its answer, and an answer is only good for a minute
//! ([`INTERCEPT_TIMEOUT`]). Nothing is ever answered out of it, though: the
//! C++'s `GetInterceptTaskInfo` copies the answer into `_last_data` and then
//! answers `false` on every path, so the next task of that name goes out like
//! any other. That is the C++'s own doing and not a slip — `45426aa0` ("no
//! need cgi intercepter") turned the `return true` into a `return false` in
//! 2022 — so the port answers [`None`] where the C++ answers `false`:
//! [`TaskIntercept::intercept_task_info_at`] is [`None`] whether or not there
//! is an answer, and what is left of the look-up is the forgetting of one
//! that has gone stale.

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
    /// `data` — the answer the app gave, as the bytes it gave them: the
    /// C++'s `std::string` holds a response buffer, which is protobuf or
    /// compressed often enough that a `String` would refuse it, and the
    /// crate's other wire paths (`longlink::Unpacked::body`) are `Vec<u8>`
    /// for the same reason.
    pub data: Vec<u8>,
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
    pub fn add_intercept_task(&mut self, name: impl Into<String>, data: impl Into<Vec<u8>>) {
        self.add_intercept_task_at(gettickcount(), name, data)
    }

    /// The same, with the reading handed in: a task without a name is not
    /// written down at all, which is the one thing the C++ refuses.
    pub fn add_intercept_task_at(
        &mut self,
        now: u64,
        name: impl Into<String>,
        data: impl Into<Vec<u8>>,
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

    /// `GetInterceptTaskInfo(_name, _last_data)` — which is to say, nothing at
    /// all: the C++ copies the answer into `_last_data` and then answers
    /// `false`, so no task is ever answered out of what was kept, however
    /// fresh it is. What the look-up still does is the forgetting of an answer
    /// older than [`INTERCEPT_TIMEOUT`]; there is nothing to hand back either
    /// way.
    pub fn intercept_task_info(&mut self, name: &str) -> Option<Vec<u8>> {
        self.intercept_task_info_at(gettickcount(), name)
    }

    /// The same, with the reading handed in.
    pub fn intercept_task_info_at(&mut self, now: u64, name: &str) -> Option<Vec<u8>> {
        // `_last_data = info->second.data; return false;` — what was kept is
        // copied out and then refused, so no task is answered out of it
        let stale =
            now.saturating_sub(self.intercept_tasks.get(name)?.intercept_time) > INTERCEPT_TIMEOUT;
        if stale {
            self.intercept_tasks.remove(name);
        }
        None
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
    fn an_answer_is_not_handed_back_even_while_it_is_fresh() {
        let mut intercept = TaskIntercept::new();
        intercept.add_intercept_task_at(1_000, "task", b"the answer".to_vec());
        assert_eq!(intercept.len(), 1);

        // the C++ copies the answer out and then answers `false`
        assert_eq!(intercept.intercept_task_info_at(1_000, "task"), None);
        assert_eq!(
            intercept.intercept_task_info_at(1_000 + INTERCEPT_TIMEOUT, "task"),
            None
        );
        assert_eq!(intercept.len(), 1, "but it is still being kept");
    }

    #[test]
    fn an_answer_that_is_too_old_is_forgotten() {
        let mut intercept = TaskIntercept::new();
        intercept.add_intercept_task_at(1_000, "task", b"the answer".to_vec());

        assert_eq!(
            intercept.intercept_task_info_at(1_000 + INTERCEPT_TIMEOUT + 1, "task"),
            None
        );
        assert!(intercept.is_empty(), "it was erased on the way out");
    }

    #[test]
    fn a_task_without_a_name_is_not_written_down() {
        let mut intercept = TaskIntercept::new();
        intercept.add_intercept_task_at(0, "", b"the answer".to_vec());
        assert!(intercept.is_empty());
        assert_eq!(intercept.intercept_task_info_at(0, ""), None);
    }

    #[test]
    fn a_second_answer_for_the_same_task_replaces_the_first() {
        let mut intercept = TaskIntercept::new();
        intercept.add_intercept_task_at(0, "task", b"the first".to_vec());
        intercept.add_intercept_task_at(1_000, "task", b"the second".to_vec());
        assert_eq!(intercept.len(), 1);
        assert_eq!(
            intercept.intercept_task_info_at(1_000, "task"),
            None,
            "the second is what is kept, but nothing is handed out of it"
        );

        intercept.clear();
        assert!(intercept.is_empty());
    }

    #[test]
    fn the_methods_that_take_no_reading_ask_the_clock_themselves() {
        let mut intercept = TaskIntercept::default();
        intercept.add_intercept_task("task", b"the answer".to_vec());
        assert_eq!(intercept.intercept_task_info("task"), None);
        assert_eq!(intercept.intercept_task_info("other"), None);
        assert!(format!("{intercept:?}").contains("TaskIntercept"));
    }
}
