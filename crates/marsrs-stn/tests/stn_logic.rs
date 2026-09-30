//! `mars/stn/stn_logic.cc` and `mars/stn/stn_manager.cc`, through the public
//! api.
//!
//! The samples are what the C++ does to the same calls: a task that goes out
//! with the body the app wrote and ends with the answer the app read, one the
//! app cannot encode, one whose answer the app refused, one the app is asked to
//! be logged in for, a push the server sent, a long link the app makes, marks
//! main and destroys, a reset that throws everything away, and the addresses
//! the app set.
//!
//! What a host is left to do here is what the C++'s own platform does: wire the
//! channels, run the loops, hand the answers back, and drain what the queues
//! asked for ([`StnLogic::run_pending`] is the C++'s message queue thread).

use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use marsrs_stn::sent::{self, Answer, Failure};
use marsrs_stn::task_profile::TaskFailHandleType;
use marsrs_stn::task_profile::LOCAL_CHANNEL_SELECT;
use marsrs_stn::{
    App, CgiProfile as Cgi, ConnectProfile, ErrCmdType, LongLinkStatus, LonglinkConfig, NetStatus,
    RespHandle, RunId, StnLogic, Task, DEFAULT_LONGLINK_NAME, NET_TYPE_WIFI,
};

/// The channel every sample starts on.
const MAIN: &str = DEFAULT_LONGLINK_NAME;
/// The host every long-link task in these samples goes out on.
const LONG_HOST: &str = "long.weixin.qq.com";
/// The host every short-link task in these samples goes out on.
const SHORT_HOST: &str = "short.weixin.qq.com";
/// The reading every task in these samples is started at.
const START: u64 = 100 * 1000;

/// Everything the app was asked, as one value a sample reads.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Said {
    encoded: Vec<(u32, String, u16)>,
    decoded: Vec<(u32, Vec<u8>)>,
    /// The channel the app was told a task is going out on, which is the one
    /// the queue picked for it and not the `channel_select` it was made with.
    encoded_on: Vec<(u32, i32)>,
    /// The channel the app was told an answer came back on.
    decoded_on: Vec<(u32, i32)>,
    ended: Vec<(u32, ErrCmdType, i32, String)>,
    /// The connect every task that ended is reported with.
    profiles: Vec<Cgi>,
    pushed: Vec<(String, u32, Vec<u8>)>,
    status: Vec<(NetStatus, NetStatus)>,
    authed: Vec<(String, String)>,
}

/// An app that answers every question the way a sample wants, and writes down
/// what it was asked.
struct Rec {
    said: Arc<Mutex<Said>>,
    /// How the app shall read the next answer.
    answer: Arc<Mutex<(i32, TaskFailHandleType)>>,
    /// Whether the app is logged in.
    authed: Arc<Mutex<bool>>,
    /// What the app shall write for the next task, if it writes anything.
    body: Arc<Mutex<Option<Vec<u8>>>>,
}

impl App for Rec {
    fn makesure_authed(&mut self, host: &str, user_id: &str) -> bool {
        self.said
            .lock()
            .unwrap()
            .authed
            .push((host.to_string(), user_id.to_string()));
        *self.authed.lock().unwrap()
    }

    fn req2buf(
        &mut self,
        taskid: u32,
        _user_id: &str,
        channel_select: i32,
        host: &str,
        sequence: u16,
    ) -> Result<Vec<u8>, i32> {
        let mut said = self.said.lock().unwrap();
        said.encoded.push((taskid, host.to_string(), sequence));
        said.encoded_on.push((taskid, channel_select));
        drop(said);
        match self.body.lock().unwrap().clone() {
            Some(body) => Ok(body),
            None => Err(-300),
        }
    }

    fn buf2resp(
        &mut self,
        taskid: u32,
        _user_id: &str,
        body: &[u8],
        channel_select: i32,
    ) -> (i32, TaskFailHandleType) {
        let mut said = self.said.lock().unwrap();
        said.decoded.push((taskid, body.to_vec()));
        said.decoded_on.push((taskid, channel_select));
        drop(said);
        *self.answer.lock().unwrap()
    }

    fn on_task_end(
        &mut self,
        taskid: u32,
        _user_id: &str,
        err_type: ErrCmdType,
        err_code: i32,
        profile: &Cgi,
    ) -> i32 {
        let mut said = self.said.lock().unwrap();
        said.ended
            .push((taskid, err_type, err_code, profile.nettype.clone()));
        said.profiles.push(profile.clone());
        err_code
    }

    fn on_push(&mut self, channel_id: &str, cmdid: u32, _taskid: u32, body: &[u8]) {
        self.said
            .lock()
            .unwrap()
            .pushed
            .push((channel_id.to_string(), cmdid, body.to_vec()));
    }

    fn report_connect_status(&mut self, all: NetStatus, longlink: NetStatus) {
        self.said.lock().unwrap().status.push((all, longlink));
    }
}

/// What the queues asked a channel for: the channel, the task, and the request
/// it went out as.
type Sent = Arc<Mutex<Vec<(String, u32, Vec<u8>)>>>;

/// STN with an app in it, and the host's channels wired the way a host wires
/// them.
struct Host {
    logic: StnLogic,
    said: Arc<Mutex<Said>>,
    sent: Sent,
    answer: Arc<Mutex<(i32, TaskFailHandleType)>>,
    authed: Arc<Mutex<bool>>,
    body: Arc<Mutex<Option<Vec<u8>>>>,
}

impl Host {
    /// A logic with a core in it, an app that is logged in and writes the cgi of
    /// the task it is asked about, and the two queues' channels.
    fn new() -> Self {
        let said = Arc::new(Mutex::new(Said::default()));
        let answer = Arc::new(Mutex::new((0, TaskFailHandleType::Normal)));
        let authed = Arc::new(Mutex::new(true));
        let body: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));

        let mut logic = StnLogic::new();
        logic.set_callback(Rec {
            said: Arc::clone(&said),
            answer: Arc::clone(&answer),
            authed: Arc::clone(&authed),
            body: Arc::clone(&body),
        });
        assert!(logic.create_at(START));

        let core = logic.net_core().expect("no net core");
        core.set_net_info(|| NET_TYPE_WIFI);

        let sent: Sent = Arc::new(Mutex::new(Vec::new()));
        let recorder = sent.clone();
        core.longlink().set_send(move |name, task, body| {
            recorder
                .lock()
                .unwrap()
                .push((name.to_string(), task.taskid, body.to_vec()));
            Some(RunId(u64::from(task.taskid)))
        });
        let recorder = sent.clone();
        core.shortlink().set_start_run(move |task, _request| {
            recorder
                .lock()
                .unwrap()
                .push(("short".to_string(), task.taskid, Vec::new()));
            Some(RunId(u64::from(task.taskid)))
        });
        core.longlink().set_make_sure_connected(|_name| true);
        core.longlink().set_channel_profile(|name| {
            let mut profile = ConnectProfile::new();
            profile.host = name.to_string();
            profile.ip = "1.1.1.1".to_string();
            profile.port = 8080;
            profile
        });

        Self {
            logic,
            said,
            sent,
            answer,
            authed,
            body,
        }
    }

    /// What the app shall write for the next task it is asked about. [`None`]
    /// is a body it cannot write.
    fn write(&mut self, body: Option<&[u8]>) {
        *self.body.lock().unwrap() = body.map(<[u8]>::to_vec);
    }

    /// Whether the app is logged in.
    fn authed(&mut self, authed: bool) {
        *self.authed.lock().unwrap() = authed;
    }

    /// How the app shall read the next answer.
    fn read(&mut self, err_code: i32, handle: TaskFailHandleType) {
        *self.answer.lock().unwrap() = (err_code, handle);
    }

    /// `StartTask` — a task that may use anything, and the host of the link it
    /// is going out on.
    fn start(&mut self, taskid: u32) {
        let mut task = Task::new(taskid, 12);
        task.cgi = format!("/cgi-bin/{taskid}");
        task.channel_select = Task::CHANNEL_ALL;
        task.longlink_host_list = vec![LONG_HOST.to_string()];
        task.user_id = "user".to_string();
        task.retry_count = 1;
        task.total_timeout = 10 * 60 * 1000;
        assert!(self.logic.start_task_at(START, task));
    }

    /// `StartTask` on the short link — a task the app named a short host for,
    /// and that is not going out before the app says it is logged in.
    fn start_short(&mut self, taskid: u32) {
        let mut task = Task::new(taskid, 12);
        task.cgi = format!("/cgi-bin/{taskid}");
        task.channel_select = Task::CHANNEL_SHORT;
        task.shortlink_host_list = vec![SHORT_HOST.to_string()];
        task.user_id = "user".to_string();
        task.need_authed = true;
        task.retry_count = 1;
        task.total_timeout = 10 * 60 * 1000;
        assert!(self.logic.start_task_at(START, task));
    }

    /// `StartTask` on the long link — a task that asked for it by name, which
    /// is what lets the app be told about the connect it ran on.
    fn start_long(&mut self, taskid: u32) {
        let mut task = Task::new(taskid, 12);
        task.cgi = format!("/cgi-bin/{taskid}");
        task.channel_select = Task::CHANNEL_LONG;
        task.longlink_host_list = vec![LONG_HOST.to_string()];
        task.user_id = "user".to_string();
        task.retry_count = 1;
        task.total_timeout = 10 * 60 * 1000;
        assert!(self.logic.start_task_at(START, task));
    }

    /// What the host's link is in: the C++ asks the link, and a sample says.
    fn bring_up(&mut self, name: &str, status: LongLinkStatus) {
        let link = Arc::clone(
            self.logic
                .net_core()
                .expect("no net core")
                .long_link(name)
                .expect("no such link"),
        );
        link.lock().unwrap().set_status(status);
    }

    /// `__OnResponse` of a long-link read that came back.
    fn answered(&mut self, taskid: u32) -> Option<RespHandle> {
        let mut profile = pair("2.2.2.2", 443);
        profile.net_type = "wifi".to_string();
        let answer = marsrs_stn::longlink_task_manager::Response {
            name: MAIN.to_string(),
            err_type: ErrCmdType::Ok,
            err_code: 0,
            cmdid: 12,
            taskid,
            body: b"hello".to_vec(),
            profile,
        };
        self.logic
            .net_core()
            .expect("no net core")
            .longlink()
            .on_response_at(START + 100, answer)
    }

    /// A read that came back for no task at all, which is a push.
    fn pushed(&mut self) {
        let answer = marsrs_stn::longlink_task_manager::Response {
            name: MAIN.to_string(),
            err_type: ErrCmdType::Ok,
            err_code: 0,
            cmdid: 6,
            taskid: Task::INVALID_TASK_ID,
            body: b"push".to_vec(),
            profile: ConnectProfile::new(),
        };
        assert!(self
            .logic
            .net_core()
            .expect("no net core")
            .longlink()
            .on_response_at(START, answer)
            .is_none());
    }

    /// What the C++'s message queue thread would have done.
    fn run_pending(&mut self) {
        self.logic.run_pending_at(START + 100);
    }

    fn said(&self) -> Said {
        self.said.lock().unwrap().clone()
    }

    fn sent(&self) -> Vec<(String, u32, Vec<u8>)> {
        self.sent.lock().unwrap().clone()
    }
}

/// A connect that came back on this pair.
fn pair(ip: &str, port: u16) -> ConnectProfile {
    let mut profile = ConnectProfile::new();
    profile.ip = ip.to_string();
    profile.port = port;
    profile
}

#[test]
fn a_task_goes_out_with_the_body_the_app_wrote_and_ends_with_what_the_app_read() {
    let mut host = Host::new();
    host.write(Some(b"/cgi-bin/7"));
    host.bring_up(MAIN, LongLinkStatus::Connected);

    host.start_long(7);
    // the app was asked for the body, and told the host the task is going to
    assert_eq!(
        host.sent(),
        vec![(MAIN.to_string(), 7, b"/cgi-bin/7".to_vec())]
    );
    let said = host.said();
    assert_eq!(said.encoded.len(), 1);
    assert_eq!(said.encoded[0].0, 7);
    assert_eq!(said.encoded[0].1, LONG_HOST);
    assert!(
        said.ended.is_empty(),
        "a task that is out is not one that ended"
    );

    // the answer comes back: the app reads it, and it is the app that ends the
    // task
    assert_eq!(host.answered(7), Some(RespHandle::Ended));
    assert!(!host.logic.has_task(7));
    host.run_pending();

    let said = host.said();
    assert_eq!(said.decoded, vec![(7, b"hello".to_vec())]);
    // the app is handed the connect the answer came in on, not the channel's
    assert_eq!(said.ended, vec![(7, ErrCmdType::Ok, 0, "wifi".to_string())]);
}

#[test]
fn a_task_the_app_cannot_encode_is_one_that_ends_at_once() {
    let mut host = Host::new();
    host.write(None);
    host.bring_up(MAIN, LongLinkStatus::Connected);

    host.start(7);
    host.run_pending();

    assert_eq!(host.sent(), Vec::new(), "nothing went out");
    assert_eq!(
        host.said().ended,
        // the connect the channel has, which is the one the task never used
        vec![(7, ErrCmdType::EnDecode, -300, String::new())]
    );
}

#[test]
fn an_answer_the_app_refused_ends_the_task_with_what_the_app_said() {
    let mut host = Host::new();
    host.write(Some(b"/cgi-bin/7"));
    host.bring_up(MAIN, LongLinkStatus::Connected);
    host.read(-500, TaskFailHandleType::TaskEnd);

    host.start_long(7);
    assert_eq!(host.answered(7), Some(RespHandle::Ended));
    assert!(!host.logic.has_task(7));

    // the app read the answer, and what it said about it is what the task ends
    // with — not the server's `kEctOK`
    assert_eq!(host.said().decoded, vec![(7, b"hello".to_vec())]);
    assert_eq!(
        host.said().ended,
        vec![(7, ErrCmdType::EnDecode, -500, "wifi".to_string())]
    );
}

#[test]
fn a_task_the_app_named_a_short_host_for_is_asked_the_same_two_questions() {
    let mut host = Host::new();
    host.write(Some(b"/cgi-bin/9"));

    host.start_short(9);

    // the app was asked whether it is logged in, and for the body — on the
    // short host it named, not on the long one
    let said = host.said();
    assert_eq!(
        said.authed,
        vec![(SHORT_HOST.to_string(), "user".to_string())]
    );
    assert_eq!(said.encoded.len(), 1);
    assert_eq!(said.encoded[0].0, 9);
    assert_eq!(said.encoded[0].1, SHORT_HOST);
    assert_eq!(host.sent(), vec![("short".to_string(), 9, Vec::new())]);
}

#[test]
fn a_task_that_needs_the_app_logged_in_is_asked_before_it_goes_out() {
    let mut host = Host::new();
    host.write(Some(b"/cgi-bin/7"));
    host.authed(false);
    host.bring_up(MAIN, LongLinkStatus::Connected);

    let mut task = Task::new(7, 12);
    task.cgi = "/cgi-bin/7".to_string();
    task.channel_select = Task::CHANNEL_ALL;
    task.longlink_host_list = vec![LONG_HOST.to_string()];
    task.user_id = "user".to_string();
    task.need_authed = true;
    assert!(host.logic.start_task_at(START, task));

    // the app said it is not logged in, so nothing went out and the task is
    // still there
    assert_eq!(host.sent(), Vec::new());
    assert!(host.logic.has_task(7));
    assert_eq!(
        host.said().authed,
        vec![(LONG_HOST.to_string(), "user".to_string())]
    );

    // and once it is, the task goes out
    host.authed(true);
    host.logic.touch_tasks_at(START + 1);
    assert_eq!(
        host.sent(),
        vec![(MAIN.to_string(), 7, b"/cgi-bin/7".to_vec())]
    );
}

#[test]
fn a_push_the_server_sent_is_the_apps() {
    let mut host = Host::new();

    host.pushed();

    assert_eq!(
        host.said().pushed,
        vec![(MAIN.to_string(), 6, b"push".to_vec())]
    );
}

#[test]
fn a_long_link_the_app_makes_is_marked_main_and_destroyed() {
    let mut host = Host::new();
    host.bring_up(MAIN, LongLinkStatus::Connected);

    let second = host
        .logic
        .create_long_link(LonglinkConfig::new("second"))
        .expect("a link the factory makes");
    second.lock().unwrap().set_status(LongLinkStatus::Connected);
    assert_eq!(host.logic.default_link(), Some(MAIN));
    assert!(host.logic.is_long_link_connected("second"));

    assert!(host.logic.mark_main_longlink("second"));
    assert_eq!(host.logic.default_link(), Some("second"));
    assert!(host.logic.is_default_long_link_connected());

    // a task with no channel of its own goes out on the one the app marked
    host.write(Some(b"/cgi-bin/8"));
    host.start(8);
    assert_eq!(host.sent()[0].0, "second");

    // and the link the app destroys takes the task that was out on it with it
    assert!(host.logic.destroy_long_link_at(START + 100, "second"));
    assert_eq!(host.logic.default_link(), None);
    assert!(!host.logic.has_task(8));
    assert_eq!(host.said().ended[0].0, 8);
    assert_eq!(host.said().ended[0].1, ErrCmdType::Local);
}

#[test]
fn a_reset_throws_every_task_away() {
    let mut host = Host::new();
    host.write(Some(b"/cgi-bin/7"));
    host.bring_up(MAIN, LongLinkStatus::Connected);
    host.start(7);
    assert!(host.logic.has_task(7));

    host.logic.reset_at(START + 100);

    assert!(!host.logic.has_task(7));
    assert!(host.logic.is_created(), "the core is made again");
    // and the app is still the one STN talks to
    host.write(Some(b"/cgi-bin/9"));
    assert!(host
        .logic
        .bridge()
        .lock()
        .unwrap()
        .makesure_authed(LONG_HOST, "user"));
}

#[test]
fn the_addresses_the_app_set_are_the_ones_the_net_source_keeps() {
    let mut host = Host::new();

    host.logic
        .set_longlink_svr_addr(LONG_HOST, vec![80, 443], "");
    host.logic.set_shortlink_svr_addr(8080, "");
    host.logic.set_debug_ip(LONG_HOST, "9.9.9.9");
    host.logic
        .set_backup_ips(LONG_HOST, vec!["8.8.8.8".to_string()]);

    assert_eq!(host.logic.long_link_hosts(), vec![LONG_HOST.to_string()]);
    let core = host.logic.net_core().expect("no net core");
    assert_eq!(core.net_source().longlink_ports(), vec![80, 443]);
    assert_eq!(core.net_source().shortlink_port(), 8080);
    assert_eq!(core.net_source().backup_ips(LONG_HOST), vec!["8.8.8.8"]);
}

#[test]
fn what_the_app_is_told_about_the_connection() {
    let mut host = Host::new();

    host.logic
        .net_core()
        .expect("no net core")
        .on_longlink_status_changed_at(START, LongLinkStatus::Connecting);
    host.bring_up(MAIN, LongLinkStatus::Connected);
    host.logic
        .net_core()
        .expect("no net core")
        .on_longlink_status_changed_at(START, LongLinkStatus::Connected);

    assert_eq!(
        host.said().status,
        vec![
            (NetStatus::Connecting, NetStatus::Connecting),
            (NetStatus::Connected, NetStatus::Connected),
        ]
    );
}

#[test]
fn a_task_that_ended_is_reported_with_the_cgi_profile_of_the_connect_it_ran_on() {
    let mut host = Host::new();
    host.write(Some(b"/cgi-bin/7"));
    host.bring_up(MAIN, LongLinkStatus::Connected);
    host.start_long(7);

    let mut profile = pair("2.2.2.2", 443);
    profile.start_send_packet_time = 140;
    profile.send_request_cost = 6;
    profile.conn_rtt = 40;
    profile.net_type = "wifi".to_string();
    let answer = marsrs_stn::longlink_task_manager::Response {
        name: MAIN.to_string(),
        err_type: ErrCmdType::Ok,
        err_code: 0,
        cmdid: 12,
        taskid: 7,
        body: b"hello".to_vec(),
        profile: profile.clone(),
    };
    assert_eq!(
        host.logic
            .net_core()
            .expect("no net core")
            .longlink()
            .on_response_at(START + 100, answer),
        Some(RespHandle::Ended)
    );

    // what the app is handed is the C++'s own conversion of the connect
    assert_eq!(host.said().profiles, vec![Cgi::of(&profile)]);
    assert_eq!(
        host.said().profiles[0].send_packet_finished_time,
        146,
        "start plus the cost"
    );
    assert_eq!(host.said().profiles[0].rtt, 40);
    assert_eq!(host.said().profiles[0].nettype, "wifi");
}

#[test]
fn the_app_is_told_the_channel_the_task_is_going_out_on() {
    let mut host = Host::new();
    host.write(Some(b"/cgi-bin/9"));

    // no long link is up, so a task that may use either goes out on the short
    // link: the app is told `kChannelShort`, which is what the C++ hands for a
    // short link, and not the `kChannelBoth` the task was made with
    let mut task = Task::new(9, 12);
    task.cgi = "/cgi-bin/9".to_string();
    task.channel_select = Task::CHANNEL_BOTH;
    task.shortlink_host_list = vec![SHORT_HOST.to_string()];
    task.user_id = "user".to_string();
    assert!(host.logic.start_task_at(START, task));
    assert_eq!(host.said().encoded_on, vec![(9, Task::CHANNEL_SHORT)]);

    // and the long link hands its own: a task that may use anything, which is
    // the `kChannelAll` the app made it with, is told `kChannelLong`
    host.bring_up(MAIN, LongLinkStatus::Connected);
    host.write(Some(b"/cgi-bin/8"));
    host.start(8);
    assert_eq!(host.said().encoded_on[1], (8, Task::CHANNEL_LONG));

    // the answer comes back on the same channel the request went out on
    assert_eq!(host.answered(8), Some(RespHandle::Ended));
    assert_eq!(host.said().decoded_on, vec![(8, Task::CHANNEL_LONG)]);
}

#[test]
fn a_task_that_may_use_anything_is_reported_with_no_connect() {
    let mut host = Host::new();
    host.write(Some(b"/cgi-bin/7"));
    host.bring_up(MAIN, LongLinkStatus::Connected);
    // `kChannelAll`, which is what `Host::start` asks for
    host.start(7);
    assert_eq!(host.answered(7), Some(RespHandle::Ended));

    // the C++ asks the queue `channel_select` names, and `kChannelAll` names
    // none of them: the app is told about no connect at all, whatever the link
    // the task went out on made of it
    assert_eq!(
        host.said().ended,
        vec![(7, ErrCmdType::Ok, 0, String::new())]
    );
}

/// The end of a task an app awaited, which a sample drives itself: the port
/// carries no executor, and what wakes a [`sent::Sent`] is the pass the host
/// makes.
fn settle(host: &mut Host, awaited: sent::Sent) -> Result<Answer, Failure> {
    let mut awaited = pin!(awaited);
    let mut context = Context::from_waker(Waker::noop());
    for _ in 0..8 {
        if let Poll::Ready(outcome) = awaited.as_mut().poll(&mut context) {
            return outcome;
        }
        // a pass is what ends the task, and what wakes whoever awaited it
        host.run_pending();
    }
    panic!("a task nothing drained is one that never ended");
}

/// The end of a task an app awaited, when there is nothing to wait for: one
/// poll, which is all a [`sent::Sent`] that is already over needs.
fn settled(awaited: sent::Sent) -> Result<Answer, Failure> {
    let mut awaited = pin!(awaited);
    let mut context = Context::from_waker(Waker::noop());
    match awaited.as_mut().poll(&mut context) {
        Poll::Ready(outcome) => outcome,
        // a task no queue took is not one an app is left waiting on
        Poll::Pending => panic!("a task that never started ended without a pass"),
    }
}

/// A waker that counts: what an executor does with a wake is poll again, so
/// a call that ends a task is a call that wakes whoever awaited it, and the
/// count is the only thing that says so — polling again finds the end either
/// way.
struct Counting(AtomicUsize);

impl Wake for Counting {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// A task an app awaits, on the long link, with the body the app handed.
fn awaited(host: &mut Host, taskid: u32) -> sent::Sent {
    let mut task = Task::new(taskid, 12);
    task.cgi = format!("/cgi-bin/{taskid}");
    task.channel_select = Task::CHANNEL_LONG;
    task.longlink_host_list = vec![LONG_HOST.to_string()];
    task.user_id = "user".to_string();
    task.retry_count = 1;
    task.total_timeout = 10 * 60 * 1000;
    host.logic
        .send_at(START, task, format!("/cgi-bin/{taskid}").into_bytes())
}

#[test]
fn an_awaited_task_ends_with_the_body_the_server_answered_with() {
    let mut host = Host::new();
    host.bring_up(MAIN, LongLinkStatus::Connected);

    let awaited = awaited(&mut host, 7);
    // the body the app handed is the one that went out, and neither of the two
    // questions a request and an answer are asked was asked of the app
    assert_eq!(
        host.sent(),
        vec![(MAIN.to_string(), 7, b"/cgi-bin/7".to_vec())]
    );
    let said = host.said();
    assert_eq!(said.encoded, Vec::new(), "no `Req2Buf` was asked of it");
    assert!(
        said.ended.is_empty(),
        "a task that is out is not one that ended"
    );

    let _ended = host.answered(7);
    let answer = settle(&mut host, awaited).expect("the task came back with one");
    assert_eq!(answer.body, b"hello".to_vec());
    assert_eq!(answer.profile.nettype, "wifi");

    let said = host.said();
    assert_eq!(said.decoded, Vec::new(), "no `Buf2Resp` was asked of it");
    // and the app still hears the end: what an app awaits is a value, and not
    // a question the app stops being asked
    assert_eq!(said.ended, vec![(7, ErrCmdType::Ok, 0, "wifi".to_string())]);
}

#[test]
fn an_awaited_task_that_failed_ends_with_where_it_failed() {
    let mut host = Host::new();
    host.bring_up(MAIN, LongLinkStatus::Connected);
    let awaited = awaited(&mut host, 7);
    // the link goes down, which fails every task that was out on it
    assert!(host.logic.destroy_long_link_at(START + 100, MAIN));

    match settle(&mut host, awaited).expect_err("the task came back with none") {
        Failure::Ended {
            err_type, err_code, ..
        } => {
            assert_eq!(err_type, ErrCmdType::Local);
            assert!(err_code < 0, "a task that failed says how: {err_code}");
        }
        other => panic!("the task failed, and not like this: {other:?}"),
    }
}

/// `DestroyLonglink_ext` takes every task of the link with it, and those are
/// ended inside the call — so the call is what wakes whoever awaited one of
/// them, and not the next pass. An app's executor polls when it is woken and
/// not on a timer, and a host whose [`StnLogic::due_delay`] came back `None`
/// makes no pass at all.
#[test]
fn a_task_the_destroyed_link_took_with_it_wakes_whoever_awaited_it() {
    let mut host = Host::new();
    host.bring_up(MAIN, LongLinkStatus::Connected);
    let awaited = awaited(&mut host, 7);

    let counting = Arc::new(Counting(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&counting));
    let mut awaited = pin!(awaited);
    let mut context = Context::from_waker(&waker);

    assert!(
        awaited.as_mut().poll(&mut context).is_pending(),
        "a task that is out is not one that ended"
    );
    assert_eq!(
        counting.0.load(Ordering::SeqCst),
        0,
        "nothing has ended it yet"
    );

    assert!(host.logic.destroy_long_link_at(START + 100, MAIN));
    assert!(!host.logic.has_task(7));
    assert!(
        counting.0.load(Ordering::SeqCst) > 0,
        "no pass is going to end a task no queue holds"
    );

    match awaited.as_mut().poll(&mut context) {
        Poll::Ready(Err(Failure::Ended {
            err_type, err_code, ..
        })) => {
            assert_eq!(err_type, ErrCmdType::Local);
            assert!(err_code < 0, "the link that went away is why: {err_code}");
        }
        other => panic!("the task ended with the link, and not like this: {other:?}"),
    }
}

#[test]
fn a_task_no_queue_took_ends_before_the_app_awaits_it() {
    // no net core, so nothing to start it on
    let mut logic = StnLogic::new();
    let mut task = Task::new(7, 12);
    task.channel_select = Task::CHANNEL_LONG;
    assert_eq!(
        settled(logic.send_at(START, task, Vec::new())),
        Err(Failure::NotCreated)
    );

    // a net core, and a task with no channel to go out on: the core refuses it
    // and reports the end itself, so there is nothing to wait for
    let mut host = Host::new();
    let mut task = Task::new(8, 12);
    task.channel_select = 0;
    match settled(host.logic.send_at(START, task, Vec::new())) {
        Err(Failure::Ended {
            err_type, err_code, ..
        }) => {
            assert_eq!(err_type, ErrCmdType::Local);
            assert_eq!(err_code, LOCAL_CHANNEL_SELECT);
        }
        other => panic!("the core refused it, and not like this: {other:?}"),
    }

    // and a core that was released: no task goes out, and no end is reported
    let mut host = Host::new();
    host.logic.net_core().expect("no net core").release();
    let awaited = host.logic.send_at(START, Task::new(9, 12), Vec::new());
    assert_eq!(settled(awaited), Err(Failure::Refused));
}

/// `StopTask` — the queue drops the task and reports nothing, which in the C++
/// is the whole of it. Here an app may be awaiting the task, and nothing is
/// ever going to report it again, so the stop is what answers it.
#[test]
fn a_task_the_app_stopped_is_answered_as_cancelled() {
    let mut host = Host::new();
    host.bring_up(MAIN, LongLinkStatus::Connected);
    let awaited = awaited(&mut host, 7);

    assert!(
        host.logic.stop_task(7),
        "the long link's queue had the task"
    );
    assert!(!host.logic.has_task(7));

    // the app is not asked about a task it broke off itself, so what ends the
    // await is the stop and not a pass
    assert!(host.said().ended.is_empty());
    assert_cancelled(settled(awaited));
}

/// `ClearTasks` and `OnDestroy` throw the queues away with everything on them,
/// and [`StnLogic`] outlives both: every task still being awaited is answered
/// here or not at all.
#[test]
fn tasks_a_core_threw_away_are_answered_as_cancelled() {
    let mut host = Host::new();
    host.bring_up(MAIN, LongLinkStatus::Connected);
    let first = awaited(&mut host, 7);
    let second = awaited(&mut host, 8);
    host.logic.clear_tasks();
    assert!(!host.logic.has_task(7));
    assert!(!host.logic.has_task(8));
    assert_cancelled(settled(first));
    assert_cancelled(settled(second));

    // and a core the app destroyed takes every queue with it
    let mut host = Host::new();
    host.bring_up(MAIN, LongLinkStatus::Connected);
    let awaited = awaited(&mut host, 9);
    assert!(host.logic.destroy());
    assert_cancelled(settled(awaited));
}

/// `ReleaseNet` leaves the logic created and empties every queue, so an await
/// of a task it threw away has to be answered here too: a released core ends
/// nothing, and a pass never comes for it again.
#[test]
fn tasks_a_released_core_threw_away_are_answered_as_cancelled() {
    let mut host = Host::new();
    host.bring_up(MAIN, LongLinkStatus::Connected);
    let awaited = awaited(&mut host, 7);
    host.logic.release();
    assert!(host.logic.is_created(), "the core is still there");
    assert!(!host.logic.has_task(7));
    assert_cancelled(settled(awaited));
}

/// What a task that was dropped rather than run out ends with: `kEctCanceld`,
/// and no error code of its own.
fn assert_cancelled(outcome: Result<Answer, Failure>) {
    match outcome {
        Err(Failure::Ended {
            err_type, err_code, ..
        }) => {
            assert_eq!(err_type, ErrCmdType::Canceld);
            assert_eq!(err_code, 0);
        }
        other => panic!("the task was cancelled, and not like this: {other:?}"),
    }
}
