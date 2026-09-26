//! `mars/stn/src/shortlink_task_manager.cc` — the queue of short-link tasks.
//!
//! A task that goes out on a short link is a [`TaskProfile`] in a list, and
//! this is the list: what a task is waiting for, when it may go out
//! ([`ShortLinkTaskManager::run_loop_at`]), and what an answer does to it
//! ([`ShortLinkTaskManager::on_response_at`]). Two decisions are what the
//! queue is for:
//!
//! * **when a task is over** — [`Timeout`] is the five ways the C++ notices a
//!   task that answered nothing, and [`RespHandle`] is whether an answer ends
//!   the task or buys it another try;
//! * **which try goes through a proxy** — every task that failed without one
//!   flips [`ShortLinkTaskManager::default_use_proxy`], which is the "the
//!   network will not let us through a proxy" switch of the C++.
//!
//! A run is not here. The C++ makes a `ShortLinkInterface` per task, runs it on
//! a `MessageQueue` and keeps an `intptr_t` to it; the port has neither sockets
//! nor threads, so what the queue keeps is a [`RunId`] the host handed out when
//! it started one ([`StartRun`]) and hands back with every answer. Everything
//! the C++ asked the worker — its profile, whether it is keep-alive, when it
//! sent — is an argument here instead. What is left out:
//!
//! * the `DEF_TASK_RUN_LOOP_TIMING` message `__RunLoop` posts to itself is
//!   [`ShortLinkTaskManager::due_time`] instead, which is when the host has to
//!   call the loop, and [`None`] when it never has to;
//! * `Req2Buf` / `Buf2Resp` / `MakesureAuthed` — the app's own encoder and
//!   decoder — are hooks, and so is `fun_callback_`;
//! * the QUIC branch of `__RunOnStartTask`, the tls and handshake callbacks,
//!   `get_real_host_` and `task_connection_detail_`, are not ported; nor is the
//!   weak-network bookkeeping of `__OnRecv`, which is `net_source_`'s and comes
//!   with `net_core`.

use mars_comm::tickcount::gettickcount;

use crate::config::{DYN_TIME_TASK_FAILED_PKG_LEN, MOBILE_PACKAGE_INTERVAL, WIFI_PACKAGE_INTERVAL};
use crate::dynamic_timeout::{DynamicTimeout, DynamicTimeoutStatus, NetworkKind};
use crate::long_link::{ECT_SOCKET_MAKE_SOCKET_PREPARED, ECT_SOCKET_SHUTDOWN};
use crate::shortlink::is_keep_alive;
use crate::simple_ipport_sort::IpPortItem;
use crate::socket_operator::SocketFd;
use crate::socket_pool::{CachedSocket, CloseSocket, SocketPool};
use crate::task::Task;
use crate::task_intercept::TaskIntercept;
use crate::task_profile::{
    compare_task, first_pkg_timeout, read_write_timeout, ConnectProfile, ErrCmdType,
    PrepareProfile, RunId, TaskFailHandleType, TaskProfile, HANDSHAKE_MISUNDERSTAND,
    HTTP_FIRST_PKG_TIMEOUT, HTTP_LONG_POLLING_TIMEOUT, HTTP_PKG_PKG_TIMEOUT,
    HTTP_READ_WRITE_TIMEOUT, LOCAL_ANTI_AVALANCHE, LOCAL_CANCEL, LOCAL_RESET, LOCAL_TASK_TIMEOUT,
};

/// `DEF_TASK_RETRY_INTERNAL` — how long a task that is going to be tried again
/// waits before the next try.
pub const RETRY_INTERNAL: u64 = 1000;

/// `DEF_TASK_RUN_LOOP_TIMING` — how often the C++ runs its loop while there is
/// something in the queue. The port says when it is actually needed
/// ([`ShortLinkTaskManager::due_time`]) instead, so this is only what a host
/// that would rather poll falls back to.
pub const RUN_LOOP_TIMING: u64 = 1000;

/// One of the five ways `__RunOnTimeout` notices a task that answered nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timeout {
    /// `kEctLocalTaskTimeout` — the task ran out of time, retries and all. It is
    /// the one timeout that applies to a task whose run never started.
    Task,
    /// `kEctHttpReadWriteTimeout` — the whole read took too long.
    ReadWrite,
    /// `kEctHttpLongPollingTimeout` — a long-polling task heard nothing at all.
    LongPolling,
    /// `kEctHttpFirstPkgTimeout` — the first package never came.
    FirstPkg,
    /// `kEctHttpPkgPkgTimeout` — a package came, and then nothing for longer
    /// than the network is given between two of them.
    PkgPkg,
}

impl Timeout {
    /// `kEctLocal` for the task's own timeout, `kEctHttp` for a read that ran
    /// out of time.
    pub fn err_type(self) -> ErrCmdType {
        match self {
            Timeout::Task => ErrCmdType::Local,
            _ => ErrCmdType::Http,
        }
    }

    /// `socket_timeout_code` — the error code of this timeout.
    pub fn err_code(self) -> i32 {
        match self {
            Timeout::Task => LOCAL_TASK_TIMEOUT,
            Timeout::ReadWrite => HTTP_READ_WRITE_TIMEOUT,
            Timeout::LongPolling => HTTP_LONG_POLLING_TIMEOUT,
            Timeout::FirstPkg => HTTP_FIRST_PKG_TIMEOUT,
            Timeout::PkgPkg => HTTP_PKG_PKG_TIMEOUT,
        }
    }

    /// What the app is told to do about it: `kTaskFailHandleTaskTimeout` for a
    /// task that ran out of time, `kTaskFailHandleDefault` for the rest —
    /// which is to say, try again if the task has tries left.
    pub fn fail_handle(self) -> TaskFailHandleType {
        match self {
            Timeout::Task => TaskFailHandleType::TaskTimeout,
            _ => TaskFailHandleType::Default,
        }
    }
}

/// What a queue did with an answer: [`ShortLinkTaskManager::on_response_at`]
/// and [`crate::LongLinkTaskManager::on_response_at`] both answer with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RespHandle {
    /// The task is over: it left the queue and the app was told.
    Ended,
    /// The task has another try coming: it is still in the queue, waiting out
    /// [`RETRY_INTERNAL`].
    Retried,
    /// The task answered, but the app was asked about *every* task instead
    /// (`kTaskFailHandleSessionTimeout` / `kTaskFailHandleRetryAllTasks`): it
    /// stays in the queue until the queue's own `retry_tasks` says what to do.
    Deferred,
}

impl RespHandle {
    /// Whether the task left the queue.
    pub fn is_ended(self) -> bool {
        self == RespHandle::Ended
    }
}

/// `ShortlinkConfig` plus the two timeouts `__RunOnStartTask` worked out — what
/// the host is handed to start one task on.
#[derive(Debug, Clone, PartialEq)]
pub struct RunRequest {
    /// `bufreq` — what `Req2Buf` wrote, which is the same bytes the anti-avalanche
    /// check weighed and the two timeouts were worked out from: the C++ hands
    /// them to `SendRequest`, so a host does not encode the task twice.
    pub body: Vec<u8>,
    /// `use_proxy` — whether this try goes through a proxy.
    pub use_proxy: bool,
    /// `debug_host_` — empty when the app set none.
    pub debug_host: String,
    /// `transfer_profile.first_pkg_timeout`.
    pub first_pkg_timeout: u64,
    /// `transfer_profile.read_write_timeout`.
    pub read_write_timeout: u64,
    /// How many tasks of this queue are on a run already: what makes every
    /// first-package timeout a little longer.
    pub sent_count: i32,
}

/// What a run answers with — `__OnResponse`'s arguments, minus the worker,
/// which is the [`RunId`] here, and minus the extension, which the app's own
/// decoder reads out of the body.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    /// `_err_type`.
    pub err_type: ErrCmdType,
    /// `_status` — the error code, or the HTTP status of an answer.
    pub status: i32,
    /// `_body`.
    pub body: Vec<u8>,
    /// `_cancel_retry` — a run that asked for the task not to be tried again.
    pub cancel_retry: bool,
    /// `_conn_profile` — the connect the run was made on, which is what the
    /// socket pool and the app's error report read.
    pub profile: ConnectProfile,
}

/// `ShortLinkChannelFactory::Create` + `SendRequest` — the host starts one run
/// of one task and names it. [`None`] is a run that never began.
pub type StartRun = dyn FnMut(&Task, &RunRequest) -> Option<RunId> + Send;

/// `ShortLinkChannelFactory::Destory` — the queue is done with a run: whether
/// the task is over or has another try coming, the one that was out is not the
/// one that answers any more.
pub type DestroyRun = dyn FnMut(RunId) + Send;

/// `fun_callback_` — a task that is over, and what the app says about it: the
/// answer is the code the task is remembered with.
///
/// The connect profile comes along because the C++'s `NetCore::__CallBack`
/// reads `GetConnectProfile(_taskid, …)` out of the queue it was called from,
/// and a hook is not given a borrow of the queue that calls it: what the app is
/// handed for a task that answered is the connect of the run that answered.
pub type Callback =
    dyn FnMut(ErrCmdType, i32, TaskFailHandleType, &Task, u32, &ConnectProfile) -> i32 + Send;

/// `fun_notify_network_err_` — an error the app is told about, and the pair it
/// happened on. The C++ passes `__LINE__` too, which is its own bookkeeping and
/// not a port's.
pub type NotifyNetworkErr = dyn FnMut(ErrCmdType, i32, &str, &str, u16) + Send;

/// `fun_notify_retry_all_tasks` — a session timeout, or an answer that could
/// not be read: every task has to be looked at again.
pub type NotifyRetryAllTasks = dyn FnMut(ErrCmdType, i32, TaskFailHandleType, u32, &str) + Send;

/// `fun_shortlink_response_` — the status that came back, whatever it was.
pub type ResponseStatus = dyn FnMut(i32) + Send;

/// `Req2Buf` — the app writes the body of the request. `Err` is its error code,
/// which fails the task with `kEctEnDecode`.
pub type Req2Buf = dyn FnMut(&Task) -> Result<Vec<u8>, i32> + Send;

/// `Buf2Resp` — the app reads the body of an answer: `(err_code, handle_type)`.
pub type Buf2Resp = dyn FnMut(&Task, &[u8]) -> (i32, TaskFailHandleType) + Send;

/// `MakesureAuthed(host, user_id)` — whether the task may go out now, which for
/// a task that needs it is the app's to answer.
pub type MakeSureAuthed = dyn FnMut(&str, &str) -> bool + Send;

/// `fun_anti_avalanche_check_` — whether this task may go out at all.
pub type AntiAvalancheCheck = dyn FnMut(&Task, &[u8]) -> bool + Send;

/// `should_intercept_result_` — whether an answer is one to keep and hand out
/// again, which is what fills the [`TaskIntercept`].
pub type ShouldIntercept = dyn FnMut(i32) -> bool + Send;

/// `getNetInfo()` — which network the timeouts are the ones of, and which of
/// the two package intervals a task is given between two packages.
pub type NetInfo = dyn FnMut() -> NetworkKind + Send;

/// `StnManager::GenSequenceId()` — the sequence id of the request, which the
/// C++ draws again for every try: `client_sequence_id 在buf2resp这里生成,防止重
/// 试sequence_id一样`, a retry is not taken for the request it is a retry of.
pub type GenSequenceId = dyn FnMut() -> u16 + Send;

/// `ShortLinkTaskManager`.
pub struct ShortLinkTaskManager {
    /// `lst_cmd_`, sorted by [`crate::task_profile::compare_task`].
    tasks: Vec<TaskProfile>,
    /// `default_use_proxy_`.
    default_use_proxy: bool,
    /// `tasks_continuous_fail_count_`.
    tasks_continuous_fail_count: u32,
    /// `dynamic_timeout_`.
    dynamic_timeout: DynamicTimeout,
    /// `debug_host_`.
    debug_host: String,
    /// `socket_pool_`.
    socket_pool: SocketPool,
    /// `task_intercept_`.
    intercept: TaskIntercept,
    /// What the next run an unset [`StartRun`] is asked for is called, so that
    /// two runs never answer for the same task.
    next_run_id: u64,
    start: Option<Box<StartRun>>,
    destroy: Option<Box<DestroyRun>>,
    callback: Option<Box<Callback>>,
    notify_network_err: Option<Box<NotifyNetworkErr>>,
    notify_retry_all_tasks: Option<Box<NotifyRetryAllTasks>>,
    response_status: Option<Box<ResponseStatus>>,
    req2buf: Option<Box<Req2Buf>>,
    buf2resp: Option<Box<Buf2Resp>>,
    make_sure_authed: Option<Box<MakeSureAuthed>>,
    anti_avalanche: Option<Box<AntiAvalancheCheck>>,
    should_intercept: Option<Box<ShouldIntercept>>,
    net_info: Option<Box<NetInfo>>,
    gen_sequence_id: Option<Box<GenSequenceId>>,
    /// `closefunc` — what the C++ closes a socket with, which the queue needs
    /// for one a run answered badly on. The pool's own is
    /// [`SocketPool::set_close`].
    close: Option<Box<CloseSocket>>,
}

impl ShortLinkTaskManager {
    /// `ShortLinkTaskManager(...)` — a queue with nothing in it, and a proxy
    /// every try goes through until one that did not comes back.
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            default_use_proxy: true,
            tasks_continuous_fail_count: 0,
            dynamic_timeout: DynamicTimeout::new(),
            debug_host: String::new(),
            socket_pool: SocketPool::new(),
            intercept: TaskIntercept::new(),
            next_run_id: 0,
            start: None,
            destroy: None,
            callback: None,
            notify_network_err: None,
            notify_retry_all_tasks: None,
            response_status: None,
            req2buf: None,
            buf2resp: None,
            make_sure_authed: None,
            anti_avalanche: None,
            should_intercept: None,
            net_info: None,
            gen_sequence_id: None,
            close: None,
        }
    }

    /// `StartTask(_task, _prepare_profile)` — `false` when the task is a
    /// `send_only` one, which a short link is not: an answer is what a short
    /// link is for.
    pub fn start_task(&mut self, task: Task, prepare_profile: PrepareProfile) -> bool {
        self.start_task_at(gettickcount(), task, prepare_profile)
    }

    /// The same, with the reading the task is started at handed in.
    pub fn start_task_at(&mut self, now: u64, task: Task, prepare_profile: PrepareProfile) -> bool {
        if task.send_only {
            return false;
        }

        let mut profile = TaskProfile::new_at(now, task, prepare_profile);
        profile.link_type = Task::CHANNEL_SHORT;
        self.tasks.push(profile);
        // `lst_cmd_.sort(__CompareTask)` — a stable sort, so two tasks of one
        // priority stay in the order they were asked for
        self.tasks.sort_by(compare_task);

        self.run_loop_at(now);
        true
    }

    /// `StopTask(_taskid)` — `true` when there was such a task. Its run, if one
    /// was out, is over.
    pub fn stop_task(&mut self, taskid: u32) -> bool {
        let Some(at) = self.tasks.iter().position(|p| p.task.taskid == taskid) else {
            return false;
        };
        self.stop_run_at(at);
        self.tasks.remove(at);
        true
    }

    /// `HasTask(_taskid)`.
    pub fn has_task(&self, taskid: u32) -> bool {
        self.tasks.iter().any(|p| p.task.taskid == taskid)
    }

    /// `ClearTasks()`.
    pub fn clear_tasks(&mut self) {
        for at in 0..self.tasks.len() {
            self.stop_run_at(at);
        }
        self.tasks.clear();
    }

    /// `RedoTasks()` — every task that is out is cancelled and tried again,
    /// which is what a network change asks for: the sockets the pool was
    /// keeping are not the network's any more.
    pub fn redo_tasks(&mut self) {
        self.redo_tasks_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn redo_tasks_at(&mut self, now: u64) {
        let mut i = 0;
        while i < self.tasks.len() {
            self.tasks[i].last_failed_dyntime_status = DynamicTimeoutStatus::default();
            if self.tasks[i].running.is_some() {
                let profile = self.tasks[i].transfer_profile.connect_profile.clone();
                let ended = self.single_resp_handle_at(
                    now,
                    i,
                    ErrCmdType::Local,
                    LOCAL_CANCEL,
                    TaskFailHandleType::Default,
                    profile,
                );
                if !ended {
                    i += 1;
                }
                continue;
            }
            i += 1;
        }

        self.socket_pool.clear();
        self.run_loop_at(now);
    }

    /// `TouchTasks()` — `__RunLoop`.
    pub fn touch_tasks(&mut self) {
        self.touch_tasks_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn touch_tasks_at(&mut self, now: u64) {
        self.run_loop_at(now);
    }

    /// `RetryTasks(_err_type, _err_code, _fail_handle, _src_taskid)` — what the
    /// app answered to [`RespHandle::Deferred`].
    pub fn retry_tasks(
        &mut self,
        err_type: ErrCmdType,
        err_code: i32,
        fail_handle: TaskFailHandleType,
        src_taskid: u32,
    ) {
        self.retry_tasks_at(gettickcount(), err_type, err_code, fail_handle, src_taskid)
    }

    /// The same, with the reading handed in.
    pub fn retry_tasks_at(
        &mut self,
        now: u64,
        err_type: ErrCmdType,
        err_code: i32,
        fail_handle: TaskFailHandleType,
        src_taskid: u32,
    ) {
        self.batch_error_resp_handle_at(now, err_type, err_code, fail_handle, src_taskid, true);
        self.run_loop_at(now);
    }

    /// `__RunLoop` — `__RunOnTimeout` and then `__RunOnStartTask`, which is one
    /// pass of the queue.
    pub fn run_loop(&mut self) {
        self.run_loop_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn run_loop_at(&mut self, now: u64) {
        if self.tasks.is_empty() {
            return;
        }
        self.run_on_timeout_at(now);
        self.run_on_start_task_at(now);
    }

    /// `__OnSend` — the run sent the request, which is what the read-write and
    /// the first-package timeouts are counted from. `false` when no run of that
    /// name is in the queue.
    pub fn on_send(&mut self, run_id: RunId) -> bool {
        self.on_send_at(gettickcount(), run_id)
    }

    /// The same, with the reading handed in.
    pub fn on_send_at(&mut self, now: u64, run_id: RunId) -> bool {
        let Some(at) = self.locate(run_id) else {
            return false;
        };
        let profile = &mut self.tasks[at].transfer_profile;
        if profile.first_start_send_time == 0 {
            profile.first_start_send_time = now;
        }
        profile.start_send_time = now;
        true
    }

    /// `__OnRecv` — a package of the answer came in, which is what the
    /// pkg-pkg timeout is counted from. `false` when no run of that name is in
    /// the queue.
    pub fn on_recv(&mut self, run_id: RunId, cached_size: usize, total_size: usize) -> bool {
        self.on_recv_at(gettickcount(), run_id, cached_size, total_size)
    }

    /// The same, with the reading handed in.
    pub fn on_recv_at(
        &mut self,
        now: u64,
        run_id: RunId,
        cached_size: usize,
        total_size: usize,
    ) -> bool {
        let Some(at) = self.locate(run_id) else {
            return false;
        };
        let profile = &mut self.tasks[at].transfer_profile;
        profile.last_receive_pkg_time = now;
        profile.received_size = cached_size;
        profile.receive_data_size = total_size;
        true
    }

    /// `__OnResponse` — what a run answered with. [`None`] when no run of that
    /// name is in the queue, which is an answer about a task the queue has
    /// forgotten.
    pub fn on_response(&mut self, run_id: RunId, response: Response) -> Option<RespHandle> {
        self.on_response_at(gettickcount(), run_id, response)
    }

    /// The same, with the reading handed in.
    pub fn on_response_at(
        &mut self,
        now: u64,
        run_id: RunId,
        response: Response,
    ) -> Option<RespHandle> {
        if let Some(status) = self.response_status.as_mut() {
            status(response.status);
        }

        // "must used iter pWorker, not used aSelf": a run, not a task, is what
        // an answer is about
        let at = self.locate(run_id)?;
        let task = self.tasks[at].task.clone();
        let err_type = response.err_type;
        let status = response.status;
        let body_len = response.body.len();
        let profile = response.profile;

        self.cache_or_close_socket_at(now, &task, err_type, status, &profile);

        if err_type != ErrCmdType::Ok {
            if err_type == ErrCmdType::Socket && status == ECT_SOCKET_MAKE_SOCKET_PREPARED {
                let network = self.network();
                self.dynamic_timeout
                    .record_at(network, DYN_TIME_TASK_FAILED_PKG_LEN, 0, now);
                self.tasks[at].set_last_failed_status();
            }
            if err_type == ErrCmdType::Socket {
                self.tasks[at].force_no_retry = response.cancel_retry;
            }
            if status == HANDSHAKE_MISUNDERSTAND {
                // the two ends did not agree on the handshake: a try that is
                // not the task's to pay for
                self.tasks[at].remain_retry_count += 1;
            }
            let ended = self.single_resp_handle_at(
                now,
                at,
                err_type,
                status,
                TaskFailHandleType::Default,
                profile,
            );
            return Some(handle_of(ended));
        }

        let profile_of = &mut self.tasks[at].transfer_profile;
        profile_of.received_size = body_len;
        profile_of.receive_data_size = body_len;
        profile_of.last_receive_pkg_time = now;

        let (err_code, handle) = self.decode(&task, &response.body);
        self.socket_pool.report_at(
            now,
            profile.is_reused_fd,
            true,
            handle == TaskFailHandleType::Normal,
        );
        if self.should_intercept(err_code) {
            self.intercept
                .add_intercept_task_at(now, &task.cgi, response.body.clone());
        }

        match handle {
            TaskFailHandleType::Normal => {
                let network = self.network();
                let cost = now.saturating_sub(self.tasks[at].transfer_profile.start_send_time);
                let total = self.tasks[at].transfer_profile.send_data_size + body_len;
                self.dynamic_timeout
                    .record_at(network, total as u32, cost, now);
                let ended = self.single_resp_handle_at(
                    now,
                    at,
                    ErrCmdType::Ok,
                    err_code,
                    handle,
                    profile.clone(),
                );
                self.notify_network_err(ErrCmdType::Ok, err_code, &profile);
                Some(handle_of(ended))
            }
            TaskFailHandleType::SessionTimeout | TaskFailHandleType::RetryAllTasks => {
                self.notify_retry_all_tasks(
                    ErrCmdType::EnDecode,
                    err_code,
                    handle,
                    task.taskid,
                    &task.user_id,
                );
                Some(RespHandle::Deferred)
            }
            TaskFailHandleType::TaskEnd => {
                let ended = self.single_resp_handle_at(
                    now,
                    at,
                    ErrCmdType::EnDecode,
                    err_code,
                    handle,
                    profile,
                );
                Some(handle_of(ended))
            }
            // `kTaskFailHandleDefault` and anything the app made up: the
            // C++'s `default:`
            TaskFailHandleType::Default
            | TaskFailHandleType::SlientTaskEnd
            | TaskFailHandleType::TaskTimeout => {
                let ended = self.single_resp_handle_at(
                    now,
                    at,
                    ErrCmdType::EnDecode,
                    err_code,
                    handle,
                    profile.clone(),
                );
                self.notify_network_err(ErrCmdType::EnDecode, handle as i32, &profile);
                Some(handle_of(ended))
            }
        }
    }

    /// `GetTasksContinuousFailCount()` — how many tasks in a row failed
    /// without one that did not.
    pub fn tasks_continuous_fail_count(&self) -> u32 {
        self.tasks_continuous_fail_count
    }

    /// `default_use_proxy_` — whether a try goes through a proxy. A task that
    /// came back is what changes it.
    pub fn default_use_proxy(&self) -> bool {
        self.default_use_proxy
    }

    /// `GetConnectProfile(_taskid)` — the connect of the run that is out.
    /// [`None`] for a task that is waiting for one.
    pub fn connect_profile(&self, taskid: u32) -> Option<&ConnectProfile> {
        self.tasks
            .iter()
            .find(|p| p.running.is_some() && p.task.taskid == taskid)
            .map(|p| &p.transfer_profile.connect_profile)
    }

    /// `lst_cmd_`.
    pub fn tasks(&self) -> &[TaskProfile] {
        &self.tasks
    }

    /// How many tasks are in the queue.
    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Whether there is nothing in the queue.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// When the host has to call [`ShortLinkTaskManager::run_loop_at`] again:
    /// the earliest of the deadlines the tasks are waiting on — a timeout that
    /// is about to run out, or a retry that is about to be due. [`None`] when
    /// there is nothing to wait for, which is when the C++ stops its loop.
    pub fn due_time(&mut self) -> Option<u64> {
        let network = self.network();
        let timeouts = self
            .tasks
            .iter()
            .filter_map(|profile| next_deadline(profile, network))
            .map(|(_, at)| at);
        let retries = self
            .tasks
            .iter()
            .filter(|profile| profile.running.is_none() && profile.retry_time_interval > 0)
            .map(|profile| {
                profile
                    .retry_start_time
                    .saturating_add(profile.retry_time_interval)
            });
        timeouts.chain(retries).min()
    }

    /// `SetDebugHost(_host)` — the host a run is pointed at instead of the ones
    /// the task came with.
    pub fn set_debug_host(&mut self, host: impl Into<String>) {
        self.debug_host = host.into();
    }

    /// `debug_host_`.
    pub fn debug_host(&self) -> &str {
        &self.debug_host
    }

    /// `socket_pool_`.
    pub fn socket_pool(&mut self) -> &mut SocketPool {
        &mut self.socket_pool
    }

    /// `__OnGetCacheSocket` — the socket the pool still has for a pair, which
    /// is what a host wires [`crate::ShortLink`]'s `set_cache_socket` to: the
    /// C++ gives the same getter to every worker it makes, and a run that finds
    /// one connects with it instead of making a new one.
    pub fn cache_socket(&mut self, item: &IpPortItem) -> Option<SocketFd> {
        self.socket_pool.get_socket(item)
    }

    /// The same, with the reading handed in.
    pub fn cache_socket_at(&mut self, now: u64, item: &IpPortItem) -> Option<SocketFd> {
        self.socket_pool.get_socket_at(now, item)
    }

    /// `task_intercept_`.
    pub fn intercept(&mut self) -> &mut TaskIntercept {
        &mut self.intercept
    }

    /// `dynamic_timeout_`.
    pub fn dynamic_timeout(&mut self) -> &mut DynamicTimeout {
        &mut self.dynamic_timeout
    }

    /// `ShortLinkChannelFactory::Create` — the host starts a run. An unset one
    /// names runs nobody answers, which is what makes a task time out.
    pub fn set_start_run(
        &mut self,
        start: impl FnMut(&Task, &RunRequest) -> Option<RunId> + Send + 'static,
    ) {
        self.start = Some(Box::new(start));
    }

    /// `ShortLinkChannelFactory::Destory`.
    pub fn set_destroy_run(&mut self, destroy: impl FnMut(RunId) + Send + 'static) {
        self.destroy = Some(Box::new(destroy));
    }

    /// `fun_callback_`.
    pub fn set_callback(
        &mut self,
        callback: impl FnMut(ErrCmdType, i32, TaskFailHandleType, &Task, u32, &ConnectProfile) -> i32
            + Send
            + 'static,
    ) {
        self.callback = Some(Box::new(callback));
    }

    /// `fun_notify_network_err_`.
    pub fn set_notify_network_err(
        &mut self,
        notify: impl FnMut(ErrCmdType, i32, &str, &str, u16) + Send + 'static,
    ) {
        self.notify_network_err = Some(Box::new(notify));
    }

    /// `fun_notify_retry_all_tasks`.
    pub fn set_notify_retry_all_tasks(
        &mut self,
        notify: impl FnMut(ErrCmdType, i32, TaskFailHandleType, u32, &str) + Send + 'static,
    ) {
        self.notify_retry_all_tasks = Some(Box::new(notify));
    }

    /// `fun_shortlink_response_`.
    pub fn set_response_status(&mut self, status: impl FnMut(i32) + Send + 'static) {
        self.response_status = Some(Box::new(status));
    }

    /// `Req2Buf`.
    pub fn set_req2buf(
        &mut self,
        req2buf: impl FnMut(&Task) -> Result<Vec<u8>, i32> + Send + 'static,
    ) {
        self.req2buf = Some(Box::new(req2buf));
    }

    /// `Buf2Resp`.
    pub fn set_buf2resp(
        &mut self,
        buf2resp: impl FnMut(&Task, &[u8]) -> (i32, TaskFailHandleType) + Send + 'static,
    ) {
        self.buf2resp = Some(Box::new(buf2resp));
    }

    /// `MakesureAuthed` — unset is a task that is authed, which is what lets
    /// every task go out.
    pub fn set_make_sure_authed(
        &mut self,
        make_sure_authed: impl FnMut(&str, &str) -> bool + Send + 'static,
    ) {
        self.make_sure_authed = Some(Box::new(make_sure_authed));
    }

    /// `fun_anti_avalanche_check_`.
    pub fn set_anti_avalanche_check(
        &mut self,
        check: impl FnMut(&Task, &[u8]) -> bool + Send + 'static,
    ) {
        self.anti_avalanche = Some(Box::new(check));
    }

    /// `should_intercept_result_` — unset is an answer that is not kept.
    pub fn set_should_intercept(&mut self, should: impl FnMut(i32) -> bool + Send + 'static) {
        self.should_intercept = Some(Box::new(should));
    }

    /// `getNetInfo()` — unset is Wi-Fi, the tighter of the two.
    pub fn set_net_info(&mut self, net_info: impl FnMut() -> NetworkKind + Send + 'static) {
        self.net_info = Some(Box::new(net_info));
    }

    /// `StnManager::GenSequenceId()` — unset is a task that is reported under
    /// sequence id `0`, which is what the C++ warns about: a retry of it would
    /// be taken for the request it is a retry of.
    pub fn set_gen_sequence_id(&mut self, gen: impl FnMut() -> u16 + Send + 'static) {
        self.gen_sequence_id = Some(Box::new(gen));
    }

    /// `closefunc` — what a socket the queue is done with is closed with.
    pub fn set_close_socket(&mut self, close: impl FnMut(SocketFd) + Send + 'static) {
        self.close = Some(Box::new(close));
    }

    /// `__RunOnTimeout` — the tasks that answered nothing.
    fn run_on_timeout_at(&mut self, now: u64) {
        self.socket_pool.clean_timeout_at(now);
        let network = self.network();

        // The C++ walks its list once, and so does this: a task that is tried
        // again has its send readings cleared, which is what would make it look
        // like it timed out once more.
        let timed_out: Vec<(u32, Timeout)> = self
            .tasks
            .iter()
            .filter_map(|profile| {
                timed_out(profile, now, network).map(|timeout| (profile.task.taskid, timeout))
            })
            .collect();

        for (taskid, timeout) in timed_out {
            let Some(at) = self.tasks.iter().position(|p| p.task.taskid == taskid) else {
                continue;
            };
            let profile = self.tasks[at].transfer_profile.connect_profile.clone();
            self.dynamic_timeout
                .record_at(network, DYN_TIME_TASK_FAILED_PKG_LEN, 0, now);
            self.tasks[at].set_last_failed_status();
            self.single_resp_handle_at(
                now,
                at,
                timeout.err_type(),
                timeout.err_code(),
                timeout.fail_handle(),
                profile.clone(),
            );
            self.notify_network_err(timeout.err_type(), timeout.err_code(), &profile);
        }
    }

    /// `__RunOnStartTask` — the tasks that may go out now.
    fn run_on_start_task_at(&mut self, now: u64) {
        let mobile = self.network() == NetworkKind::Mobile;
        let mut sent_count: i32 = 0;
        let mut i = 0;

        while i < self.tasks.len() {
            if self.tasks[i].running.is_some() {
                sent_count += 1;
                i += 1;
                continue;
            }

            if !may_start(&self.tasks[i], now) {
                i += 1;
                continue;
            }

            // proxy: the last try of a task that may be tried again goes the
            // other way, which is how a network that will not let one through
            // is found
            let use_proxy = {
                let profile = &self.tasks[i];
                if profile.remain_retry_count == 0 && profile.task.retry_count > 0 {
                    !self.default_use_proxy
                } else {
                    self.default_use_proxy
                }
            };
            self.tasks[i].use_proxy = use_proxy;

            // a retry goes out on the fallback hosts, and on tcp
            if !self.tasks[i].history.is_empty() {
                self.tasks[i].task.transport_protocol = Task::TRANSPORT_PROTOCOL_TCP;
                let fallback = self.tasks[i].task.shortlink_fallback_hostlist.clone();
                self.tasks[i].task.shortlink_host_list = fallback;
            }

            let hosts = self.tasks[i].task.shortlink_host_list.clone();
            if hosts.is_empty() {
                // a task with nowhere to go; the C++ does not advance here
                i += 1;
                continue;
            }
            let host = hosts[0].clone();

            let mut task = self.tasks[i].task.clone();
            if task.need_authed && !self.authed(&host, &task.user_id) {
                i += 1;
                continue;
            }

            // `first->task.client_sequence_id = …GenSequenceId()` — one per
            // try, and before `Req2Buf`, which is what the app is handed the
            // request to write with: a retry goes out under a new one
            let sequence_id = self.sequence_id();
            self.tasks[i].task.client_sequence_id = sequence_id;
            // what the C++ makes the worker from is the task it just drew on,
            // not a copy one number behind it
            task.client_sequence_id = sequence_id;

            let body = match self.encode(&task) {
                Ok(body) => body,
                Err(code) => {
                    let ended = self.single_resp_handle_at(
                        now,
                        i,
                        ErrCmdType::EnDecode,
                        code,
                        TaskFailHandleType::TaskEnd,
                        ConnectProfile::new(),
                    );
                    if !ended {
                        i += 1;
                    }
                    continue;
                }
            };

            if !self.allowed(&task, &body) {
                let ended = self.single_resp_handle_at(
                    now,
                    i,
                    ErrCmdType::Local,
                    LOCAL_ANTI_AVALANCHE,
                    TaskFailHandleType::TaskEnd,
                    ConnectProfile::new(),
                );
                if !ended {
                    i += 1;
                }
                continue;
            }

            // a cgi that was answered already: the C++ asks here too, and
            // what it asks answers `false` whatever it has, so nothing comes
            // of it and the task goes out like any other
            if let Some(answer) = self.intercept.intercept_task_info_at(now, &task.cgi) {
                let len = answer.len();
                let (err_code, handle) = self.decode(&task, &answer);
                {
                    let profile = &mut self.tasks[i].transfer_profile;
                    profile.received_size = len;
                    profile.receive_data_size = len;
                    profile.last_receive_pkg_time = now;
                }
                let ended = self.single_resp_handle_at(
                    now,
                    i,
                    ErrCmdType::EnDecode,
                    err_code,
                    handle,
                    ConnectProfile::new(),
                );
                if !ended {
                    i += 1;
                }
                continue;
            }

            let status = self.dynamic_timeout.status();
            let first_pkg = first_pkg_timeout(
                i64::from(task.server_process_cost),
                body.len(),
                sent_count,
                status,
                mobile,
            );
            let read_write = if task.long_polling {
                read_write_timeout(task.long_polling_timeout.max(0) as u64, mobile)
            } else {
                read_write_timeout(first_pkg, mobile)
            };
            {
                let profile = &mut self.tasks[i];
                profile.transfer_profile.loop_start_task_time = now;
                profile.transfer_profile.first_pkg_timeout = first_pkg;
                profile.transfer_profile.read_write_timeout = read_write;
                profile.transfer_profile.send_data_size = body.len();
                profile.current_dyntime_status = if task.server_process_cost <= 0 {
                    status
                } else {
                    // a task that said how long the server needs is not one the
                    // network's own status may shorten
                    DynamicTimeoutStatus::Evaluating
                };
            }

            let request = RunRequest {
                body: body.clone(),
                use_proxy,
                debug_host: self.debug_host.clone(),
                first_pkg_timeout: first_pkg,
                read_write_timeout: read_write,
                sent_count,
            };
            // `if (!first->running_id) { first = next; continue; }` — a run
            // that never began is not one the queue counts as out, and the
            // tasks behind it are not given the timeouts of a busier queue
            let Some(run) = self.start_run(&task, &request) else {
                i += 1;
                continue;
            };
            self.tasks[i].running = Some(run);
            sent_count += 1;
            i += 1;
        }
    }

    /// `__SingleRespHandle` — one answer, for one task: whether the task is
    /// over. `false` is a task that has another try coming.
    fn single_resp_handle_at(
        &mut self,
        now: u64,
        at: usize,
        err_type: ErrCmdType,
        err_code: i32,
        fail_handle: TaskFailHandleType,
        profile: ConnectProfile,
    ) -> bool {
        if err_type == ErrCmdType::Ok {
            self.tasks_continuous_fail_count = 0;
            self.default_use_proxy = self.tasks[at].use_proxy;
        } else {
            self.tasks_continuous_fail_count = self.tasks_continuous_fail_count.saturating_add(1);
        }

        let cost = now.saturating_sub(self.tasks[at].start_task_time);
        self.tasks[at].transfer_profile.connect_profile = profile.clone();

        let over = self.tasks[at].force_no_retry
            || self.tasks[at].remain_retry_count <= 0
            || err_type == ErrCmdType::Ok
            || fail_handle == TaskFailHandleType::TaskEnd
            || fail_handle == TaskFailHandleType::TaskTimeout;

        if over {
            let task = self.tasks[at].task.clone();
            let was_running = self.tasks[at].running.is_some();
            let cgi_retcode = self.callback.as_mut().map_or(0, |callback| {
                callback(
                    err_type,
                    err_code,
                    fail_handle,
                    &task,
                    cost as u32,
                    &profile,
                )
            });
            {
                let profile = &mut self.tasks[at];
                profile.end_task_time = now;
                profile.err_type = err_type;
                // what the app says about an answer it was given is the code
                // the task is remembered with
                profile.err_code = if was_running && err_type == ErrCmdType::Ok {
                    cgi_retcode
                } else {
                    err_code
                };
                profile.transfer_profile.error_type = err_type;
                profile.transfer_profile.error_code = err_code;
                profile.push_history();
            }
            self.stop_run_at(at);
            self.tasks.remove(at);
            return true;
        }

        {
            let profile = &mut self.tasks[at];
            profile.remain_retry_count -= 1;
            profile.transfer_profile.error_type = err_type;
            profile.transfer_profile.error_code = err_code;
            profile.err_type = err_type;
            profile.err_code = err_code;
        }

        // a task that is not out has no run to try again on
        if !self.stop_run_at(at) {
            return false;
        }

        let profile = &mut self.tasks[at];
        profile.push_history();
        profile.init_send_param_at(now);
        profile.retry_start_time = if fail_handle == TaskFailHandleType::SessionTimeout {
            0
        } else {
            now
        };
        profile.retry_time_interval = RETRY_INTERNAL;
        false
    }

    /// `__BatchErrorRespHandle` — one answer for every task. `running_only` is
    /// the C++'s `_callback_runing_task_only`, which only the destructor says
    /// `false` to.
    fn batch_error_resp_handle_at(
        &mut self,
        now: u64,
        err_type: ErrCmdType,
        err_code: i32,
        fail_handle: TaskFailHandleType,
        src_taskid: u32,
        running_only: bool,
    ) {
        let mut i = 0;
        while i < self.tasks.len() {
            if running_only && self.tasks[i].running.is_none() {
                i += 1;
                continue;
            }

            // a session timeout is only for a task that was authed
            if fail_handle == TaskFailHandleType::SessionTimeout && !self.tasks[i].task.need_authed
            {
                i += 1;
                continue;
            }

            let taskid = self.tasks[i].task.taskid;
            if fail_handle == TaskFailHandleType::SessionTimeout
                && src_taskid != Task::INVALID_TASK_ID
                && taskid == src_taskid
                && self.tasks[i].allow_sessiontimeout_retry
            {
                // the task is not failed: it is put back, with one more try
                self.tasks[i].allow_sessiontimeout_retry = false;
                self.tasks[i].remain_retry_count += 1;
                self.stop_run_at(i);
                self.tasks[i].push_history();
                self.tasks[i].init_send_param_at(now);
                i += 1;
                continue;
            }

            let is_source = src_taskid == Task::INVALID_TASK_ID || src_taskid == taskid;
            let code = if is_source { err_code } else { 0 };
            let profile = self.tasks[i].transfer_profile.connect_profile.clone();
            let ended = self.single_resp_handle_at(now, i, err_type, code, fail_handle, profile);
            if !ended {
                i += 1;
            }
        }
    }

    /// `__LocateBySeq` — the task a run answers for.
    fn locate(&self, run_id: RunId) -> Option<usize> {
        self.tasks
            .iter()
            .position(|profile| profile.running == Some(run_id))
    }

    /// `__DeleteShortLink` — the run is over. `false` when there was none.
    fn stop_run_at(&mut self, at: usize) -> bool {
        let Some(run) = self.tasks[at].running.take() else {
            return false;
        };
        if let Some(destroy) = self.destroy.as_mut() {
            destroy(run);
        }
        true
    }

    fn start_run(&mut self, task: &Task, request: &RunRequest) -> Option<RunId> {
        match self.start.as_mut() {
            Some(start) => start(task, request),
            None => {
                self.next_run_id += 1;
                Some(RunId(self.next_run_id))
            }
        }
    }

    fn network(&mut self) -> NetworkKind {
        match self.net_info.as_mut() {
            Some(net_info) => net_info(),
            None => NetworkKind::Wifi,
        }
    }

    fn encode(&mut self, task: &Task) -> Result<Vec<u8>, i32> {
        match self.req2buf.as_mut() {
            Some(req2buf) => req2buf(task),
            None => Ok(Vec::new()),
        }
    }

    fn decode(&mut self, task: &Task, body: &[u8]) -> (i32, TaskFailHandleType) {
        match self.buf2resp.as_mut() {
            Some(buf2resp) => buf2resp(task, body),
            None => (0, TaskFailHandleType::Normal),
        }
    }

    fn allowed(&mut self, task: &Task, body: &[u8]) -> bool {
        match self.anti_avalanche.as_mut() {
            Some(check) => check(task, body),
            None => true,
        }
    }

    fn authed(&mut self, host: &str, user_id: &str) -> bool {
        match self.make_sure_authed.as_mut() {
            Some(make_sure_authed) => make_sure_authed(host, user_id),
            None => true,
        }
    }

    fn should_intercept(&mut self, err_code: i32) -> bool {
        match self.should_intercept.as_mut() {
            Some(should) => should(err_code),
            None => false,
        }
    }

    fn sequence_id(&mut self) -> u16 {
        match self.gen_sequence_id.as_mut() {
            Some(gen) => gen(),
            None => 0,
        }
    }

    fn notify_network_err(
        &mut self,
        err_type: ErrCmdType,
        err_code: i32,
        profile: &ConnectProfile,
    ) {
        if let Some(notify) = self.notify_network_err.as_mut() {
            notify(err_type, err_code, &profile.ip, &profile.host, profile.port);
        }
    }

    fn notify_retry_all_tasks(
        &mut self,
        err_type: ErrCmdType,
        err_code: i32,
        fail_handle: TaskFailHandleType,
        src_taskid: u32,
        user_id: &str,
    ) {
        if let Some(notify) = self.notify_retry_all_tasks.as_mut() {
            notify(err_type, err_code, fail_handle, src_taskid, user_id);
        }
    }

    /// What `__OnResponse` does with the socket of a run that is over: one the
    /// server said to keep goes into the pool, one that answered badly is
    /// closed.
    fn cache_or_close_socket_at(
        &mut self,
        now: u64,
        task: &Task,
        err_type: ErrCmdType,
        status: i32,
        profile: &ConnectProfile,
    ) {
        if !is_keep_alive(task) || !profile.socket_fd.is_valid() {
            return;
        }
        let socket = profile.socket_fd;

        if err_type != ErrCmdType::Ok {
            if let Some(close) = self.close.as_mut() {
                close(socket);
            }
            // a socket that was closed by the server is nobody's fault
            if status != ECT_SOCKET_SHUTDOWN {
                self.socket_pool
                    .report_at(now, profile.is_reused_fd, false, false);
            }
            return;
        }

        let Some(item) = profile.ip_items.get(profile.ip_index as usize) else {
            return;
        };
        self.socket_pool.add_cache(CachedSocket::new_at(
            now,
            item.clone(),
            socket,
            profile.keepalive_timeout,
        ));
    }
}

impl Default for ShortLinkTaskManager {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ShortLinkTaskManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShortLinkTaskManager")
            .field("tasks", &self.tasks.len())
            .field("default_use_proxy", &self.default_use_proxy)
            .field(
                "tasks_continuous_fail_count",
                &self.tasks_continuous_fail_count,
            )
            .field("debug_host", &self.debug_host)
            .field("socket_pool", &self.socket_pool.len())
            .field("intercept", &self.intercept.len())
            .finish_non_exhaustive()
    }
}

impl Drop for ShortLinkTaskManager {
    /// `~ShortLinkTaskManager()` — every task the queue still has is failed
    /// with `kEctLocal` / `kEctLocalReset` / `kTaskFailHandleTaskEnd`, running
    /// or not, which is what the C++ does with `_callback_runing_task_only`
    /// said `false` to.
    fn drop(&mut self) {
        self.batch_error_resp_handle_at(
            gettickcount(),
            ErrCmdType::Local,
            LOCAL_RESET,
            TaskFailHandleType::TaskEnd,
            Task::INVALID_TASK_ID,
            false,
        );
    }
}

/// `true` is a task that left the queue.
fn handle_of(ended: bool) -> RespHandle {
    if ended {
        RespHandle::Ended
    } else {
        RespHandle::Retried
    }
}

/// Whether a task may go out now: the `retry_time_interval` of the C++, which
/// is a wait that starts when the retry was decided on.
fn may_start(profile: &TaskProfile, now: u64) -> bool {
    profile.retry_time_interval <= now.saturating_sub(profile.retry_start_time)
}

/// The five timeouts of `__RunOnTimeout` that apply to a task, and when each of
/// them runs out — in the order the C++ asks about them, which is the order it
/// takes its one answer from.
fn deadlines(profile: &TaskProfile, network: NetworkKind) -> [Option<(Timeout, u64)>; 5] {
    let running = profile.running.is_some();
    let sent = profile.transfer_profile.start_send_time;
    let pkg = profile.transfer_profile.last_receive_pkg_time;
    let long_polling = profile.task.long_polling;
    let pkg_pkg_interval = match network {
        NetworkKind::Mobile => MOBILE_PACKAGE_INTERVAL,
        NetworkKind::Wifi => WIFI_PACKAGE_INTERVAL,
    };

    [
        Some((
            Timeout::Task,
            profile.start_task_time.saturating_add(profile.task_timeout),
        )),
        (running && sent > 0).then_some((
            Timeout::ReadWrite,
            sent.saturating_add(profile.transfer_profile.read_write_timeout),
        )),
        (running && long_polling && sent > 0 && pkg == 0).then_some((
            Timeout::LongPolling,
            sent.saturating_add(profile.task.long_polling_timeout.max(0) as u64),
        )),
        (running && !long_polling && sent > 0 && pkg == 0).then_some((
            Timeout::FirstPkg,
            sent.saturating_add(profile.transfer_profile.first_pkg_timeout),
        )),
        (running && sent > 0 && pkg > 0)
            .then_some((Timeout::PkgPkg, pkg.saturating_add(pkg_pkg_interval))),
    ]
}

/// The timeout a task is waiting on, and when it runs out: the soonest of the
/// five, which is when the host has to look again.
fn next_deadline(profile: &TaskProfile, network: NetworkKind) -> Option<(Timeout, u64)> {
    deadlines(profile, network)
        .into_iter()
        .flatten()
        .min_by_key(|(_, at)| *at)
}

/// Which of the five a task has run into at `now`, if any: the C++'s
/// `if ... else if ...` takes the first one it asks about that has, not the
/// one that ran out first — a loop that runs late reports the task's own
/// timeout before the read's, and the read's before the first package's.
fn timed_out(profile: &TaskProfile, now: u64, network: NetworkKind) -> Option<Timeout> {
    deadlines(profile, network)
        .into_iter()
        .flatten()
        .find(|(_, at)| *at <= now)
        .map(|(timeout, _)| timeout)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::long_link::ECT_SOCKET_SHUTDOWN;
    use crate::simple_ipport_sort::IpPortItem;
    use crate::socket_operator::SocketFd;
    use crate::task_profile::LOCAL_TASK_TIMEOUT;

    use super::*;

    /// A task that may go out: it needs no auth, it has somewhere to go, and it
    /// may be tried twice.
    fn task(taskid: u32) -> Task {
        let mut task = Task::new(taskid, 1);
        task.cgi = format!("/cgi-bin/{taskid}");
        task.shortlink_host_list = vec!["short.weixin.qq.com".to_string()];
        task.shortlink_fallback_hostlist = vec!["fallback.weixin.qq.com".to_string()];
        task.need_authed = false;
        task.retry_count = 1;
        task
    }

    /// A task that asked for the socket to be kept.
    fn keep_alive_task(taskid: u32) -> Task {
        let mut task = task(taskid);
        task.headers
            .insert("Connection".to_string(), "Keep-Alive".to_string());
        task
    }

    fn prepare() -> PrepareProfile {
        PrepareProfile::new_at(100_000)
    }

    /// An answer, with nothing in it and nowhere it came from.
    fn failed(err_type: ErrCmdType, status: i32) -> Response {
        Response {
            err_type,
            status,
            body: Vec::new(),
            cancel_retry: false,
            profile: ConnectProfile::new(),
        }
    }

    fn answered(body: &[u8]) -> Response {
        Response {
            err_type: ErrCmdType::Ok,
            status: 200,
            body: body.to_vec(),
            cancel_retry: false,
            profile: ConnectProfile::new(),
        }
    }

    /// What a run was asked for, by the task it is the run of.
    type Started = Arc<Mutex<Vec<(u32, bool)>>>;
    /// What the app was told about a task that is over.
    type Ended = Arc<Mutex<Vec<(ErrCmdType, i32, TaskFailHandleType, u32)>>>;
    /// What the app was asked about a task it may want tried again.
    type Asked = Arc<Mutex<Vec<(ErrCmdType, i32, TaskFailHandleType, u32, String)>>>;

    /// A run of every task, named after the task, and a note of what it was
    /// asked for.
    fn runs(manager: &mut ShortLinkTaskManager) -> Started {
        let started: Started = Arc::new(Mutex::new(Vec::new()));
        let recorder = started.clone();
        manager.set_start_run(move |task, request| {
            recorder
                .lock()
                .unwrap()
                .push((task.taskid, request.use_proxy));
            Some(RunId(u64::from(task.taskid)))
        });
        started
    }

    /// What the app was told about the tasks that are over.
    fn endings(manager: &mut ShortLinkTaskManager) -> Ended {
        let ended: Ended = Arc::new(Mutex::new(Vec::new()));
        let recorder = ended.clone();
        manager.set_callback(
            move |err_type, err_code, fail_handle, task, _cost, _profile| {
                recorder
                    .lock()
                    .unwrap()
                    .push((err_type, err_code, fail_handle, task.taskid));
                0
            },
        );
        ended
    }

    #[test]
    fn each_of_the_five_timeouts_is_the_error_the_c_gives_it() {
        assert_eq!(Timeout::Task.err_type(), ErrCmdType::Local);
        assert_eq!(Timeout::Task.err_code(), LOCAL_TASK_TIMEOUT);
        assert_eq!(Timeout::Task.fail_handle(), TaskFailHandleType::TaskTimeout);

        // a read that ran out of time is the http one, and the app's call
        for timeout in [
            Timeout::ReadWrite,
            Timeout::LongPolling,
            Timeout::FirstPkg,
            Timeout::PkgPkg,
        ] {
            assert_eq!(timeout.err_type(), ErrCmdType::Http);
            assert_eq!(timeout.fail_handle(), TaskFailHandleType::Default);
        }
        assert_eq!(Timeout::ReadWrite.err_code(), HTTP_READ_WRITE_TIMEOUT);
        assert_eq!(
            Timeout::LongPolling.err_code(),
            HTTP_LONG_POLLING_TIMEOUT,
            "the same number as the long link's task timeout, and not one"
        );
        assert_eq!(Timeout::FirstPkg.err_code(), HTTP_FIRST_PKG_TIMEOUT);
        assert_eq!(Timeout::PkgPkg.err_code(), HTTP_PKG_PKG_TIMEOUT);
    }

    #[test]
    fn only_an_ended_task_left_the_queue() {
        assert!(RespHandle::Ended.is_ended());
        assert!(!RespHandle::Retried.is_ended());
        assert!(!RespHandle::Deferred.is_ended());
        assert_eq!(handle_of(true), RespHandle::Ended);
        assert_eq!(handle_of(false), RespHandle::Retried);
    }

    #[test]
    fn a_task_may_go_out_once_the_wait_after_its_retry_is_over() {
        let mut profile = TaskProfile::new_at(0, task(1), prepare());
        assert!(may_start(&profile, 0), "a task that never retried");

        profile.retry_time_interval = RETRY_INTERNAL;
        profile.retry_start_time = 1_000;
        assert!(!may_start(&profile, 1_500));
        assert!(may_start(&profile, 2_000));
    }

    #[test]
    fn a_task_is_waiting_on_the_soonest_of_its_timeouts() {
        let mut manager = ShortLinkTaskManager::new();
        let started = runs(&mut manager);
        manager.start_task_at(100_000, task(7), prepare());
        assert_eq!(*started.lock().unwrap(), vec![(7, true)]);

        let profile = &manager.tasks()[0];
        // nothing has been sent, so the only thing it is waiting on is itself
        assert_eq!(
            next_deadline(profile, NetworkKind::Wifi),
            Some((Timeout::Task, 140_000)),
            "(15s + 5s) * the two tries the task has in it"
        );
        assert_eq!(timed_out(profile, 139_999, NetworkKind::Wifi), None);
        assert_eq!(
            timed_out(profile, 140_000, NetworkKind::Wifi),
            Some(Timeout::Task)
        );

        assert!(manager.on_send_at(100_000, RunId(7)));
        let profile = &manager.tasks()[0];
        // the first package is what it is waiting on now, and it is due before
        // the read as a whole is
        assert_eq!(
            next_deadline(profile, NetworkKind::Wifi),
            Some((Timeout::FirstPkg, 112_000))
        );

        // a package came in, so what it is waiting on is the next one
        manager.on_recv_at(100_500, RunId(7), 10, 20);
        let profile = &manager.tasks()[0];
        assert_eq!(
            next_deadline(profile, NetworkKind::Wifi),
            Some((Timeout::PkgPkg, 108_500)),
            "8s of wifi between two packages"
        );
        // by 110_000 the package-to-package one is out and nothing else is
        assert_eq!(
            timed_out(profile, 110_000, NetworkKind::Wifi),
            Some(Timeout::PkgPkg)
        );
        // by 117_333 the read as a whole is out too, and the C++ asks about it
        // first: it is the read that is reported, not the earlier one
        assert_eq!(
            timed_out(profile, 117_333, NetworkKind::Wifi),
            Some(Timeout::ReadWrite)
        );
    }

    #[test]
    fn a_long_polling_task_is_waiting_on_its_own_timeout() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        let mut task = task(7);
        task.long_polling = true;
        task.long_polling_timeout = 300_000;
        manager.start_task_at(100_000, task, prepare());
        assert!(manager.on_send_at(100_000, RunId(7)));

        let profile = &manager.tasks()[0];
        assert_eq!(
            next_deadline(profile, NetworkKind::Wifi),
            Some((Timeout::LongPolling, 400_000)),
            "300s of long polling, and the read of one that big on top"
        );
        assert_eq!(
            timed_out(profile, 400_000, NetworkKind::Wifi),
            Some(Timeout::LongPolling)
        );
    }

    #[test]
    fn a_read_of_a_task_that_answered_nothing_times_out_before_the_task_does() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        let ended = endings(&mut manager);
        manager.start_task_at(100_000, task(7), prepare());
        assert!(manager.on_send_at(100_000, RunId(7)));

        manager.run_loop_at(118_000);
        assert_eq!(manager.len(), 1, "the task has another try coming");
        assert_eq!(
            manager.tasks()[0].remain_retry_count,
            0,
            "one of the two tries is spent"
        );
        assert_eq!(manager.tasks()[0].retry_time_interval, RETRY_INTERNAL);
        assert!(ended.lock().unwrap().is_empty(), "nothing is over yet");
        assert_eq!(manager.tasks_continuous_fail_count(), 1);
    }

    #[test]
    fn a_loop_that_runs_late_reports_the_timeout_the_cpp_asks_about_first() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        let ended = endings(&mut manager);
        let mut task = task(7);
        task.retry_count = 0;
        manager.start_task_at(100_000, task, prepare());
        assert!(manager.on_send_at(100_000, RunId(7)));

        // by 118_000 both the first package (112_000) and the read as a whole
        // (117_333) are out: the C++'s `if ... else if` never gets as far as
        // the first package, so the read is what the app is told ran out
        manager.run_loop_at(118_000);
        assert_eq!(
            *ended.lock().unwrap(),
            vec![(
                ErrCmdType::Http,
                HTTP_READ_WRITE_TIMEOUT,
                TaskFailHandleType::Default,
                7
            )]
        );
    }

    #[test]
    fn the_try_after_one_that_failed_goes_the_other_way_about_the_proxy() {
        let mut manager = ShortLinkTaskManager::new();
        let started = runs(&mut manager);
        manager.start_task_at(100_000, task(7), prepare());
        assert!(manager.on_send_at(100_000, RunId(7)));
        assert!(manager.default_use_proxy());

        manager.run_loop_at(118_000);
        assert_eq!(
            manager.on_response_at(118_000, RunId(7), failed(ErrCmdType::Socket, -1)),
            None,
            "the run that timed out is not the one that answers any more"
        );

        // the retry is not due yet, and then it is
        manager.run_loop_at(118_500);
        assert_eq!(*started.lock().unwrap(), vec![(7, true)]);
        manager.run_loop_at(119_000);
        assert_eq!(
            *started.lock().unwrap(),
            vec![(7, true), (7, false)],
            "the last try of a task that may be tried again goes without one"
        );
        assert!(
            manager.connect_profile(7).is_some(),
            "the second try is out"
        );
    }

    /// `first->task.client_sequence_id = …GenSequenceId()` — the C++ draws one
    /// for every try, in `Req2Buf`'s own block, so that a retry is not taken
    /// for the request it is a retry of.
    #[test]
    fn every_try_of_a_task_is_written_under_a_sequence_id_of_its_own() {
        let mut manager = ShortLinkTaskManager::new();
        let drawn: Arc<Mutex<u16>> = Arc::new(Mutex::new(0));
        let counter = Arc::clone(&drawn);
        manager.set_gen_sequence_id(move || {
            let mut drawn = counter.lock().unwrap();
            *drawn += 1;
            *drawn
        });
        let written: Arc<Mutex<Vec<u16>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&written);
        manager.set_req2buf(move |task| {
            recorder.lock().unwrap().push(task.client_sequence_id);
            Ok(b"body".to_vec())
        });
        manager.set_start_run(move |task, _request| Some(RunId(u64::from(task.taskid))));

        manager.start_task_at(100_000, task(7), prepare());
        assert_eq!(*written.lock().unwrap(), vec![1]);
        assert_eq!(manager.tasks()[0].task.client_sequence_id, 1);

        // the answer is a failure the task is tried again for, and the retry is
        // a request of its own and not a copy of the one that failed
        assert!(manager.on_send_at(100_000, RunId(7)));
        assert_eq!(
            manager.on_response_at(
                100_500,
                RunId(7),
                failed(ErrCmdType::Socket, ECT_SOCKET_SHUTDOWN)
            ),
            Some(RespHandle::Retried)
        );
        manager.run_loop_at(102_000);
        assert_eq!(*written.lock().unwrap(), vec![1, 2]);
        assert_eq!(manager.tasks()[0].task.client_sequence_id, 2);
    }

    /// `++sent_count` — a run the host did not start is not one that is out:
    /// the C++ leaves the task where it is *before* the count, so the tasks
    /// behind it are not given the timeouts of a queue with one more run on it.
    #[test]
    fn a_run_that_never_began_is_not_one_the_queue_counts() {
        let mut manager = ShortLinkTaskManager::new();
        let counts: Arc<Mutex<Vec<(u32, i32)>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&counts);
        manager.set_start_run(move |task, request| {
            recorder
                .lock()
                .unwrap()
                .push((task.taskid, request.sent_count));
            // the first task's run never begins
            if task.taskid == 7 {
                None
            } else {
                Some(RunId(u64::from(task.taskid)))
            }
        });

        manager.start_task_at(100_000, task(7), prepare());
        manager.start_task_at(100_000, task(8), prepare());

        // the second pass asks for the first task's run again, and the second
        // task's is the first run that is out — so it is given the timeouts of
        // an empty queue and not of one that has already sent one
        assert_eq!(*counts.lock().unwrap(), vec![(7, 0), (7, 0), (8, 0)]);
        assert!(
            manager
                .tasks()
                .iter()
                .any(|profile| profile.task.taskid == 7),
            "a task whose run never began stays where it is"
        );
    }

    #[test]
    fn a_task_that_runs_out_of_time_is_over_and_the_app_is_told() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        let ended = endings(&mut manager);
        manager.start_task_at(100_000, task(7), prepare());

        manager.run_loop_at(140_000);
        assert_eq!(
            *ended.lock().unwrap(),
            vec![(
                ErrCmdType::Local,
                LOCAL_TASK_TIMEOUT,
                TaskFailHandleType::TaskTimeout,
                7
            )]
        );
        assert!(manager.is_empty());
        assert_eq!(manager.tasks_continuous_fail_count(), 1);
    }

    #[test]
    fn a_task_that_answered_is_over_and_what_the_app_said_is_remembered() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        let ended = endings(&mut manager);
        manager.set_buf2resp(|_task, body| {
            assert_eq!(body, b"hello");
            (-11, TaskFailHandleType::Normal)
        });
        manager.start_task_at(100_000, task(7), prepare());

        assert_eq!(
            manager.on_response_at(100_500, RunId(7), answered(b"hello")),
            Some(RespHandle::Ended),
            "an answer ends the task even with a server code in it"
        );
        assert!(manager.is_empty());
        assert_eq!(
            *ended.lock().unwrap(),
            vec![(ErrCmdType::Ok, -11, TaskFailHandleType::Normal, 7)]
        );
        assert_eq!(manager.tasks_continuous_fail_count(), 0);
    }

    #[test]
    fn an_answer_the_app_could_not_read_asks_about_every_task() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        let ended = endings(&mut manager);
        let asked: Asked = Arc::new(Mutex::new(Vec::new()));
        let recorder = asked.clone();
        manager.set_notify_retry_all_tasks(move |err_type, err_code, handle, taskid, user_id| {
            recorder.lock().unwrap().push((
                err_type,
                err_code,
                handle,
                taskid,
                user_id.to_string(),
            ));
        });
        manager.set_buf2resp(|_task, _body| (0, TaskFailHandleType::SessionTimeout));
        let mut authed = task(7);
        authed.need_authed = true;
        manager.start_task_at(100_000, authed, prepare());

        assert_eq!(
            manager.on_response_at(100_500, RunId(7), answered(b"hello")),
            Some(RespHandle::Deferred)
        );
        assert_eq!(manager.len(), 1, "the task is still in the queue");
        assert_eq!(
            *asked.lock().unwrap(),
            vec![(
                ErrCmdType::EnDecode,
                0,
                TaskFailHandleType::SessionTimeout,
                7,
                String::new()
            )]
        );

        // ... and when the app says so, it is tried again with one more try
        manager.retry_tasks_at(
            101_000,
            ErrCmdType::EnDecode,
            0,
            TaskFailHandleType::SessionTimeout,
            7,
        );
        assert_eq!(manager.len(), 1);
        assert!(!manager.tasks()[0].allow_sessiontimeout_retry, "one each");
        assert_eq!(manager.tasks()[0].remain_retry_count, 2);
        assert_eq!(
            manager.tasks()[0].retry_start_time,
            0,
            "a session timeout is retried at once"
        );
        assert!(ended.lock().unwrap().is_empty());
    }

    #[test]
    fn a_task_the_queue_is_asked_to_stop_is_over_and_its_run_is_dropped() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        let dropped: Arc<Mutex<Vec<RunId>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = dropped.clone();
        manager.set_destroy_run(move |run| recorder.lock().unwrap().push(run));
        manager.start_task_at(100_000, task(7), prepare());
        manager.start_task_at(100_000, task(8), prepare());

        assert!(manager.has_task(7));
        assert!(manager.stop_task(7));
        assert!(!manager.has_task(7));
        assert!(!manager.stop_task(7));
        assert_eq!(*dropped.lock().unwrap(), vec![RunId(7)]);

        manager.clear_tasks();
        assert!(manager.is_empty());
        assert_eq!(*dropped.lock().unwrap(), vec![RunId(7), RunId(8)]);
    }

    #[test]
    fn the_most_urgent_task_comes_first_and_a_send_only_one_does_not_come_at_all() {
        let mut manager = ShortLinkTaskManager::new();
        let started = runs(&mut manager);
        let mut urgent = task(7);
        urgent.priority = Task::TASK_PRIORITY_HIGHEST;
        let mut slow = task(8);
        slow.priority = Task::TASK_PRIORITY_LOWEST;
        let mut send_only = task(9);
        send_only.send_only = true;

        assert!(!manager.start_task_at(100_000, send_only, prepare()));
        assert!(manager.start_task_at(100_000, slow, prepare()));
        assert!(manager.start_task_at(100_000, urgent, prepare()));

        assert_eq!(
            manager
                .tasks()
                .iter()
                .map(|p| p.task.taskid)
                .collect::<Vec<_>>(),
            vec![7, 8],
            "a short link is for a task that wants an answer"
        );
        assert_eq!(
            *started.lock().unwrap(),
            vec![(8, true), (7, true)],
            "the slow one was asked for first, and so it went out first"
        );
    }

    #[test]
    fn a_task_with_nowhere_to_go_stays_in_the_queue() {
        let mut manager = ShortLinkTaskManager::new();
        let started = runs(&mut manager);
        let mut task = task(7);
        task.shortlink_host_list.clear();
        manager.start_task_at(100_000, task, prepare());

        assert_eq!(manager.len(), 1);
        assert!(started.lock().unwrap().is_empty());
        assert!(!manager.tasks()[0].is_running());
    }

    #[test]
    fn a_task_that_is_not_authed_waits() {
        let mut manager = ShortLinkTaskManager::new();
        let started = runs(&mut manager);
        let mut task = task(7);
        task.need_authed = true;
        manager.set_make_sure_authed(|host, user_id| {
            assert_eq!(host, "short.weixin.qq.com");
            assert!(user_id.is_empty());
            false
        });
        manager.start_task_at(100_000, task, prepare());

        assert!(started.lock().unwrap().is_empty());
        assert_eq!(manager.len(), 1);
    }

    #[test]
    fn a_cgi_that_was_answered_already_goes_out_again() {
        let mut manager = ShortLinkTaskManager::new();
        let started = runs(&mut manager);
        let ended = endings(&mut manager);
        manager
            .intercept()
            .add_intercept_task_at(100_000, "/cgi-bin/7", b"kept".to_vec());
        let mut answered = task(7);
        answered.retry_count = 0;
        manager.start_task_at(100_000, answered, prepare());

        assert_eq!(
            started.lock().unwrap().len(),
            1,
            "it went out like any other"
        );
        assert_eq!(manager.len(), 1, "and it is still out");
        assert!(ended.lock().unwrap().is_empty());
    }

    #[test]
    fn a_task_the_app_will_not_let_out_is_failed_with_the_avalanche_code() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        let ended = endings(&mut manager);
        manager.set_anti_avalanche_check(|_task, body| {
            assert!(body.is_empty());
            false
        });
        manager.start_task_at(100_000, task(7), prepare());

        assert_eq!(
            *ended.lock().unwrap(),
            vec![(
                ErrCmdType::Local,
                LOCAL_ANTI_AVALANCHE,
                TaskFailHandleType::TaskEnd,
                7
            )]
        );
        assert!(manager.is_empty());
    }

    #[test]
    fn a_request_the_app_could_not_write_fails_the_task() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        let ended = endings(&mut manager);
        manager.set_req2buf(|_task| Err(-300));
        manager.start_task_at(100_000, task(7), prepare());

        assert_eq!(
            *ended.lock().unwrap(),
            vec![(ErrCmdType::EnDecode, -300, TaskFailHandleType::TaskEnd, 7)]
        );
    }

    #[test]
    fn a_network_change_cancels_the_runs_that_are_out() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        let ended = endings(&mut manager);
        let dropped: Arc<Mutex<Vec<RunId>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = dropped.clone();
        manager.set_destroy_run(move |run| recorder.lock().unwrap().push(run));
        manager.start_task_at(100_000, task(7), prepare());
        assert!(manager.on_send_at(100_000, RunId(7)));

        manager.redo_tasks_at(100_500);
        assert_eq!(*dropped.lock().unwrap(), vec![RunId(7)], "the run is gone");
        assert!(
            ended.lock().unwrap().is_empty(),
            "a task with tries left is not failed by a network change"
        );
        assert_eq!(manager.len(), 1);
        assert!(!manager.tasks()[0].is_running());
        assert_eq!(manager.tasks()[0].remain_retry_count, 0);
        assert_eq!(
            manager.due_time(),
            Some(101_500),
            "the retry of the cancelled task"
        );
    }

    #[test]
    fn a_socket_the_server_said_to_keep_is_kept_and_one_that_answered_badly_is_closed() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        let closed: Arc<Mutex<Vec<SocketFd>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = closed.clone();
        manager.set_close_socket(move |socket| recorder.lock().unwrap().push(socket));
        let task = keep_alive_task(7);
        manager.start_task_at(100_000, task.clone(), prepare());

        let mut profile = ConnectProfile::new();
        profile.socket_fd = SocketFd(3);
        profile.keepalive_timeout = 5_000;
        profile.ip_index = 0;
        profile.ip_items = vec![IpPortItem::new("1.1.1.1", 80)];
        let kept = Response {
            err_type: ErrCmdType::Ok,
            status: 200,
            body: b"hello".to_vec(),
            cancel_retry: false,
            profile: profile.clone(),
        };
        assert_eq!(
            manager.on_response_at(100_500, RunId(7), kept),
            Some(RespHandle::Ended)
        );
        assert_eq!(manager.socket_pool().len(), 1, "the socket is kept");
        assert!(closed.lock().unwrap().is_empty());
        assert_eq!(
            manager.cache_socket_at(100_500, &IpPortItem::new("1.1.1.1", 80)),
            Some(SocketFd(3)),
            "and the next run of the same pair is connected with it"
        );

        // and one that answered badly is not
        manager.start_task_at(100_500, task, prepare());
        let mut broken = profile;
        broken.is_reused_fd = true;
        let failed = Response {
            err_type: ErrCmdType::Socket,
            status: -1,
            body: Vec::new(),
            cancel_retry: true,
            profile: broken,
        };
        manager.on_response_at(101_000, RunId(7), failed);
        assert_eq!(*closed.lock().unwrap(), vec![SocketFd(3)]);
        assert!(
            manager.socket_pool().is_banned_at(101_000),
            "a socket from the pool that did not answer bans it"
        );

        // unless the server was the one that closed it
        let shut = Response {
            err_type: ErrCmdType::Socket,
            status: ECT_SOCKET_SHUTDOWN,
            body: Vec::new(),
            cancel_retry: false,
            profile: ConnectProfile::new(),
        };
        manager.start_task_at(101_000, keep_alive_task(7), prepare());
        let mut profile = ConnectProfile::new();
        profile.socket_fd = SocketFd(4);
        manager.on_response_at(101_100, RunId(7), Response { profile, ..shut });
        assert_eq!(closed.lock().unwrap().len(), 2, "closed either way");
    }

    #[test]
    fn a_queue_that_is_dropped_fails_every_task_it_still_has() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        let ended = endings(&mut manager);
        manager.start_task_at(100_000, task(7), prepare());
        // a second one that never went out: no auth, no hosts
        let mut waiting = task(8);
        waiting.shortlink_host_list.clear();
        manager.start_task_at(100_000, waiting, prepare());
        assert_eq!(manager.len(), 2);

        drop(manager);
        assert_eq!(
            *ended.lock().unwrap(),
            vec![
                (
                    ErrCmdType::Local,
                    LOCAL_RESET,
                    TaskFailHandleType::TaskEnd,
                    7
                ),
                (
                    ErrCmdType::Local,
                    LOCAL_RESET,
                    TaskFailHandleType::TaskEnd,
                    8
                ),
            ],
            "~ShortLinkTaskManager() — every task, running or not"
        );
    }

    #[test]
    fn a_task_that_asked_for_no_retry_is_over_whatever_the_app_says() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        let ended = endings(&mut manager);
        manager.start_task_at(100_000, task(7), prepare());

        let mut response = failed(ErrCmdType::Socket, -1);
        response.cancel_retry = true;
        assert_eq!(
            manager.on_response_at(100_500, RunId(7), response),
            Some(RespHandle::Ended),
            "a run that says do not retry is the end of the task"
        );
        assert!(manager.is_empty());
        assert_eq!(
            *ended.lock().unwrap(),
            vec![(ErrCmdType::Socket, -1, TaskFailHandleType::Default, 7)]
        );
    }

    #[test]
    fn a_handshake_the_two_ends_did_not_agree_on_costs_the_task_nothing() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        manager.start_task_at(100_000, task(7), prepare());

        assert_eq!(
            manager.on_response_at(
                100_500,
                RunId(7),
                failed(ErrCmdType::Socket, HANDSHAKE_MISUNDERSTAND)
            ),
            Some(RespHandle::Retried)
        );
        assert_eq!(
            manager.tasks()[0].remain_retry_count,
            1,
            "the try it was given back is the one it spent"
        );
    }

    #[test]
    fn an_answer_the_app_keeps_is_not_one_the_queue_hands_out_again() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        endings(&mut manager);
        manager.set_should_intercept(|err_code| err_code == 0);
        manager.start_task_at(100_000, task(7), prepare());
        manager.on_response_at(100_500, RunId(7), answered(b"hello"));

        assert_eq!(manager.intercept().len(), 1, "it was kept");
        assert_eq!(
            manager
                .intercept()
                .intercept_task_info_at(100_600, "/cgi-bin/7"),
            None,
            "and the C++ answers `false` even while it is still good"
        );
    }

    #[test]
    fn the_debug_host_is_what_the_run_is_pointed_at() {
        let mut manager = ShortLinkTaskManager::new();
        let asked: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = asked.clone();
        manager.set_start_run(move |_task, request| {
            recorder.lock().unwrap().push(request.debug_host.clone());
            Some(RunId(1))
        });
        manager.set_debug_host("debug.weixin.qq.com");
        manager.start_task_at(100_000, task(7), prepare());

        assert_eq!(
            *asked.lock().unwrap(),
            vec!["debug.weixin.qq.com".to_string()]
        );
        assert_eq!(manager.debug_host(), "debug.weixin.qq.com");
    }

    #[test]
    fn a_retry_goes_out_on_the_fallback_hosts_and_on_tcp() {
        let mut manager = ShortLinkTaskManager::new();
        let asked: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = asked.clone();
        manager.set_start_run(move |task, _request| {
            recorder
                .lock()
                .unwrap()
                .push(task.shortlink_host_list.clone());
            Some(RunId(u64::from(task.taskid)))
        });
        let mut task = task(7);
        task.transport_protocol = Task::TRANSPORT_PROTOCOL_QUIC;
        manager.start_task_at(100_000, task, prepare());
        assert!(manager.on_send_at(100_000, RunId(7)));
        manager.run_loop_at(118_000);

        manager.run_loop_at(119_000);
        assert_eq!(
            *asked.lock().unwrap(),
            vec![
                vec!["short.weixin.qq.com".to_string()],
                vec!["fallback.weixin.qq.com".to_string()],
            ]
        );
        assert_eq!(
            manager.tasks()[0].task.transport_protocol,
            Task::TRANSPORT_PROTOCOL_TCP,
            "a retry is forced onto tcp"
        );
    }

    #[test]
    fn the_debug_of_a_queue_is_what_the_host_would_want_to_see() {
        let mut manager = ShortLinkTaskManager::new();
        runs(&mut manager);
        manager.set_debug_host("debug.weixin.qq.com");
        manager.start_task_at(100_000, task(7), prepare());

        let debug = format!("{manager:?}");
        assert!(debug.contains("ShortLinkTaskManager"));
        assert!(debug.contains("default_use_proxy: true"));
        assert!(debug.contains("debug_host: \"debug.weixin.qq.com\""));
        assert!(debug.contains("tasks: 1"));
    }
}
