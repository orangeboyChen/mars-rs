//! `mars/stn/jni/com_tencent_mars_stn_StnLogic_C2Java.cc` — the app STN asks,
//! when the app is Java.
//!
//! The C++'s C2Java file is thirteen functions, one for every question STN asks
//! the app, and each of them does the same three things: attach the thread to
//! the VM, call one static method of `com/tencent/mars/stn/StnLogic` (which
//! forwards it to the `ICallBack` the app handed to `setCallBack`), and turn
//! what Java answered into the C++'s own types — a `std::vector<std::string>`
//! out of a `String[]`, an `AutoBuffer` out of a `ByteArrayOutputStream`, a
//! fail handle out of an `int`.
//!
//! Here the thirteen are one [`Question`] and one [`Answer`]: what the C++'s
//! out-parameters (`int& _error_code`, `AutoBuffer& _outbuffer`,
//! `unsigned short& server_sequence_id`) carry back is part of the answer
//! instead, and [`JavaApp`] — one [`App`] — is all that stands between STN and
//! Java. The JVM half is [`crate::jni_bridge`]: one hook that answers a
//! [`Question`] by calling Java, which is what the C++'s `VarCache`,
//! `ScopeJEnv` and `JNU_CallStaticMethodByMethodInfo` are together.
//!
//! Not ported: `user_context` — the `Object` the Java api hands back — is
//! `null`, because a port has nothing to put in it; `isLogoned` (nothing in the
//! port asks the app whether it is logged in); `OnLongLinkNetworkError`,
//! `OnShortLinkNetworkError`, `OnLongLinkStatusChange`, `ReportTaskLimited` and
//! `ReportDnsProfile` (the Java api has no method for them, so the C++'s C2Java
//! has none either: STN gets [`App`]'s own answers). Two readings of
//! `StnLogic$CgiProfile` are left at `0` — its `startHandshakeTime` and
//! `handshakeSuccessfulTime`, which the port's [`CgiProfile`] does not keep —
//! and one the Java class has no field for, `send_packet_finished_time`, is not
//! handed over either.

use mars_stn::{
    App, CgiProfile, ErrCmdType, ExtraInfo, IdentifyBuffer, NetStatus, TaskFailHandleType,
    TaskProfile, LOCAL_START_TASK_FAIL,
};

/// One of the thirteen questions: what STN asked, in the arguments the Java
/// method takes.
///
/// The C++ hands the app more than Java asks for — `user_id`, whether the host
/// is a long-link one, the `ExtraInfo` of the dns question — and Java's own api
/// takes none of it, so it is not handed over here either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    /// `makesureAuthed(String)` — the host, but not the user: the Java api has
    /// no user, and the C++ does not hand `_user_id` over either.
    MakesureAuthed {
        /// `host`
        host: String,
    },
    /// `trafficData(int, int)`.
    TrafficData {
        /// `_send`
        send: i64,
        /// `_recv`
        recv: i64,
    },
    /// `onNewDns(String)`.
    OnNewDns {
        /// `host`
        host: String,
    },
    /// `onPush(String, int, int, byte[])` — the C++ passes an `AutoBuffer`
    /// extension too, which Java's api does not take.
    OnPush {
        /// `channel_id`
        channel_id: String,
        /// `cmdid`
        cmdid: u32,
        /// `taskid`
        taskid: u32,
        /// `data`
        body: Vec<u8>,
    },
    /// `req2Buf(int, Object, ByteArrayOutputStream, int[], int, String, int)`.
    Req2Buf {
        /// `taskID`
        taskid: u32,
        /// `channelSelect`
        channel_select: i32,
        /// `host`
        host: String,
        /// `client_sequence_id`
        sequence: u16,
    },
    /// `buf2Resp(int, Object, byte[], int[], int, int[])`.
    Buf2Resp {
        /// `taskID`
        taskid: u32,
        /// `respBuffer`
        body: Vec<u8>,
        /// `channelSelect`
        channel_select: i32,
    },
    /// `onTaskEnd(int, Object, int, int, CgiProfile)`.
    OnTaskEnd {
        /// `taskID`
        taskid: u32,
        /// `errType`
        err_type: ErrCmdType,
        /// `errCode`
        err_code: i32,
        /// `profile`
        profile: CgiProfile,
    },
    /// `reportConnectStatus(int, int)`.
    ReportConnectStatus {
        /// `all_connstatus`
        all: NetStatus,
        /// `longlink_connstatus`
        longlink: NetStatus,
    },
    /// `getLongLinkIdentifyCheckBuffer(String, ByteArrayOutputStream,
    /// ByteArrayOutputStream, int[])`.
    ///
    /// What the check would go out with is not handed over: the Java api does
    /// not take it and the C++'s C2Java does not hand it over either — what
    /// Java writes into `reqRespCmdID[0]` is what goes out.
    IdentifyCheckBuffer {
        /// `channel_id`
        channel_id: String,
    },
    /// `onLongLinkIdentifyResp(String, byte[], byte[])`.
    IdentifyResponse {
        /// `channel_id`
        channel_id: String,
        /// `buffer` — what the server answered.
        response: Vec<u8>,
        /// `hashCodeBuffer` — the hash the app handed out.
        hash: Vec<u8>,
    },
    /// `requestDoSync()`.
    RequestSync,
    /// `requestNetCheckShortLinkHosts()`.
    NetCheckShortLinkHosts,
    /// `reportTaskProfile(String)` — the report, as the json the Java side
    /// reads, which [`task_profile_json`] writes.
    ReportTaskProfile {
        /// the json
        json: String,
    },
}

/// What Java answered, in the terms STN asks in.
///
/// A question nobody answered — no VM to attach to, a call that could not be
/// made — is [`Answer::Nothing`], and every answer read out of it
/// ([`Answer::yes`] and friends) is what STN takes when the app said nothing: a
/// task nobody can encode is one that cannot be started, a check that was never
/// answered is asked for again on the next connect.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Answer {
    /// `makesureAuthed`, `onLongLinkIdentifyResp`.
    Yes(bool),
    /// `trafficData`, `onPush`, `reportConnectStatus`, `requestDoSync`,
    /// `reportTaskProfile` — and Java that was not asked at all.
    #[default]
    Nothing,
    /// `onNewDns`, `requestNetCheckShortLinkHosts` — the ips, or the hosts, the
    /// app knows.
    Ips(Vec<String>),
    /// `req2Buf` — what to send, or the code the task ends with: the C++'s
    /// `false` and the `errCode[0]` the app filled.
    Encoded(Result<Vec<u8>, i32>),
    /// `buf2Resp` — how the app read the answer: the `int` it answered with is
    /// what STN is to do about the task, and `errCode[0]` is the code it is
    /// remembered with.
    Decoded {
        /// the fail handle Java answered with.
        handle: i32,
        /// `errCode[0]`
        err_code: i32,
    },
    /// `onTaskEnd` — the code the task is remembered with.
    Ended(i32),
    /// `getLongLinkIdentifyCheckBuffer` — when the buffer goes out, the buffer,
    /// the hash of it, and the cmdid to send it with.
    Identified {
        /// `ECHECK_NOW`, `ECHECK_NEXT` or `ECHECK_NEVER`.
        mode: i32,
        /// `identifyReqBuf`
        buffer: Vec<u8>,
        /// `hashCodeBuffer`
        hash: Vec<u8>,
        /// `reqRespCmdID[0]`
        cmdid: u32,
    },
}

impl Answer {
    /// `makesureAuthed` / `onLongLinkIdentifyResp` — `false` when the answer is
    /// not this one, which is what Java's own `StnLogic` returns for a callback
    /// that was never set.
    pub fn yes(&self) -> bool {
        match self {
            Self::Yes(yes) => *yes,
            _ => false,
        }
    }

    /// `onNewDns` / `requestNetCheckShortLinkHosts` — no ips when the answer is
    /// not this one, i.e. dns the platform's own resolver does.
    pub fn ips(&self) -> Vec<String> {
        match self {
            Self::Ips(ips) => ips.clone(),
            _ => Vec::new(),
        }
    }

    /// `req2Buf` — [`Err`] when the answer is not this one: a task nobody can
    /// encode is one that cannot be started, which is what
    /// [`mars_stn::StnCallbackBridge`] says for an app that is not there.
    pub fn encoded(&self) -> Result<Vec<u8>, i32> {
        match self {
            Self::Encoded(encoded) => encoded.clone(),
            _ => Err(LOCAL_START_TASK_FAIL),
        }
    }

    /// `buf2Resp` — `(0, Normal)` when the answer is not this one: an answer
    /// nobody read is a good one.
    pub fn decoded(&self) -> (i32, TaskFailHandleType) {
        match self {
            Self::Decoded { handle, err_code } => (*err_code, fail_handle(*handle)),
            _ => (0, TaskFailHandleType::Normal),
        }
    }

    /// `onTaskEnd` — `0` when the answer is not this one, which is "the net
    /// core is done with the task".
    pub fn ended(&self) -> i32 {
        match self {
            Self::Ended(code) => *code,
            _ => 0,
        }
    }

    /// `getLongLinkIdentifyCheckBuffer` — [`IdentifyBuffer::next`] when the
    /// answer is not this one, i.e. ask again on the next connect.
    pub fn identified(&self) -> IdentifyBuffer {
        match self {
            Self::Identified {
                mode,
                buffer,
                hash,
                cmdid,
            } => identify_buffer(*mode, buffer.clone(), hash.clone(), *cmdid),
            _ => IdentifyBuffer::next(Vec::new()),
        }
    }
}

/// `kTaskFailHandle*` — the int Java answered `buf2Resp` with, as what STN is
/// to do about the task. An int that is not one of them is
/// [`TaskFailHandleType::Normal`], which is what the C++'s `switch` leaves it
/// at.
pub fn fail_handle(handle: i32) -> TaskFailHandleType {
    match handle {
        -1 => TaskFailHandleType::Default,
        -12 => TaskFailHandleType::RetryAllTasks,
        -13 => TaskFailHandleType::SessionTimeout,
        -14 => TaskFailHandleType::TaskEnd,
        -15 => TaskFailHandleType::TaskTimeout,
        -16 => TaskFailHandleType::SlientTaskEnd,
        _ => TaskFailHandleType::Normal,
    }
}

/// `ECHECK_NOW` / `ECHECK_NEXT` / `ECHECK_NEVER` — when the identify buffer
/// goes out.
///
/// The comment above Java's `getLongLinkIdentifyCheckBuffer` reads
/// `ECHECK_NOW, ECHECK_NEVER, ECHECK_NEXT`, but the C++ switches the int it got
/// back on its own `kCheckNow, kCheckNext, kCheckNever`, and that is the one it
/// is: `0` sends it now, `1` asks again on the next connect and anything else
/// stops asking.
///
/// Not ported: the C++ returns before it reads the streams for the two that are
/// not "now", so a hash it hands over for them is always empty. This one hands
/// over what Java wrote, because the port's [`IdentifyBuffer`] keeps a hash for
/// all three and the checker asks for it once the app is ready.
pub fn identify_buffer(mode: i32, buffer: Vec<u8>, hash: Vec<u8>, cmdid: u32) -> IdentifyBuffer {
    match mode {
        0 => IdentifyBuffer::now(buffer, hash, cmdid),
        1 => IdentifyBuffer::next(hash),
        _ => IdentifyBuffer::never(hash),
    }
}

/// `C2Java_ReportTaskProfile` — the report, as the json the Java side parses.
///
/// The C++ writes it field by field into an `XMessage`; the json is the wire
/// format between the two halves, so the keys and their order are the C++'s.
/// Like the C++, no string is escaped: a cgi with a quote in it is the app's
/// own problem.
pub fn task_profile_json(profile: &TaskProfile) -> String {
    let connections: Vec<String> = profile
        .history
        .iter()
        .map(|transfer| {
            let connect = &transfer.connect_profile;
            format!(
                concat!(
                    "{{\"startTime\":{},\"dnsTime\":{},\"dnsEndTime\":{},\"connTime\":{},",
                    "\"connErrCode\":{},\"tryIPCount\":{},\"ip\":\"{}\",\"port\":{},",
                    "\"host\":\"{}\",\"ipType\":{},\"disconnTime\":{},",
                    "\"disconnErrType\":{},\"disconnErrCode\":{}}}"
                ),
                connect.start_time,
                connect.dns_time,
                connect.dns_endtime,
                connect.conn_time,
                connect.conn_errcode,
                connect.tryip_count,
                connect.ip,
                connect.port,
                connect.host,
                connect.ip_type as i32,
                connect.disconn_time,
                connect.disconn_errtype as i32,
                connect.disconn_errcode,
            )
        })
        .collect();

    format!(
        concat!(
            "{{\"taskId\":{},\"cmdId\":{},\"cgi\":\"{}\",\"startTaskTime\":{},",
            "\"endTaskTime\":{},\"dyntimeStatus\":{},\"errCode\":{},\"errType\":{},",
            "\"channelSelect\":{},\"historyNetLinkers\":[{}]}}"
        ),
        profile.task.taskid,
        profile.task.cmdid,
        profile.task.cgi,
        profile.start_task_time,
        profile.end_task_time,
        profile.current_dyntime_status as i32,
        profile.err_code,
        profile.err_type as i32,
        profile.link_type,
        connections.join(","),
    )
}

/// The app STN talks to, when the app is Java: [`App`] answered by one hook,
/// which [`JavaApp::jvm`] makes ask the JVM and a test makes write down what it
/// was asked.
pub struct JavaApp {
    ask: Box<dyn FnMut(Question) -> Answer + Send>,
}

impl JavaApp {
    /// The app whose answers come from Java.
    pub fn jvm() -> Self {
        Self::new(crate::jni_bridge::ask_java)
    }

    /// The same, with another answerer: a host that is not Java, or a test.
    pub fn new(ask: impl FnMut(Question) -> Answer + Send + 'static) -> Self {
        Self { ask: Box::new(ask) }
    }

    fn ask(&mut self, question: Question) -> Answer {
        (self.ask)(question)
    }
}

impl std::fmt::Debug for JavaApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JavaApp").finish_non_exhaustive()
    }
}

impl App for JavaApp {
    fn makesure_authed(&mut self, host: &str, _user_id: &str) -> bool {
        self.ask(Question::MakesureAuthed {
            host: host.to_owned(),
        })
        .yes()
    }

    fn traffic_data(&mut self, send: i64, recv: i64) {
        let _ = self.ask(Question::TrafficData { send, recv });
    }

    fn on_new_dns(&mut self, host: &str, _longlink_host: bool, _extra: &ExtraInfo) -> Vec<String> {
        self.ask(Question::OnNewDns {
            host: host.to_owned(),
        })
        .ips()
    }

    fn on_push(&mut self, channel_id: &str, cmdid: u32, taskid: u32, body: &[u8]) {
        let _ = self.ask(Question::OnPush {
            channel_id: channel_id.to_owned(),
            cmdid,
            taskid,
            body: body.to_vec(),
        });
    }

    fn req2buf(
        &mut self,
        taskid: u32,
        _user_id: &str,
        channel_select: i32,
        host: &str,
        sequence: u16,
    ) -> Result<Vec<u8>, i32> {
        self.ask(Question::Req2Buf {
            taskid,
            channel_select,
            host: host.to_owned(),
            sequence,
        })
        .encoded()
    }

    fn buf2resp(
        &mut self,
        taskid: u32,
        _user_id: &str,
        body: &[u8],
        channel_select: i32,
    ) -> (i32, TaskFailHandleType) {
        self.ask(Question::Buf2Resp {
            taskid,
            body: body.to_vec(),
            channel_select,
        })
        .decoded()
    }

    fn on_task_end(
        &mut self,
        taskid: u32,
        _user_id: &str,
        err_type: ErrCmdType,
        err_code: i32,
        profile: &CgiProfile,
    ) -> i32 {
        self.ask(Question::OnTaskEnd {
            taskid,
            err_type,
            err_code,
            profile: profile.clone(),
        })
        .ended()
    }

    fn report_connect_status(&mut self, all: NetStatus, longlink: NetStatus) {
        let _ = self.ask(Question::ReportConnectStatus { all, longlink });
    }

    fn identify_check_buffer(&mut self, channel_id: &str, _cmdid: u32) -> IdentifyBuffer {
        self.ask(Question::IdentifyCheckBuffer {
            channel_id: channel_id.to_owned(),
        })
        .identified()
    }

    fn identify_response(&mut self, channel_id: &str, response: &[u8], hash: &[u8]) -> bool {
        self.ask(Question::IdentifyResponse {
            channel_id: channel_id.to_owned(),
            response: response.to_vec(),
            hash: hash.to_vec(),
        })
        .yes()
    }

    fn request_sync(&mut self) {
        let _ = self.ask(Question::RequestSync);
    }

    fn net_check_shortlink_hosts(&mut self) -> Vec<String> {
        self.ask(Question::NetCheckShortLinkHosts).ips()
    }

    fn report_task_profile(&mut self, profile: &TaskProfile) {
        let _ = self.ask(Question::ReportTaskProfile {
            json: task_profile_json(profile),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mars_stn::{DynamicTimeoutStatus, IpSourceType, PrepareProfile, Task, TransferProfile};
    use std::sync::{Arc, Mutex, PoisonError};

    fn poisoned<T>(error: PoisonError<T>) -> T {
        error.into_inner()
    }

    /// An app that answers `answer` to everything and writes down what it was
    /// asked — i.e. what the JVM hook would have been handed.
    fn app(answer: Answer) -> (Arc<Mutex<Vec<Question>>>, JavaApp) {
        let questions = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&questions);
        let application = JavaApp::new(move |question| {
            recorded.lock().unwrap_or_else(poisoned).push(question);
            answer.clone()
        });
        (questions, application)
    }

    fn asked(questions: &Arc<Mutex<Vec<Question>>>) -> Vec<Question> {
        questions.lock().unwrap_or_else(poisoned).clone()
    }

    /// A task that ran: one try in its history, which is the `historyNetLinkers`
    /// of the report.
    fn report() -> TaskProfile {
        let task = Task::new(7, 8);
        let mut profile = TaskProfile::new_at(
            0,
            Task {
                cgi: "cgi".to_owned(),
                ..task.clone()
            },
            PrepareProfile {
                start_task_call_time: 0,
                begin_process_hosts_time: 0,
                end_process_hosts_time: 0,
            },
        );
        profile.end_task_time = 200;
        profile.err_code = -15;
        profile.err_type = ErrCmdType::Local;
        profile.current_dyntime_status = DynamicTimeoutStatus::Evaluating;

        let mut transfer = TransferProfile::new(task);
        transfer.connect_profile.start_time = 10;
        transfer.connect_profile.dns_time = 11;
        transfer.connect_profile.dns_endtime = 12;
        transfer.connect_profile.conn_time = 13;
        transfer.connect_profile.conn_errcode = -1;
        transfer.connect_profile.tryip_count = 2;
        transfer.connect_profile.ip = "10.0.0.1".to_owned();
        transfer.connect_profile.port = 80;
        transfer.connect_profile.host = "host".to_owned();
        transfer.connect_profile.ip_type = IpSourceType::Dns;
        transfer.connect_profile.disconn_time = 14;
        transfer.connect_profile.disconn_errtype = ErrCmdType::Local;
        transfer.connect_profile.disconn_errcode = 15;
        profile.history.push(transfer);
        profile
    }

    #[test]
    fn the_host_a_task_goes_out_on_is_the_one_java_is_asked_to_auth() {
        let (questions, mut app) = app(Answer::Yes(true));
        assert!(app.makesure_authed("long.host.com", "user"));
        assert_eq!(
            asked(&questions),
            vec![Question::MakesureAuthed {
                host: "long.host.com".to_owned()
            }]
        );
    }

    #[test]
    fn what_went_out_and_came_in_is_handed_to_java_as_two_ints() {
        let (questions, mut app) = app(Answer::Nothing);
        app.traffic_data(1024, 4096);
        assert_eq!(
            asked(&questions),
            vec![Question::TrafficData {
                send: 1024,
                recv: 4096
            }]
        );
    }

    #[test]
    fn the_ips_java_knows_for_a_host_are_the_ones_stn_connects_to() {
        let (questions, mut app) = app(Answer::Ips(vec![
            "1.2.3.4".to_owned(),
            "5.6.7.8".to_owned(),
        ]));
        assert_eq!(
            app.on_new_dns("host", true, &ExtraInfo::new()),
            vec!["1.2.3.4".to_owned(), "5.6.7.8".to_owned()]
        );
        assert_eq!(
            asked(&questions),
            vec![Question::OnNewDns {
                host: "host".to_owned()
            }]
        );
    }

    #[test]
    fn a_push_is_handed_over_with_the_body_it_came_in_with() {
        let (questions, mut app) = app(Answer::Nothing);
        app.on_push("longlink", 6, 7, b"push");
        assert_eq!(
            asked(&questions),
            vec![Question::OnPush {
                channel_id: "longlink".to_owned(),
                cmdid: 6,
                taskid: 7,
                body: b"push".to_vec(),
            }]
        );
    }

    #[test]
    fn a_task_the_app_encoded_goes_out_as_the_bytes_it_wrote() {
        let (questions, mut app) = app(Answer::Encoded(Ok(b"body".to_vec())));
        assert_eq!(app.req2buf(7, "user", 1, "host", 3), Ok(b"body".to_vec()));
        assert_eq!(
            asked(&questions),
            vec![Question::Req2Buf {
                taskid: 7,
                channel_select: 1,
                host: "host".to_owned(),
                sequence: 3,
            }]
        );
    }

    #[test]
    fn a_task_the_app_could_not_encode_ends_with_the_code_it_gave() {
        let (_questions, mut app) = app(Answer::Encoded(Err(-5)));
        assert_eq!(app.req2buf(7, "user", 0, "host", 0), Err(-5));
    }

    #[test]
    fn an_answer_the_app_read_comes_back_with_the_code_and_the_handle_it_answered() {
        let (questions, mut app) = app(Answer::Decoded {
            handle: -14,
            err_code: 9,
        });
        assert_eq!(
            app.buf2resp(7, "user", b"answer", 2),
            (9, TaskFailHandleType::TaskEnd)
        );
        assert_eq!(
            asked(&questions),
            vec![Question::Buf2Resp {
                taskid: 7,
                body: b"answer".to_vec(),
                channel_select: 2,
            }]
        );
    }

    #[test]
    fn a_task_that_is_over_comes_back_with_the_code_java_answered() {
        let (questions, mut app) = app(Answer::Ended(3));
        let profile = CgiProfile {
            rtt: 30,
            ..CgiProfile::default()
        };
        assert_eq!(
            app.on_task_end(7, "user", ErrCmdType::Local, -15, &profile),
            3
        );
        assert_eq!(
            asked(&questions),
            vec![Question::OnTaskEnd {
                taskid: 7,
                err_type: ErrCmdType::Local,
                err_code: -15,
                profile: profile.clone(),
            }]
        );
    }

    #[test]
    fn the_connect_status_java_hears_is_the_one_stn_is_in() {
        let (questions, mut app) = app(Answer::Nothing);
        app.report_connect_status(NetStatus::Connected, NetStatus::Connecting);
        assert_eq!(
            asked(&questions),
            vec![Question::ReportConnectStatus {
                all: NetStatus::Connected,
                longlink: NetStatus::Connecting,
            }]
        );
    }

    #[test]
    fn a_check_java_answered_now_goes_out_with_the_buffer_and_the_cmdid_it_wrote() {
        let (questions, mut app) = app(Answer::Identified {
            mode: 0,
            buffer: b"check".to_vec(),
            hash: b"hash".to_vec(),
            cmdid: 99,
        });
        assert_eq!(
            app.identify_check_buffer("longlink", 6),
            IdentifyBuffer::now(b"check".to_vec(), b"hash".to_vec(), 99)
        );
        assert_eq!(
            asked(&questions),
            vec![Question::IdentifyCheckBuffer {
                channel_id: "longlink".to_owned(),
            }]
        );
    }

    #[test]
    fn a_check_java_is_not_ready_for_waits_for_another_connect_or_stops_asking() {
        // what the app answers is one of three: send it now, ask again on the
        // next connect, or stop asking at all
        for (mode, expected) in [
            (1, IdentifyBuffer::next(b"hash".to_vec())),
            (2, IdentifyBuffer::never(b"hash".to_vec())),
        ] {
            let (_questions, mut application) = app(Answer::Identified {
                mode,
                buffer: Vec::new(),
                hash: b"hash".to_vec(),
                cmdid: 0,
            });
            assert_eq!(application.identify_check_buffer("longlink", 6), expected);
        }
    }

    #[test]
    fn whether_the_check_came_back_is_what_java_says_about_the_answer() {
        let (questions, mut app) = app(Answer::Yes(true));
        assert!(app.identify_response("longlink", b"hash", b"hash"));
        assert_eq!(
            asked(&questions),
            vec![Question::IdentifyResponse {
                channel_id: "longlink".to_owned(),
                response: b"hash".to_vec(),
                hash: b"hash".to_vec(),
            }]
        );
    }

    #[test]
    fn the_sync_and_the_hosts_to_check_are_asked_for_without_an_answer() {
        let (questions, mut app) = app(Answer::Nothing);
        app.request_sync();
        assert!(app.net_check_shortlink_hosts().is_empty());
        assert_eq!(
            asked(&questions),
            vec![Question::RequestSync, Question::NetCheckShortLinkHosts]
        );
    }

    #[test]
    fn the_hosts_java_answers_with_are_the_ones_the_net_check_tries() {
        let (_questions, mut app) = app(Answer::Ips(vec!["short.host".to_owned()]));
        assert_eq!(
            app.net_check_shortlink_hosts(),
            vec!["short.host".to_owned()]
        );
    }

    #[test]
    fn the_report_java_is_handed_is_the_json_the_cpp_writes() {
        let (questions, mut app) = app(Answer::Nothing);
        app.report_task_profile(&report());
        assert_eq!(
            asked(&questions),
            vec![Question::ReportTaskProfile {
                json: concat!(
                    r#"{"taskId":7,"cmdId":8,"cgi":"cgi","startTaskTime":0,"#,
                    r#""endTaskTime":200,"dyntimeStatus":1,"errCode":-15,"errType":9,"#,
                    r#""channelSelect":0,"historyNetLinkers":[{"startTime":10,"dnsTime":11,"#,
                    r#""dnsEndTime":12,"connTime":13,"connErrCode":-1,"tryIPCount":2,"#,
                    r#""ip":"10.0.0.1","port":80,"host":"host","ipType":2,"#,
                    r#""disconnTime":14,"disconnErrType":9,"disconnErrCode":15}]}"#
                )
                .to_owned()
            }]
        );
    }

    #[test]
    fn a_question_java_was_not_asked_is_answered_the_way_stn_takes_it() {
        let (questions, mut app) = app(Answer::Nothing);
        assert!(!app.makesure_authed("host", "user"));
        assert!(app.on_new_dns("host", false, &ExtraInfo::new()).is_empty());
        assert_eq!(
            app.req2buf(7, "user", 0, "host", 0),
            Err(LOCAL_START_TASK_FAIL)
        );
        assert_eq!(
            app.buf2resp(7, "user", b"answer", 0),
            (0, TaskFailHandleType::Normal)
        );
        assert_eq!(
            app.on_task_end(7, "user", ErrCmdType::Ok, 0, &CgiProfile::default()),
            0
        );
        assert_eq!(
            app.identify_check_buffer("longlink", 0),
            IdentifyBuffer::next(Vec::new())
        );
        assert!(!app.identify_response("longlink", b"hash", b"hash"));
        assert!(app.net_check_shortlink_hosts().is_empty());
        // java is asked all the same: what it answered is read out of the
        // answer, not of the question
        assert_eq!(asked(&questions).len(), 8);
    }

    #[test]
    fn the_handle_java_answered_buf2resp_with_is_what_stn_does_about_the_task() {
        let handles = [
            (0, TaskFailHandleType::Normal),
            (-1, TaskFailHandleType::Default),
            (-12, TaskFailHandleType::RetryAllTasks),
            (-13, TaskFailHandleType::SessionTimeout),
            (-14, TaskFailHandleType::TaskEnd),
            (-15, TaskFailHandleType::TaskTimeout),
            (-16, TaskFailHandleType::SlientTaskEnd),
            (-99, TaskFailHandleType::Normal),
        ];
        for (handle, expected) in handles {
            assert_eq!(fail_handle(handle), expected, "handle {handle}");
        }
    }

    #[test]
    fn the_app_is_debug_without_being_asked_anything() {
        let (_questions, app) = app(Answer::Nothing);
        assert!(format!("{app:?}").contains("JavaApp"));
    }
}
