//! `mars/stn/src/longlink_task_manager.cc` — the queue of long-link tasks.
//!
//! The short-link queue is about *runs*: one task, one socket, one answer. This
//! one is about *channels*: a task names the long link it wants
//! ([`Task::channel_name`]), and everything the C++ asks of a long link is
//! asked of a name. Two things follow from that:
//!
//! * **a channel is what the queue talks to** — its profile, whether it is up,
//!   the send, the stop, the disconnect, and the four calls a network change
//!   makes on it are all hooks here, which is what the C++'s
//!   `LongLinkMetaData` is for: a host that owns the links wires them
//!   ([`crate::LongLinkMetaData::channel`] is what it wires them to);
//! * **an answer is for a channel, not for a task** — a timeout and an error
//!   that came in are batched per channel, so one task that heard nothing fails
//!   with everything else that was out on the same link, and the link itself is
//!   taken down ([`DisconnectInternalCode`]).
//!
//! What is left out: the `MessageQueue` the C++ runs its loop on
//! ([`LongLinkTaskManager::due_time`] is when a host has to call it instead),
//! `ActiveLogic` and the Android wake lock, `NetSource`'s weak-network
//! bookkeeping, `get_real_host_`, the tls and handshake callbacks, the report
//! (`ReportTaskProfile`), `server_sequence_id`, and the minor long links the
//! C++ makes for itself (`AddMinorLink`, `IsMinorAvailable`, `FixMinorRealhost`)
//! — a minor link is only a [`Task::CHANNEL_MINOR_LONG`] channel here, which is
//! whose name the first host of [`Task::minorlong_host_list`] is.
//!
//! A task that is over is not taken off the link: the C++ stops a task on the
//! link only when the app asked to stop it ([`LongLinkTaskManager::stop_task`]).

use mars_comm::tickcount::gettickcount;

use crate::config::{MOBILE_PACKAGE_INTERVAL, WIFI_PACKAGE_INTERVAL};
use crate::dynamic_timeout::{DynamicTimeout, DynamicTimeoutStatus, NetworkKind};
use crate::long_link::DisconnectInternalCode;
use crate::longlink::LongLinkEncoder;
use crate::net_source::LonglinkConfig;
use crate::task::Task;
use crate::task_intercept::TaskIntercept;
use crate::task_profile::{
    compare_task, first_pkg_timeout, read_write_timeout, ConnectProfile, ErrCmdType,
    PrepareProfile, RunId, TaskFailHandleType, TaskProfile, HANDSHAKE_MISUNDERSTAND,
    LOCAL_ANTI_AVALANCHE, LOCAL_CANCEL, LOCAL_CHANNEL_ID, LOCAL_LONG_LINK_RELEASED,
    LOCAL_LONG_LINK_UNAVAILABLE, LOCAL_RESET, LOCAL_TASK_TIMEOUT, LONG_FIRST_PKG_TIMEOUT,
    LONG_PKG_PKG_TIMEOUT, LONG_READ_WRITE_TIMEOUT, LONG_TASK_TIMEOUT,
};

/// `fun_callback_`, `Req2Buf` and the rest of what the app answers — the same
/// questions the short-link queue asks, so the same types.
pub use crate::shortlink_task_manager::{
    AntiAvalancheCheck, Buf2Resp, Callback, MakeSureAuthed, NetInfo, NotifyRetryAllTasks, Req2Buf,
    RespHandle, ShouldIntercept, RETRY_INTERNAL,
};

/// One of the four ways `__RunOnTimeout` notices a task that answered nothing.
///
/// The C++ asks about them in this order, and the last one it asks about is the
/// one a task is failed with when two of them ran out at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timeout {
    /// `kEctLocalTaskTimeout` — the task ran out of time, retries and all. It is
    /// the one timeout that applies to a task that never went out.
    Task,
    /// `kEctLongFirstPkgTimeout` — the first package never came.
    FirstPkg,
    /// `kEctLongPkgPkgTimeout` — a package came, and then nothing for longer
    /// than the network is given between two of them.
    PkgPkg,
    /// `kEctLongReadWriteTimeout` — the whole read took too long.
    ReadWrite,
}

impl Timeout {
    /// `kEctLocal` for the task's own timeout, which is failed on its own;
    /// `kEctNetMsgXP` for the three that fail the whole channel.
    pub fn err_type(self) -> ErrCmdType {
        match self {
            Timeout::Task => ErrCmdType::Local,
            _ => ErrCmdType::NetMsgXp,
        }
    }

    /// The error code of this timeout: the `kEctLong*` one, or
    /// `kEctLocalTaskTimeout` for a task that ran out of time.
    pub fn err_code(self) -> i32 {
        match self {
            Timeout::Task => LOCAL_TASK_TIMEOUT,
            Timeout::FirstPkg => LONG_FIRST_PKG_TIMEOUT,
            Timeout::PkgPkg => LONG_PKG_PKG_TIMEOUT,
            Timeout::ReadWrite => LONG_READ_WRITE_TIMEOUT,
        }
    }
}

/// What `__OnResponse` was handed: the answer of a task, and the name of the
/// long link it came in on.
///
/// The C++'s `_extension` is not here: it is the app's own decoder that reads a
/// long-link answer, and it gets the body.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    /// `_name` — the channel the answer came in on.
    pub name: String,
    /// `_error_type`.
    pub err_type: ErrCmdType,
    /// `_error_code`.
    pub err_code: i32,
    /// `_cmdid`.
    pub cmdid: u32,
    /// `_taskid` — what the queue finds the task by. [`Task::INVALID_TASK_ID`]
    /// is the server pushing.
    pub taskid: u32,
    /// `_body`.
    pub body: Vec<u8>,
    /// `_connect_profile`.
    pub profile: ConnectProfile,
}

/// One answer for a whole channel: what failed, how, what the app is told to do
/// about it, and the task it was about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Failure {
    /// `_err_type`.
    err_type: ErrCmdType,
    /// `_err_code` — what the task the answer was about is failed with; the rest
    /// of the channel gets `0`.
    err_code: i32,
    /// `_fail_handle`.
    fail_handle: TaskFailHandleType,
    /// `_src_taskid` — [`Task::INVALID_TASK_ID`] is every task of the channel.
    src_taskid: u32,
}

/// `fun_notify_network_err_(_name, _line, _err_type, _err_code, _ip, _port)` —
/// an error the app is told about, and the channel and pair it happened on. The
/// C++ passes `__LINE__` too, which is its own bookkeeping and not a port's.
pub type NotifyNetworkErr = dyn FnMut(&str, ErrCmdType, i32, &str, u16) + Send;

/// `fun_on_push_(_channel_id, _cmdid, _taskid, _body, _extend)` — the server
/// pushed, which is an answer with no task behind it.
pub type OnPush = dyn FnMut(&str, u32, u32, &[u8]) + Send;

/// `StnManager::GenSequenceId()` — the sequence id a task is reported under,
/// which is why it is made once and not per try.
pub type GenSequenceId = dyn FnMut() -> u16 + Send;

/// `LongLink::Profile()` — the connect a channel is on.
pub type ChannelProfile = dyn FnMut(&str) -> ConnectProfile + Send;

/// `Channel()->SvrTrigOff()` and `Monitor()->MakeSureConnected()` — the link is
/// up, or it is being made. `false` is a link the task has to wait for.
pub type MakeSureConnected = dyn FnMut(&str) -> bool + Send;

/// `Channel()->Send(_bufreq, _extension, _task)` — the request goes out on the
/// channel, and the run it is out on is named. [`None`] is a write that did not
/// happen.
pub type SendOnChannel = dyn FnMut(&str, &Task, &[u8]) -> Option<RunId> + Send;

/// `Channel()->Stop(_taskid)` — the link is told the task is not waited for any
/// more, which is what [`LongLinkTaskManager::stop_task`] is.
pub type StopOnChannel = dyn FnMut(&str, u32) + Send;

/// `Channel()->Disconnect(_code)` — the link is taken down, and told why.
pub type DisconnectChannel = dyn FnMut(&str, DisconnectInternalCode) + Send;

/// What `RedoTasks()` does to a channel before its tasks are looked at again:
/// the C++'s `Checker()->CancelConnect()`, `Channel()->Disconnect(kReset)`,
/// `Channel()->SvrTrigOff()` and `Monitor()->MakeSureConnected()`, which is a
/// link taken apart and made again.
pub type ResetChannel = dyn FnMut(&str) + Send;

/// `Monitor()->NetworkChange()` — whether the channel has to be looked at again
/// now that the network is another one.
pub type NetworkChange = dyn FnMut(&str) -> bool + Send;

/// `LongLinkTaskManager` — the queue of tasks that go out on a long link, and
/// the channels they go out on.
pub struct LongLinkTaskManager {
    /// `lst_cmd_`, sorted by [`crate::task_profile::compare_task`].
    tasks: Vec<TaskProfile>,
    /// `longlink_metas_` — what the queue keeps of a channel is its config:
    /// the name a task names it by, whether it is the main one, and what kind
    /// of link it is.
    channels: Vec<LonglinkConfig>,
    /// `lastbatcherrortime_`.
    last_batch_error_time: u64,
    /// `retry_interval_` — one wait for the whole queue, which is what the C++
    /// keeps instead of one per task.
    retry_interval: u64,
    /// `tasks_continuous_fail_count_`.
    tasks_continuous_fail_count: u32,
    /// `dynamic_timeout_` — the C++ shares one with the short-link queue; this
    /// one is the queue's own, and [`LongLinkTaskManager::dynamic_timeout`] is
    /// the one to share.
    dynamic_timeout: DynamicTimeout,
    /// `task_intercept_`.
    intercept: TaskIntercept,
    /// `forbid_tls_host_`.
    forbid_tls_hosts: Vec<String>,
    /// `default_longlink_encoder` — one for every channel: the C++ gives each
    /// channel its own, but nothing about the port's encoder is per-connection.
    encoder: LongLinkEncoder,
    /// What the next run an unset [`SendOnChannel`] is asked for is called, so
    /// that two runs never answer for the same task.
    next_run_id: u64,
    callback: Option<Box<Callback>>,
    notify_network_err: Option<Box<NotifyNetworkErr>>,
    notify_retry_all_tasks: Option<Box<NotifyRetryAllTasks>>,
    on_push: Option<Box<OnPush>>,
    req2buf: Option<Box<Req2Buf>>,
    buf2resp: Option<Box<Buf2Resp>>,
    make_sure_authed: Option<Box<MakeSureAuthed>>,
    anti_avalanche: Option<Box<AntiAvalancheCheck>>,
    should_intercept: Option<Box<ShouldIntercept>>,
    net_info: Option<Box<NetInfo>>,
    gen_sequence_id: Option<Box<GenSequenceId>>,
    channel_profile: Option<Box<ChannelProfile>>,
    make_sure_connected: Option<Box<MakeSureConnected>>,
    send: Option<Box<SendOnChannel>>,
    stop: Option<Box<StopOnChannel>>,
    disconnect: Option<Box<DisconnectChannel>>,
    reset_channel: Option<Box<ResetChannel>>,
    network_change: Option<Box<NetworkChange>>,
}

impl LongLinkTaskManager {
    /// `LongLinkTaskManager(...)` — a queue with nothing in it and no channels:
    /// a host adds the links ([`LongLinkTaskManager::add_long_link`]) and wires
    /// what the queue asks of them.
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            channels: Vec::new(),
            last_batch_error_time: 0,
            retry_interval: 0,
            tasks_continuous_fail_count: 0,
            dynamic_timeout: DynamicTimeout::new(),
            intercept: TaskIntercept::new(),
            forbid_tls_hosts: Vec::new(),
            encoder: LongLinkEncoder::new(),
            next_run_id: 0,
            callback: None,
            notify_network_err: None,
            notify_retry_all_tasks: None,
            on_push: None,
            req2buf: None,
            buf2resp: None,
            make_sure_authed: None,
            anti_avalanche: None,
            should_intercept: None,
            net_info: None,
            gen_sequence_id: None,
            channel_profile: None,
            make_sure_connected: None,
            send: None,
            stop: None,
            disconnect: None,
            reset_channel: None,
            network_change: None,
        }
    }

    /// `AddLongLink(_config)` — `false` when the queue has a channel of that
    /// name already.
    ///
    /// A host on a host list the app forbade tls for is one that does not get
    /// it, which is the C++'s `__ForbidUseTls`.
    pub fn add_long_link(&mut self, config: LonglinkConfig) -> bool {
        if self
            .channels
            .iter()
            .any(|channel| channel.name == config.name)
        {
            return false;
        }
        let mut config = config;
        let need_tls = !self.forbid_tls(&config.host_list);
        config.need_tls = need_tls;
        self.channels.push(config);
        true
    }

    /// `ReleaseLongLink(_name)` — the channel is gone, and every task that was
    /// going out on it is failed with `kEctLocal` /
    /// `kEctLocalLongLinkReleased` / `kTaskFailHandleTaskEnd`. `false` when
    /// there was no such channel.
    pub fn remove_long_link(&mut self, name: &str) -> bool {
        self.remove_long_link_at(gettickcount(), name)
    }

    /// The same, with the reading handed in.
    pub fn remove_long_link_at(&mut self, now: u64, name: &str) -> bool {
        if !self.has_channel(name) {
            return false;
        }
        let mut i = 0;
        while i < self.tasks.len() {
            if self.tasks[i].channel_name != name {
                i += 1;
                continue;
            }
            let profile = self.profile_of(name);
            let ended = self.single_resp_handle_at(
                now,
                i,
                ErrCmdType::Local,
                LOCAL_LONG_LINK_RELEASED,
                TaskFailHandleType::TaskEnd,
                profile,
            );
            if !ended {
                i += 1;
            }
        }
        self.channels.retain(|channel| channel.name != name);
        true
    }

    /// `longlink_metas_`.
    pub fn channels(&self) -> &[LonglinkConfig] {
        &self.channels
    }

    /// `DefaultLongLink()` — the name of the channel whose config says it is
    /// the main one, which is the link the app is given when it asks for "the"
    /// long link.
    pub fn default_channel(&self) -> Option<&str> {
        self.channels
            .iter()
            .find(|channel| channel.is_main())
            .map(|channel| channel.name.as_str())
    }

    /// `StartTask(_task, _channel)` — the task is put in the queue and the loop
    /// is run. `_channel` is one of the `Task::CHANNEL_*`, which is what a task
    /// is remembered as going out on.
    pub fn start_task(&mut self, task: Task, channel: i32) -> bool {
        self.start_task_at(gettickcount(), task, channel)
    }

    /// The same, with the reading the task is started at handed in.
    pub fn start_task_at(&mut self, now: u64, task: Task, channel: i32) -> bool {
        let mut profile = TaskProfile::new_at(now, task, PrepareProfile::new_at(now));
        profile.link_type = channel;
        // a minor long link is named after the first host it came with: the
        // redirect was fixed before the task got here
        profile.channel_name = if channel == Task::CHANNEL_MINOR_LONG {
            profile
                .task
                .minorlong_host_list
                .first()
                .cloned()
                .unwrap_or_default()
        } else {
            profile.task.channel_name.clone()
        };

        self.tasks.push(profile);
        // `lst_cmd_.sort(__CompareTask)` — a stable sort, so two tasks of one
        // priority stay in the order they were asked for
        self.tasks.sort_by(compare_task);

        self.run_loop_at(now);
        true
    }

    /// `StopTask(_taskid)` — `true` when there was such a task. Its channel is
    /// told it is not waited for any more.
    pub fn stop_task(&mut self, taskid: u32) -> bool {
        let Some(at) = self.locate(taskid) else {
            return false;
        };
        let name = self.tasks[at].channel_name.clone();
        if !self.has_channel(&name) {
            // the C++'s "longlink nullptr": a task whose channel is gone is not
            // one the queue can stop
            return false;
        }
        if let Some(stop) = self.stop.as_mut() {
            stop(&name, taskid);
        }
        self.tasks.remove(at);
        true
    }

    /// `HasTask(_taskid)`.
    pub fn has_task(&self, taskid: u32) -> bool {
        self.tasks
            .iter()
            .any(|profile| profile.task.taskid == taskid)
    }

    /// `ClearTasks()` — every channel is taken down with `kReset` and the queue
    /// is emptied, which is not the same as failing the tasks: the app is not
    /// told about any of them.
    pub fn clear_tasks(&mut self) {
        for name in self.channel_names() {
            self.disconnect(&name, DisconnectInternalCode::Reset);
        }
        self.tasks.clear();
    }

    /// `GetTaskCount(_name)` — how many tasks are going out on that channel.
    pub fn task_count(&self, name: &str) -> usize {
        self.tasks
            .iter()
            .filter(|profile| profile.channel_name == name)
            .count()
    }

    /// `RedoTasks()` — the network is another one now: every channel is taken
    /// apart and made again, and every task that was out is cancelled and tried
    /// again.
    pub fn redo_tasks(&mut self) {
        self.redo_tasks_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn redo_tasks_at(&mut self, now: u64) {
        for name in self.channel_names() {
            if let Some(reset) = self.reset_channel.as_mut() {
                reset(&name);
            }
        }
        self.redo_tasks_of_at(now, "");
    }

    /// `__RedoTasks(_name)` — the tasks of one channel are cancelled and tried
    /// again, which is what a channel whose own monitor said the network changed
    /// asks for. The channel itself is left alone.
    pub fn redo_tasks_of(&mut self, name: &str) {
        self.redo_tasks_of_at(gettickcount(), name)
    }

    /// The same, with the reading handed in.
    pub fn redo_tasks_of_at(&mut self, now: u64, name: &str) {
        let mut i = 0;
        while i < self.tasks.len() {
            if !name.is_empty() && self.tasks[i].channel_name != name {
                i += 1;
                continue;
            }
            self.tasks[i].last_failed_dyntime_status = DynamicTimeoutStatus::default();
            if self.tasks[i].is_running() {
                let channel = self.tasks[i].channel_name.clone();
                let profile = self.profile_of(&channel);
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

        self.retry_interval = 0;
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

    /// `RetryTasks(...)` — what the app answered to [`RespHandle::Deferred`]:
    /// every task of that user is looked at again.
    pub fn retry_tasks(
        &mut self,
        err_type: ErrCmdType,
        err_code: i32,
        fail_handle: TaskFailHandleType,
        src_taskid: u32,
        user_id: &str,
    ) {
        self.retry_tasks_at(
            gettickcount(),
            err_type,
            err_code,
            fail_handle,
            src_taskid,
            user_id,
        )
    }

    /// The same, with the reading handed in.
    pub fn retry_tasks_at(
        &mut self,
        now: u64,
        err_type: ErrCmdType,
        err_code: i32,
        fail_handle: TaskFailHandleType,
        src_taskid: u32,
        user_id: &str,
    ) {
        // the C++ walks a copy: one task's channel can be failed wholesale, and
        // every task of that user is still looked at
        let channels: Vec<String> = self
            .tasks
            .iter()
            .filter(|profile| profile.task.user_id == user_id)
            .map(|profile| profile.channel_name.clone())
            .collect();

        for name in channels {
            self.batch_error_resp_handle_at(
                now,
                name,
                Failure {
                    err_type,
                    err_code,
                    fail_handle,
                    src_taskid,
                },
                true,
            );
        }
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

    /// `__OnSend` — the request went out, which is what the read-write and the
    /// first-package timeouts are counted from. `false` when the queue has no
    /// such task.
    pub fn on_send(&mut self, taskid: u32) -> bool {
        self.on_send_at(gettickcount(), taskid)
    }

    /// The same, with the reading handed in.
    pub fn on_send_at(&mut self, now: u64, taskid: u32) -> bool {
        let Some(at) = self.locate(taskid) else {
            return false;
        };
        let profile = &mut self.tasks[at].transfer_profile;
        if profile.first_start_send_time == 0 {
            profile.first_start_send_time = now;
        }
        profile.start_send_time = now;
        true
    }

    /// `__OnRecv` — a package of the answer came in, which is what the pkg-pkg
    /// timeout is counted from. `false` when the queue has no such task.
    pub fn on_recv(&mut self, taskid: u32, cached_size: usize, total_size: usize) -> bool {
        self.on_recv_at(gettickcount(), taskid, cached_size, total_size)
    }

    /// The same, with the reading handed in.
    pub fn on_recv_at(
        &mut self,
        now: u64,
        taskid: u32,
        cached_size: usize,
        total_size: usize,
    ) -> bool {
        let Some(at) = self.locate(taskid) else {
            return false;
        };
        let profile = &mut self.tasks[at].transfer_profile;
        profile.last_receive_pkg_time = now;
        profile.received_size = cached_size;
        profile.receive_data_size = total_size;
        true
    }

    /// `__OnResponse` — what a channel answered with.
    ///
    /// [`None`] is an answer that is not about a task the queue knows: the
    /// server pushing, an answer for a channel the queue has no link for, or one
    /// for a task that is over.
    pub fn on_response(&mut self, response: Response) -> Option<RespHandle> {
        self.on_response_at(gettickcount(), response)
    }

    /// The same, with the reading handed in.
    pub fn on_response_at(&mut self, now: u64, response: Response) -> Option<RespHandle> {
        if !self.has_channel(&response.name) {
            // "longlink response but longlink destroyed"
            return None;
        }

        if response.err_type == ErrCmdType::Ok && self.encoder.is_push(response.taskid) {
            if let Some(push) = self.on_push.as_mut() {
                push(
                    &response.name,
                    response.cmdid,
                    response.taskid,
                    &response.body,
                );
            }
            return None;
        }

        // The C++ locates before it asks whether the answer is an error:
        // `__Locate` has no answer for `kInvalidTaskID`, and an error that came
        // in for the channel — not for a task — carries exactly that.
        let at = self.locate(response.taskid);
        let taskid = response.taskid;

        if response.err_type != ErrCmdType::Ok {
            if response.err_code == HANDSHAKE_MISUNDERSTAND {
                if let Some(at) = at {
                    // the two ends did not agree on the handshake: a try that is
                    // not the task's to pay for
                    self.tasks[at].remain_retry_count += 1;
                }
            }
            // one error for the whole channel it came in on
            self.batch_error_resp_handle_at(
                now,
                response.name,
                Failure {
                    err_type: response.err_type,
                    err_code: response.err_code,
                    fail_handle: TaskFailHandleType::Default,
                    src_taskid: Task::INVALID_TASK_ID,
                },
                true,
            );
            // The C++ answers nothing; the port says whether there is still a
            // task waiting, which for an error with no task id of its own is
            // any task on the channel.
            let waiting = if taskid == Task::INVALID_TASK_ID {
                !self.tasks.is_empty()
            } else {
                self.has_task(taskid)
            };
            return Some(still_there(waiting));
        }

        let Some(at) = at else {
            // "task no found": an answer for a task that is over, or one that
            // carries no task id at all
            return None;
        };
        let name = self.tasks[at].channel_name.clone();

        let task = self.tasks[at].task.clone();
        let len = response.body.len();
        {
            let profile = &mut self.tasks[at].transfer_profile;
            profile.received_size = len;
            profile.receive_data_size = len;
            profile.last_receive_pkg_time = now;
        }

        let (err_code, handle) = self.decode(&task, &response.body);
        if self.should_intercept(err_code) {
            self.intercept
                .add_intercept_task_at(now, &task.cgi, response.body.clone());
        }

        match handle {
            TaskFailHandleType::Normal => {
                let network = self.network();
                let cost = now.saturating_sub(self.tasks[at].transfer_profile.start_send_time);
                let total = self.tasks[at].transfer_profile.send_data_size + len;
                self.dynamic_timeout
                    .record_at(network, total as u32, cost, now);
                let ended = self.single_resp_handle_at(
                    now,
                    at,
                    ErrCmdType::Ok,
                    err_code,
                    handle,
                    response.profile.clone(),
                );
                self.notify_network_err(
                    &response.name,
                    ErrCmdType::Ok,
                    err_code,
                    &response.profile,
                );
                Some(ended_or_retried(ended))
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
                    response.profile,
                );
                Some(ended_or_retried(ended))
            }
            TaskFailHandleType::SlientTaskEnd => {
                // over, and the app is not told
                self.tasks.remove(at);
                Some(RespHandle::Ended)
            }
            // `kTaskFailHandleDefault` and anything the app made up: the C++'s
            // `default:`, which fails the channel and not just the task
            TaskFailHandleType::Default | TaskFailHandleType::TaskTimeout => {
                self.batch_error_resp_handle_at(
                    now,
                    name,
                    Failure {
                        err_type: ErrCmdType::EnDecode,
                        err_code,
                        fail_handle: handle,
                        src_taskid: taskid,
                    },
                    true,
                );
                self.notify_network_err(
                    &response.name,
                    ErrCmdType::EnDecode,
                    handle as i32,
                    &response.profile,
                );
                Some(still_there(self.has_task(taskid)))
            }
        }
    }

    /// `OnNetworkChange()` — a channel whose own monitor says the network is
    /// another one now has its tasks cancelled and tried again.
    pub fn on_network_change(&mut self) {
        self.on_network_change_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn on_network_change_at(&mut self, now: u64) {
        for name in self.channel_names() {
            let changed = match self.network_change.as_mut() {
                Some(network_change) => network_change(&name),
                // an unwired monitor is one that did what it was asked: the
                // network changed for every channel
                None => true,
            };
            if changed {
                self.redo_tasks_of_at(now, &name);
            }
        }
    }

    /// `GetConnectProfile(_taskid)` — the connect of the channel the task is
    /// going out on, which the C++ hands back by value; a task the queue does
    /// not know is one with nothing in its profile.
    pub fn connect_profile(&mut self, taskid: u32) -> ConnectProfile {
        match self.locate(taskid) {
            Some(at) => {
                let name = self.tasks[at].channel_name.clone();
                self.profile_of(&name)
            }
            None => ConnectProfile::new(),
        }
    }

    /// `DisconnectByTaskId(_taskid, _code)` — the channel a task is going out on
    /// is taken down. `false` when there is no such task, or no such channel.
    pub fn disconnect_by_taskid(&mut self, taskid: u32, code: DisconnectInternalCode) -> bool {
        let Some(at) = self.locate(taskid) else {
            return false;
        };
        let name = self.tasks[at].channel_name.clone();
        if !self.has_channel(&name) {
            return false;
        }
        self.disconnect(&name, code);
        true
    }

    /// `AddForbidTlsHost(_host)` — hosts the app said not to do tls on. A host
    /// without "long" in it is not one of them, which is the C++'s own test.
    pub fn add_forbid_tls_host(&mut self, hosts: &[String]) {
        for host in hosts {
            if !host.contains("long") {
                continue;
            }
            if !self.forbid_tls_hosts.contains(host) {
                self.forbid_tls_hosts.push(host.clone());
            }
        }
    }

    /// `__ForbidUseTls(_host_list)` — whether a channel of these hosts is one
    /// the app forbade tls for.
    pub fn forbid_tls(&self, host_list: &[String]) -> bool {
        if host_list.is_empty() || self.forbid_tls_hosts.is_empty() {
            return false;
        }
        host_list
            .iter()
            .any(|host| self.forbid_tls_hosts.contains(host))
    }

    /// `GetTasksContinuousFailCount()` — how many tasks in a row failed without
    /// one that did not.
    pub fn tasks_continuous_fail_count(&self) -> u32 {
        self.tasks_continuous_fail_count
    }

    /// `retry_interval_` — how long every task of the queue waits after a whole
    /// channel was failed at once.
    pub fn retry_interval(&self) -> u64 {
        self.retry_interval
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

    /// When the host has to call [`LongLinkTaskManager::run_loop_at`] again:
    /// the earliest of the deadlines the tasks are waiting on — a timeout that
    /// is about to run out, or the wait every task owes the queue after a
    /// channel was failed. [`None`] when there is nothing to wait for, which is
    /// when the C++ stops its loop.
    pub fn due_time(&mut self) -> Option<u64> {
        let network = self.network();
        let (last_batch_error_time, retry_interval) =
            (self.last_batch_error_time, self.retry_interval);

        let deadlines = self
            .tasks
            .iter()
            .filter_map(|profile| next_deadline(profile, network));
        // a task that has been tried once already waits out the queue's own
        // interval, which is not one it keeps for itself
        let retries = self
            .tasks
            .iter()
            .filter(|profile| !profile.is_running() && profile.retried())
            .map(|_| last_batch_error_time.saturating_add(retry_interval));

        deadlines.chain(retries).min()
    }

    /// `task_intercept_`.
    pub fn intercept(&mut self) -> &mut TaskIntercept {
        &mut self.intercept
    }

    /// `dynamic_timeout_`.
    pub fn dynamic_timeout(&mut self) -> &mut DynamicTimeout {
        &mut self.dynamic_timeout
    }

    /// The encoder the queue asks whether an answer is the server pushing.
    pub fn set_encoder(&mut self, encoder: LongLinkEncoder) {
        self.encoder = encoder;
    }

    /// `fun_callback_`.
    /// `fun_callback_` — a task that is over, and what the app says about it:
    /// the answer is the code the task is remembered with.
    ///
    /// The connect profile comes along for the same reason it does in
    /// [`crate::ShortLinkTaskManager::set_callback`]: the C++ reads it out of
    /// the queue from inside `NetCore::__CallBack`, and a hook cannot borrow
    /// the queue that called it.
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
        notify: impl FnMut(&str, ErrCmdType, i32, &str, u16) + Send + 'static,
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

    /// `fun_on_push_`.
    pub fn set_on_push(&mut self, push: impl FnMut(&str, u32, u32, &[u8]) + Send + 'static) {
        self.on_push = Some(Box::new(push));
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

    /// `GenSequenceId` — unset is a task that is reported under sequence id `0`.
    pub fn set_gen_sequence_id(&mut self, gen: impl FnMut() -> u16 + Send + 'static) {
        self.gen_sequence_id = Some(Box::new(gen));
    }

    /// `LongLink::Profile()` — unset is a channel with nothing in its profile.
    pub fn set_channel_profile(
        &mut self,
        profile: impl FnMut(&str) -> ConnectProfile + Send + 'static,
    ) {
        self.channel_profile = Some(Box::new(profile));
    }

    /// `Channel()->SvrTrigOff()` + `Monitor()->MakeSureConnected()` — unset is a
    /// channel that is up, which is what lets every task go out.
    pub fn set_make_sure_connected(
        &mut self,
        make_sure_connected: impl FnMut(&str) -> bool + Send + 'static,
    ) {
        self.make_sure_connected = Some(Box::new(make_sure_connected));
    }

    /// `Channel()->Send(...)` — the request goes out. Unset is a run nobody
    /// answers, which is what makes a task time out.
    pub fn set_send(
        &mut self,
        send: impl FnMut(&str, &Task, &[u8]) -> Option<RunId> + Send + 'static,
    ) {
        self.send = Some(Box::new(send));
    }

    /// `Channel()->Stop(...)`.
    pub fn set_stop(&mut self, stop: impl FnMut(&str, u32) + Send + 'static) {
        self.stop = Some(Box::new(stop));
    }

    /// `Channel()->Disconnect(...)`.
    pub fn set_disconnect(
        &mut self,
        disconnect: impl FnMut(&str, DisconnectInternalCode) + Send + 'static,
    ) {
        self.disconnect = Some(Box::new(disconnect));
    }

    /// What `RedoTasks()` does to a channel before its tasks are looked at
    /// again: the connect is cancelled, the link is taken down with `kReset`,
    /// the server's trigger is taken off, and it is made again.
    pub fn set_reset_channel(&mut self, reset: impl FnMut(&str) + Send + 'static) {
        self.reset_channel = Some(Box::new(reset));
    }

    /// `Monitor()->NetworkChange()` — unset is a monitor that says the network
    /// changed for every channel.
    pub fn set_network_change(
        &mut self,
        network_change: impl FnMut(&str) -> bool + Send + 'static,
    ) {
        self.network_change = Some(Box::new(network_change));
    }

    /// `__RunOnTimeout` — the tasks that answered nothing.
    ///
    /// A task that is out is waiting on three timeouts at once and the C++
    /// asks about them in order, so the last one it asks about is the one the
    /// channel is failed with; and it is failed once per channel, not once per
    /// task, which is what takes the link down.
    fn run_on_timeout_at(&mut self, now: u64) {
        let network = self.network();

        // one pass over the queue: a task that is tried again has its readings
        // cleared, which is what would make it look like it timed out once more
        let timed_out: Vec<(u32, String, Vec<Timeout>)> = self
            .tasks
            .iter()
            .map(|profile| {
                (
                    profile.task.taskid,
                    profile.channel_name.clone(),
                    due(profile, now, network),
                )
            })
            .collect();

        // `batchMap[channel_name] = (socket_timeout_code, src_taskid)`
        let mut batch: Vec<(String, i32, u32)> = Vec::new();

        for (taskid, name, dues) in timed_out {
            let Some(at) = self.locate(taskid) else {
                continue;
            };
            let read = dues
                .iter()
                .rev()
                .find(|timeout| **timeout != Timeout::Task)
                .copied();
            let over = dues.contains(&Timeout::Task);

            if let Some(timeout) = read {
                if timeout == Timeout::FirstPkg {
                    self.tasks[at].set_last_failed_status();
                }
                upsert(&mut batch, name.clone(), (timeout.err_code(), taskid));
            }

            if over {
                if !batch.iter().any(|(channel, _, _)| *channel == name) {
                    batch.push((name.clone(), LONG_TASK_TIMEOUT, taskid));
                }
                let profile = self.profile_of(&name);
                self.single_resp_handle_at(
                    now,
                    at,
                    ErrCmdType::Local,
                    LOCAL_TASK_TIMEOUT,
                    TaskFailHandleType::TaskTimeout,
                    profile,
                );
            }
        }

        for (name, code, src_taskid) in batch {
            if code == LONG_TASK_TIMEOUT {
                // a task that ran out of its own time is one the queue failed
                // already; the rest of the channel is told with it
                self.batch_error_resp_handle_at(
                    now,
                    name,
                    Failure {
                        err_type: ErrCmdType::NetMsgXp,
                        err_code: LOCAL_TASK_TIMEOUT,
                        fail_handle: TaskFailHandleType::Default,
                        src_taskid,
                    },
                    true,
                );
                continue;
            }
            let profile = self.profile_of(&name);
            self.notify_network_err(&name, ErrCmdType::NetMsgXp, code, &profile);
            self.batch_error_resp_handle_at(
                now,
                name,
                Failure {
                    err_type: ErrCmdType::NetMsgXp,
                    err_code: code,
                    fail_handle: TaskFailHandleType::Default,
                    src_taskid,
                },
                true,
            );
        }
    }

    /// `__RunOnStartTask` — the tasks that may go out now.
    fn run_on_start_task_at(&mut self, now: u64) {
        let mobile = self.network() == NetworkKind::Mobile;
        // one gate for the whole queue: `retry_interval_` is not a wait a task
        // keeps for itself
        let can_retry = now.saturating_sub(self.last_batch_error_time) >= self.retry_interval;
        let mut sent_count: i32 = 0;
        let mut i = 0;

        while i < self.tasks.len() {
            if self.tasks[i].is_running() {
                sent_count += 1;
                i += 1;
                continue;
            }

            // the wait is not one a task owes for its first try
            if self.tasks[i].retried() && !can_retry {
                i += 1;
                continue;
            }

            let name = self.tasks[i].channel_name.clone();
            let task = self.tasks[i].task.clone();
            // `get_real_host_` is not here: the hosts the task came with are
            // the ones it is given
            let host = task.longlink_host_list.first().cloned().unwrap_or_default();

            if task.need_authed && !self.authed(&host, &task.user_id) {
                i += 1;
                continue;
            }

            if !self.has_channel(&name) {
                // the C++'s "longlink nullptr": a task whose channel is gone
                // stays where it is
                i += 1;
                continue;
            }

            if !self.tasks[i].antiavalanche_checked {
                // once: a retry is the same request, and the sequence id is
                // what ties the task to the server-side report
                self.tasks[i].task.client_sequence_id = self.sequence_id();
            }

            let body = match self.encode(&task) {
                Ok(body) => body,
                Err(code) => {
                    let profile = self.profile_of(&name);
                    if !self.single_resp_handle_at(
                        now,
                        i,
                        ErrCmdType::EnDecode,
                        code,
                        TaskFailHandleType::TaskEnd,
                        profile,
                    ) {
                        i += 1;
                    }
                    continue;
                }
            };

            if !self.allowed(&task, &body) {
                let profile = self.profile_of(&name);
                if !self.single_resp_handle_at(
                    now,
                    i,
                    ErrCmdType::Local,
                    LOCAL_ANTI_AVALANCHE,
                    TaskFailHandleType::TaskEnd,
                    profile,
                ) {
                    i += 1;
                }
                continue;
            }
            self.tasks[i].antiavalanche_checked = true;

            if !self.make_sure_connected(&name) {
                if task.channel_id != 0 {
                    // a task that asked for a link that is not up is one that
                    // will not be waited for
                    let profile = self.profile_of(&name);
                    if !self.single_resp_handle_at(
                        now,
                        i,
                        ErrCmdType::Local,
                        LOCAL_CHANNEL_ID,
                        TaskFailHandleType::TaskEnd,
                        profile,
                    ) {
                        i += 1;
                    }
                    continue;
                }
                i += 1;
                continue;
            }

            let profile = self.profile_of(&name);
            if task.channel_id != 0 && profile.start_time != task.channel_id {
                // the link was made again: this is not the one the task asked
                // for, and the C++ does not put it on another one
                if !self.single_resp_handle_at(
                    now,
                    i,
                    ErrCmdType::Local,
                    LOCAL_CHANNEL_ID,
                    TaskFailHandleType::TaskEnd,
                    profile,
                ) {
                    i += 1;
                }
                continue;
            }

            if let Some(answer) = self.intercept.intercept_task_info_at(now, &task.cgi) {
                let len = answer.len();
                let (err_code, handle) = self.decode(&task, &answer);
                {
                    let profile = &mut self.tasks[i].transfer_profile;
                    profile.received_size = len;
                    profile.receive_data_size = len;
                    profile.last_receive_pkg_time = now;
                }
                // nothing went out, so nothing is in the profile: the C++'s own
                // `ConnectProfile profile;`
                if !self.single_resp_handle_at(
                    now,
                    i,
                    ErrCmdType::EnDecode,
                    err_code,
                    handle,
                    ConnectProfile::new(),
                ) {
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
            let read_write = read_write_timeout(first_pkg, mobile);
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

            let Some(run) = self.send(&name, &task, &body) else {
                // the C++ leaves the task in the queue and tries again later
                i += 1;
                continue;
            };
            self.tasks[i].running = Some(run);
            sent_count += 1;

            if task.send_only {
                // a task that is only sent is over the moment it went out
                let profile = self.profile_of(&name);
                if !self.single_resp_handle_at(
                    now,
                    i,
                    ErrCmdType::Ok,
                    0,
                    TaskFailHandleType::Normal,
                    profile,
                ) {
                    i += 1;
                }
                continue;
            }
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
        self.tasks[at].transfer_profile.connect_profile = profile.clone();
        self.tasks[at].link_type = profile.link_type;

        if err_type == ErrCmdType::Ok {
            self.retry_interval = 0;
            self.tasks_continuous_fail_count = 0;
        } else {
            self.tasks_continuous_fail_count = self.tasks_continuous_fail_count.saturating_add(1);
        }

        let cost = now.saturating_sub(self.tasks[at].start_task_time);
        let over = self.tasks[at].remain_retry_count <= 0
            || err_type == ErrCmdType::Ok
            || fail_handle == TaskFailHandleType::TaskEnd
            || fail_handle == TaskFailHandleType::TaskTimeout;

        if over {
            let task = self.tasks[at].task.clone();
            let was_running = self.tasks[at].is_running();
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
                // what the app says about an answer it was given is the code the
                // task is remembered with — except for a task that was only
                // sent, or one that never went out
                profile.err_code = if !task.send_only && was_running && err_type == ErrCmdType::Ok {
                    cgi_retcode
                } else {
                    err_code
                };
                profile.transfer_profile.error_type = err_type;
                profile.transfer_profile.error_code = err_code;
                profile.push_history();
            }
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
        self.tasks[at].push_history();
        self.tasks[at].init_send_param_at(now);
        false
    }

    /// `__BatchErrorRespHandle` — one answer for every task of a channel.
    /// `running_only` is the C++'s `_callback_runing_task_only`, which only the
    /// destructor says `false` to.
    fn batch_error_resp_handle_at(
        &mut self,
        now: u64,
        name: String,
        failure: Failure,
        running_only: bool,
    ) {
        let Failure {
            err_type,
            err_code,
            fail_handle,
            src_taskid,
        } = failure;
        let mut i = 0;
        while i < self.tasks.len() {
            if running_only && !self.tasks[i].is_running() {
                i += 1;
                continue;
            }
            if !name.is_empty() && self.tasks[i].channel_name != name {
                i += 1;
                continue;
            }

            let channel = self.tasks[i].channel_name.clone();
            // the task the error came from is the one it is remembered on; the
            // rest of the channel gets it without the code
            let is_source =
                src_taskid == Task::INVALID_TASK_ID || src_taskid == self.tasks[i].task.taskid;
            let code = if is_source { err_code } else { 0 };
            let profile = self.profile_of(&channel);
            if !self.single_resp_handle_at(now, i, err_type, code, fail_handle, profile) {
                i += 1;
            }
        }

        self.last_batch_error_time = now;

        if err_type != ErrCmdType::Local && !self.tasks.is_empty() {
            self.retry_interval = RETRY_INTERNAL;
        }

        if matches!(
            fail_handle,
            TaskFailHandleType::SessionTimeout | TaskFailHandleType::RetryAllTasks
        ) {
            self.disconnect(&name, DisconnectInternalCode::DecodeErr);
            self.retry_interval = 0;
        }

        // not a long-link callback: a link that failed on dns, on a socket or on
        // a cancel is one the C++ leaves alone
        if fail_handle == TaskFailHandleType::Default
            && !matches!(
                err_type,
                ErrCmdType::Dns | ErrCmdType::Socket | ErrCmdType::Canceld
            )
        {
            self.disconnect(&name, DisconnectInternalCode::DecodeErr);
        }

        if err_type == ErrCmdType::NetMsgXp {
            self.disconnect(&name, DisconnectInternalCode::TaskTimeout);
        }
    }

    /// `__Locate` — the task an answer is about, which the C++ finds by task id:
    /// a long link answers for a task, not for a run.
    fn locate(&self, taskid: u32) -> Option<usize> {
        if taskid == Task::INVALID_TASK_ID {
            return None;
        }
        self.tasks
            .iter()
            .position(|profile| profile.task.taskid == taskid)
    }

    /// Whether the queue has a channel of that name.
    fn has_channel(&self, name: &str) -> bool {
        self.channels.iter().any(|channel| channel.name == name)
    }

    fn channel_names(&self) -> Vec<String> {
        self.channels
            .iter()
            .map(|channel| channel.name.clone())
            .collect()
    }

    /// `__GetConnectionProfile` — the connect of a channel, or one that says
    /// the link is not there, which is what the C++ hands a task whose channel
    /// it could not find.
    fn profile_of(&mut self, name: &str) -> ConnectProfile {
        if !self.has_channel(name) {
            let mut profile = ConnectProfile::new();
            profile.disconn_errtype = ErrCmdType::Local;
            profile.conn_errcode = LOCAL_LONG_LINK_UNAVAILABLE;
            return profile;
        }
        match self.channel_profile.as_mut() {
            Some(profile) => profile(name),
            None => ConnectProfile::new(),
        }
    }

    fn send(&mut self, name: &str, task: &Task, body: &[u8]) -> Option<RunId> {
        match self.send.as_mut() {
            Some(send) => send(name, task, body),
            None => {
                self.next_run_id += 1;
                Some(RunId(self.next_run_id))
            }
        }
    }

    fn make_sure_connected(&mut self, name: &str) -> bool {
        match self.make_sure_connected.as_mut() {
            Some(make_sure_connected) => make_sure_connected(name),
            None => true,
        }
    }

    fn disconnect(&mut self, name: &str, code: DisconnectInternalCode) {
        if !self.has_channel(name) {
            return;
        }
        if let Some(disconnect) = self.disconnect.as_mut() {
            disconnect(name, code);
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
        name: &str,
        err_type: ErrCmdType,
        err_code: i32,
        profile: &ConnectProfile,
    ) {
        if let Some(notify) = self.notify_network_err.as_mut() {
            notify(name, err_type, err_code, &profile.ip, profile.port);
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
}

impl Default for LongLinkTaskManager {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for LongLinkTaskManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LongLinkTaskManager")
            .field("tasks", &self.tasks.len())
            .field("channels", &self.channels.len())
            .field("retry_interval", &self.retry_interval)
            .field(
                "tasks_continuous_fail_count",
                &self.tasks_continuous_fail_count,
            )
            .field("intercept", &self.intercept.len())
            .finish_non_exhaustive()
    }
}

impl Drop for LongLinkTaskManager {
    /// `~LongLinkTaskManager()` — every task the queue still has is failed with
    /// `kEctLocal` / `kEctLocalReset` / `kTaskFailHandleTaskEnd`, running or
    /// not, which is what the C++ does with `_callback_runing_task_only` said
    /// `false` to.
    fn drop(&mut self) {
        self.batch_error_resp_handle_at(
            gettickcount(),
            String::new(),
            Failure {
                err_type: ErrCmdType::Local,
                err_code: LOCAL_RESET,
                fail_handle: TaskFailHandleType::TaskEnd,
                src_taskid: Task::INVALID_TASK_ID,
            },
            false,
        );
        self.channels.clear();
    }
}

/// `true` is a task that left the queue.
fn ended_or_retried(ended: bool) -> RespHandle {
    if ended {
        RespHandle::Ended
    } else {
        RespHandle::Retried
    }
}

/// Whether the task a whole channel was failed for is still in the queue.
fn still_there(is_there: bool) -> RespHandle {
    if is_there {
        RespHandle::Retried
    } else {
        RespHandle::Ended
    }
}

/// How long a package may be in coming after the one before it.
fn pkg_pkg_interval(network: NetworkKind) -> u64 {
    match network {
        NetworkKind::Mobile => MOBILE_PACKAGE_INTERVAL,
        NetworkKind::Wifi => WIFI_PACKAGE_INTERVAL,
    }
}

/// The deadlines a task is waiting on, in the order the C++ asks about them.
fn deadlines(profile: &TaskProfile, network: NetworkKind) -> Vec<(Timeout, u64)> {
    let mut out = vec![(
        Timeout::Task,
        profile.start_task_time.saturating_add(profile.task_timeout),
    )];

    if !profile.is_running() || profile.transfer_profile.start_send_time == 0 {
        return out;
    }

    let transfer = &profile.transfer_profile;
    let sent = transfer.start_send_time;
    if transfer.last_receive_pkg_time == 0 {
        out.push((
            Timeout::FirstPkg,
            sent.saturating_add(transfer.first_pkg_timeout),
        ));
    } else {
        out.push((
            Timeout::PkgPkg,
            transfer
                .last_receive_pkg_time
                .saturating_add(pkg_pkg_interval(network)),
        ));
    }
    out.push((
        Timeout::ReadWrite,
        sent.saturating_add(transfer.read_write_timeout),
    ));
    out
}

/// When the queue has to be looked at again for this task: the earliest of the
/// deadlines it is waiting on.
fn next_deadline(profile: &TaskProfile, network: NetworkKind) -> Option<u64> {
    deadlines(profile, network)
        .into_iter()
        .map(|(_, at)| at)
        .min()
}

/// Every one of the four timeouts that has run out at `now`, in the order the
/// C++ asks about them.
fn due(profile: &TaskProfile, now: u64, network: NetworkKind) -> Vec<Timeout> {
    deadlines(profile, network)
        .into_iter()
        .filter(|(_, at)| *at <= now)
        .map(|(timeout, _)| timeout)
        .collect()
}

/// `batchMap[_name] = (_code, _src_taskid)`: one entry per channel, and the last
/// task of a channel that timed out is the one the channel is failed for.
fn upsert(batch: &mut Vec<(String, i32, u32)>, name: String, entry: (i32, u32)) {
    match batch.iter_mut().find(|(channel, _, _)| *channel == name) {
        Some(found) => found.1 = entry.0,
        None => batch.push((name, entry.0, entry.1)),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::long_link::DisconnectInternalCode;
    use crate::net_source::LonglinkConfig;
    use crate::task::Task;
    use crate::task_profile::{
        ConnectProfile, ErrCmdType, RunId, TaskFailHandleType, HANDSHAKE_MISUNDERSTAND,
        LOCAL_ANTI_AVALANCHE, LOCAL_CHANNEL_ID, LOCAL_LONG_LINK_RELEASED,
        LOCAL_LONG_LINK_UNAVAILABLE, LOCAL_RESET, LOCAL_TASK_TIMEOUT, LONG_FIRST_PKG_TIMEOUT,
    };

    use super::{LongLinkTaskManager, Response, Timeout, RETRY_INTERNAL};
    use crate::RespHandle;

    /// The reading every test starts from.
    const NOW: u64 = 100 * 1000;
    /// The one channel every test's queue has.
    const CHANNEL: &str = "long.weixin.qq.com";

    /// What went out: the channel, the task, and how long the request was.
    type Sent = Arc<Mutex<Vec<(String, u32, usize)>>>;
    /// What the app was told about a task that is over.
    type Ended = Arc<Mutex<Vec<(ErrCmdType, i32, TaskFailHandleType, u32)>>>;
    /// What the app was told about a pair that failed: the channel it was on.
    type Notified = Arc<Mutex<Vec<(String, ErrCmdType, i32)>>>;
    /// Which channels were taken down, and why.
    type Down = Arc<Mutex<Vec<(String, DisconnectInternalCode)>>>;

    /// A task that goes out on [`CHANNEL`], with one try in it.
    fn task(taskid: u32) -> Task {
        let mut task = Task::new(taskid, 1);
        task.cgi = format!("/cgi-bin/{taskid}");
        task.channel_name = CHANNEL.to_string();
        task.longlink_host_list = vec![CHANNEL.to_string()];
        task.retry_count = 1;
        task
    }

    /// A queue with [`CHANNEL`] in it.
    fn manager() -> LongLinkTaskManager {
        let mut manager = LongLinkTaskManager::new();
        manager.add_long_link(LonglinkConfig::new(CHANNEL));
        manager
    }

    /// The queue with the three hooks every test reads: what went out, what the
    /// app was told, and which channels went down.
    fn wire(manager: &mut LongLinkTaskManager) -> (Sent, Ended, Notified, Down) {
        let sent: Sent = Arc::new(Mutex::new(Vec::new()));
        let ended: Ended = Arc::new(Mutex::new(Vec::new()));
        let notified: Notified = Arc::new(Mutex::new(Vec::new()));
        let down: Down = Arc::new(Mutex::new(Vec::new()));

        let record = Arc::clone(&sent);
        let mut next: u64 = 0;
        manager.set_send(move |name, task, body| {
            record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((name.to_string(), task.taskid, body.len()));
            next += 1;
            Some(RunId(next))
        });

        let record = Arc::clone(&ended);
        manager.set_callback(move |err_type, err_code, handle, task, _cost, _profile| {
            record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((err_type, err_code, handle, task.taskid));
            0
        });

        let record = Arc::clone(&notified);
        manager.set_notify_network_err(move |name, err_type, err_code, _ip, _port| {
            record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((name.to_string(), err_type, err_code));
        });

        let record = Arc::clone(&down);
        manager.set_disconnect(move |name, code| {
            record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((name.to_string(), code));
        });

        (sent, ended, notified, down)
    }

    /// An answer that came in on [`CHANNEL`] for `taskid`.
    fn answered(taskid: u32, body: &[u8]) -> Response {
        Response {
            name: CHANNEL.to_string(),
            err_type: ErrCmdType::Ok,
            err_code: 0,
            cmdid: 1,
            taskid,
            body: body.to_vec(),
            profile: ConnectProfile::new(),
        }
    }

    /// An answer that did not come: what a channel hands `__OnResponse` when the
    /// read failed.
    fn failed(taskid: u32, err_type: ErrCmdType, err_code: i32) -> Response {
        Response {
            err_type,
            err_code,
            ..answered(taskid, b"")
        }
    }

    /// How long the one task in the queue may take, in all.
    fn task_timeout(manager: &LongLinkTaskManager) -> u64 {
        manager.tasks()[0].task_timeout
    }

    /// How long the one task in the queue may wait for its first package.
    fn first_pkg_timeout(manager: &LongLinkTaskManager) -> u64 {
        manager.tasks()[0].transfer_profile.first_pkg_timeout
    }

    #[test]
    fn a_task_that_was_asked_for_goes_out_at_once() {
        let mut manager = manager();
        let (sent, _, _, _) = wire(&mut manager);

        assert!(manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG));
        assert_eq!(
            *sent.lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(CHANNEL.to_string(), 7, 0)]
        );
        assert!(manager.tasks()[0].is_running());
        assert_eq!(manager.task_count(CHANNEL), 1);
        assert_eq!(manager.tasks()[0].link_type, Task::CHANNEL_LONG);
    }

    #[test]
    fn a_task_whose_channel_is_not_there_waits() {
        let mut manager = manager();
        let (sent, ended, _, _) = wire(&mut manager);

        let mut task = task(7);
        task.channel_name = "long.other.qq.com".to_string();
        manager.start_task_at(NOW, task, Task::CHANNEL_LONG);

        assert!(
            sent.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "nowhere to go out on"
        );
        assert!(!manager.tasks()[0].is_running());
        assert!(
            ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "and it is not failed"
        );
    }

    #[test]
    fn a_minor_channel_is_named_after_the_first_host_it_came_with() {
        let mut manager = manager();
        manager.add_long_link(LonglinkConfig::new("minor.weixin.qq.com"));
        let (sent, _, _, _) = wire(&mut manager);

        let mut task = task(7);
        task.minorlong_host_list = vec!["minor.weixin.qq.com".to_string()];
        manager.start_task_at(NOW, task, Task::CHANNEL_MINOR_LONG);

        assert_eq!(manager.tasks()[0].channel_name, "minor.weixin.qq.com");
        assert_eq!(manager.tasks()[0].link_type, Task::CHANNEL_MINOR_LONG);
        assert_eq!(
            sent.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1
        );
    }

    #[test]
    fn a_task_the_app_stopped_is_one_the_channel_is_told_about() {
        let mut manager = manager();
        let (_, _, _, _) = wire(&mut manager);
        let stopped = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&stopped);
        manager.set_stop(move |name, taskid| {
            record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((name.to_string(), taskid));
        });

        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);
        assert!(manager.stop_task(7));
        assert_eq!(
            *stopped
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(CHANNEL.to_string(), 7)]
        );
        assert!(!manager.has_task(7));
        assert!(!manager.stop_task(7));
    }

    #[test]
    fn a_task_whose_channel_is_gone_is_one_the_queue_cannot_stop() {
        let mut manager = manager();
        let _ = wire(&mut manager);
        let mut task = task(7);
        task.channel_name = "long.other.qq.com".to_string();
        manager.start_task_at(NOW, task, Task::CHANNEL_LONG);

        assert!(!manager.stop_task(7), "no channel to tell");
        assert!(manager.has_task(7), "and the task is still there");
    }

    #[test]
    fn clear_tasks_takes_every_channel_down_and_tells_the_app_nothing() {
        let mut manager = manager();
        let (_, ended, _, down) = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        manager.clear_tasks();
        assert!(manager.is_empty());
        assert_eq!(
            *down.lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(CHANNEL.to_string(), DisconnectInternalCode::Reset)]
        );
        assert!(ended
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
    }

    #[test]
    fn a_task_that_answered_is_over() {
        let mut manager = manager();
        let (_, ended, notified, _) = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert_eq!(
            manager.on_response_at(NOW + 100, answered(7, b"hello")),
            Some(RespHandle::Ended)
        );
        assert!(!manager.has_task(7));
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(ErrCmdType::Ok, 0, TaskFailHandleType::Normal, 7)]
        );
        assert_eq!(
            *notified
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(CHANNEL.to_string(), ErrCmdType::Ok, 0)]
        );
    }

    #[test]
    fn a_task_that_is_only_sent_is_over_the_moment_it_went_out() {
        let mut manager = manager();
        let (sent, ended, _, _) = wire(&mut manager);

        let mut task = task(7);
        task.send_only = true;
        manager.start_task_at(NOW, task, Task::CHANNEL_LONG);

        assert_eq!(
            sent.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1,
            "it did go out"
        );
        assert!(!manager.has_task(7));
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(ErrCmdType::Ok, 0, TaskFailHandleType::Normal, 7)]
        );
    }

    #[test]
    fn a_task_the_app_said_to_end_is_over() {
        let mut manager = manager();
        let (_, ended, _, _) = wire(&mut manager);
        manager.set_buf2resp(|_task, _body| (0, TaskFailHandleType::TaskEnd));
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        manager.on_response_at(NOW + 100, answered(7, b"hello"));
        assert!(!manager.has_task(7));
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(ErrCmdType::EnDecode, 0, TaskFailHandleType::TaskEnd, 7)]
        );
    }

    #[test]
    fn a_session_timeout_is_what_the_app_is_asked_about() {
        let mut manager = manager();
        let _ = wire(&mut manager);
        let asked = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&asked);
        manager.set_notify_retry_all_tasks(move |err_type, err_code, handle, taskid, user_id| {
            record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((err_type, err_code, handle, taskid, user_id.to_string()));
        });
        manager.set_buf2resp(|_task, _body| (-13, TaskFailHandleType::SessionTimeout));
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert_eq!(
            manager.on_response_at(NOW + 100, answered(7, b"hello")),
            Some(RespHandle::Deferred)
        );
        assert!(manager.has_task(7), "the app has not answered yet");
        assert_eq!(
            *asked
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(
                ErrCmdType::EnDecode,
                -13,
                TaskFailHandleType::SessionTimeout,
                7,
                String::new()
            )]
        );
    }

    #[test]
    fn an_answer_the_app_could_not_read_fails_every_task_of_the_channel() {
        let mut manager = manager();
        let (_, ended, notified, down) = wire(&mut manager);
        // `-1` is what the app read out of the body; only the task the answer
        // was about is failed with it
        manager.set_buf2resp(|_task, _body| (-1, TaskFailHandleType::Default));
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);
        manager.start_task_at(NOW, task(8), Task::CHANNEL_LONG);

        assert_eq!(
            manager.on_response_at(NOW + 100, answered(7, b"hello")),
            Some(RespHandle::Retried)
        );
        assert_eq!(manager.len(), 2, "both have another try coming");
        assert!(ended
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
        assert_eq!(
            *notified
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(CHANNEL.to_string(), ErrCmdType::EnDecode, -1)],
            "the app is told what the *handle* was"
        );
        assert_eq!(
            *down.lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(CHANNEL.to_string(), DisconnectInternalCode::DecodeErr)]
        );
        assert_eq!(manager.retry_interval(), RETRY_INTERNAL);
    }

    #[test]
    fn a_slient_task_end_leaves_without_the_app_being_told() {
        let mut manager = manager();
        let (_, ended, _, _) = wire(&mut manager);
        manager.set_buf2resp(|_task, _body| (0, TaskFailHandleType::SlientTaskEnd));
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert_eq!(
            manager.on_response_at(NOW + 100, answered(7, b"hello")),
            Some(RespHandle::Ended)
        );
        assert!(!manager.has_task(7));
        assert!(ended
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
    }

    #[test]
    fn the_server_pushing_is_not_a_task() {
        let mut manager = manager();
        let _ = wire(&mut manager);
        let pushed = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&pushed);
        manager.set_on_push(move |name, cmdid, taskid, body| {
            record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((name.to_string(), cmdid, taskid, body.to_vec()));
        });
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert_eq!(manager.on_response_at(NOW + 10, answered(0, b"push")), None);
        assert_eq!(
            *pushed
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(CHANNEL.to_string(), 1, 0, b"push".to_vec())]
        );
        assert!(manager.has_task(7), "and no task was touched");
    }

    #[test]
    fn an_answer_for_a_channel_the_queue_has_no_link_for_is_nothing() {
        let mut manager = manager();
        let _ = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        let mut response = answered(7, b"hello");
        response.name = "long.other.qq.com".to_string();
        assert_eq!(manager.on_response_at(NOW + 10, response), None);
        assert!(manager.has_task(7));
    }

    #[test]
    fn an_answer_for_a_task_the_queue_forgot_is_nothing() {
        let mut manager = manager();
        let _ = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert_eq!(
            manager.on_response_at(NOW + 10, answered(99, b"hello")),
            None
        );
    }

    #[test]
    fn an_error_that_came_in_is_one_the_channel_is_failed_with() {
        let mut manager = manager();
        let (_, ended, _, down) = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert_eq!(
            manager.on_response_at(NOW + 10, failed(7, ErrCmdType::Socket, -5001)),
            Some(RespHandle::Retried)
        );
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![],
            "the task is not over yet"
        );
        assert_eq!(manager.retry_interval(), RETRY_INTERNAL);
        assert!(
            down.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "a socket error is the link's own, not one to take it down for"
        );
    }

    #[test]
    fn an_error_for_no_task_in_particular_fails_the_channel_anyway() {
        let mut manager = manager();
        let (_, ended, _, _) = wire(&mut manager);
        let mut no_retry = task(7);
        no_retry.retry_count = 0;
        manager.start_task_at(NOW, no_retry, Task::CHANNEL_LONG);

        // `kInvalidTaskID`: the read failed for the link and not for one task,
        // and `__Locate` has no answer for that id. The C++ fails the channel
        // all the same.
        assert_eq!(
            manager.on_response_at(
                NOW + 10,
                failed(Task::INVALID_TASK_ID, ErrCmdType::Socket, -5001)
            ),
            Some(RespHandle::Ended)
        );
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(ErrCmdType::Socket, -5001, TaskFailHandleType::Default, 7)],
            "the app hears about it and the task leaves the queue"
        );
    }

    #[test]
    fn a_handshake_the_two_ends_did_not_agree_on_is_a_try_the_task_gets_back() {
        let mut manager = manager();
        let _ = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        manager.on_response_at(
            NOW + 10,
            failed(7, ErrCmdType::EnDecode, HANDSHAKE_MISUNDERSTAND),
        );
        assert_eq!(
            manager.tasks()[0].remain_retry_count,
            1,
            "the try the batch took is given back"
        );
    }

    #[test]
    fn a_task_that_heard_nothing_runs_into_the_first_pkg_timeout() {
        let mut manager = manager();
        let (_, _, notified, down) = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);
        let first_pkg = first_pkg_timeout(&manager);
        manager.on_send_at(NOW, 7);

        manager.run_loop_at(NOW + first_pkg);
        assert_eq!(
            *notified
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(
                CHANNEL.to_string(),
                ErrCmdType::NetMsgXp,
                LONG_FIRST_PKG_TIMEOUT
            )]
        );
        assert_eq!(
            *down.lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![
                (CHANNEL.to_string(), DisconnectInternalCode::DecodeErr),
                (CHANNEL.to_string(), DisconnectInternalCode::TaskTimeout)
            ],
            "the C++ takes it down twice: once for the handle, once for the timeout"
        );
        assert!(manager.has_task(7), "and it is tried again");
        assert_eq!(manager.retry_interval(), RETRY_INTERNAL);
    }

    #[test]
    fn a_task_that_ran_out_of_time_is_failed_on_its_own_and_takes_the_channel_down() {
        let mut manager = manager();
        let (_, ended, notified, down) = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);
        let timeout = task_timeout(&manager);

        manager.run_loop_at(NOW + timeout);
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(
                ErrCmdType::Local,
                LOCAL_TASK_TIMEOUT,
                TaskFailHandleType::TaskTimeout,
                7
            )]
        );
        assert!(!manager.has_task(7));
        assert_eq!(
            *down.lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![
                (CHANNEL.to_string(), DisconnectInternalCode::DecodeErr),
                (CHANNEL.to_string(), DisconnectInternalCode::TaskTimeout)
            ],
            "the C++ takes it down twice: once for the handle, once for the timeout"
        );
        assert!(
            notified
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "a task that ran out of its own time is not a channel error"
        );
    }

    #[test]
    fn a_task_that_never_went_out_also_runs_out_of_time() {
        let mut manager = manager();
        let (sent, ended, _, _) = wire(&mut manager);
        manager.set_make_sure_connected(|_name| false);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);
        assert!(sent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());

        manager.run_loop_at(NOW + task_timeout(&manager));
        assert_eq!(
            ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1
        );
    }

    #[test]
    fn a_run_that_was_out_when_the_network_changed_is_cancelled_and_tried_again() {
        let mut manager = manager();
        let (sent, ended, _, _) = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        manager.on_network_change_at(NOW + 10);
        assert_eq!(
            *sent.lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(CHANNEL.to_string(), 7, 0), (CHANNEL.to_string(), 7, 0)],
            "the run that was out is cancelled and the task goes out again"
        );
        assert!(
            ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "a cancelled task is not a task that is over"
        );
        assert_eq!(manager.retry_interval(), 0, "without waiting");
    }

    #[test]
    fn a_network_change_is_one_a_channel_can_say_no_to() {
        let mut manager = manager();
        let (sent, _, _, _) = wire(&mut manager);
        manager.set_network_change(|_name| false);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        manager.on_network_change_at(NOW + 10);
        assert_eq!(
            sent.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1,
            "nothing was cancelled"
        );
    }

    #[test]
    fn redoing_tasks_takes_every_channel_apart_first() {
        let mut manager = manager();
        let _ = wire(&mut manager);
        let reset = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&reset);
        manager.set_reset_channel(move |name| {
            record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(name.to_string())
        });
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        manager.redo_tasks_at(NOW + 10);
        assert_eq!(
            *reset
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![CHANNEL.to_string()]
        );
    }

    #[test]
    fn the_app_retrying_is_only_about_the_tasks_of_one_user() {
        let mut manager = manager();
        let (_, ended, _, _) = wire(&mut manager);
        manager.add_long_link(LonglinkConfig::new("long.bob.qq.com"));
        let mut alice = task(7);
        alice.user_id = "alice".to_string();
        let mut bob = task(8);
        bob.user_id = "bob".to_string();
        bob.channel_name = "long.bob.qq.com".to_string();
        manager.start_task_at(NOW, alice, Task::CHANNEL_LONG);
        manager.start_task_at(NOW, bob, Task::CHANNEL_LONG);

        manager.retry_tasks_at(
            NOW + 10,
            ErrCmdType::Local,
            LOCAL_TASK_TIMEOUT,
            TaskFailHandleType::TaskEnd,
            Task::INVALID_TASK_ID,
            "alice",
        );
        assert!(!manager.has_task(7));
        assert!(manager.has_task(8), "bob's task was not asked about");
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(
                ErrCmdType::Local,
                LOCAL_TASK_TIMEOUT,
                TaskFailHandleType::TaskEnd,
                7
            )]
        );
    }

    #[test]
    fn what_the_app_says_about_one_task_is_said_about_every_task_of_the_channel() {
        let mut manager = manager();
        let (_, ended, _, _) = wire(&mut manager);
        let mut alice = task(7);
        alice.user_id = "alice".to_string();
        let mut bob = task(8);
        bob.user_id = "bob".to_string();
        manager.start_task_at(NOW, alice, Task::CHANNEL_LONG);
        manager.start_task_at(NOW, bob, Task::CHANNEL_LONG);

        manager.retry_tasks_at(
            NOW + 10,
            ErrCmdType::Local,
            LOCAL_TASK_TIMEOUT,
            TaskFailHandleType::TaskEnd,
            7,
            "alice",
        );
        assert!(manager.is_empty(), "bob was out on the same link");
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![
                (
                    ErrCmdType::Local,
                    LOCAL_TASK_TIMEOUT,
                    TaskFailHandleType::TaskEnd,
                    7
                ),
                (ErrCmdType::Local, 0, TaskFailHandleType::TaskEnd, 8),
            ],
            "and only the task it was about is failed with the code"
        );
    }

    #[test]
    fn after_a_channel_was_failed_every_task_waits_out_the_queue() {
        let mut manager = manager();
        let (sent, _, _, _) = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);
        manager.start_task_at(NOW, task(8), Task::CHANNEL_LONG);
        assert_eq!(
            sent.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            2
        );

        manager.on_response_at(NOW + 10, failed(7, ErrCmdType::EnDecode, -1));
        assert_eq!(manager.retry_interval(), RETRY_INTERNAL);

        manager.run_loop_at(NOW + 500);
        assert_eq!(
            sent.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            2,
            "nothing went out yet"
        );
        manager.run_loop_at(NOW + 10 + RETRY_INTERNAL);
        assert_eq!(
            sent.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            4,
            "and now both went out again"
        );
    }

    #[test]
    fn a_task_that_is_not_authed_waits() {
        let mut manager = manager();
        let (sent, ended, _, _) = wire(&mut manager);
        manager.set_make_sure_authed(|_host, _user_id| false);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert!(sent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
        assert!(ended
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
        assert!(manager.has_task(7));
    }

    #[test]
    fn a_task_the_app_could_not_encode_is_over() {
        let mut manager = manager();
        let (sent, ended, _, _) = wire(&mut manager);
        manager.set_req2buf(|_task| Err(-1234));
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert!(sent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(ErrCmdType::EnDecode, -1234, TaskFailHandleType::TaskEnd, 7)]
        );
        assert_eq!(manager.tasks_continuous_fail_count(), 1);
    }

    #[test]
    fn a_task_the_queue_is_asked_to_hold_back_is_over() {
        let mut manager = manager();
        let (sent, ended, _, _) = wire(&mut manager);
        manager.set_anti_avalanche_check(|_task, _body| false);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert!(sent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(
                ErrCmdType::Local,
                LOCAL_ANTI_AVALANCHE,
                TaskFailHandleType::TaskEnd,
                7
            )]
        );
    }

    #[test]
    fn a_task_pinned_to_a_link_that_was_made_again_is_over() {
        let mut manager = manager();
        let (sent, ended, _, _) = wire(&mut manager);
        manager.set_channel_profile(|_name| {
            let mut profile = ConnectProfile::new();
            profile.start_time = 4242;
            profile
        });

        let mut task = task(7);
        task.channel_id = 7;
        manager.start_task_at(NOW, task, Task::CHANNEL_LONG);

        assert!(
            sent.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "it is not put on another one"
        );
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(
                ErrCmdType::Local,
                LOCAL_CHANNEL_ID,
                TaskFailHandleType::TaskEnd,
                7
            )]
        );
    }

    #[test]
    fn a_task_pinned_to_a_link_that_is_not_up_is_over() {
        let mut manager = manager();
        let (sent, ended, _, _) = wire(&mut manager);
        manager.set_make_sure_connected(|_name| false);

        let mut task = task(7);
        task.channel_id = 7;
        manager.start_task_at(NOW, task, Task::CHANNEL_LONG);

        assert!(sent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(
                ErrCmdType::Local,
                LOCAL_CHANNEL_ID,
                TaskFailHandleType::TaskEnd,
                7
            )]
        );
    }

    #[test]
    fn a_link_that_is_not_up_is_one_a_task_with_no_pinning_waits_for() {
        let mut manager = manager();
        let (sent, ended, _, _) = wire(&mut manager);
        let up = Arc::new(Mutex::new(false));
        manager.set_make_sure_connected(move |_name| {
            *up.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
        });
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert!(sent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
        assert!(
            ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "and it is not failed"
        );
        assert!(manager.has_task(7));
    }

    #[test]
    fn the_sequence_id_a_task_is_reported_under_is_made_once() {
        let mut manager = manager();
        let (sent, _, _, _) = wire(&mut manager);
        let up = Arc::new(Mutex::new(false));
        let link_up = Arc::clone(&up);
        manager.set_make_sure_connected(move |_name| {
            *link_up
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        });
        let made = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&made);
        manager.set_gen_sequence_id(move || {
            record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(());
            1
        });

        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);
        assert_eq!(
            made.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1
        );
        *up.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        manager.run_loop_at(NOW + 10);

        assert_eq!(
            sent.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1,
            "it went out on the second try"
        );
        assert_eq!(
            made.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1,
            "and it kept its sequence id"
        );
        assert_eq!(manager.tasks()[0].task.client_sequence_id, 1);
    }

    #[test]
    fn an_answer_the_app_said_to_keep_is_answered_again_without_going_out() {
        let mut manager = manager();
        let (sent, ended, _, _) = wire(&mut manager);
        manager.set_should_intercept(|_err_code| true);
        manager.set_buf2resp(|_task, _body| (0, TaskFailHandleType::Normal));

        let mut first = task(7);
        first.cgi = "/cgi-bin/kept".to_string();
        manager.start_task_at(NOW, first, Task::CHANNEL_LONG);
        manager.on_response_at(NOW + 10, answered(7, b"hello"));
        assert_eq!(
            sent.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1
        );

        let mut second = task(8);
        second.cgi = "/cgi-bin/kept".to_string();
        manager.start_task_at(NOW + 20, second, Task::CHANNEL_LONG);

        assert_eq!(
            sent.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1,
            "the second one never went out"
        );
        assert_eq!(
            ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1,
            "and it is not over yet: the app said Normal, so the try is spent"
        );
        assert_eq!(
            manager.tasks()[0].remain_retry_count,
            0,
            "the try it was answered with is gone"
        );

        // the answer is still good, so the next look is the one that ends it
        manager.run_loop_at(NOW + 30);
        assert_eq!(
            sent.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1,
            "and still nothing went out"
        );
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![
                (ErrCmdType::Ok, 0, TaskFailHandleType::Normal, 7),
                (ErrCmdType::EnDecode, 0, TaskFailHandleType::Normal, 8),
            ]
        );
    }

    #[test]
    fn a_channel_that_was_released_fails_the_tasks_it_had() {
        let mut manager = manager();
        let (_, ended, _, _) = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert!(manager.remove_long_link_at(NOW + 10, CHANNEL));
        assert!(manager.channels().is_empty());
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(
                ErrCmdType::Local,
                LOCAL_LONG_LINK_RELEASED,
                TaskFailHandleType::TaskEnd,
                7
            )]
        );
        assert!(!manager.remove_long_link(CHANNEL));
    }

    #[test]
    fn the_default_channel_is_the_one_the_config_calls_main() {
        let mut manager = LongLinkTaskManager::new();
        assert_eq!(manager.default_channel(), None);

        let mut config = LonglinkConfig::new(CHANNEL);
        config.is_main = true;
        assert!(manager.add_long_link(config));
        assert_eq!(manager.default_channel(), Some(CHANNEL));
        assert!(
            !manager.add_long_link(LonglinkConfig::new(CHANNEL)),
            "a second channel of one name is not added"
        );
        assert_eq!(manager.channels().len(), 1);
    }

    #[test]
    fn a_channel_of_a_host_the_app_forbade_tls_for_does_not_get_it() {
        let mut manager = LongLinkTaskManager::new();
        manager.add_forbid_tls_host(&[
            "long.weixin.qq.com".to_string(),
            "short.weixin.qq.com".to_string(),
        ]);
        assert!(manager.forbid_tls(&["long.weixin.qq.com".to_string()]));
        assert!(
            !manager.forbid_tls(&["short.weixin.qq.com".to_string()]),
            "a host without \"long\" in it is not one of them"
        );

        let mut config = LonglinkConfig::new("long.other.qq.com");
        config.host_list = vec!["long.weixin.qq.com".to_string()];
        manager.add_long_link(config);
        assert!(!manager.channels()[0].need_tls);
    }

    #[test]
    fn the_queue_fails_what_is_left_when_it_goes_away() {
        let mut manager = manager();
        let (_, ended, _, _) = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        drop(manager);
        assert_eq!(
            *ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(
                ErrCmdType::Local,
                LOCAL_RESET,
                TaskFailHandleType::TaskEnd,
                7
            )]
        );
    }

    #[test]
    fn a_task_that_is_out_is_not_sent_twice() {
        let mut manager = manager();
        let (sent, _, _, _) = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        manager.run_loop_at(NOW + 10);
        manager.run_loop_at(NOW + 20);
        assert_eq!(
            sent.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1
        );
    }

    #[test]
    fn the_queue_says_when_it_has_to_be_looked_at_again() {
        let mut manager = manager();
        let _ = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert_eq!(manager.due_time(), Some(NOW + task_timeout(&manager)));
        manager.on_send_at(NOW, 7);
        assert_eq!(manager.due_time(), Some(NOW + first_pkg_timeout(&manager)));

        manager.on_recv_at(NOW + 100, 7, 10, 10);
        assert_eq!(
            manager.due_time(),
            Some(NOW + 100 + crate::config::WIFI_PACKAGE_INTERVAL),
            "a package came in, so the pkg-pkg wait is what is left"
        );
    }

    #[test]
    fn the_queue_reads_a_task_s_connect_off_the_channel_it_is_on() {
        let mut manager = manager();
        let (_, _, _, down) = wire(&mut manager);
        manager.set_channel_profile(|name| {
            let mut profile = ConnectProfile::new();
            profile.host = name.to_string();
            profile
        });
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert_eq!(manager.connect_profile(7).host, CHANNEL);
        assert_eq!(manager.connect_profile(99), ConnectProfile::new());

        assert!(manager.disconnect_by_taskid(7, DisconnectInternalCode::Reset));
        assert_eq!(
            *down.lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(CHANNEL.to_string(), DisconnectInternalCode::Reset)]
        );
        assert!(!manager.disconnect_by_taskid(99, DisconnectInternalCode::Reset));
    }

    #[test]
    fn a_task_without_a_channel_is_failed_with_a_profile_that_says_there_is_none() {
        let mut manager = manager();
        let (_, ended, _, _) = wire(&mut manager);
        let mut task = task(7);
        task.channel_name = "long.other.qq.com".to_string();
        manager.start_task_at(NOW, task, Task::CHANNEL_LONG);

        // the C++'s "longlink nullptr": what the queue says about a channel it
        // has no link for
        let profile = manager.connect_profile(7);
        assert_eq!(profile.disconn_errtype, ErrCmdType::Local);
        assert_eq!(profile.conn_errcode, LOCAL_LONG_LINK_UNAVAILABLE);
        assert_eq!(profile.link_type, Task::CHANNEL_LONG);

        assert!(
            ended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "and a task that is going nowhere is not failed for it"
        );
    }

    #[test]
    fn the_timeout_a_task_ran_into_is_what_the_app_is_told() {
        assert_eq!(Timeout::Task.err_type(), ErrCmdType::Local);
        assert_eq!(Timeout::Task.err_code(), LOCAL_TASK_TIMEOUT);
        assert_eq!(Timeout::FirstPkg.err_type(), ErrCmdType::NetMsgXp);
        assert_eq!(Timeout::FirstPkg.err_code(), LONG_FIRST_PKG_TIMEOUT);
        assert_eq!(
            Timeout::PkgPkg.err_code(),
            crate::task_profile::LONG_PKG_PKG_TIMEOUT
        );
        assert_eq!(
            Timeout::ReadWrite.err_code(),
            crate::task_profile::LONG_READ_WRITE_TIMEOUT
        );
    }

    #[test]
    fn the_queue_reads_the_clock_itself_when_the_host_does_not_hand_one_in() {
        let mut manager = manager();
        let _ = wire(&mut manager);
        manager.set_encoder(crate::longlink::LongLinkEncoder::new());

        assert!(manager.start_task(task(7), Task::CHANNEL_LONG));
        assert!(manager.has_task(7));
        assert!(manager.on_send(7));
        assert!(manager.on_recv(7, 10, 10));
        manager.run_loop();
        manager.touch_tasks();
        manager.redo_tasks_of(CHANNEL);

        assert!(manager.stop_task(7), "the run the redo made is stopped");
        assert!(!manager.has_task(7));
        assert!(LongLinkTaskManager::default().is_empty());
    }

    #[test]
    fn the_queue_is_the_tasks_and_the_channels_it_has() {
        let mut manager = manager();
        let _ = wire(&mut manager);
        manager.start_task_at(NOW, task(7), Task::CHANNEL_LONG);

        assert_eq!(manager.len(), 1);
        assert!(!manager.is_empty());
        assert_eq!(manager.channels().len(), 1);
        assert_eq!(manager.task_count("long.other.qq.com"), 0);
        assert!(format!("{manager:?}").contains("LongLinkTaskManager"));
    }
}
