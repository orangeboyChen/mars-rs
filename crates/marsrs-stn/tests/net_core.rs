//! `mars/stn/src/net_core.cc`, through the public api.
//!
//! The samples are what the C++ does to the same calls: a task that goes out on
//! the long link and answers, one that goes out on the short link because the
//! link is down, one that answered nothing and is started again when the link
//! comes back, one the app is asked about before it is ended, a push the
//! server sent, three short-link errors in a row, a second long link the app
//! marks main and then destroys, and what the whole of it leaves behind when
//! the app is done with it.
//!
//! What a host is left to do here is what the C++'s own platform does: run the
//! two queues' loops, hand the answers back, and drain what the queues asked
//! for ([`NetCore::run_pending`] is the C++'s message queue thread).

use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard};

use marsrs_stn::longlink_task_manager::Response as LongAnswer;
use marsrs_stn::net_source::NO_NET;
use marsrs_stn::shortlink_task_manager::{Response as ShortAnswer, RETRY_INTERNAL};
use marsrs_stn::task_profile::{TaskFailHandleType, LOCAL_ANTI_AVALANCHE, LONG_FIRST_PKG_TIMEOUT};
use marsrs_stn::{
    CallFrom, ConnectProfile, DisconnectInternalCode, ErrCmdType, LongLinkEncoder, LongLinkStatus,
    LonglinkConfig, NetCore, NetStatus, RespHandle, RunId, SignallingKeeper, Task,
    DEFAULT_LONGLINK_NAME, DEFAULT_PERIOD, NET_TYPE_WIFI,
};

/// The channel every sample starts on.
const MAIN: &str = DEFAULT_LONGLINK_NAME;
/// The host every short-link task in these samples goes out on.
const SHORT_HOST: &str = "short.weixin.qq.com";
/// The reading every task in these samples is started at.
const START: u64 = 100 * 1000;
/// The unix second the two ip reports are stamped with.
const NOW_SECS: u64 = 1_700_000_000;

/// What a queue asked a channel for: the task, and the request it went out as.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sent {
    channel: String,
    taskid: u32,
    body: Vec<u8>,
}

/// What the app was told about a task that is over: the task, the user it was
/// started for, how it failed, and the ip of the connect it ran on.
type Ended = Vec<(u32, String, ErrCmdType, i32, String)>;
/// What the app was told about the connection.
type Status = Vec<(NetStatus, NetStatus)>;
/// What the app was told about a pair of a link.
type LongErr = Vec<(ErrCmdType, i32, String, u16)>;
/// What the app was told about a pair of a short link.
type ShortErr = Vec<(ErrCmdType, i32, String, u16)>;
/// What the server pushed, and with which cmdid.
type Pushed = Vec<(String, u32, Vec<u8>)>;
/// What the app's own report was given for a task that is over: the task, how
/// it ended, and how many tries it took.
type Reported = Vec<(u32, ErrCmdType, i32, usize)>;
/// What the app was told about a task the two gates refused: which gate, and
/// the number that gate answered with.
type Limited = Vec<(i32, u32)>;

/// The net core, with the two queues and the app wired the way a host wires
/// them.
struct App {
    core: NetCore,
    sent: Arc<Mutex<Vec<Sent>>>,
    ended: Arc<Mutex<Ended>>,
    status: Arc<Mutex<Status>>,
    long_err: Arc<Mutex<LongErr>>,
    short_err: Arc<Mutex<ShortErr>>,
    pushed: Arc<Mutex<Pushed>>,
    reported: Arc<Mutex<Reported>>,
    /// What the app's report of a refused task was given: the gate, and its
    /// reading.
    limited: Arc<Mutex<Limited>>,
    /// What the app's timeout hook was given: the short-link queue hands out
    /// every try that ended, and not only the task that is over.
    timeout_or_remote: Arc<Mutex<Reported>>,
    /// How the app shall read the next answer.
    answer: Arc<Mutex<(i32, TaskFailHandleType)>>,
}

impl App {
    /// A core with the default long link, an encoder that writes the cgi, a
    /// decoder that reads an answer as a good one until a sample says
    /// otherwise, and a short-link run that is named after the task it is the
    /// run of.
    fn new() -> Self {
        let mut core = NetCore::new_at(START);
        core.set_net_info(|| NET_TYPE_WIFI);
        core.set_clock(|| NOW_SECS);

        let sent: Arc<Mutex<Vec<Sent>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = sent.clone();
        core.shortlink().set_start_run(move |task, _request| {
            recorder.lock().unwrap().push(Sent {
                channel: "short".to_string(),
                taskid: task.taskid,
                body: task.cgi.clone().into_bytes(),
            });
            Some(RunId(u64::from(task.taskid)))
        });
        let recorder = sent.clone();
        core.longlink().set_send(move |name, task, body| {
            recorder.lock().unwrap().push(Sent {
                channel: name.to_string(),
                taskid: task.taskid,
                body: body.to_vec(),
            });
            Some(RunId(u64::from(task.taskid)))
        });
        core.shortlink()
            .set_req2buf(|task, _channel| Ok(task.cgi.clone().into_bytes()));
        core.longlink()
            .set_req2buf(|task, _channel| Ok(task.cgi.clone().into_bytes()));
        core.longlink().set_make_sure_connected(|_name| true);
        core.longlink().set_channel_profile(|name| {
            let mut profile = ConnectProfile::new();
            profile.host = name.to_string();
            profile.ip = "1.1.1.1".to_string();
            profile.port = 8080;
            profile
        });

        let answer = Arc::new(Mutex::new((0, TaskFailHandleType::Normal)));
        let decoder = answer.clone();
        core.shortlink()
            .set_buf2resp(move |_task, _body, _channel| *decoder.lock().unwrap());
        let decoder = answer.clone();
        core.longlink()
            .set_buf2resp(move |_task, _body, _channel| *decoder.lock().unwrap());

        let ended: Arc<Mutex<Ended>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = ended.clone();
        core.set_on_task_end(move |taskid, user_id, err_type, err_code, profile| {
            recorder.lock().unwrap().push((
                taskid,
                user_id.to_string(),
                err_type,
                err_code,
                profile.ip.clone(),
            ));
            err_code
        });
        let status: Arc<Mutex<Status>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = status.clone();
        core.set_report_connect_status(move |all, longlink| {
            recorder.lock().unwrap().push((all, longlink));
        });
        let long_err: Arc<Mutex<LongErr>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = long_err.clone();
        core.set_on_longlink_network_err(move |err_type, err_code, ip, port| {
            recorder
                .lock()
                .unwrap()
                .push((err_type, err_code, ip.to_string(), port));
        });
        let short_err: Arc<Mutex<ShortErr>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = short_err.clone();
        core.set_on_shortlink_network_err(move |err_type, err_code, ip, _host, port| {
            recorder
                .lock()
                .unwrap()
                .push((err_type, err_code, ip.to_string(), port));
        });
        let pushed: Arc<Mutex<Pushed>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = pushed.clone();
        core.set_on_push(move |name, cmdid, _taskid, body| {
            recorder
                .lock()
                .unwrap()
                .push((name.to_string(), cmdid, body.to_vec()));
        });
        let reported: Arc<Mutex<Reported>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = reported.clone();
        core.set_report_task_profile(move |profile| {
            recorder.lock().unwrap().push((
                profile.task.taskid,
                profile.err_type,
                profile.err_code,
                profile.history.len(),
            ));
        });
        let limited: Arc<Mutex<Limited>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = limited.clone();
        core.set_report_task_limited(move |check_type, _task, param| {
            recorder.lock().unwrap().push((check_type, param));
            param
        });
        let timeout_or_remote: Arc<Mutex<Reported>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = timeout_or_remote.clone();
        core.set_on_shortlink_timeout_or_remote_shutdown(move |profile| {
            recorder.lock().unwrap().push((
                profile.task.taskid,
                profile.err_type,
                profile.err_code,
                profile.history.len(),
            ));
        });

        Self {
            core,
            sent,
            ended,
            status,
            long_err,
            short_err,
            pushed,
            reported,
            limited,
            timeout_or_remote,
            answer,
        }
    }

    /// `StartTask` — a task that may use anything, with one more try in it
    /// than the one it is given, and a deadline long enough that the time it
    /// spent is not the whole of it.
    fn start(&mut self, taskid: u32) {
        let mut task = Task::new(taskid, 12);
        task.cgi = format!("/cgi-bin/{taskid}");
        task.channel_select = Task::CHANNEL_ALL;
        task.shortlink_host_list = vec![SHORT_HOST.to_string()];
        task.retry_count = 1;
        task.total_timeout = 10 * 60 * 1000;
        task.user_id = "user".to_string();
        assert!(self.core.start_task_at(START, task));
    }

    /// `StartTask` of a task whose body is the same as every other one's, which
    /// is what the frequency gate counts; `false` when a gate refused it.
    fn start_same_body(&mut self, taskid: u32) -> bool {
        let mut task = Task::new(taskid, 12);
        task.cgi = "/cgi-bin/same".to_string();
        task.channel_select = Task::CHANNEL_ALL;
        task.shortlink_host_list = vec![SHORT_HOST.to_string()];
        task.retry_count = 1;
        task.total_timeout = 10 * 60 * 1000;
        task.user_id = "user".to_string();
        self.core.start_task_at(START, task)
    }

    /// What the host's link is in: the C++ asks the link, and a sample says.
    fn bring_up(&self, name: &str, status: LongLinkStatus) {
        let link = Arc::clone(self.core.long_link(name).expect("no such link"));
        link.lock().unwrap().set_status(status);
    }

    /// How the app shall read the next answer.
    fn answer_with(&mut self, err_code: i32, handle: TaskFailHandleType) {
        *self.answer.lock().unwrap() = (err_code, handle);
    }

    /// `__OnResponse` of a long-link read that came back.
    fn answered(&mut self, taskid: u32) -> Option<RespHandle> {
        let answer = LongAnswer {
            name: MAIN.to_string(),
            err_type: ErrCmdType::Ok,
            err_code: 0,
            cmdid: 12,
            taskid,
            body: b"hello".to_vec(),
            profile: Self::pair("2.2.2.2", 443),
        };
        self.core.longlink().on_response_at(START + 100, answer)
    }

    /// `__OnResponse` of a short-link read that came back.
    fn answered_short(&mut self, taskid: u32) -> Option<RespHandle> {
        let answer = ShortAnswer {
            err_type: ErrCmdType::Ok,
            status: 200,
            body: b"hello".to_vec(),
            cancel_retry: false,
            profile: Self::pair("2.2.2.2", 443),
        };
        self.core
            .shortlink()
            .on_response_at(START + 100, RunId(u64::from(taskid)), answer)
    }

    /// What the message queue thread would have done.
    fn run_pending(&mut self) {
        self.core.run_pending_at(START + 100);
    }

    fn pair(ip: &str, port: u16) -> ConnectProfile {
        let mut profile = ConnectProfile::new();
        profile.ip = ip.to_string();
        profile.port = port;
        profile
    }

    fn sent(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }

    /// The main link's signalling keeper, which is the one the app starts and
    /// stops through [`NetCore::keep_signal`].
    fn keeper(&mut self) -> MutexGuard<'_, SignallingKeeper> {
        self.core
            .long_link_meta(MAIN)
            .expect("no such link")
            .keeper()
    }

    fn ended(&self) -> Ended {
        self.ended.lock().unwrap().clone()
    }

    fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    fn long_err(&self) -> LongErr {
        self.long_err.lock().unwrap().clone()
    }

    fn short_err(&self) -> ShortErr {
        self.short_err.lock().unwrap().clone()
    }

    fn pushed(&self) -> Pushed {
        self.pushed.lock().unwrap().clone()
    }

    /// What the app's own report was given for the tasks that are over.
    fn reported(&self) -> Reported {
        self.reported.lock().unwrap().clone()
    }

    /// What the app was told about the tasks the two gates refused.
    fn limited(&self) -> Limited {
        self.limited.lock().unwrap().clone()
    }

    /// What the app's timeout hook was given for the tries that ended.
    fn timeout_or_remote(&self) -> Reported {
        self.timeout_or_remote.lock().unwrap().clone()
    }
}

#[test]
fn a_task_goes_out_on_the_long_link_and_answers() {
    let mut app = App::new();
    app.bring_up(MAIN, LongLinkStatus::Connected);
    app.start(7);

    assert_eq!(
        app.sent(),
        vec![Sent {
            channel: MAIN.to_string(),
            taskid: 7,
            body: b"/cgi-bin/7".to_vec(),
        }]
    );
    assert!(app.core.longlink().has_task(7));
    assert_eq!(
        app.core.connect_profile(7, Task::CHANNEL_LONG).ip,
        "1.1.1.1"
    );

    assert_eq!(app.answered(7), Some(RespHandle::Ended));
    assert!(!app.core.has_task(7));
    // the task asked for `kChannelAll`, which names no queue: whatever the
    // answer came in on, `GetConnectProfile` answers with nothing
    assert_eq!(
        app.ended(),
        vec![(7, "user".to_string(), ErrCmdType::Ok, 0, String::new())]
    );

    // what the queue asked for is the app's own report, and the host runs it
    assert_eq!(app.core.pending_count(), 1);
    app.run_pending();
    assert_eq!(
        app.long_err(),
        vec![(ErrCmdType::Ok, 0, "2.2.2.2".to_string(), 443)]
    );
}

#[test]
fn a_task_that_asked_for_the_long_link_is_ended_on_the_answer_it_came_in_on() {
    let mut app = App::new();
    app.bring_up(MAIN, LongLinkStatus::Connected);
    let mut long = task_of(7);
    long.channel_select = Task::CHANNEL_LONG;
    long.user_id = "user".to_string();
    assert!(app.core.start_task_at(START, long));
    assert!(app.core.longlink().has_task(7));

    assert_eq!(app.answered(7), Some(RespHandle::Ended));
    // `kChannelLong` names the queue the task went out on, and what that
    // queue remembers is the connect the answer came in on
    assert_eq!(
        app.ended(),
        vec![(
            7,
            "user".to_string(),
            ErrCmdType::Ok,
            0,
            "2.2.2.2".to_string()
        )]
    );
}

/// `mars/stn/src/anti_avalanche.cc` — a task one of the two gates refused is
/// told to the app, and the gate is the only thing that can tell it: a task it
/// refused ends there, so no report of a finished one ever carries it.
///
/// The gates are asked by the queues and not by `StartTask`, so the pass the
/// host makes is what refuses the body — the hundred and sixth of them.
#[test]
fn a_task_the_gates_refused_is_told_to_the_app() {
    let mut app = App::new();
    app.bring_up(MAIN, LongLinkStatus::Connected);
    // 105 sends of one body are let through; the 106th is the avalanche
    for send in 0..106 {
        assert!(app.start_same_body(1000 + send), "send {send} of 106");
    }
    app.run_pending();

    let limited = app.limited();
    assert_eq!(limited.len(), 1, "one task was refused, and told once");
    // `kFrequencyLimit`, and the number the gate weighed it against
    assert_eq!(limited[0].0, 1);
    // the task it refused is ended as one, and the 105 before it are not
    let ended = app.ended();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].0, 1105);
    assert_eq!(ended[0].3, LOCAL_ANTI_AVALANCHE);
}

/// `shortlink_task_manager.cc:1206` and `longlink_task_manager.cc:760` — a task
/// that is over is handed to the app's own report once, whole: both queues keep
/// the profile to themselves until then.
#[test]
fn a_task_that_is_over_is_handed_to_the_apps_own_report() {
    let mut app = App::new();
    app.bring_up(MAIN, LongLinkStatus::Connected);
    app.start(7);
    assert_eq!(app.answered(7), Some(RespHandle::Ended));
    assert_eq!(app.reported(), vec![(7, ErrCmdType::Ok, 0, 1)]);

    // the short link's own: a link that is down is what sends a task the app
    // asked for `kChannelAll` of out on the short link
    let mut app = App::new();
    app.start(7);
    assert_eq!(app.answered_short(7), Some(RespHandle::Ended));
    assert_eq!(app.reported(), vec![(7, ErrCmdType::Ok, 0, 1)]);
}

/// `net_core.h:223` — the core hands the short-link queue's timeout hook to the
/// app, and the long link has none of it.
#[test]
fn a_try_that_ended_is_handed_to_the_apps_timeout_hook() {
    let mut app = App::new();
    app.start(7);
    assert_eq!(app.answered_short(7), Some(RespHandle::Ended));
    assert_eq!(
        app.timeout_or_remote(),
        vec![(7, ErrCmdType::Ok, 0, 1)],
        "a short-link try that ended is handed out through the core"
    );

    let mut app = App::new();
    app.bring_up(MAIN, LongLinkStatus::Connected);
    app.start(7);
    assert_eq!(app.answered(7), Some(RespHandle::Ended));
    assert!(
        app.timeout_or_remote().is_empty(),
        "the hook is the short-link queue's, and a long-link try is not asked about"
    );
}

#[test]
fn a_task_goes_out_on_the_short_link_while_the_long_link_is_down() {
    let mut app = App::new();
    app.start(7);

    assert_eq!(
        app.sent(),
        vec![Sent {
            channel: "short".to_string(),
            taskid: 7,
            body: b"/cgi-bin/7".to_vec(),
        }]
    );
    assert!(!app.core.longlink().has_task(7));

    assert_eq!(app.answered_short(7), Some(RespHandle::Ended));
    // `kChannelAll` names no queue, so the short link the task went out on is
    // not asked about the connect it made
    assert_eq!(
        app.ended(),
        vec![(7, "user".to_string(), ErrCmdType::Ok, 0, String::new())]
    );

    app.run_pending();
    assert_eq!(
        app.short_err(),
        vec![(ErrCmdType::Ok, 0, "2.2.2.2".to_string(), 443)]
    );
}

#[test]
fn a_task_that_answered_nothing_is_started_again_when_the_link_comes_back() {
    let mut app = App::new();
    app.bring_up(MAIN, LongLinkStatus::Connected);
    app.answer_with(-1, TaskFailHandleType::TaskTimeout);
    app.start(7);
    app.answered(7);

    // the task is kept, not failed: the app is not told about it
    assert!(app.ended().is_empty());
    assert_eq!(app.core.zombie().len(), 1);
    assert!(app.core.has_task(7), "a zombie is a task STN still has");

    // the link came back: the host is asked to start the zombies again, which
    // the C++ posts to its message queue
    app.bring_up(MAIN, LongLinkStatus::Connected);
    app.core
        .on_longlink_status_changed_at(START + 500, LongLinkStatus::Connected);
    assert_eq!(app.core.pending_count(), 2);
    app.core.run_pending_at(START + 500);

    assert_eq!(app.core.zombie().len(), 0);
    assert!(app.core.longlink().has_task(7));
    assert_eq!(app.sent().len(), 2, "the task went out again");
    assert_eq!(
        app.status(),
        vec![(NetStatus::Connected, NetStatus::Connected)]
    );
}

#[test]
fn a_task_the_app_takes_over_is_not_ended() {
    let mut app = App::new();
    let ended = Arc::new(Mutex::new(Vec::new()));
    let recorder = ended.clone();
    app.core
        .set_task_callback(move |from, err_type, err_code, _handle, task| {
            recorder
                .lock()
                .unwrap()
                .push((from, task.taskid, err_type, err_code));
            0
        });

    app.bring_up(MAIN, LongLinkStatus::Connected);
    app.start(7);
    app.answered(7);

    assert_eq!(
        ended.lock().unwrap().clone(),
        vec![(CallFrom::Long, 7, ErrCmdType::Ok, 0)]
    );
    assert!(app.ended().is_empty(), "the app said it took the task");
}

#[test]
fn a_push_the_server_sent_is_preprocessed_and_then_the_apps() {
    let mut app = App::new();
    let preprocessed = Arc::new(Mutex::new(Vec::new()));
    let recorder = preprocessed.clone();
    app.core.set_push_preprocess(move |cmdid, _body| {
        recorder.lock().unwrap().push(cmdid);
    });

    let answer = LongAnswer {
        name: MAIN.to_string(),
        err_type: ErrCmdType::Ok,
        err_code: 0,
        cmdid: 6,
        taskid: Task::INVALID_TASK_ID,
        body: b"push".to_vec(),
        profile: ConnectProfile::new(),
    };
    assert!(app.core.longlink().on_response_at(START, answer).is_none());

    assert_eq!(app.pushed(), vec![(MAIN.to_string(), 6, b"push".to_vec())]);
    assert_eq!(preprocessed.lock().unwrap().clone(), vec![6]);
}

#[test]
fn three_short_link_errors_in_a_row_are_a_server_the_app_cannot_reach() {
    let mut app = App::new();
    // a link that failed is the one whose own answer is not the app's: what
    // the app is told about everything is the short link's
    app.bring_up(MAIN, LongLinkStatus::ConnectFailed);
    app.answer_with(-1, TaskFailHandleType::TaskTimeout);

    for taskid in [7, 8, 9] {
        app.start(taskid);
        app.answered_short(taskid);
        app.run_pending();
    }

    assert_eq!(
        app.status(),
        vec![
            (NetStatus::Unknown, NetStatus::ServerFailed),
            (NetStatus::Unknown, NetStatus::ServerFailed),
            (NetStatus::ServerFailed, NetStatus::ServerFailed),
        ]
    );
    assert_eq!(app.short_err().len(), 3);
    assert_eq!(app.ended().len(), 0, "every one of them was kept");
    assert_eq!(app.core.zombie().len(), 3);
}

#[test]
fn a_second_long_link_is_made_marked_main_and_destroyed() {
    let mut app = App::new();
    app.bring_up(MAIN, LongLinkStatus::Connected);
    let second = app
        .core
        .create_long_link(LonglinkConfig::new("second"))
        .expect("a link the factory makes");
    second.lock().unwrap().set_status(LongLinkStatus::Connected);
    assert_eq!(app.core.default_link(), Some(MAIN));

    assert!(app.core.mark_main_longlink("second"));
    assert_eq!(app.core.default_link(), Some("second"));
    assert!(app.core.is_default_long_link_connected());

    // a task with no channel of its own goes on the one the app marked main
    app.start(7);
    assert_eq!(app.sent()[0].channel, "second");

    // the link is destroyed: the task that was out on it is failed with it
    assert!(app.core.destroy_long_link_at(START + 100, "second"));
    assert_eq!(app.core.default_link(), None);
    assert!(!app.core.has_task(7));
    assert_eq!(app.ended().len(), 1);
    assert_eq!(app.ended()[0].0, 7);
    assert_eq!(app.ended()[0].1, "user");
    assert_eq!(app.ended()[0].2, ErrCmdType::Local);
}

#[test]
fn the_app_is_told_how_far_the_connection_has_got() {
    let mut app = App::new();
    // nothing has tried yet: the long link is idle, and so is the short link
    app.core
        .on_longlink_status_changed_at(START, LongLinkStatus::Connecting);
    assert_eq!(
        app.status(),
        vec![(NetStatus::Connecting, NetStatus::Connecting)]
    );

    app.bring_up(MAIN, LongLinkStatus::Connected);
    app.core
        .on_longlink_status_changed_at(START, LongLinkStatus::Connected);
    assert_eq!(
        app.status()[1],
        (NetStatus::Connected, NetStatus::Connected)
    );

    // a link that failed to connect: the app is told about the short link's
    // own answer, which is nothing yet
    app.bring_up(MAIN, LongLinkStatus::ConnectFailed);
    app.core
        .on_longlink_status_changed_at(START, LongLinkStatus::ConnectFailed);
    assert_eq!(
        app.status()[2],
        (NetStatus::Unknown, NetStatus::ServerFailed)
    );

    // a link that went down is not one the app is told about at all
    app.bring_up(MAIN, LongLinkStatus::DisConnected);
    app.core
        .on_longlink_status_changed_at(START, LongLinkStatus::DisConnected);
    assert_eq!(app.status().len(), 3);
}

#[test]
fn a_network_change_looks_at_every_task_again() {
    let mut app = App::new();
    // a link that failed is not one a task may go out on
    app.bring_up(MAIN, LongLinkStatus::ConnectFailed);
    app.answer_with(-1, TaskFailHandleType::TaskTimeout);
    for taskid in [7, 8] {
        app.start(taskid);
        app.answered_short(taskid);
        app.run_pending();
    }
    assert_eq!(app.core.zombie().len(), 2);

    app.core.on_network_change_at(START + 200);

    // the zombies are started again, and the counters the app's answer is
    // worked out from start over
    assert!(app.core.has_pending());
    app.core.run_pending_at(START + 200);
    assert_eq!(app.core.zombie().len(), 0);
    assert_eq!(app.ended().len(), 0, "a cancelled task is not a failed one");
}

#[test]
fn a_task_that_is_stopped_and_a_core_that_is_released() {
    let mut app = App::new();
    app.bring_up(MAIN, LongLinkStatus::Connected);
    app.start(7);
    app.start(8);

    assert!(app.core.stop_task(7));
    assert!(!app.core.has_task(7));
    assert!(!app.core.stop_task(7));
    assert!(app.core.has_task(8));

    app.core.clear_tasks();
    assert!(!app.core.has_task(8));

    // a released core takes nothing more and tells the app nothing more
    app.core.release();
    assert!(app.core.is_released());
    assert_eq!(app.core.default_link(), None);
    assert!(!app.core.start_task(task_of(9)));
    assert_eq!(app.sent().len(), 2);
    assert_eq!(app.status(), vec![]);
}

#[test]
fn the_host_waits_for_the_earliest_of_the_queues() {
    let mut app = App::new();
    app.bring_up(MAIN, LongLinkStatus::Connected);
    app.start(7);

    let due = app
        .core
        .due_time()
        .expect("a task that is out is waited on");
    assert!(due > START, "the queue's own timeouts are ahead of it");
    app.core.touch_tasks_at(due);

    // a core with nothing in it waits for the timing sync's alarm, which is
    // the only thing that is still armed
    app.core.clear_tasks();
    assert_eq!(app.core.due_time(), app.core.timing_sync().due_time());
}

#[test]
fn a_short_link_error_the_app_is_asked_about_is_a_retry() {
    let mut app = App::new();
    app.answer_with(0, TaskFailHandleType::SessionTimeout);
    app.start(7);
    assert_eq!(app.answered_short(7), Some(RespHandle::Deferred));

    // the queue asked the net core to look at every task of that user again,
    // which the C++ posts: the task is still there, and it goes out again
    assert_eq!(app.core.pending_count(), 1);
    app.run_pending();
    assert!(app.core.has_task(7));
    assert_eq!(app.sent().len(), 2);
}

/// The alarm a host sleeps on belongs to what is out, not to what a queue
/// already asked for. A follow-up is posted by a queue that could not do its
/// own work while it was running — a retry, a long link that failed — and
/// nothing arms an alarm for it, so a host that waits for the next one reports
/// the error whenever that one happens to go off.
#[test]
fn a_follow_up_that_is_waiting_is_due_at_the_reading_the_host_has() {
    let mut app = App::new();
    app.answer_with(0, TaskFailHandleType::SessionTimeout);
    app.start(7);
    assert_eq!(app.answered_short(7), Some(RespHandle::Deferred));
    assert_eq!(app.core.pending_count(), 1);

    // and not at the retry interval, nor at the timing sync's alarm: both of
    // those are later than the reading the host is asking about
    assert_eq!(app.core.due_time_at(START), Some(START));

    // a host that drains it has nothing to wait for but the alarms again
    app.run_pending();
    assert!(app.core.due_time_at(START).is_none_or(|due| due > START));
}

#[test]
fn the_signalling_the_app_asked_for_goes_on_until_the_link_s_data_runs_out() {
    let mut app = App::new();
    app.bring_up(MAIN, LongLinkStatus::Connected);

    // `KeepSignal()`: the first buffer goes out at once
    app.core.keep_signal_at(START);
    assert_eq!(app.keeper().sent(), 1);

    // what the link's own run says: data moved, so the next one is posted a
    // period later, and that is what the host is told to wait for
    app.keeper().on_network_data_changed_at(START + 100);
    assert_eq!(
        app.core.due_time_at(START + 200),
        Some(START + 100 + DEFAULT_PERIOD)
    );

    // ... and a host that runs what is due sends it, once
    app.core.run_pending_at(START + 100 + DEFAULT_PERIOD);
    assert_eq!(app.keeper().sent(), 2);
    assert!(app
        .core
        .due_time_at(START + 100 + DEFAULT_PERIOD)
        .is_none_or(|due| due > START + 100 + DEFAULT_PERIOD));

    // `StopSignal()` — the app is waiting for nothing, and nothing goes out
    app.core.stop_signal();
    assert!(!app.keeper().is_keeping());
}

#[test]
fn a_task_the_network_cannot_take_is_not_started() {
    let mut app = App::new();
    app.core.set_net_info(|| NO_NET);
    let mut task = task_of(7);
    task.network_status_sensitive = true;

    assert!(!app.core.start_task_at(START, task));
    assert_eq!(app.ended().len(), 1);
    assert_eq!(app.ended()[0].2, ErrCmdType::Local);
    assert!(app.sent().is_empty());
}

#[test]
fn the_setters_reach_the_pieces_they_are_for() {
    let mut app = App::new();
    app.bring_up(MAIN, LongLinkStatus::Connected);
    app.start(7);
    assert!(app
        .core
        .disconnect_long_link_by_taskid(7, DisconnectInternalCode::Reset));

    // a task that is still out on the long link, which is what a redo below
    // has to have something to cancel
    app.start(8);

    // the short link's own host, and not one the core keeps for itself
    app.core.set_debug_host(SHORT_HOST);
    assert_eq!(app.core.shortlink().debug_host(), SHORT_HOST);

    // a host the C++'s `__ForbidUseTls` takes a long-link one for, which is
    // what the name it is given has to have in it
    app.core.forbid_longlink_tls_host(&[MAIN.to_string()]);
    assert!(app.core.longlink().forbid_tls(&[MAIN.to_string()]));

    // the net source's, and not a pair the core keeps: the net source keeps
    // no reading of them a host can ask for, so what they do to a list is
    // `tests/simple_ipport_sort.rs`'s to say, and here they are the calls
    // the core forwards
    app.core.add_server_ban("1.2.3.4");
    app.core.init_history_to_banned_list();

    app.core.set_ip_connect_timeout(1000, 2000);
    assert_eq!(app.core.net_source().ip_connect_timeout(), (1000, 2000));

    app.core.set_packer_encoder(3, "encoder");
    assert_eq!(app.core.packer_encoder_version(), 3);
    assert_eq!(app.core.packer_encoder_name(), "encoder");

    // `ActiveLogic::SignalActive` is a flag the two gates read and not one
    // the core keeps to itself: the speed the funnel drains at is the one
    // thing that says which way the app went
    let inactive = app.core.anti_avalanche().flow_limit().funnel_speed();
    app.core.set_active(true);
    assert_ne!(
        app.core.anti_avalanche().flow_limit().funnel_speed(),
        inactive,
        "the funnel drains at the speed of an app that never went active"
    );

    // `MakeSureLongLinkConnected` is the link's own question and not one the
    // core answers: what the core has to do is reach the link the name is
    // for, and what reaching it does is start a run on it. A link that is
    // up answers "connected" and starts nothing, so the question is put to
    // one that is not. A second run is not started while one is in flight,
    // so the default link's turn is told apart by the scene a disconnect
    // left on it, which a new run clears.
    let link = Arc::clone(app.core.long_link(MAIN).expect("the default link"));
    link.lock()
        .unwrap()
        .set_status(LongLinkStatus::DisConnected);
    assert!(!link.lock().unwrap().is_running());
    app.core.make_sure_long_link_connected(MAIN);
    assert!(link.lock().unwrap().is_running());
    {
        let mut link = link.lock().unwrap();
        link.disconnect(DisconnectInternalCode::Reset);
        link.end_run();
        assert!(link.disconnect_code().is_set());
    }
    app.core.make_sure_default_long_link_connected();
    assert_eq!(
        link.lock().unwrap().disconnect_code(),
        DisconnectInternalCode::None,
        "the default link was not asked"
    );

    // `KeepSignal()` — the first buffer goes out at once, which is the one
    // thing a keeper nobody touched does not do
    app.core.keep_signal_at(START);
    assert_eq!(app.keeper().sent(), 1);

    // `StopSignal()` ends a keeper that has a post waiting, which is what the
    // link's own data gave it, and not one that was only ever touched
    app.keeper().on_network_data_changed_at(START);
    app.core.stop_signal();
    assert!(!app.keeper().is_keeping(), "the app is waiting for nothing");

    // `RedoTasks` cancels every run that is out and the queue starts it
    // again, so what the app sees of one is a second send of the same task
    let before = app.sent();
    assert_eq!(before.iter().filter(|sent| sent.taskid == 8).count(), 1);
    app.core.redo_tasks_at(START + 100);
    let after = app.sent();
    assert_eq!(
        after.iter().filter(|sent| sent.taskid == 8).count(),
        2,
        "a redo sent nothing again: {after:?}"
    );

    // a core that is told not to use the long link puts everything on the
    // short one, and makes no more links
    app.core.set_need_use_long_link(false);
    assert!(!app.core.use_long_link());
    assert!(app
        .core
        .create_long_link(LonglinkConfig::new("second"))
        .is_none());
    app.start(9);
    assert_eq!(
        app.sent().last().map(|sent| sent.channel.as_str()),
        Some("short")
    );
}

/// A task that may use anything, with somewhere to go on either link.
fn task_of(taskid: u32) -> Task {
    let mut task = Task::new(taskid, 12);
    task.cgi = format!("/cgi-bin/{taskid}");
    task.channel_select = Task::CHANNEL_ALL;
    task.shortlink_host_list = vec![SHORT_HOST.to_string()];
    task.total_timeout = 10 * 60 * 1000;
    task
}

/// The channel the queue asks is the link the core made, and not one nobody
/// wired. Every other sample here hands the queue a channel of its own, which
/// is the app's right and hides what the core leaves it with: a task put on a
/// long link sat in the queue until its own timeout ran out, and the link was
/// never asked for it at all. What shows it is a core with no hooks of the
/// app's — what a task goes out on is then the link's own queue, which is what
/// the host's run writes off.
#[test]
fn a_task_goes_out_on_the_link_the_core_made_and_not_on_a_channel_nobody_wired() {
    let mut core = NetCore::new_at(START);
    core.set_net_info(|| NET_TYPE_WIFI);
    core.set_clock(|| NOW_SECS);
    core.longlink()
        .set_req2buf(|task, _channel| Ok(task.cgi.clone().into_bytes()));

    let link = Arc::clone(core.long_link(MAIN).expect("the default link"));
    link.lock().unwrap().set_status(LongLinkStatus::Connected);

    let mut task = Task::new(7, 12);
    task.cgi = "/cgi-bin/7".to_string();
    task.channel_select = Task::CHANNEL_LONG;
    task.total_timeout = 10 * 60 * 1000;
    assert!(core.start_task_at(START, task));

    let queued: Vec<(u32, Vec<u8>)> = link
        .lock()
        .unwrap()
        .queued()
        .iter()
        .map(|data| (data.task.taskid, data.buffer.clone()))
        .collect();
    assert_eq!(queued.len(), 1, "one task went out on the link");
    assert_eq!(queued[0].0, 7);
    assert!(
        queued[0].1.ends_with(b"/cgi-bin/7"),
        "and what it carries is the body the app wrote: {:?}",
        queued[0].1
    );
    assert!(core.longlink().has_task(7), "the queue still waits on it");
}

/// `SetNeedUseLongLink` is one assignment in the C++ (`net_core.cc:1316`), and
/// it is one here: a setter that re-wired the core to change the flag put the
/// channel hooks of `wire_longlink_channels` back over the ones the host had
/// installed on the queue itself, which is what that function's own comment
/// promises it does not do.
#[test]
fn setting_the_flag_leaves_the_hooks_of_the_queue_alone() {
    let mut core = NetCore::new_at(START);
    core.set_net_info(|| NET_TYPE_WIFI);
    core.set_clock(|| NOW_SECS);
    core.longlink()
        .set_req2buf(|task, _channel| Ok(task.cgi.clone().into_bytes()));

    let asked = Arc::new(AtomicUsize::new(0));
    let counting = Arc::clone(&asked);
    core.longlink().set_make_sure_connected(move |_| {
        counting.fetch_add(1, AtomicOrdering::SeqCst);
        true
    });

    core.set_need_use_long_link(true);

    let mut task = Task::new(7, 12);
    task.cgi = "/cgi-bin/7".to_string();
    task.channel_select = Task::CHANNEL_LONG;
    task.total_timeout = 10 * 60 * 1000;
    assert!(core.start_task_at(START, task));

    assert_eq!(
        asked.load(AtomicOrdering::SeqCst),
        1,
        "the queue asked the hook the app gave it, and not one a re-wire put back"
    );
}

#[test]
fn an_encoder_the_app_set_is_the_one_every_link_is_made_with() {
    // An encoder the app asked for, and not the one a link makes for itself:
    // `0` is "the link decides", so an interval of its own is a difference
    // every heartbeat after this one can be seen to carry.
    let mut encoder = LongLinkEncoder::new();
    encoder.noop_interval = 60_000;

    let mut core = NetCore::with_encoder_at(START, true, encoder);
    let link = core.create_long_link(LonglinkConfig::new("second"));
    assert!(link.is_some());
    assert!(core.long_link_meta("second").is_some());
    assert_eq!(core.longlink().channels().len(), 2);

    // … and both of the links the core made are made with it, the one the app
    // named and the one it did not. A core that handed the default to the
    // factory passes the three assertions above untouched.
    for name in [MAIN, "second"] {
        let link = Arc::clone(core.long_link(name).unwrap_or_else(|| {
            panic!("the core made no link named {name}");
        }));
        let made = link.lock().unwrap_or_else(|p| p.into_inner()).encoder();
        assert_eq!(
            made, encoder,
            "the link named {name} was made with {made:?} and not with {encoder:?}"
        );
    }
}

/// What the host drains is not the follow-ups alone. A task that answered
/// nothing has no follow-up waiting on it — nothing posted one — so a pass that
/// only drained them would leave it in its queue until the process ended. What
/// ends it is the queue's own loop, and [`NetCore::due_time`] is the tick the
/// host is to make the pass at.
#[test]
fn a_pass_the_host_makes_at_the_due_tick_times_a_task_out() {
    let mut app = App::new();
    app.bring_up(MAIN, LongLinkStatus::Connected);
    app.start(7);
    assert!(app.core.longlink().on_send_at(START, 7));
    assert_eq!(app.sent().len(), 1);

    let Some(due) = app.core.due_time() else {
        panic!("a task that is out is waiting on its first package")
    };
    assert!(due > START, "and not on a tick that has already gone");
    app.core.run_pending_at(due);

    assert_eq!(
        app.long_err(),
        vec![(
            ErrCmdType::NetMsgXp,
            LONG_FIRST_PKG_TIMEOUT,
            "1.1.1.1".to_string(),
            8080
        )],
        "the queue read its first-package timeout and told the app about it"
    );
    assert_eq!(
        app.core.due_time(),
        Some(due + RETRY_INTERNAL),
        "and the try that is left waits out the retry interval"
    );

    app.core.run_pending_at(due + RETRY_INTERNAL);
    assert_eq!(app.sent().len(), 2, "and then the task goes out again");
}
