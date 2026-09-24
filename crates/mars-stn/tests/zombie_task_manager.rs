//! `mars/stn/src/zombie_task_manager.cc`, through the public api.
//!
//! The samples are what the C++ answers for the same calls: a task that is
//! sensitive to the network status is not kept, a kept one loses the time it
//! already spent, `RedoTasks` starts the most urgent first, and the periodic
//! check only restarts a task that waited `RETRY_INTERVAL` while the net core
//! waited too.

use std::sync::{Arc, Mutex};

use mars_stn::task_profile::{ErrCmdType, TaskFailHandleType, LOCAL_TASK_TIMEOUT};
use mars_stn::zombie_task_manager::{ZombieTaskManager, RETRY_INTERVAL, TIMER_INTERVAL};
use mars_stn::Task;

/// A task kept for a minute, which is long enough for every check below.
fn kept(taskid: u32, priority: i32) -> Task {
    let mut task = Task::new(taskid, 1);
    task.total_timeout = 60_000;
    task.priority = priority;
    task
}

/// What the app was told: the task it failed with, and how.
type Failed = Arc<Mutex<Vec<(u32, ErrCmdType, i32, TaskFailHandleType)>>>;
/// What the app was asked to start again.
type Started = Arc<Mutex<Vec<u32>>>;

/// A manager that records what it starts and what it fails.
fn manager() -> (ZombieTaskManager, Started, Failed) {
    let mut manager = ZombieTaskManager::new_at(0);
    let started = Arc::new(Mutex::new(Vec::new()));
    let failed = Arc::new(Mutex::new(Vec::new()));
    let record_start = Arc::clone(&started);
    manager.set_start_task(move |task| {
        record_start
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(task.taskid);
    });
    let record_fail = Arc::clone(&failed);
    manager.set_callback(move |err_type, err_code, fail_handle, task, _cost| {
        record_fail.lock().unwrap_or_else(|p| p.into_inner()).push((
            task.taskid,
            err_type,
            err_code,
            fail_handle,
        ));
    });
    (manager, started, failed)
}

#[test]
fn only_a_task_that_can_be_kept_is_saved() {
    let (mut manager, _started, _failed) = manager();

    // sensitive to the network status
    let mut sensitive = kept(1, Task::TASK_PRIORITY_NORMAL);
    sensitive.network_status_sensitive = true;
    assert!(!manager.save_task_at(0, &sensitive, 0));

    // ... and one whose deadline the time it spent has used up
    let mut spent = kept(2, Task::TASK_PRIORITY_NORMAL);
    spent.total_timeout = 500;
    assert!(!manager.save_task_at(0, &spent, 500));

    // a kept one is there, and the check is due a period later
    assert!(manager.save_task_at(1_000, &kept(3, Task::TASK_PRIORITY_NORMAL), 0));
    assert!(manager.has_task(3));
    assert_eq!(manager.len(), 1);
    assert_eq!(manager.due_time(), Some(1_000 + TIMER_INTERVAL));
}

#[test]
fn a_saved_task_loses_the_time_it_spent_and_its_retries() {
    let (mut manager, started, _failed) = manager();
    let mut task = kept(7, Task::TASK_PRIORITY_NORMAL);
    task.total_timeout = 1_000;
    task.retry_count = 3;
    assert!(manager.save_task_at(1_000, &task, 300));

    manager.redo_tasks_at(1_100);
    // 1_000 - 300 saved, 100 more spent on the way: 600 is what is left
    let started = started.lock().unwrap_or_else(|p| p.into_inner()).clone();
    assert_eq!(started, vec![7]);
    assert_eq!(task.retry_count, 3, "the caller's task is not touched");
    assert_eq!(task.total_timeout, 1_000);
}

#[test]
fn redoing_the_tasks_starts_them_most_urgent_first() {
    let (mut manager, started, failed) = manager();
    assert!(manager.save_task_at(0, &kept(1, Task::TASK_PRIORITY_LOWEST), 0));
    assert!(manager.save_task_at(0, &kept(2, Task::TASK_PRIORITY_HIGHEST), 0));
    let mut dead = kept(3, Task::TASK_PRIORITY_NORMAL);
    dead.total_timeout = 100;
    assert!(manager.save_task_at(0, &dead, 0));

    // 100ms after it was saved, the third one's deadline has run out and the
    // other two are still well inside theirs
    manager.redo_tasks_at(200);
    assert_eq!(
        *started.lock().unwrap_or_else(|p| p.into_inner()),
        vec![2, 1],
        "priority 0 before priority 5"
    );
    assert_eq!(
        *failed.lock().unwrap_or_else(|p| p.into_inner()),
        vec![(
            3,
            ErrCmdType::Local,
            LOCAL_TASK_TIMEOUT,
            TaskFailHandleType::TaskEnd
        )]
    );
    assert!(manager.is_empty());
    assert_eq!(manager.due_time(), None);
}

#[test]
fn the_periodic_check_waits_for_the_task_and_for_the_net_core() {
    let (mut manager, started, _failed) = manager();
    // a deadline long enough for the task to still be alive after two
    // RETRY_INTERVALs
    let mut task = kept(4, Task::TASK_PRIORITY_NORMAL);
    task.total_timeout = 10 * 60_000;
    assert!(manager.save_task_at(0, &task, 0));

    // the task has not waited RETRY_INTERVAL yet
    manager.on_timer_check_at(RETRY_INTERVAL - 1);
    assert!(started.lock().unwrap_or_else(|p| p.into_inner()).is_empty());

    // the task has, but the net core started one just now
    manager.on_net_core_start_task_at(RETRY_INTERVAL);
    manager.on_timer_check_at(RETRY_INTERVAL * 2 - 1);
    assert!(started.lock().unwrap_or_else(|p| p.into_inner()).is_empty());
    assert_eq!(manager.net_core_last_start_task_time(), RETRY_INTERVAL);

    // both have waited now
    manager.on_timer_check_at(RETRY_INTERVAL * 2);
    assert_eq!(*started.lock().unwrap_or_else(|p| p.into_inner()), vec![4]);
    assert!(manager.is_empty());
    assert_eq!(manager.due_time(), None, "the check cancels itself");
}

#[test]
fn a_zombie_whose_deadline_ran_out_is_failed() {
    let (mut manager, started, failed) = manager();
    let mut task = kept(5, Task::TASK_PRIORITY_NORMAL);
    task.total_timeout = 1_000;
    assert!(manager.save_task_at(500, &task, 0));

    manager.on_timer_check_at(1_500);
    assert!(started.lock().unwrap_or_else(|p| p.into_inner()).is_empty());
    assert_eq!(
        *failed.lock().unwrap_or_else(|p| p.into_inner()),
        vec![(
            5,
            ErrCmdType::Local,
            LOCAL_TASK_TIMEOUT,
            TaskFailHandleType::TaskEnd
        )]
    );
    assert!(manager.is_empty());
}

#[test]
fn the_check_keeps_going_while_there_is_something_to_check() {
    let (mut manager, _started, _failed) = manager();
    assert!(manager.save_task_at(0, &kept(6, Task::TASK_PRIORITY_NORMAL), 0));

    manager.on_timer_check_at(TIMER_INTERVAL);
    assert_eq!(manager.len(), 1);
    assert_eq!(manager.due_time(), Some(TIMER_INTERVAL * 2));
}

#[test]
fn stopping_and_clearing_the_tasks() {
    let (mut manager, _started, _failed) = manager();
    assert!(manager.save_task_at(0, &kept(1, Task::TASK_PRIORITY_NORMAL), 0));
    assert!(manager.save_task_at(0, &kept(2, Task::TASK_PRIORITY_NORMAL), 0));

    assert!(manager.stop_task(1));
    assert!(!manager.has_task(1));
    assert!(!manager.stop_task(1), "it is gone already");
    assert_eq!(manager.len(), 1);
    // the remaining one keeps the check alive
    assert_eq!(manager.due_time(), Some(TIMER_INTERVAL));

    manager.clear_tasks();
    assert!(manager.is_empty());
    assert_eq!(manager.due_time(), None);
}

#[test]
fn without_the_app_the_tasks_are_still_kept() {
    let mut manager = ZombieTaskManager::default();
    assert!(manager.save_task_at(0, &kept(1, Task::TASK_PRIORITY_NORMAL), 0));
    manager.redo_tasks_at(10);
    assert!(manager.is_empty(), "started and forgotten");
    manager.on_timer_check_at(10);
    assert!(format!("{manager:?}").contains("ZombieTaskManager"));
}

#[test]
fn the_methods_that_take_no_reading_ask_the_clock_themselves() {
    let mut manager = ZombieTaskManager::new();
    assert!(manager.save_task(&kept(1, Task::TASK_PRIORITY_NORMAL), 0));
    assert!(manager.has_task(1));
    assert!(manager.due_time().is_some());

    manager.on_net_core_start_task();
    manager.on_timer_check();
    // the deadline is a minute away, so the check has nothing to do yet
    assert!(manager.has_task(1));
    manager.redo_tasks();
    assert!(manager.is_empty());
}
