//! The task intercept, through the public api.
//!
//! An answer the app gave for a task comes back while it is fresh, is forgotten
//! the moment it is asked for and found to be more than a minute old, and a
//! second answer for the same task replaces the first.

use mars_stn::task_intercept::{TaskIntercept, TaskInterceptInfo, INTERCEPT_TIMEOUT};

#[test]
fn an_answer_comes_back_while_it_is_fresh() {
    let mut intercept = TaskIntercept::new();
    intercept.add_intercept_task_at(1_000, "task", b"the answer".to_vec());
    assert_eq!(intercept.len(), 1);
    assert!(!intercept.is_empty());

    assert_eq!(
        intercept.intercept_task_info_at(1_000, "task"),
        Some(b"the answer".to_vec())
    );
    // ... and right up to the minute
    assert_eq!(
        intercept.intercept_task_info_at(1_000 + INTERCEPT_TIMEOUT, "task"),
        Some(b"the answer".to_vec())
    );
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
fn a_task_that_was_never_written_down_has_no_answer() {
    let mut intercept = TaskIntercept::new();
    intercept.add_intercept_task_at(0, "task", b"the answer".to_vec());
    assert_eq!(intercept.intercept_task_info_at(0, "other"), None);
    assert_eq!(intercept.len(), 1, "asking did not forget anything");
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
        Some(b"the second".to_vec())
    );

    intercept.clear();
    assert!(intercept.is_empty());
}

#[test]
fn the_methods_that_take_no_reading_ask_the_clock_themselves() {
    let mut intercept = TaskIntercept::default();
    intercept.add_intercept_task("task", b"the answer".to_vec());
    assert_eq!(
        intercept.intercept_task_info("task"),
        Some(b"the answer".to_vec())
    );
    assert_eq!(intercept.intercept_task_info("other"), None);
    assert!(format!("{intercept:?}").contains("TaskIntercept"));

    let info = TaskInterceptInfo {
        name: "task".to_string(),
        intercept_time: 0,
        data: b"the answer".to_vec(),
    };
    assert_eq!(info.name, "task");
}
