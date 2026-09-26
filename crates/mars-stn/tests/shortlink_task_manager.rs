//! `mars/stn/src/shortlink_task_manager.cc`, through the public api.
//!
//! The samples are what the C++ does to the same calls: a task that goes out
//! and answers, one that answered nothing and is tried again — the last try
//! without a proxy — one whose run never sent anything and so runs out of its
//! own time, a network change that cancels what is out, an answer the app says
//! is a session timeout, and the queue the tasks are kept in: the most urgent
//! goes first, a cgi that was answered already is answered from what was kept,
//! and a socket the server said to keep is kept.
//!
//! The app's own side — the encoder, the decoder, the run itself — is what a
//! `NetCore` would wire, and the tests put it on the queue the way a host does.

use std::sync::{Arc, Mutex};

use mars_stn::dynamic_timeout::NetworkKind;
use mars_stn::shortlink_task_manager::{Response, RunRequest};
use mars_stn::task_profile::LOCAL_RESET;
use mars_stn::{
    ConnectProfile, ErrCmdType, IpPortItem, PrepareProfile, RespHandle, RunId,
    ShortLinkTaskManager, Task, TaskFailHandleType, TaskIntercept, RETRY_INTERNAL,
};

/// The reading every task in these samples is started at.
const START: u64 = 100 * 1000;
/// `(15s + 5s) * (0 + 1)` — what a task with one try in it is given, from
/// [`mars_stn::compute_task_timeout`].
const ONE_TRY: u64 = 20 * 1000;
/// `kBaseFirstPackageWifiTimeout` — what the first package of a small request
/// on wi-fi is waited for.
const FIRST_PKG: u64 = 12 * 1000;

/// What a run was asked for, which is all the queue says to the host.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sent {
    taskid: u32,
    use_proxy: bool,
    sent_count: i32,
}

/// What the app was told about a task that is over.
type Ended = Vec<(ErrCmdType, i32, TaskFailHandleType, u32)>;

/// What the app was asked about an answer it may want tried again.
type Asked = Vec<(ErrCmdType, i32, TaskFailHandleType, u32, String)>;

/// The queue, and the app wired to it the way a `NetCore` wires it.
struct App {
    manager: ShortLinkTaskManager,
    sent: Arc<Mutex<Vec<Sent>>>,
    destroyed: Arc<Mutex<Vec<RunId>>>,
    ended: Arc<Mutex<Ended>>,
    asked: Arc<Mutex<Asked>>,
    bodies: Arc<Mutex<Vec<Vec<u8>>>>,
    closed: Arc<Mutex<Vec<mars_stn::SocketFd>>>,
    answer: Arc<Mutex<(i32, TaskFailHandleType)>>,
}

impl App {
    /// A queue with the app's own side on it: every task may go out, the
    /// encoder writes the cgi, the decoder reads an answer as a good one, and
    /// a run is named after the task it is the run of.
    fn new() -> Self {
        let mut manager = ShortLinkTaskManager::new();

        let sent: Arc<Mutex<Vec<Sent>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = sent.clone();
        let bodies: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let body_recorder = bodies.clone();
        manager.set_start_run(move |task: &Task, request: &RunRequest| {
            recorder.lock().unwrap().push(Sent {
                taskid: task.taskid,
                use_proxy: request.use_proxy,
                sent_count: request.sent_count,
            });
            body_recorder.lock().unwrap().push(request.body.clone());
            Some(RunId(u64::from(task.taskid)))
        });

        let destroyed: Arc<Mutex<Vec<RunId>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = destroyed.clone();
        manager.set_destroy_run(move |run| recorder.lock().unwrap().push(run));

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

        let closed: Arc<Mutex<Vec<mars_stn::SocketFd>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = closed.clone();
        manager.set_close_socket(move |socket| recorder.lock().unwrap().push(socket));

        let answer = Arc::new(Mutex::new((0, TaskFailHandleType::Normal)));
        let decoder = answer.clone();
        manager.set_buf2resp(move |_task, _body| *decoder.lock().unwrap());
        manager.set_req2buf(|task| Ok(task.cgi.clone().into_bytes()));
        manager.set_make_sure_authed(|_host, _user_id| true);
        manager.set_anti_avalanche_check(|_task, _body| true);
        manager.set_net_info(|| NetworkKind::Wifi);

        Self {
            manager,
            sent,
            destroyed,
            ended,
            asked,
            bodies,
            closed,
            answer,
        }
    }

    /// `mars` — one task to `short.weixin.qq.com`, with one try left in it
    /// after the first.
    fn task(&self, taskid: u32) -> Task {
        let mut task = Task::new(taskid, 1);
        task.cgi = format!("/cgi-bin/{taskid}");
        task.shortlink_host_list = vec!["short.weixin.qq.com".to_string()];
        task.shortlink_fallback_hostlist = vec!["fallback.weixin.qq.com".to_string()];
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
        assert!(self
            .manager
            .start_task_at(START, task, PrepareProfile::new_at(START)));
    }

    /// How the app shall read the next answer.
    fn answer_with(&mut self, err_code: i32, handle: TaskFailHandleType) {
        *self.answer.lock().unwrap() = (err_code, handle);
    }

    fn sent(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }

    fn destroyed(&self) -> Vec<RunId> {
        self.destroyed.lock().unwrap().clone()
    }

    fn ended(&self) -> Ended {
        self.ended.lock().unwrap().clone()
    }

    fn asked(&self) -> Asked {
        self.asked.lock().unwrap().clone()
    }

    fn closed(&self) -> Vec<mars_stn::SocketFd> {
        self.closed.lock().unwrap().clone()
    }

    fn bodies(&self) -> Vec<Vec<u8>> {
        self.bodies.lock().unwrap().clone()
    }
}

/// `__OnResponse` of a run that came back: a 200 with a body.
fn answered(body: &[u8]) -> Response {
    Response {
        err_type: ErrCmdType::Ok,
        status: 200,
        body: body.to_vec(),
        cancel_retry: false,
        profile: ConnectProfile::new(),
    }
}

/// The same, with the pair and the socket the run was made on.
fn answered_on(socket: mars_stn::SocketFd, keepalive_ms: u32, body: &[u8]) -> Response {
    let mut profile = ConnectProfile::new();
    profile.socket_fd = socket;
    profile.keepalive_timeout = keepalive_ms;
    profile.ip_index = 0;
    profile.ip_items = vec![IpPortItem::new("1.1.1.1", 80)];
    profile.host = "short.weixin.qq.com".to_string();
    profile.ip = "1.1.1.1".to_string();
    profile.port = 80;
    Response {
        err_type: ErrCmdType::Ok,
        status: 200,
        body: body.to_vec(),
        cancel_retry: false,
        profile,
    }
}

/// `__OnResponse` of a run that failed.
fn failed(err_type: ErrCmdType, status: i32) -> Response {
    Response {
        err_type,
        status,
        body: Vec::new(),
        cancel_retry: false,
        profile: ConnectProfile::new(),
    }
}

/// The same, with the socket the run was made on.
fn failed_on(socket: mars_stn::SocketFd, err_type: ErrCmdType, status: i32) -> Response {
    let mut profile = ConnectProfile::new();
    profile.socket_fd = socket;
    Response {
        err_type,
        status,
        body: Vec::new(),
        cancel_retry: false,
        profile,
    }
}

#[test]
fn a_task_that_goes_out_and_answers_is_over() {
    let mut app = App::new();
    app.start(7);

    assert_eq!(
        app.sent(),
        vec![Sent {
            taskid: 7,
            use_proxy: true,
            sent_count: 0
        }]
    );
    assert_eq!(
        app.bodies(),
        vec![b"/cgi-bin/7".to_vec()],
        "the run is started with what the encoder wrote"
    );
    assert!(app.manager.on_send_at(START, RunId(7)));
    assert_eq!(
        app.manager
            .on_response_at(START + 500, RunId(7), answered(b"hello")),
        Some(RespHandle::Ended)
    );

    assert!(app.manager.is_empty(), "the task left the queue");
    assert_eq!(
        app.ended(),
        vec![(ErrCmdType::Ok, 0, TaskFailHandleType::Normal, 7)]
    );
    assert_eq!(app.destroyed(), vec![RunId(7)], "and so did its run");
    assert_eq!(app.manager.tasks_continuous_fail_count(), 0);
}

#[test]
fn a_task_that_answered_nothing_is_tried_again_and_the_last_try_goes_without_a_proxy() {
    let mut app = App::new();
    app.start(7);
    assert!(app.manager.on_send_at(START, RunId(7)));

    // the first package is due before the task is
    assert_eq!(app.manager.due_time(), Some(START + FIRST_PKG));
    app.manager.run_loop_at(START + FIRST_PKG);

    assert!(app.ended().is_empty(), "a task with a try left is not over");
    assert_eq!(app.manager.len(), 1);
    assert_eq!(app.manager.tasks()[0].remain_retry_count, 0);
    assert_eq!(
        app.manager.due_time(),
        Some(START + FIRST_PKG + RETRY_INTERNAL),
        "the retry waits out DEF_TASK_RETRY_INTERNAL"
    );

    app.manager.run_loop_at(START + FIRST_PKG + RETRY_INTERNAL);
    assert_eq!(
        app.sent(),
        vec![
            Sent {
                taskid: 7,
                use_proxy: true,
                sent_count: 0
            },
            Sent {
                taskid: 7,
                use_proxy: false,
                sent_count: 0
            },
        ],
        "the last try of a task that may be tried again goes the other way"
    );

    // ... and the try that came back without one is the one the proxy is off
    // for from now on
    assert!(app
        .manager
        .on_send_at(START + FIRST_PKG + RETRY_INTERNAL, RunId(7)));
    assert_eq!(
        app.manager.on_response_at(
            START + FIRST_PKG + RETRY_INTERNAL + 500,
            RunId(7),
            answered(b"hello")
        ),
        Some(RespHandle::Ended)
    );
    assert!(app.manager.is_empty());
    assert!(!app.manager.default_use_proxy());
    assert_eq!(
        app.ended(),
        vec![(ErrCmdType::Ok, 0, TaskFailHandleType::Normal, 7)]
    );
}

#[test]
fn a_try_that_ran_out_of_time_is_the_error_the_c_gives_it() {
    let mut app = App::new();
    let mut task = app.task(7);
    task.retry_count = 3;
    app.start_task(task);
    assert!(app.manager.on_send_at(START, RunId(7)));

    app.manager.run_loop_at(START + FIRST_PKG);
    assert_eq!(app.manager.len(), 1);
    assert_eq!(app.manager.tasks()[0].remain_retry_count, 2);
    assert_eq!(
        app.manager.tasks()[0].err_code,
        mars_stn::Timeout::FirstPkg.err_code()
    );
    assert_eq!(
        app.manager.tasks()[0].err_type,
        mars_stn::Timeout::FirstPkg.err_type()
    );

    // the retry goes out on the fallback hosts, and on tcp
    app.manager.run_loop_at(START + FIRST_PKG + RETRY_INTERNAL);
    assert_eq!(
        app.manager.tasks()[0].task.shortlink_host_list,
        vec!["fallback.weixin.qq.com".to_string()]
    );
    assert_eq!(
        app.manager.tasks()[0].task.transport_protocol,
        Task::TRANSPORT_PROTOCOL_TCP
    );
}

#[test]
fn a_run_that_never_sent_anything_runs_out_of_the_task_s_own_time() {
    let mut app = App::new();
    let mut task = app.task(7);
    task.retry_count = 0;
    app.start_task(task);

    // nothing was sent, so the only thing it is waiting on is itself
    assert!(app.manager.tasks()[0].is_running());
    assert_eq!(app.manager.due_time(), Some(START + ONE_TRY));

    app.manager.run_loop_at(START + ONE_TRY);
    assert!(app.manager.is_empty());
    assert_eq!(
        app.ended(),
        vec![(
            mars_stn::Timeout::Task.err_type(),
            mars_stn::Timeout::Task.err_code(),
            mars_stn::Timeout::Task.fail_handle(),
            7
        )]
    );
    assert_eq!(app.destroyed(), vec![RunId(7)]);
}

#[test]
fn a_network_change_puts_what_is_out_back_in_the_queue() {
    let mut app = App::new();
    app.start(7);
    assert!(app.manager.on_send_at(START, RunId(7)));

    app.manager.redo_tasks_at(START + 500);
    assert_eq!(app.destroyed(), vec![RunId(7)], "the run is cancelled");
    assert!(
        app.ended().is_empty(),
        "a task with a try left is not failed by a network change"
    );
    assert_eq!(app.manager.len(), 1);
    assert!(!app.manager.tasks()[0].is_running());
    assert_eq!(app.manager.due_time(), Some(START + 500 + RETRY_INTERNAL));

    // and the socket the pool was keeping is not the network's any more
    assert!(app.manager.socket_pool().is_empty());
}

#[test]
fn an_answer_the_app_could_not_read_is_asked_about_and_tried_again() {
    let mut app = App::new();
    let mut task = app.task(7);
    task.need_authed = true;
    app.start_task(task);
    assert!(app.manager.on_send_at(START, RunId(7)));
    app.answer_with(0, TaskFailHandleType::SessionTimeout);

    assert_eq!(
        app.manager
            .on_response_at(START + 500, RunId(7), answered(b"hello")),
        Some(RespHandle::Deferred),
        "every task has to be looked at again"
    );
    assert_eq!(app.manager.len(), 1, "and the task is still in the queue");
    assert_eq!(
        app.asked(),
        vec![(
            ErrCmdType::EnDecode,
            0,
            TaskFailHandleType::SessionTimeout,
            7,
            String::new()
        )]
    );

    app.manager.retry_tasks_at(
        START + 1_000,
        ErrCmdType::EnDecode,
        0,
        TaskFailHandleType::SessionTimeout,
        7,
    );
    assert_eq!(app.manager.len(), 1);
    assert_eq!(
        app.manager.tasks()[0].remain_retry_count,
        2,
        "a session timeout buys the task one more try"
    );
    assert!(app.ended().is_empty());
    assert_eq!(app.sent().len(), 2, "and it goes out again at once");
}

#[test]
fn the_most_urgent_task_goes_out_first_and_a_send_only_one_does_not_go_at_all() {
    let mut app = App::new();
    let mut urgent = app.task(7);
    urgent.priority = Task::TASK_PRIORITY_HIGHEST;
    let mut slow = app.task(8);
    slow.priority = Task::TASK_PRIORITY_LOWEST;
    let mut send_only = app.task(9);
    send_only.send_only = true;

    assert!(!app
        .manager
        .start_task_at(START, send_only, PrepareProfile::new_at(START)));
    app.start_task(slow);
    app.start_task(urgent);

    assert_eq!(
        app.sent()
            .into_iter()
            .map(|sent| sent.taskid)
            .collect::<Vec<_>>(),
        vec![8, 7],
        "the slow one was asked for first, so it went out first"
    );
    assert!(!app.manager.has_task(9), "a short link is for an answer");
}

#[test]
fn a_task_the_app_cannot_write_is_failed_at_once() {
    let mut app = App::new();
    app.manager.set_req2buf(|_task| Err(-1_234));
    app.start(7);

    assert!(!app.manager.has_task(7));
    assert_eq!(
        app.ended(),
        vec![(ErrCmdType::EnDecode, -1_234, TaskFailHandleType::TaskEnd, 7)]
    );
    assert!(app.sent().is_empty(), "it never went out");
}

#[test]
fn a_cgi_that_was_answered_already_goes_out_again() {
    let mut app = App::new();
    app.manager
        .intercept()
        .add_intercept_task_at(START, "/cgi-bin/7", b"kept".to_vec());
    let mut task = app.task(7);
    task.retry_count = 0;
    app.start_task(task);

    assert_eq!(
        app.sent().len(),
        1,
        "the C++ answers `false` out of what it kept, so it went out"
    );
    assert!(app.manager.has_task(7), "and it is still out");
    assert!(app.ended().is_empty());
}

#[test]
fn a_socket_the_server_said_to_keep_is_kept_and_one_that_was_broken_is_closed() {
    let mut app = App::new();
    let mut kept = app.task(7);
    kept.headers
        .insert("Connection".to_string(), "Keep-Alive".to_string());
    app.start_task(kept);
    app.manager.on_send_at(START, RunId(7));
    app.manager.on_response_at(
        START + 500,
        RunId(7),
        answered_on(mars_stn::SocketFd(3), 5_000, b"hello"),
    );

    assert_eq!(
        app.manager.socket_pool().len(),
        1,
        "the socket is kept for the next task that wants the same pair"
    );
    assert!(app.closed().is_empty());

    // a run the peer hung up on: nothing to keep
    let mut broken = app.task(8);
    broken
        .headers
        .insert("Connection".to_string(), "Keep-Alive".to_string());
    app.start_task(broken);
    app.manager.on_send_at(START, RunId(8));
    app.manager.on_response_at(
        START + 500,
        RunId(8),
        failed_on(
            mars_stn::SocketFd(4),
            ErrCmdType::Socket,
            mars_stn::ECT_SOCKET_SHUTDOWN,
        ),
    );

    assert_eq!(app.closed(), vec![mars_stn::SocketFd(4)], "it is closed");
    assert_eq!(app.manager.len(), 1, "and the task has a try coming");
    assert_eq!(app.manager.tasks()[0].remain_retry_count, 0);
}

#[test]
fn a_task_with_nowhere_to_go_waits_in_the_queue() {
    let mut app = App::new();
    let mut homeless = app.task(7);
    homeless.shortlink_host_list.clear();
    app.start_task(homeless);

    assert_eq!(app.manager.len(), 1);
    assert!(app.sent().is_empty());
    assert!(!app.manager.tasks()[0].is_running());
    assert_eq!(app.manager.due_time(), Some(START + 2 * ONE_TRY));
}

#[test]
fn a_task_the_queue_is_asked_to_stop_is_gone_and_so_is_its_run() {
    let mut app = App::new();
    app.start(7);
    app.start(8);

    assert!(app.manager.has_task(7));
    assert!(app.manager.stop_task(7));
    assert!(!app.manager.has_task(7));
    assert!(!app.manager.stop_task(7));
    assert_eq!(app.destroyed(), vec![RunId(7)]);

    app.manager.clear_tasks();
    assert!(app.manager.is_empty());
    assert_eq!(app.destroyed(), vec![RunId(7), RunId(8)]);
    assert!(app.ended().is_empty(), "the app is not told about a stop");
}

#[test]
fn the_debug_host_the_app_set_is_the_one_a_run_is_pointed_at() {
    let mut app = App::new();
    app.manager.set_debug_host("debug.weixin.qq.com");
    app.start(7);

    assert_eq!(app.manager.debug_host(), "debug.weixin.qq.com");
    assert_eq!(app.sent().len(), 1);
}

#[test]
fn a_task_that_failed_is_not_what_turns_the_proxy_off() {
    let mut app = App::new();
    let mut task = app.task(7);
    task.retry_count = 0;
    app.start_task(task);
    app.manager.on_send_at(START, RunId(7));

    assert!(app.manager.default_use_proxy());
    app.manager.on_response_at(
        START + 500,
        RunId(7),
        failed(ErrCmdType::Socket, mars_stn::ECT_SOCKET_SHUTDOWN),
    );

    assert!(app.manager.is_empty());
    assert_eq!(app.manager.tasks_continuous_fail_count(), 1);
    // the try that failed went through one, so the queue is off proxies until
    // a task that went through one comes back
    assert!(app.manager.default_use_proxy());
}

#[test]
fn what_is_left_in_the_queue_is_failed_when_the_queue_is_dropped() {
    let ended: Arc<Mutex<Ended>> = Arc::new(Mutex::new(Vec::new()));
    {
        let mut app = App::new();
        let recorder = ended.clone();
        app.manager
            .set_callback(move |err_type, err_code, handle, task, _cost, _profile| {
                recorder
                    .lock()
                    .unwrap()
                    .push((err_type, err_code, handle, task.taskid));
                0
            });
        app.start(7);
        app.start(8);
        assert_eq!(app.manager.len(), 2);
    }

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
        "kEctLocalReset, one for each task that was still in the queue"
    );
}

/// `TaskIntercept` is the port's, and this is the one call the queue makes on
/// it that the samples above do not: an answer the app says is one to keep.
#[test]
fn an_answer_the_app_said_to_keep_is_kept() {
    let mut app = App::new();
    app.manager.set_should_intercept(|_err_code| true);
    app.start(7);
    app.manager.on_send_at(START, RunId(7));
    app.manager
        .on_response_at(START + 500, RunId(7), answered(b"kept"));

    assert_eq!(app.manager.intercept().len(), 1);
    assert_eq!(
        app.manager
            .intercept()
            .intercept_task_info_at(START + 500, "/cgi-bin/7"),
        None,
        "and it is never handed back: the C++ answers `false` while it copies"
    );
    let _: &mut TaskIntercept = app.manager.intercept();
}
