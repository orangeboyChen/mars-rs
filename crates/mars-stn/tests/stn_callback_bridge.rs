//! `mars/stn/stn_callback_bridge.cc`, through the public api.
//!
//! The samples are what the C++ does to the same calls: a task that ended and
//! the app's report of the connect it ran on, a body the app wrote for a task
//! that is going out, an answer the app read, a push and a connection the app
//! is told about, an error that is fanned out to the listeners before the app
//! hears it, and every question STN asks an app that is not there yet.

use std::sync::{Arc, Mutex};

use mars_stn::{
    App, CgiProfile, ConnectProfile, DnsProfile, DnsType, ErrCmdType, IdentifyBuffer,
    LongLinkStatus, NetStatus, PrepareProfile, StnCallbackBridge, Task, TaskFailHandleType,
    TaskProfile,
};

/// Everything the app was asked, as one value a sample reads.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Said {
    pushed: Vec<(String, u32, u32, Vec<u8>)>,
    encoded: Vec<(u32, String, u16)>,
    decoded: Vec<(u32, Vec<u8>)>,
    ended: Vec<(u32, ErrCmdType, i32, CgiProfile)>,
    status: Vec<(NetStatus, NetStatus)>,
    long_err: Vec<(ErrCmdType, i32, String, u16)>,
    short_err: Vec<(ErrCmdType, i32, String, String, u16)>,
    link_status: Vec<LongLinkStatus>,
    identify: Vec<(String, u32)>,
    identify_response: Vec<(String, Vec<u8>)>,
    sync: usize,
    hosts: Vec<Vec<String>>,
    limited: Vec<(i32, u32)>,
    profiles: usize,
    dns: Vec<String>,
    authed: Vec<(String, String)>,
    traffic: Vec<(i64, i64)>,
    new_dns: Vec<(String, bool)>,
}

/// An app that answers every question STN asks and writes down what it was
/// asked.
struct Rec {
    said: Arc<Mutex<Said>>,
}

impl App for Rec {
    fn makesure_authed(&mut self, host: &str, user_id: &str) -> bool {
        self.said
            .lock()
            .unwrap()
            .authed
            .push((host.to_string(), user_id.to_string()));
        true
    }

    fn traffic_data(&mut self, send: i64, recv: i64) {
        self.said.lock().unwrap().traffic.push((send, recv));
    }

    fn on_new_dns(
        &mut self,
        host: &str,
        longlink_host: bool,
        _extra: &mars_stn::ExtraInfo,
    ) -> Vec<String> {
        self.said
            .lock()
            .unwrap()
            .new_dns
            .push((host.to_string(), longlink_host));
        vec![format!("10.0.0.{longlink_host}")]
    }

    fn on_push(&mut self, channel_id: &str, cmdid: u32, taskid: u32, body: &[u8]) {
        self.said.lock().unwrap().pushed.push((
            channel_id.to_string(),
            cmdid,
            taskid,
            body.to_vec(),
        ));
    }

    fn req2buf(
        &mut self,
        taskid: u32,
        _user_id: &str,
        _channel_select: i32,
        host: &str,
        sequence: u16,
    ) -> Result<Vec<u8>, i32> {
        self.said
            .lock()
            .unwrap()
            .encoded
            .push((taskid, host.to_string(), sequence));
        Ok(format!("/cgi-bin/{taskid}").into_bytes())
    }

    fn buf2resp(
        &mut self,
        taskid: u32,
        _user_id: &str,
        body: &[u8],
        _channel_select: i32,
    ) -> (i32, TaskFailHandleType) {
        self.said
            .lock()
            .unwrap()
            .decoded
            .push((taskid, body.to_vec()));
        (-1, TaskFailHandleType::TaskTimeout)
    }

    fn on_task_end(
        &mut self,
        taskid: u32,
        _user_id: &str,
        err_type: ErrCmdType,
        err_code: i32,
        profile: &CgiProfile,
    ) -> i32 {
        self.said
            .lock()
            .unwrap()
            .ended
            .push((taskid, err_type, err_code, profile.clone()));
        err_code
    }

    fn report_connect_status(&mut self, all: NetStatus, longlink: NetStatus) {
        self.said.lock().unwrap().status.push((all, longlink));
    }

    fn on_long_link_network_error(
        &mut self,
        err_type: ErrCmdType,
        err_code: i32,
        ip: &str,
        port: u16,
    ) {
        self.said
            .lock()
            .unwrap()
            .long_err
            .push((err_type, err_code, ip.to_string(), port));
    }

    fn on_short_link_network_error(
        &mut self,
        err_type: ErrCmdType,
        err_code: i32,
        ip: &str,
        host: &str,
        port: u16,
    ) {
        self.said.lock().unwrap().short_err.push((
            err_type,
            err_code,
            ip.to_string(),
            host.to_string(),
            port,
        ));
    }

    fn on_long_link_status_change(&mut self, status: LongLinkStatus) {
        self.said.lock().unwrap().link_status.push(status);
    }

    fn identify_check_buffer(&mut self, channel_id: &str, cmdid: u32) -> IdentifyBuffer {
        self.said
            .lock()
            .unwrap()
            .identify
            .push((channel_id.to_string(), cmdid));
        IdentifyBuffer::now(b"check".to_vec(), b"hash".to_vec(), cmdid)
    }

    fn identify_response(&mut self, channel_id: &str, response: &[u8], _hash: &[u8]) -> bool {
        self.said
            .lock()
            .unwrap()
            .identify_response
            .push((channel_id.to_string(), response.to_vec()));
        true
    }

    fn request_sync(&mut self) {
        self.said.lock().unwrap().sync += 1;
    }

    fn net_check_shortlink_hosts(&mut self) -> Vec<String> {
        self.said
            .lock()
            .unwrap()
            .hosts
            .push(vec!["check.host".to_string()]);
        vec!["check.host".to_string()]
    }

    fn report_task_profile(&mut self, _profile: &TaskProfile) {
        self.said.lock().unwrap().profiles += 1;
    }

    fn report_task_limited(&mut self, check_type: i32, _task: &Task) -> u32 {
        self.said.lock().unwrap().limited.push((check_type, 1));
        1
    }

    fn report_dns_profile(&mut self, profile: &DnsProfile) {
        self.said.lock().unwrap().dns.push(profile.host.clone());
    }
}

/// A bridge with the app in it, and the app's own book.
fn bridged() -> (StnCallbackBridge, Arc<Mutex<Said>>) {
    let said = Arc::new(Mutex::new(Said::default()));
    let mut bridge = StnCallbackBridge::new();
    bridge.set_callback(Rec {
        said: Arc::clone(&said),
    });
    (bridge, said)
}

fn said_of(cell: &Arc<Mutex<Said>>) -> Said {
    cell.lock().unwrap().clone()
}

/// A connect that wrote and read, on a short link over wifi.
fn connect() -> ConnectProfile {
    let mut profile = ConnectProfile::new();
    profile.net_type = "wifi".to_string();
    profile.channel_type = Task::CHANNEL_SHORT;
    profile.transport_protocol = Task::TRANSPORT_PROTOCOL_TCP;
    profile.ip = "1.2.3.4".to_string();
    profile.port = 8080;
    profile.start_time = 100;
    profile.start_connect_time = 110;
    profile.connect_successful_time = 130;
    profile.start_send_packet_time = 140;
    profile.send_request_cost = 6;
    profile.start_read_packet_time = 150;
    profile.read_packet_finished_time = 190;
    profile.conn_rtt = 40;
    profile
}

#[test]
fn a_task_that_ended_is_reported_with_the_connect_it_ran_on() {
    let (mut bridge, said) = bridged();

    assert_eq!(
        bridge.on_task_end(7, "user", ErrCmdType::Ok, 0, &connect()),
        0
    );

    let said = said_of(&said);
    assert_eq!(said.ended.len(), 1);
    let (taskid, err_type, err_code, cgi) = &said.ended[0];
    assert_eq!(*taskid, 7);
    assert_eq!(*err_type, ErrCmdType::Ok);
    assert_eq!(*err_code, 0);
    // the app is told how far the run got, in the C++'s own words
    assert_eq!(cgi.start_time, 100);
    assert_eq!(cgi.connect_successful_time, 130);
    assert_eq!(cgi.start_send_packet_time, 140);
    assert_eq!(cgi.send_packet_finished_time, 146, "start plus the cost");
    assert_eq!(cgi.read_packet_finished_time, 190);
    assert_eq!(cgi.channel_type, Task::CHANNEL_SHORT);
    assert_eq!(cgi.rtt, 40);
    assert_eq!(cgi.nettype, "wifi");
}

#[test]
fn a_body_the_app_wrote_is_what_goes_out_and_an_answer_it_read_is_what_ends_it() {
    let (mut bridge, said) = bridged();

    assert_eq!(
        bridge.req2buf(7, "user", Task::CHANNEL_SHORT, "short.host", 3),
        Ok(b"/cgi-bin/7".to_vec())
    );
    assert_eq!(
        bridge.buf2resp(7, "user", b"answer", Task::CHANNEL_SHORT),
        (-1, TaskFailHandleType::TaskTimeout)
    );

    let said = said_of(&said);
    assert_eq!(
        said.encoded,
        vec![(7, "short.host".to_string(), 3)],
        "the app is told the host the task is going to"
    );
    assert_eq!(said.decoded, vec![(7, b"answer".to_vec())]);
}

#[test]
fn a_push_and_a_connection_are_the_apps_business() {
    let (mut bridge, said) = bridged();

    bridge.on_push("long.weixin.qq.com", 6, 0, b"push");
    bridge.report_connect_status(NetStatus::Connected, NetStatus::Connected);
    bridge.on_long_link_status_change(LongLinkStatus::Connected);

    let said = said_of(&said);
    assert_eq!(
        said.pushed,
        vec![("long.weixin.qq.com".to_string(), 6, 0, b"push".to_vec())]
    );
    assert_eq!(
        said.status,
        vec![(NetStatus::Connected, NetStatus::Connected)]
    );
    assert_eq!(said.link_status, vec![LongLinkStatus::Connected]);
}

#[test]
fn an_error_goes_to_the_listeners_before_it_goes_to_the_app() {
    let (mut bridge, said) = bridged();
    let order: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let heard = Arc::clone(&order);
    bridge.add_long_link_error_listener(move |err_type, err_code, ip, port| {
        heard
            .lock()
            .unwrap()
            .push(format!("long {err_type:?} {err_code} {ip}:{port}"));
    });
    let heard = Arc::clone(&order);
    bridge.add_short_link_error_listener(move |err_type, err_code, ip, host, port| {
        heard
            .lock()
            .unwrap()
            .push(format!("short {err_type:?} {err_code} {ip} {host}:{port}"));
    });
    assert_eq!(bridge.error_listener_count(), 2);

    bridge.on_long_link_network_error(ErrCmdType::Socket, -10091, "1.2.3.4", 8080);
    bridge.on_short_link_network_error(ErrCmdType::Http, -500, "1.2.3.4", "short.host", 80);

    assert_eq!(
        order.lock().unwrap().clone(),
        vec![
            "long Socket -10091 1.2.3.4:8080".to_string(),
            "short Http -500 1.2.3.4 short.host:80".to_string(),
        ]
    );
    // and then the app, which is the C++'s order
    let said = said_of(&said);
    assert_eq!(
        said.long_err,
        vec![(ErrCmdType::Socket, -10091, "1.2.3.4".to_string(), 8080)]
    );
    assert_eq!(
        said.short_err,
        vec![(
            ErrCmdType::Http,
            -500,
            "1.2.3.4".to_string(),
            "short.host".to_string(),
            80
        )]
    );
}

#[test]
fn the_check_the_app_answers_and_the_sync_it_is_asked_for() {
    let (mut bridge, said) = bridged();

    assert_eq!(
        bridge.identify_check_buffer("long.weixin.qq.com", 6),
        IdentifyBuffer::now(b"check".to_vec(), b"hash".to_vec(), 6)
    );
    assert!(bridge.identify_response("long.weixin.qq.com", b"resp", b"hash"));
    bridge.request_sync();

    let said = said_of(&said);
    assert_eq!(said.identify, vec![("long.weixin.qq.com".to_string(), 6)]);
    assert_eq!(
        said.identify_response,
        vec![("long.weixin.qq.com".to_string(), b"resp".to_vec())]
    );
    assert_eq!(said.sync, 1);
}

#[test]
fn a_dns_question_and_a_limit_and_a_task_the_app_is_told_about() {
    let (mut bridge, said) = bridged();

    assert_eq!(
        bridge.on_new_dns("short.host", false, &mars_stn::ExtraInfo::new()),
        vec!["10.0.0.false".to_string()]
    );
    assert_eq!(
        bridge.net_check_shortlink_hosts(),
        vec!["check.host".to_string()]
    );
    bridge.report_task_profile(&TaskProfile::new(Task::new(7, 12), PrepareProfile::new()));
    assert_eq!(bridge.report_task_limited(1, &Task::new(7, 12)), 1);

    let mut dns = DnsProfile::new("short.host");
    dns.dnstype = DnsType::Dns;
    dns.end_time = 200;
    bridge.report_dns_profile(&dns);

    let said = said_of(&said);
    assert_eq!(said.new_dns, vec![("short.host".to_string(), false)]);
    assert_eq!(said.hosts, vec![vec!["check.host".to_string()]]);
    assert_eq!(said.profiles, 1);
    assert_eq!(said.limited, vec![(1, 1)]);
    assert_eq!(said.dns, vec!["short.host".to_string()]);
}

#[test]
fn an_app_that_is_not_there_yet_is_answered_for() {
    let mut bridge = StnCallbackBridge::new();
    assert!(!bridge.has_callback());
    assert!(bridge.callback().is_none());

    // STN runs without an app, and every question has an answer it does not
    // need the app for
    assert!(bridge.makesure_authed("short.host", "user"));
    assert_eq!(
        bridge.on_new_dns("short.host", true, &mars_stn::ExtraInfo::new()),
        Vec::<String>::new()
    );
    assert_eq!(
        bridge.buf2resp(7, "user", b"answer", Task::CHANNEL_SHORT),
        (0, TaskFailHandleType::Normal)
    );
    assert_eq!(
        bridge.on_task_end(7, "user", ErrCmdType::Ok, 0, &connect()),
        0
    );
    assert_eq!(bridge.net_check_shortlink_hosts(), Vec::<String>::new());
    assert_eq!(bridge.report_task_limited(1, &Task::new(7, 12)), 0);
    assert!(!bridge.identify_response("channel", b"resp", b"hash"));
    // a check nobody answered is put off to the next connect, not failed
    assert_eq!(
        bridge.identify_check_buffer("channel", 6),
        IdentifyBuffer::next(Vec::new())
    );
    // but a task nobody can encode is one that cannot be started
    assert!(bridge
        .req2buf(7, "user", Task::CHANNEL_SHORT, "short.host", 1)
        .is_err());

    // the app that comes along later is the one that answers
    let said = Arc::new(Mutex::new(Said::default()));
    bridge.set_callback(Rec {
        said: Arc::clone(&said),
    });
    assert!(bridge.has_callback());
    assert!(bridge.makesure_authed("short.host", "user"));
    assert_eq!(
        said_of(&said).authed,
        vec![("short.host".to_string(), "user".to_string())]
    );
}
