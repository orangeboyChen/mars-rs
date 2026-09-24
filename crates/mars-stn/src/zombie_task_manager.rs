//! `mars/stn/src/zombie_task_manager.cc` — the tasks that outlived the link
//! they were started on.
//!
//! A task whose channel went away is not failed: it is *saved* here
//! ([`ZombieTaskManager::save_task`]), and started again when the link comes
//! back ([`ZombieTaskManager::redo_tasks`]) or when the periodic check decides
//! enough time has passed. What ends a zombie is its own deadline: once the
//! time since it was saved reaches `total_timeout`, the app is told it failed
//! with [`ErrCmdType::Local`] / [`LOCAL_TASK_TIMEOUT`].
//!
//! Two things the C++ gets from its `MessageQueue` are arguments here:
//!
//! * the `SingletonMessage(…, MessageTiming(3000, 3000))` that `SaveTask`
//!   starts — a check every [`TIMER_INTERVAL`] — is a due time the host
//!   compares its own clock against ([`ZombieTaskManager::due_time`]), and
//!   [`ZombieTaskManager::on_timer_check_at`] is what it calls. The C++ cancels
//!   it when the last zombie is gone, so the due time goes with it;
//! * `gettickcount()` is handed in by the `*_at` methods.
//!
//! Both `fun_start_task_` and `fun_callback_` are set with `set_*` here, and
//! an unset one does nothing — the C++ would have hit `xassert2`.

use crate::task::Task;
use crate::task_profile::{ErrCmdType, TaskFailHandleType, LOCAL_TASK_TIMEOUT};

/// `RETRY_INTERVAL` — how long a zombie waits before it is started again, and
/// how long the net core has to have been idle for that to happen.
///
/// `static uint64_t` in the C++ and never written to, so a constant here.
pub const RETRY_INTERVAL: u64 = 60 * 1000;

/// `MessageTiming(3000, 3000)` — the period of the check `SaveTask` starts.
pub const TIMER_INTERVAL: u64 = 3000;

/// `fun_start_task_`.
pub type StartTask = dyn FnMut(&Task) + Send;

/// `fun_callback_`.
pub type ZombieCallback = dyn FnMut(ErrCmdType, i32, TaskFailHandleType, &Task, u32) + Send;

/// A saved task and the reading it was saved at.
struct ZombieTask {
    task: Task,
    /// `save_time`
    save_time: u64,
}

/// `ZombieTaskManager`.
pub struct ZombieTaskManager {
    /// `lsttask_` — a `std::list`, so the order tasks were saved in is kept.
    tasks: Vec<ZombieTask>,
    /// `net_core_last_start_task_time_`
    net_core_last_start_task_time: u64,
    /// When the periodic check is due; [`None`] while there is nothing to
    /// check.
    next_check: Option<u64>,
    start: Option<Box<StartTask>>,
    callback: Option<Box<ZombieCallback>>,
}

impl ZombieTaskManager {
    /// `ZombieTaskManager(_messagequeueid)` — `net_core_last_start_task_time_`
    /// starts at the reading the clock hands out now.
    pub fn new() -> Self {
        Self::new_at(mars_comm::tickcount::gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn new_at(now: u64) -> Self {
        Self {
            tasks: Vec::new(),
            net_core_last_start_task_time: now,
            next_check: None,
            start: None,
            callback: None,
        }
    }

    /// `fun_start_task_ = …`.
    pub fn set_start_task(&mut self, start: impl FnMut(&Task) + Send + 'static) {
        self.start = Some(Box::new(start));
    }

    /// `fun_callback_ = …`.
    pub fn set_callback(
        &mut self,
        callback: impl FnMut(ErrCmdType, i32, TaskFailHandleType, &Task, u32) + Send + 'static,
    ) {
        self.callback = Some(Box::new(callback));
    }

    /// `fun_start_task_ = NULL` — a zombie that is started again does nothing.
    pub fn clear_start_task(&mut self) {
        self.start = None;
    }

    /// `fun_callback_ = NULL`.
    pub fn clear_callback(&mut self) {
        self.callback = None;
    }

    /// How many zombies are saved.
    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Whether there is nothing saved.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// `net_core_last_start_task_time_`.
    pub fn net_core_last_start_task_time(&self) -> u64 {
        self.net_core_last_start_task_time
    }

    /// The reading the periodic check is due at, [`None`] when there is none.
    pub fn due_time(&self) -> Option<u64> {
        self.next_check
    }

    /// `SaveTask(_task, _taskcosttime)` — `false` when the task is not kept: a
    /// [`Task::network_status_sensitive`] one, or one whose `total_timeout` the
    /// time it already spent has used up.
    ///
    /// What is saved is the task with its `retry_count` back at `0` and the
    /// time it already spent taken off `total_timeout`, which is the deadline
    /// that is measured from now on.
    pub fn save_task(&mut self, task: &Task, task_cost_time: u32) -> bool {
        self.save_task_at(mars_comm::tickcount::gettickcount(), task, task_cost_time)
    }

    /// The same, with the reading handed in.
    pub fn save_task_at(&mut self, now: u64, task: &Task, task_cost_time: u32) -> bool {
        if task.network_status_sensitive {
            return false;
        }

        let mut task = task.clone();
        task.retry_count = 0;
        // `total_timeout -= _taskcosttime`, and the C++ compares the `int`
        // against `0` afterwards: a task whose deadline the time it spent has
        // used up is not kept.
        task.total_timeout = task.total_timeout.saturating_sub(task_cost_time as i32);
        if task.total_timeout <= 0 {
            return false;
        }

        self.tasks.push(ZombieTask {
            task,
            save_time: now,
        });
        // `SingletonMessage(false, …, MessageTiming(3000, 3000))` — one check
        // on a 3s period, and only while there is something to check
        self.next_check = Some(
            self.next_check
                .unwrap_or(now.saturating_add(TIMER_INTERVAL)),
        );
        true
    }

    /// `StopTask(_taskid)` — `true` when there was such a zombie.
    pub fn stop_task(&mut self, taskid: u32) -> bool {
        let Some(at) = self
            .tasks
            .iter()
            .position(|zombie| zombie.task.taskid == taskid)
        else {
            return false;
        };
        self.tasks.remove(at);
        self.arm_if_needed();
        true
    }

    /// `HasTask(_taskid)`.
    pub fn has_task(&self, taskid: u32) -> bool {
        self.tasks.iter().any(|zombie| zombie.task.taskid == taskid)
    }

    /// `ClearTasks()`.
    pub fn clear_tasks(&mut self) {
        self.tasks.clear();
        self.next_check = None;
    }

    /// `RedoTasks()` — `__StartTask`: start every zombie again, most urgent
    /// first, except the ones whose deadline has run out, which are failed.
    pub fn redo_tasks(&mut self) {
        self.redo_tasks_at(mars_comm::tickcount::gettickcount())
    }

    /// The same, with the reading handed in.
    ///
    /// The tasks are taken out first, so a `fun_start_task_` that saves a task
    /// again does not feed the loop it was started from — which is what the
    /// C++'s copy of `lsttask_` does too.
    pub fn redo_tasks_at(&mut self, now: u64) {
        if self.tasks.is_empty() {
            return;
        }
        let mut batch: Vec<ZombieTask> = std::mem::take(&mut self.tasks);
        // `lsttask.sort(__compare_task)` — `std::list::sort` is stable, and so
        // is `sort_by_key`
        batch.sort_by_key(|zombie| zombie.task.priority);
        self.next_check = None;

        for zombie in batch.iter_mut() {
            let spent = now.saturating_sub(zombie.save_time);
            if spent >= zombie.task.total_timeout.max(0) as u64 {
                self.fail(zombie, spent);
            } else {
                zombie.task.total_timeout = zombie.task.total_timeout.saturating_sub(spent as i32);
                if let Some(start) = self.start.as_mut() {
                    start(&zombie.task);
                }
            }
        }
    }

    /// `OnNetCoreStartTask()`.
    pub fn on_net_core_start_task(&mut self) {
        self.on_net_core_start_task_at(mars_comm::tickcount::gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn on_net_core_start_task_at(&mut self, now: u64) {
        self.net_core_last_start_task_time = now;
    }

    /// `__TimerChecker` — what the posted check does: fail the zombies whose
    /// deadline has run out, and start the ones that waited `RETRY_INTERVAL`
    /// while the net core was idle for `RETRY_INTERVAL` too.
    pub fn on_timer_check(&mut self) {
        self.on_timer_check_at(mars_comm::tickcount::gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn on_timer_check_at(&mut self, now: u64) {
        // `uint64_t netCoreLastStartTaskTime = net_core_last_start_task_time_`:
        // the C++ reads it once, before the loop starts any task
        let net_core_last_start_task_time = self.net_core_last_start_task_time;
        let mut kept = Vec::with_capacity(self.tasks.len());

        for mut zombie in std::mem::take(&mut self.tasks) {
            let spent = now.saturating_sub(zombie.save_time);
            if spent >= zombie.task.total_timeout.max(0) as u64 {
                self.fail(&zombie, spent);
            } else if spent >= RETRY_INTERVAL
                && now.saturating_sub(net_core_last_start_task_time) >= RETRY_INTERVAL
            {
                zombie.task.total_timeout = zombie.task.total_timeout.saturating_sub(spent as i32);
                if let Some(start) = self.start.as_mut() {
                    start(&zombie.task);
                }
            } else {
                kept.push(zombie);
            }
        }
        self.tasks = kept;

        // `CancelMessage` once the last zombie is gone
        self.arm_if_needed();
        if !self.tasks.is_empty() {
            self.next_check = Some(now.saturating_add(TIMER_INTERVAL));
        }
    }

    /// `fun_callback_(kEctLocal, kEctLocalTaskTimeout, kTaskFailHandleTaskEnd,
    /// …)`.
    fn fail(&mut self, zombie: &ZombieTask, spent: u64) {
        if let Some(callback) = self.callback.as_mut() {
            callback(
                ErrCmdType::Local,
                LOCAL_TASK_TIMEOUT,
                TaskFailHandleType::TaskEnd,
                &zombie.task,
                spent as u32,
            );
        }
    }

    /// The check has nothing to do once the last zombie is gone.
    fn arm_if_needed(&mut self) {
        if self.tasks.is_empty() {
            self.next_check = None;
        }
    }
}

impl Default for ZombieTaskManager {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ZombieTaskManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZombieTaskManager")
            .field("tasks", &self.tasks.len())
            .field(
                "net_core_last_start_task_time",
                &self.net_core_last_start_task_time,
            )
            .field("next_check", &self.next_check)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn a_task(taskid: u32) -> Task {
        Task::new(taskid, 1)
    }

    /// What the app was asked to start again, and what it was told failed —
    /// the task id, the error code and the fail handle.
    type Recorded = (Arc<Mutex<Vec<u32>>>, Arc<Mutex<Vec<(u32, i32, i32)>>>);

    /// A manager that records what it starts and what it fails.
    fn manager() -> (ZombieTaskManager, Recorded) {
        let mut manager = ZombieTaskManager::new_at(0);
        let started = Arc::new(Mutex::new(Vec::new()));
        let failed = Arc::new(Mutex::new(Vec::new()));
        let record_start = Arc::clone(&started);
        manager.set_start_task(move |task| {
            record_start
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(task.taskid);
        });
        let record_fail = Arc::clone(&failed);
        manager.set_callback(move |_err_type, err_code, fail_handle, task, _cost| {
            record_fail.lock().unwrap_or_else(|e| e.into_inner()).push((
                task.taskid,
                err_code,
                fail_handle as i32,
            ));
        });
        (manager, (started, failed))
    }

    #[test]
    fn a_task_that_cannot_be_kept_is_not_saved() {
        let (mut manager, (_started, _failed)) = manager();

        // sensitive to the network status
        let mut sensitive = a_task(1);
        sensitive.network_status_sensitive = true;
        assert!(!manager.save_task_at(0, &sensitive, 0));
        assert!(manager.is_empty());

        // ... and one whose deadline the time it spent has used up
        let mut spent = a_task(2);
        spent.total_timeout = 500;
        assert!(!manager.save_task_at(0, &spent, 500));
        assert!(!manager.save_task_at(0, &spent, 501));
        assert!(manager.is_empty());
    }

    #[test]
    fn a_saved_task_loses_the_time_it_spent_and_its_retries() {
        let (mut manager, (_started, _failed)) = manager();
        let mut task = a_task(7);
        task.total_timeout = 1_000;
        task.retry_count = 3;

        assert!(manager.save_task_at(1_000, &task, 300));
        assert_eq!(manager.len(), 1);
        assert!(manager.has_task(7));
        assert_eq!(manager.due_time(), Some(1_000 + TIMER_INTERVAL));

        // the saved copy is the one that was changed
        assert_eq!(task.retry_count, 3, "the caller's task is not touched");
        assert_eq!(task.total_timeout, 1_000);

        // a second one does not move the due time of the first
        let mut other = a_task(8);
        other.total_timeout = 2_000;
        assert!(manager.save_task_at(2_000, &other, 0));
        assert_eq!(manager.due_time(), Some(1_000 + TIMER_INTERVAL));
        assert_eq!(manager.len(), 2);
    }

    #[test]
    fn redoing_the_tasks_starts_them_most_urgent_first() {
        let (mut manager, (started, failed)) = manager();
        let mut slow = a_task(1);
        slow.priority = 5;
        slow.total_timeout = 10_000;
        let mut urgent = a_task(2);
        urgent.priority = 0;
        urgent.total_timeout = 10_000;
        let mut dead = a_task(3);
        dead.total_timeout = 100;

        assert!(manager.save_task_at(0, &slow, 0));
        assert!(manager.save_task_at(0, &urgent, 0));
        assert!(manager.save_task_at(0, &dead, 0));

        manager.redo_tasks_at(200);
        assert_eq!(
            *started.lock().unwrap_or_else(|e| e.into_inner()),
            vec![2, 1],
            "urgent first, and the one whose deadline ran out is not started"
        );
        assert_eq!(
            *failed.lock().unwrap_or_else(|e| e.into_inner()),
            vec![(3, LOCAL_TASK_TIMEOUT, TaskFailHandleType::TaskEnd as i32)]
        );
        // the deadine of the ones that were started has the wait taken off it
        assert!(manager.is_empty());
        assert_eq!(manager.due_time(), None);
    }

    #[test]
    fn the_periodic_check_only_restarts_a_task_that_waited_long_enough() {
        let (mut manager, (started, failed)) = manager();
        let mut task = a_task(4);
        task.total_timeout = 10 * 60_000;

        assert!(manager.save_task_at(0, &task, 0));

        // before RETRY_INTERVAL nothing happens
        manager.on_timer_check_at(RETRY_INTERVAL - 1);
        assert!(started.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
        assert_eq!(manager.len(), 1);

        // ... and neither does it when the net core started a task recently:
        // the manager was built at 0, so `OnNetCoreStartTask` moves it
        manager.on_net_core_start_task_at(RETRY_INTERVAL);
        manager.on_timer_check_at(RETRY_INTERVAL * 2 - 1);
        assert!(started.lock().unwrap_or_else(|e| e.into_inner()).is_empty());

        // both have to have waited
        manager.on_timer_check_at(RETRY_INTERVAL * 2);
        assert_eq!(
            *started.lock().unwrap_or_else(|e| e.into_inner()),
            vec![4],
            "the task waited RETRY_INTERVAL and so did the net core"
        );
        assert!(manager.is_empty());
        assert!(failed.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
    }

    #[test]
    fn a_zombie_whose_deadline_ran_out_is_failed_by_the_check() {
        let (mut manager, (started, failed)) = manager();
        let mut task = a_task(5);
        task.total_timeout = 1_000;

        assert!(manager.save_task_at(500, &task, 0));
        manager.on_timer_check_at(1_500);
        assert!(started.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
        assert_eq!(
            *failed.lock().unwrap_or_else(|e| e.into_inner()),
            vec![(5, LOCAL_TASK_TIMEOUT, TaskFailHandleType::TaskEnd as i32)]
        );
        assert!(manager.is_empty());
        assert_eq!(manager.due_time(), None, "the check cancels itself");
    }

    #[test]
    fn the_check_keeps_going_while_there_is_something_to_check() {
        let (mut manager, (_started, _failed)) = manager();
        let mut task = a_task(6);
        task.total_timeout = 10 * 60_000;
        assert!(manager.save_task_at(0, &task, 0));

        manager.on_timer_check_at(TIMER_INTERVAL);
        assert_eq!(manager.len(), 1);
        assert_eq!(manager.due_time(), Some(TIMER_INTERVAL * 2));
        assert_eq!(manager.net_core_last_start_task_time(), 0);
    }

    #[test]
    fn stopping_and_clearing_the_tasks() {
        let (mut manager, (_started, _failed)) = manager();
        let mut one = a_task(1);
        one.total_timeout = 1_000;
        let mut two = a_task(2);
        two.total_timeout = 1_000;
        assert!(manager.save_task_at(0, &one, 0));
        assert!(manager.save_task_at(0, &two, 0));

        assert!(manager.stop_task(1));
        assert!(!manager.has_task(1));
        assert!(!manager.stop_task(1));
        assert_eq!(manager.len(), 1);

        manager.clear_tasks();
        assert!(manager.is_empty());
        assert_eq!(manager.due_time(), None);
        assert!(!manager.has_task(2));
    }

    #[test]
    fn without_the_app_the_tasks_are_still_kept() {
        let mut manager = ZombieTaskManager::default();
        let mut task = a_task(1);
        task.total_timeout = 1_000;
        assert!(manager.save_task_at(0, &task, 0));
        manager.redo_tasks_at(10);
        assert!(manager.is_empty(), "started and forgotten");
        manager.on_timer_check_at(10);
        assert!(format!("{manager:?}").contains("ZombieTaskManager"));
    }
}
