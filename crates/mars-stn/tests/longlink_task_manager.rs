//! `mars/stn/src/longlink_task_manager.cc`, through the public api.
//!
//! The samples are what the C++ does to the same calls: a task that goes out on
//! the long link and answers, one that answered nothing and runs into the first
//! package timeout, one that was pinned to a link that has been made again
//! since, one whose link is down, two tasks on one link that are failed by the
//! single answer one of them could not read, a session timeout the app is asked
//! about, a push that is not a task, a network change that cancels what is out,
//! and what the queue leaves behind when it goes away.
//!
//! The link's own side — the connect, the write, the read — is what a
//! `NetCore` would wire, so the tests put a channel on the queue the way a host
//! does: a name, a profile, whether it is up, and the four things the queue
//! does to it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mars_stn::dynamic_timeout::DynamicTimeoutStatus;
use mars_stn::dynamic_timeout::NetworkKind;
use mars_stn::task_profile::{
    TaskFailHandleType, LOCAL_CANCEL, LOCAL_CHANNEL_ID, LOCAL_LONG_LINK_RELEASED, LOCAL_RESET,
    LOCAL_TASK_TIMEOUT, LONG_FIRST_PKG_TIMEOUT,
};
use mars_stn::{
    ConnectProfile, DisconnectInternalCode, ErrCmdType, LongLinkTaskManager, LonglinkConfig,
    RespHandle, RunId, Task, RETRY_INTERNAL,
};

/// The channel every sample starts with.
const MAIN: &str = "long.weixin.qq.com";
/// The reading every task in these samples is started at.
const START: u64 = 100 * 1000;
/// `kBaseFirstPackageWifiTimeout` — what the first package of a small request
/// on wi-fi is waited for.
const FIRST_PKG: u64 = 12 * 1000;
/// `(15s + 5s) * (1 + 1)` — what a task with one try in it is given, from
/// [`mars_stn::compute_task_timeout`].
const ONE_TRY: u64 = 40 * 1000;

/// What the queue asked of a channel: the task, and the request it went out as.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sent {
    channel: String,
    taskid: u32,
    body: Vec<u8>,
}

/// What the app was told about a task that is over.
type Ended = Vec<(ErrCmdType, i32, TaskFailHandleType, u32)>;
/// What the app was asked about an answer it may want tried again.
type Asked = Vec<(ErrCmdType, i32, TaskFailHandleType, u32, String)>;
/// What the app was told about a pair that failed.
type Notified = Vec<(String, ErrCmdType, i32)>;
/// What the server pushed, and on which channel.
type Pushed = Vec<(String, u32, u32, Vec<u8>)>;

/// What a long link is in these samples: whether it is up, and the connect it
/// was made on — a link that is made again is not the one a task that asked
/// for the old connect was pinned to.
#[derive(Debug, Clone)]
struct Link {
    up: bool,
    start_time: u64,
}

/// The queue, with the channel and the app wired to it the way a `NetCore`
/// wires them.
struct App {
    manager: LongLinkTaskManager,
    links: Arc<Mutex<HashMap<String, Link>>>,
    sent: Arc<Mutex<Vec<Sent>>>,
    stopped: Arc<Mutex<Vec<(String, u32)>>>,
    down: Arc<Mutex<Vec<(String, DisconnectInternalCode)>>>,
    reset: Arc<Mutex<Vec<String>>>,
    ended: Arc<Mutex<Ended>>,
    asked: Arc<Mutex<Asked>>,
    notified: Arc<Mutex<Notified>>,
    pushed: Arc<Mutex<Pushed>>,
    answer: Arc<Mutex<(i32, TaskFailHandleType)>>,
}

impl App {
    /// A queue with [`MAIN`] up on it, an encoder that writes the cgi, a
    /// decoder that reads an answer as a good one, and a run that is named
    /// after the task it is the run of.
    fn new() -> Self {
        let mut manager = LongLinkTaskManager::new();

        let links: Arc<Mutex<HashMap<String, Link>>> = Arc::new(Mutex::new(HashMap::new()));
        let mut config = LonglinkConfig::new(MAIN);
        config.is_main = true;
        assert!(manager.add_long_link(config));
        links.lock().unwrap().insert(
            MAIN.to_string(),
            Link {
                up: true,
                start_time: START - 1,
            },
        );

        let sent: Arc<Mutex<Vec<Sent>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = sent.clone();
        manager.set_send(move |name, task, body| {
            recorder.lock().unwrap().push(Sent {
                channel: name.to_string(),
                taskid: task.taskid,
                body: body.to_vec(),
            });
            Some(RunId(u64::from(task.taskid)))
        });

        let stopped = Arc::new(Mutex::new(Vec::new()));
        let recorder = stopped.clone();
        manager.set_stop(move |name, taskid| {
            recorder.lock().unwrap().push((name.to_string(), taskid));
        });

        let down = Arc::new(Mutex::new(Vec::new()));
        let recorder = down.clone();
        manager.set_disconnect(move |name, code| {
            recorder.lock().unwrap().push((name.to_string(), code));
        });

        let reset = Arc::new(Mutex::new(Vec::new()));
        let recorder = reset.clone();
        manager.set_reset_channel(move |name| recorder.lock().unwrap().push(name.to_string()));

        let ended: Arc<Mutex<Ended>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = ended.clone();
        manager.set_callback(move |err_type, err_code, handle, task, _cost, _profile| {
            recorder
                .lock()
                .unwrap()
                .push((err_type, err_code, handle, task.taskid));
            0
        });

        let asked: Arc<Mutex<Asked>> = Arc::new(Mutex::new(Vec::new()));
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

        let notified: Arc<Mutex<Notified>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = notified.clone();
        manager.set_notify_network_err(move |name, err_type, err_code, _ip, _port| {
            recorder
                .lock()
                .unwrap()
                .push((name.to_string(), err_type, err_code));
        });

        let pushed: Arc<Mutex<Pushed>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = pushed.clone();
        manager.set_on_push(move |name, cmdid, taskid, body| {
            recorder
                .lock()
                .unwrap()
                .push((name.to_string(), cmdid, taskid, body.to_vec()));
        });

        let channel_links = links.clone();
        manager.set_channel_profile(move |name| {
            let mut profile = ConnectProfile::new();
            if let Some(link) = channel_links.lock().unwrap().get(name) {
                profile.start_time = link.start_time;
            }
            profile.host = name.to_string();
            profile.ip = "1.1.1.1".to_string();
            profile.port = 8080;
            profile.link_type = Task::CHANNEL_LONG;
            profile
        });

        let up_links = links.clone();
        manager.set_make_sure_connected(move |name| {
            up_links
                .lock()
                .unwrap()
                .get(name)
                .is_some_and(|link| link.up)
        });

        let sequence = Arc::new(Mutex::new(0u16));
        manager.set_gen_sequence_id(move || {
            let mut sequence = sequence.lock().unwrap();
            *sequence += 1;
            *sequence
        });

        let answer = Arc::new(Mutex::new((0, TaskFailHandleType::Normal)));
        let decoder = answer.clone();
        manager.set_buf2resp(move |_task, _body| *decoder.lock().unwrap());
        manager.set_req2buf(|task| Ok(task.cgi.clone().into_bytes()));
        manager.set_make_sure_authed(|_host, _user_id| true);
        manager.set_anti_avalanche_check(|_task, _body| true);
        manager.set_net_info(|| NetworkKind::Wifi);

        Self {
            manager,
            links,
            sent,
            stopped,
            down,
            reset,
            ended,
            asked,
            notified,
            pushed,
            answer,
        }
    }

    /// Another channel, up from the word go.
    fn add_channel(&mut self, name: &str, up: bool) {
        assert!(self.manager.add_long_link(LonglinkConfig::new(name)));
        self.links.lock().unwrap().insert(
            name.to_string(),
            Link {
                up,
                start_time: START - 1,
            },
        );
    }

    /// A long-link task on [`MAIN`], with one try left in it after the first.
    fn task(&self, taskid: u32) -> Task {
        let mut task = Task::new(taskid, 1);
        task.cgi = format!("/cgi-bin/{taskid}");
        task.channel_name = MAIN.to_string();
        task.longlink_host_list = vec![MAIN.to_string()];
        task.need_authed = false;
        task.retry_count = 1;
        task
    }

    /// `StartTask` — and the loop that goes with it, which is what sends it.
    fn start(&mut self, taskid: u32) {
        let task = self.task(taskid);
        self.start_task(task);
    }

    /// The same, for a task the test changed.
    fn start_task(&mut self, task: Task) {
        assert!(self.manager.start_task_at(START, task, Task::CHANNEL_LONG));
    }

    /// How the app shall read the next answer.
    fn answer_with(&mut self, err_code: i32, handle: TaskFailHandleType) {
        *self.answer.lock().unwrap() = (err_code, handle);
    }

    /// The link went away: a task that is not pinned to a connect waits for it.
    fn take_down(&mut self, name: &str) {
        self.links.lock().unwrap().get_mut(name).unwrap().up = false;
    }

    fn bring_up(&mut self, name: &str) {
        self.links.lock().unwrap().get_mut(name).unwrap().up = true;
    }

    /// The link was made again: it is up, but it is not the connect it was.
    fn remake(&mut self, name: &str, at: u64) {
        let mut link = self.links.lock().unwrap();
        let link = link.get_mut(name).unwrap();
        link.up = true;
        link.start_time = at;
    }

    fn sent(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }

    fn stopped(&self) -> Vec<(String, u32)> {
        self.stopped.lock().unwrap().clone()
    }

    fn down(&self) -> Vec<(String, DisconnectInternalCode)> {
        self.down.lock().unwrap().clone()
    }

    fn reset(&self) -> Vec<String> {
        self.reset.lock().unwrap().clone()
    }

    fn ended(&self) -> Ended {
        self.ended.lock().unwrap().clone()
    }

    fn asked(&self) -> Asked {
        self.asked.lock().unwrap().clone()
    }

    fn notified(&self) -> Notified {
        self.notified.lock().unwrap().clone()
    }

    fn pushed(&self) -> Pushed {
        self.pushed.lock().unwrap().clone()
    }
}

/// `__OnResponse` of a read that came back: a package for `taskid`.
fn answered(taskid: u32, body: &[u8]) -> mars_stn::Response {
    mars_stn::Response {
        name: MAIN.to_string(),
        err_type: ErrCmdType::Ok,
        err_code: 0,
        cmdid: 1,
        taskid,
        body: body.to_vec(),
        profile: ConnectProfile::new(),
    }
}

/// The same, for the answer the server pushed.
fn pushed(body: &[u8]) -> mars_stn::Response {
    answered(Task::INVALID_TASK_ID, body)
}

/// `__OnResponse` of a read that failed.
fn failed(taskid: u32, err_type: ErrCmdType, err_code: i32) -> mars_stn::Response {
    mars_stn::Response {
        err_type,
        err_code,
        ..answered(taskid, b"")
    }
}

#[test]
fn a_task_that_goes_out_and_answers_is_over() {
    let mut app = App::new();
    app.start(7);

    assert_eq!(
        app.sent(),
        vec![Sent {
            channel: MAIN.to_string(),
            taskid: 7,
            body: b"/cgi-bin/7".to_vec(),
        }]
    );
    assert_eq!(app.manager.connect_profile(7).host, MAIN);
    assert_eq!(app.manager.tasks()[0].task.client_sequence_id, 1);

    assert!(app.manager.on_send_at(START, 7));
    assert_eq!(
        app.manager
            .on_response_at(START + 500, answered(7, b"hello")),
        Some(RespHandle::Ended)
    );

    assert!(app.manager.is_empty(), "the task left the queue");
    assert_eq!(
        app.ended(),
        vec![(ErrCmdType::Ok, 0, TaskFailHandleType::Normal, 7)]
    );
    assert_eq!(app.manager.tasks_continuous_fail_count(), 0);
    assert_eq!(app.notified(), vec![(MAIN.to_string(), ErrCmdType::Ok, 0)]);
}

#[test]
fn a_task_that_answered_nothing_runs_into_the_first_package_timeout() {
    let mut app = App::new();
    app.start(7);
    assert!(app.manager.on_send_at(START, 7));

    assert_eq!(app.manager.due_time(), Some(START + FIRST_PKG));
    app.manager.run_loop_at(START + FIRST_PKG);

    assert!(
        app.ended().is_empty(),
        "a task that still has a try is not over"
    );
    assert_eq!(app.manager.len(), 1);
    assert_eq!(app.manager.tasks()[0].remain_retry_count, 0);
    assert_eq!(
        app.notified(),
        vec![(
            MAIN.to_string(),
            ErrCmdType::NetMsgXp,
            LONG_FIRST_PKG_TIMEOUT
        )]
    );
    assert_eq!(
        app.down(),
        vec![
            (MAIN.to_string(), DisconnectInternalCode::DecodeErr),
            (MAIN.to_string(), DisconnectInternalCode::TaskTimeout),
        ],
        "the C++ takes the link down twice: once for the handle, once for the timeout"
    );
    assert_eq!(
        app.manager.due_time(),
        Some(START + FIRST_PKG + RETRY_INTERNAL),
        "and the try that is left waits out DEF_TASK_RETRY_INTERNAL"
    );

    app.manager.run_loop_at(START + FIRST_PKG + RETRY_INTERNAL);
    assert_eq!(app.sent().len(), 2, "and then the task goes out again");
    assert!(app.manager.tasks()[0].is_running());
}

#[test]
fn a_task_that_ran_out_of_its_own_time_is_failed_with_it() {
    let mut app = App::new();
    app.start(7);

    app.manager.run_loop_at(START + ONE_TRY);
    assert!(app.manager.is_empty());
    assert_eq!(
        app.ended(),
        vec![(
            ErrCmdType::Local,
            LOCAL_TASK_TIMEOUT,
            TaskFailHandleType::TaskTimeout,
            7
        )]
    );
    assert!(
        app.notified().is_empty(),
        "its own timeout is not a channel error"
    );
    assert_eq!(
        app.down(),
        vec![
            (MAIN.to_string(), DisconnectInternalCode::DecodeErr),
            (MAIN.to_string(), DisconnectInternalCode::TaskTimeout),
        ]
    );
}

#[test]
fn a_task_the_app_gave_a_ceiling_to_runs_out_of_it_and_not_out_of_its_retries() {
    let mut app = App::new();
    let mut task = app.task(7);
    // `(15s + 5s) * 2` is what the two tries in this task would be given; the
    // ceiling cuts it to 30 s, which is what `ComputeTaskTimeout` answers
    task.total_timeout = 30 * 1000;
    app.start_task(task);
    assert_eq!(app.manager.tasks()[0].task_timeout, 30 * 1000);
    assert!(app.manager.on_send_at(START, 7));

    // the first package timeout comes first — 12 s, as ever — and the try that
    // is left goes out again after the queue's own interval
    app.manager.run_loop_at(START + FIRST_PKG);
    app.manager.run_loop_at(START + FIRST_PKG + RETRY_INTERNAL);
    assert_eq!(app.sent().len(), 2, "the second try went out");
    assert_eq!(app.manager.len(), 1, "the task is not over yet");

    // ... and then the ceiling is what runs out, in the middle of that try:
    // the task has a retry in it still, and it is failed all the same
    app.manager.run_loop_at(START + 30 * 1000);
    assert!(app.manager.is_empty());
    assert_eq!(
        app.ended(),
        vec![(
            ErrCmdType::Local,
            LOCAL_TASK_TIMEOUT,
            TaskFailHandleType::TaskTimeout,
            7
        )]
    );
}

#[test]
fn a_task_that_said_how_long_the_server_needs_waits_that_long_for_the_first_package() {
    let mut app = App::new();
    let mut task = app.task(7);
    // the C++'s `test3`: 10 s the server said it needs, and the request is
    // short enough to add nothing to it
    task.server_process_cost = 10 * 1000;
    app.start_task(task);
    assert!(app.manager.on_send_at(START, 7));

    assert_eq!(app.manager.due_time(), Some(START + 10 * 1000));
    // what the server needs is not one the network's own opinion may shorten
    assert_eq!(
        app.manager.tasks()[0].current_dyntime_status,
        DynamicTimeoutStatus::Evaluating
    );

    app.manager.run_loop_at(START + 10 * 1000);
    assert_eq!(
        app.notified(),
        vec![(
            MAIN.to_string(),
            ErrCmdType::NetMsgXp,
            LONG_FIRST_PKG_TIMEOUT
        )]
    );
}

#[test]
fn a_task_pinned_to_a_connect_that_is_no_longer_the_one_the_link_is_on_is_over() {
    let mut app = App::new();
    let mut task = app.task(7);
    task.channel_id = START - 1;
    app.start_task(task);
    assert_eq!(app.sent().len(), 1, "the link was up on that connect");

    // the link is made again: what the task asked for is the connect that is
    // gone, and a task that is already out is not put on another one
    app.remake(MAIN, START + 100);
    app.manager.on_network_change_at(START + 200);

    assert!(app.manager.is_empty());
    assert_eq!(
        app.ended(),
        vec![(
            ErrCmdType::Local,
            LOCAL_CHANNEL_ID,
            TaskFailHandleType::TaskEnd,
            7
        )]
    );
}

#[test]
fn a_task_whose_link_is_down_waits_for_it() {
    let mut app = App::new();
    app.take_down(MAIN);
    app.start(7);

    assert!(app.sent().is_empty(), "nowhere to go out on");
    assert!(app.ended().is_empty(), "and it is not failed for it");
    assert_eq!(app.manager.len(), 1);

    app.bring_up(MAIN);
    app.manager.run_loop_at(START + 10);
    assert_eq!(app.sent().len(), 1, "and now it goes out");
    assert_eq!(
        app.manager.due_time(),
        Some(START + ONE_TRY),
        "with what is left of its own time, which is counted from the moment it was put in the queue"
    );
}

#[test]
fn one_answer_one_task_could_not_read_fails_every_task_of_the_link() {
    let mut app = App::new();
    app.answer_with(-1, TaskFailHandleType::Default);
    app.start(7);
    app.start(8);
    assert_eq!(app.sent().len(), 2);

    assert_eq!(
        app.manager
            .on_response_at(START + 500, answered(7, b"hello")),
        Some(RespHandle::Retried),
        "both tasks have a try left"
    );
    assert_eq!(app.manager.len(), 2);
    assert!(app.ended().is_empty());
    assert_eq!(
        app.notified(),
        vec![(MAIN.to_string(), ErrCmdType::EnDecode, -1)],
        "the app is told what the *handle* was, once, for the whole channel"
    );
    assert_eq!(app.manager.retry_interval(), RETRY_INTERNAL);
    assert!(app
        .down()
        .contains(&(MAIN.to_string(), DisconnectInternalCode::DecodeErr)));

    // and neither goes out before the wait is over
    app.manager.run_loop_at(START + 500);
    assert_eq!(app.sent().len(), 2);
    app.manager.run_loop_at(START + 500 + RETRY_INTERNAL);
    assert_eq!(app.sent().len(), 4);
}

#[test]
fn a_session_timeout_is_what_the_app_is_asked_about() {
    let mut app = App::new();
    app.answer_with(-13, TaskFailHandleType::SessionTimeout);
    app.start(7);

    assert_eq!(
        app.manager
            .on_response_at(START + 500, answered(7, b"hello")),
        Some(RespHandle::Deferred)
    );
    assert_eq!(app.manager.len(), 1, "the queue waits for the app");
    assert_eq!(
        app.asked(),
        vec![(
            ErrCmdType::EnDecode,
            -13,
            TaskFailHandleType::SessionTimeout,
            7,
            String::new()
        )]
    );

    // what the app answers is what every task of the channel is failed with
    app.manager.retry_tasks_at(
        START + 600,
        ErrCmdType::Local,
        LOCAL_CANCEL,
        TaskFailHandleType::TaskEnd,
        7,
        "",
    );
    assert!(app.manager.is_empty());
    assert_eq!(
        app.ended(),
        vec![(
            ErrCmdType::Local,
            LOCAL_CANCEL,
            TaskFailHandleType::TaskEnd,
            7
        )]
    );
}

#[test]
fn what_the_server_pushed_is_not_a_task() {
    let mut app = App::new();
    app.start(7);

    assert_eq!(
        app.manager.on_response_at(START + 100, pushed(b"push")),
        None
    );
    assert_eq!(
        app.pushed(),
        vec![(MAIN.to_string(), 1, Task::INVALID_TASK_ID, b"push".to_vec())]
    );
    assert!(app.manager.has_task(7), "and no task was touched");
}

#[test]
fn an_answer_from_a_channel_the_queue_has_no_link_for_is_nothing() {
    let mut app = App::new();
    app.start(7);

    let mut response = answered(7, b"hello");
    response.name = "long.other.qq.com".to_string();
    assert_eq!(app.manager.on_response_at(START + 100, response), None);
    assert!(app.manager.has_task(7));
}

#[test]
fn a_network_change_cancels_the_run_that_was_out_and_sends_the_task_again() {
    let mut app = App::new();
    app.start(7);
    assert_eq!(app.sent().len(), 1);

    app.manager.on_network_change_at(START + 10);
    assert_eq!(
        app.sent()
            .iter()
            .map(|sent| (sent.channel.clone(), sent.taskid))
            .collect::<Vec<_>>(),
        vec![(MAIN.to_string(), 7), (MAIN.to_string(), 7)],
        "the run that was out is cancelled and the task goes out again"
    );
    assert!(app.ended().is_empty(), "a cancelled task is not over");
    assert_eq!(app.manager.retry_interval(), 0, "and it does not wait");
}

#[test]
fn redoing_every_task_takes_every_channel_apart_first() {
    let mut app = App::new();
    app.add_channel("long.second.qq.com", true);
    app.start(7);

    app.manager.redo_tasks_at(START + 10);
    assert_eq!(
        app.reset(),
        vec![MAIN.to_string(), "long.second.qq.com".to_string()]
    );
}

#[test]
fn a_task_the_app_stopped_is_one_the_link_is_told_about() {
    let mut app = App::new();
    app.start(7);

    assert!(app.manager.stop_task(7));
    assert_eq!(app.stopped(), vec![(MAIN.to_string(), 7)]);
    assert!(!app.manager.has_task(7));
}

#[test]
fn a_task_that_is_only_sent_is_over_the_moment_it_went_out() {
    let mut app = App::new();
    let mut task = app.task(7);
    task.send_only = true;
    app.start_task(task);

    assert_eq!(app.sent().len(), 1, "it did go out");
    assert!(app.manager.is_empty());
    assert_eq!(
        app.ended(),
        vec![(ErrCmdType::Ok, 0, TaskFailHandleType::Normal, 7)]
    );
}

#[test]
fn a_minor_task_goes_out_on_the_channel_its_hosts_named() {
    let mut app = App::new();
    app.add_channel("minor.weixin.qq.com", true);
    let mut task = app.task(7);
    task.minorlong_host_list = vec!["minor.weixin.qq.com".to_string()];
    assert!(app
        .manager
        .start_task_at(START, task, Task::CHANNEL_MINOR_LONG));

    assert_eq!(
        app.sent(),
        vec![Sent {
            channel: "minor.weixin.qq.com".to_string(),
            taskid: 7,
            body: b"/cgi-bin/7".to_vec(),
        }]
    );
    assert!(!app.manager.has_task(7) || app.manager.task_count(MAIN) == 0);
}

#[test]
fn the_most_urgent_task_is_the_one_that_goes_out_first() {
    let mut app = App::new();
    app.take_down(MAIN);
    let mut slow = app.task(7);
    slow.priority = Task::TASK_PRIORITY_LOWEST;
    app.start_task(slow);
    let mut quick = app.task(8);
    quick.priority = Task::TASK_PRIORITY_HIGHEST;
    app.start_task(quick);

    app.bring_up(MAIN);
    app.manager.run_loop_at(START + 10);

    assert_eq!(
        app.sent()
            .iter()
            .map(|sent| sent.taskid)
            .collect::<Vec<_>>(),
        vec![8, 7],
        "the queue is sorted every time a task is put in it"
    );
}

#[test]
fn a_package_that_came_in_is_what_the_pkg_pkg_timeout_is_counted_from() {
    let mut app = App::new();
    app.start(7);
    assert!(app.manager.on_send_at(START, 7));
    assert_eq!(app.manager.due_time(), Some(START + FIRST_PKG));

    assert!(app.manager.on_recv_at(START + 100, 7, 10, 10));
    assert_eq!(
        app.manager.due_time(),
        Some(START + 100 + mars_stn::config::WIFI_PACKAGE_INTERVAL),
        "the next package is what is waited for now"
    );
}

#[test]
fn a_link_that_is_released_fails_the_tasks_it_had() {
    let mut app = App::new();
    app.start(7);

    assert!(app.manager.remove_long_link_at(START + 10, MAIN));
    assert!(app.manager.channels().is_empty());
    assert!(app.manager.is_empty());
    assert_eq!(
        app.ended(),
        vec![(
            ErrCmdType::Local,
            LOCAL_LONG_LINK_RELEASED,
            TaskFailHandleType::TaskEnd,
            7
        )]
    );
}

#[test]
fn clearing_the_tasks_takes_every_link_down_and_tells_the_app_nothing() {
    let mut app = App::new();
    app.start(7);

    app.manager.clear_tasks();
    assert!(app.manager.is_empty());
    assert_eq!(
        app.down(),
        vec![(MAIN.to_string(), DisconnectInternalCode::Reset)]
    );
    assert!(app.ended().is_empty());
}

#[test]
fn what_is_left_when_the_queue_goes_away_is_failed() {
    let mut app = App::new();
    app.start(7);

    let mut manager = LongLinkTaskManager::new();
    std::mem::swap(&mut manager, &mut app.manager);
    drop(manager);
    assert_eq!(
        app.ended(),
        vec![(
            ErrCmdType::Local,
            LOCAL_RESET,
            TaskFailHandleType::TaskEnd,
            7
        )]
    );
}

#[test]
fn an_error_the_read_came_back_with_is_one_the_channel_is_failed_with() {
    let mut app = App::new();
    app.start(7);

    assert_eq!(
        app.manager
            .on_response_at(START + 100, failed(7, ErrCmdType::Socket, -5001)),
        Some(RespHandle::Retried),
        "the task has a try left"
    );
    assert!(app.ended().is_empty());
    assert_eq!(app.manager.retry_interval(), RETRY_INTERNAL);
    assert!(
        app.down().is_empty(),
        "a socket error is the link's own, not one to take it down for"
    );
}
