//! `mars/stn/stn_callback_bridge.{h,cc}` and the `Callback` of
//! `mars/stn/stn.h` — the app's whole interface to STN, and the bridge between
//! it and the net core.
//!
//! `stn.h`'s `Callback` is the app's say on everything: what a task's body is,
//! how an answer is read, what a task that ended is remembered with, and what
//! the app is told about a push, a connection, and an error. The C++ makes it a
//! class of eighteen virtuals the app inherits; here it is a trait with an
//! answer for every one of them, so an app implements the questions it cares
//! about and leaves the rest alone. Every out-parameter is gone: `Req2Buf`
//! answers [`Result`], and `OnNewDns` answers a `Vec`.
//!
//! `StnCallbackBridge` is the C++'s own `StnCallbackBridge`, which exists so
//! that the app's answer can come from something that is not C++ (its `#else`
//! branches call into Java) and so that one conversion happens in one place:
//! the [`ConnectProfile`] STN keeps becomes the [`CgiProfile`] the app is
//! handed, which is the only piece of logic in the file. A `dyn Callback` is
//! already what "something that is not C++" means in Rust, so what is left of
//! the bridge is the app it holds, that conversion, and the two
//! `boost::signals2` signals the C++ fans an error out to *before* the app
//! hears about it — a list of listeners, which is what a signal is.
//!
//! Not ported: `user_context`, a `void*` the C++ hands straight back (the app
//! asked for the task, so it knows which one it was); the `AutoBuffer`
//! `extend` of `Req2Buf` and `Buf2Resp`, which is the app's own decoder's; the
//! `flags` and `server_sequence_id` `Buf2Resp` writes; and the `#else`
//! branches, which call into Java and are what the JNI crate is for.

use mars_comm::tickcount::gettickcount;

use crate::longlink_identify_checker::IdentifyBuffer;
use crate::net_source::ExtraInfo;
use crate::task_profile::{
    ConnectProfile, ErrCmdType, TaskFailHandleType, TaskProfile, LOCAL_START_TASK_FAIL,
};
use crate::{LongLinkStatus, NetStatus, Task};

/// `SignalOnLongLinkNetworkError` — one of the C++'s `boost::signals2`s, which
/// is a list of listeners: an error is handed to every one of them, and then to
/// the app.
pub type LongLinkErrorListener = dyn FnMut(ErrCmdType, i32, &str, u16) + Send;

/// `SignalOnShortLinkNetworkError`.
pub type ShortLinkErrorListener = dyn FnMut(ErrCmdType, i32, &str, &str, u16) + Send;

/// `stn::Callback` — the app, as STN asks it questions.
///
/// The C++ calls this `Callback`; here it is [`App`], because what it is is the
/// app STN is talking to, and `Callback` is already the name of the answer a
/// queue gives a task that ended.
///
/// The C++ makes fifteen of these pure virtual (an app must answer them) and
/// three not (`OnLongLinkNetworkError`, `OnShortLinkNetworkError` and
/// `OnLongLinkStatusChange`). Here every one has an answer, and the answer is
/// what STN does when the app says nothing: a task is encoded to nothing, an
/// answer is a good one, a check that never happened is asked for again on the
/// next connect.
pub trait App: Send {
    /// `MakesureAuthed` — whether the app is logged in for this host and user,
    /// which is what a `need_authed` task waits for.
    fn makesure_authed(&mut self, _host: &str, _user_id: &str) -> bool {
        true
    }

    /// `TrafficData` — how much went out and came in, which the C++ only asks
    /// for the log's own tag.
    fn traffic_data(&mut self, _send: i64, _recv: i64) {}

    /// `OnNewDns` — the ips the app knows for a host, which STN tries before
    /// the ones the platform's dns gives it.
    fn on_new_dns(&mut self, _host: &str, _longlink_host: bool, _extra: &ExtraInfo) -> Vec<String> {
        Vec::new()
    }

    /// `OnPush` — something the server sent that no task asked for.
    fn on_push(&mut self, _channel_id: &str, _cmdid: u32, _taskid: u32, _body: &[u8]) {}

    /// `Req2Buf` — what the app wants to send. [`Err`] is the C++'s `false`,
    /// and the code in it is the one the task ends with.
    fn req2buf(
        &mut self,
        _taskid: u32,
        _user_id: &str,
        _channel_select: i32,
        _host: &str,
        _sequence: u16,
    ) -> Result<Vec<u8>, i32> {
        Ok(Vec::new())
    }

    /// `Buf2Resp` — how the app reads an answer: the code it ended with, and
    /// what STN is to do about it. [`TaskFailHandleType::Normal`] is "nothing".
    fn buf2resp(
        &mut self,
        _taskid: u32,
        _user_id: &str,
        _body: &[u8],
        _channel_select: i32,
    ) -> (i32, TaskFailHandleType) {
        (0, TaskFailHandleType::Normal)
    }

    /// `OnTaskEnd` — a task that is over, and the connect it ran on. The answer
    /// is the code the task is remembered with.
    fn on_task_end(
        &mut self,
        _taskid: u32,
        _user_id: &str,
        _err_type: ErrCmdType,
        _err_code: i32,
        _profile: &CgiProfile,
    ) -> i32 {
        0
    }

    /// `ReportConnectStatus` — the connection as the app is asked to see it:
    /// one answer for "can STN reach anything at all", and one for the long
    /// link.
    fn report_connect_status(&mut self, _all: NetStatus, _longlink: NetStatus) {}

    /// `OnLongLinkNetworkError` — only the main link's errors are the app's
    /// business.
    fn on_long_link_network_error(
        &mut self,
        _err_type: ErrCmdType,
        _err_code: i32,
        _ip: &str,
        _port: u16,
    ) {
    }

    /// `OnShortLinkNetworkError`.
    fn on_short_link_network_error(
        &mut self,
        _err_type: ErrCmdType,
        _err_code: i32,
        _ip: &str,
        _host: &str,
        _port: u16,
    ) {
    }

    /// `OnLongLinkStatusChange` — what the default long link is in.
    fn on_long_link_status_change(&mut self, _status: LongLinkStatus) {}

    /// `GetLonglinkIdentifyCheckBuffer` — the check the app is asked for before
    /// a new link is used, and the cmdid it would go out with.
    fn identify_check_buffer(&mut self, _channel_id: &str, _cmdid: u32) -> IdentifyBuffer {
        IdentifyBuffer::next(Vec::new())
    }

    /// `OnLonglinkIdentifyResponse` — whether the answer the server gave is the
    /// one the check asked for.
    fn identify_response(&mut self, _channel_id: &str, _response: &[u8], _hash: &[u8]) -> bool {
        false
    }

    /// `RequestSync` — the app is asked to sync, which is the C++'s own
    /// timing-sync alarm reaching it.
    fn request_sync(&mut self) {}

    /// `RequestNetCheckShortLinkHosts` — the hosts the network check may probe.
    fn net_check_shortlink_hosts(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// `ReportTaskProfile` — everything a task left behind, for the app's own
    /// report.
    fn report_task_profile(&mut self, _profile: &TaskProfile) {}

    /// `ReportTaskLimited` — a task the app asked to have limited, and the
    /// answer is what the limit is: `0` is "go ahead".
    fn report_task_limited(&mut self, _check_type: i32, _task: &Task) -> u32 {
        0
    }

    /// `ReportDnsProfile` — how a dns question went.
    fn report_dns_profile(&mut self, _profile: &DnsProfile) {}
}

/// `CgiProfile` — the connect a task ran on, as the app's report wants it: every
/// reading is a `gettickcount()`.
///
/// [`CgiProfile::of`] is the conversion the C++'s `OnTaskEnd` does, quirks and
/// all: a tls handshake that never finished has no start either, and the app is
/// told when the *write* finished, which the C++ keeps as a start and a cost.
///
/// Not ported: the tls-handshake and the encode/decode readings. The port's
/// [`ConnectProfile`] does not carry them, because nothing in the port writes
/// them — they come with the host's own encoder and decoder.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CgiProfile {
    /// `start_time` — when the run began.
    pub start_time: u64,
    /// `start_connect_time` — when the connect began.
    pub start_connect_time: u64,
    /// `connect_successful_time` — when it came back, socket or no socket.
    pub connect_successful_time: u64,
    /// `start_send_packet_time` — when the request went out.
    pub start_send_packet_time: u64,
    /// `send_packet_finished_time` — when the write was done, which the C++
    /// works out as `start_send_packet_time + send_request_cost`.
    pub send_packet_finished_time: u64,
    /// `start_read_packet_time` — when the read of the answer began.
    pub start_read_packet_time: u64,
    /// `read_packet_finished_time` — when the last of it came back.
    pub read_packet_finished_time: u64,
    /// `channel_type` — one of the `Task::CHANNEL_*` values.
    pub channel_type: i32,
    /// `transport_protocol` — one of the `Task::TRANSPORT_PROTOCOL*` values.
    pub transport_protocol: i32,
    /// `rtt` — how long the pair that won took to answer.
    pub rtt: u32,
    /// `nettype` — the network the connect was made on.
    pub nettype: String,
}

impl CgiProfile {
    /// What the app is handed for a task that ended on this connect — the
    /// C++'s `OnTaskEnd`, which builds a `CgiProfile` out of the
    /// `ConnectProfile` STN kept.
    pub fn of(profile: &ConnectProfile) -> Self {
        Self {
            start_time: profile.start_time,
            start_connect_time: profile.start_connect_time,
            connect_successful_time: profile.connect_successful_time,
            start_send_packet_time: profile.start_send_packet_time,
            send_packet_finished_time: profile.start_send_packet_time + profile.send_request_cost,
            start_read_packet_time: profile.start_read_packet_time,
            read_packet_finished_time: profile.read_packet_finished_time,
            channel_type: profile.channel_type,
            transport_protocol: profile.transport_protocol,
            rtt: profile.conn_rtt,
            nettype: profile.net_type.clone(),
        }
    }
}

impl From<&ConnectProfile> for CgiProfile {
    fn from(profile: &ConnectProfile) -> Self {
        Self::of(profile)
    }
}

/// `DnsType` — which dns a [`DnsProfile`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DnsType {
    /// `kType_NewDns` — the one the app answered.
    #[default]
    NewDns = 1,
    /// `kType_Dns` — the platform's.
    Dns = 2,
}

/// `DnsProfile` — how a dns question went, which the app is told about so it
/// can keep its own history of hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsProfile {
    /// `start_time` — when the question was asked.
    pub start_time: u64,
    /// `end_time` — when it came back; `0` while it has not.
    pub end_time: u64,
    /// `host` — what was asked for.
    pub host: String,
    /// `err_type` — [`ErrCmdType::Ok`] until [`DnsProfile::failed`].
    pub err_type: ErrCmdType,
    /// `err_code`.
    pub err_code: i32,
    /// `dnstype` — which dns it was.
    pub dnstype: DnsType,
}

impl DnsProfile {
    /// `DnsProfile()` — a question that has just been asked, which is what
    /// `Reset()` gives it too.
    pub fn new(host: &str) -> Self {
        Self::new_at(gettickcount(), host)
    }

    /// The same, with the reading `start_time` is set to handed in.
    pub fn new_at(now: u64, host: &str) -> Self {
        Self {
            start_time: now,
            end_time: 0,
            host: host.to_string(),
            err_type: ErrCmdType::Ok,
            err_code: 0,
            dnstype: DnsType::default(),
        }
    }

    /// `OnFailed()` — a question that did not come back, which the C++ calls
    /// `kEctLocal` with `-1`.
    pub fn failed(&mut self) {
        self.err_type = ErrCmdType::Local;
        self.err_code = -1;
    }
}

/// `StnCallbackBridge` — the app STN is talking to, and the two lists of
/// listeners an error is fanned out to.
pub struct StnCallbackBridge {
    app: Option<Box<dyn App>>,
    long_link_errors: Vec<Box<LongLinkErrorListener>>,
    short_link_errors: Vec<Box<ShortLinkErrorListener>>,
}

impl Default for StnCallbackBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for StnCallbackBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StnCallbackBridge")
            .field("app", &self.app.is_some())
            .field("long_link_errors", &self.long_link_errors.len())
            .field("short_link_errors", &self.short_link_errors.len())
            .finish()
    }
}

impl StnCallbackBridge {
    /// `StnCallbackBridge()` — one with no app in it, which is what
    /// `GetStnCallbackBridge` makes before `SetCallback` is called.
    pub fn new() -> Self {
        Self {
            app: None,
            long_link_errors: Vec::new(),
            short_link_errors: Vec::new(),
        }
    }

    /// `SetCallback` — the app STN is talking to. The one before it is dropped.
    pub fn set_callback(&mut self, app: impl App + 'static) {
        self.app = Some(Box::new(app));
    }

    /// The app, for a host that wants to ask it something of its own.
    pub fn callback(&mut self) -> Option<&mut (dyn App + 'static)> {
        self.app.as_mut().map(|app| app.as_mut())
    }

    /// Whether there is an app in the bridge: STN works without one, and every
    /// question has an answer it does not need the app for.
    pub fn has_callback(&self) -> bool {
        self.app.is_some()
    }

    /// `SignalOnLongLinkNetworkError::connect` — a listener that hears about a
    /// long-link error before the app does.
    pub fn add_long_link_error_listener(
        &mut self,
        listener: impl FnMut(ErrCmdType, i32, &str, u16) + Send + 'static,
    ) {
        self.long_link_errors.push(Box::new(listener));
    }

    /// `SignalOnShortLinkNetworkError::connect`.
    pub fn add_short_link_error_listener(
        &mut self,
        listener: impl FnMut(ErrCmdType, i32, &str, &str, u16) + Send + 'static,
    ) {
        self.short_link_errors.push(Box::new(listener));
    }

    /// How many listeners an error is fanned out to — the C++'s signal has no
    /// way of being asked, because nobody asks it.
    pub fn error_listener_count(&self) -> usize {
        self.long_link_errors.len() + self.short_link_errors.len()
    }

    /// `MakesureAuthed` — [`true`] while there is no app: the C++ warns and
    /// answers `false`, but a task that needs the app to be logged in is one
    /// the app would have answered for, and STN's own answer for a question it
    /// has nobody to ask is "yes".
    pub fn makesure_authed(&mut self, host: &str, user_id: &str) -> bool {
        self.app
            .as_mut()
            .is_none_or(|app| app.makesure_authed(host, user_id))
    }

    /// `TrafficData`.
    pub fn traffic_data(&mut self, send: i64, recv: i64) {
        if let Some(app) = self.app.as_mut() {
            app.traffic_data(send, recv);
        }
    }

    /// `OnNewDns` — [`Vec::new()`] while there is no app, which is the C++'s
    /// empty vector.
    pub fn on_new_dns(
        &mut self,
        host: &str,
        longlink_host: bool,
        extra: &ExtraInfo,
    ) -> Vec<String> {
        self.app
            .as_mut()
            .map_or_else(Vec::new, |app| app.on_new_dns(host, longlink_host, extra))
    }

    /// `OnPush`.
    pub fn on_push(&mut self, channel_id: &str, cmdid: u32, taskid: u32, body: &[u8]) {
        if let Some(app) = self.app.as_mut() {
            app.on_push(channel_id, cmdid, taskid, body);
        }
    }

    /// `Req2Buf` — [`Err`] while there is no app, which is the C++'s `false`
    /// and the end of the task: a task nobody can encode is one that cannot be
    /// started.
    pub fn req2buf(
        &mut self,
        taskid: u32,
        user_id: &str,
        channel_select: i32,
        host: &str,
        sequence: u16,
    ) -> Result<Vec<u8>, i32> {
        match self.app.as_mut() {
            Some(app) => app.req2buf(taskid, user_id, channel_select, host, sequence),
            None => Err(LOCAL_START_TASK_FAIL),
        }
    }

    /// `Buf2Resp` — `(0, Normal)` while there is no app, which is the C++'s
    /// `0`: an answer nobody reads is a good one.
    pub fn buf2resp(
        &mut self,
        taskid: u32,
        user_id: &str,
        body: &[u8],
        channel_select: i32,
    ) -> (i32, TaskFailHandleType) {
        self.app
            .as_mut()
            .map_or((0, TaskFailHandleType::Normal), |app| {
                app.buf2resp(taskid, user_id, body, channel_select)
            })
    }

    /// `OnTaskEnd` — the app is handed the [`CgiProfile`] of the connect the
    /// task ran on, which is the one thing this bridge does to an answer on its
    /// way through.
    pub fn on_task_end(
        &mut self,
        taskid: u32,
        user_id: &str,
        err_type: ErrCmdType,
        err_code: i32,
        profile: &ConnectProfile,
    ) -> i32 {
        let cgi = CgiProfile::of(profile);
        match self.app.as_mut() {
            Some(app) => app.on_task_end(taskid, user_id, err_type, err_code, &cgi),
            None => 0,
        }
    }

    /// `ReportConnectStatus`.
    pub fn report_connect_status(&mut self, all: NetStatus, longlink: NetStatus) {
        if let Some(app) = self.app.as_mut() {
            app.report_connect_status(all, longlink);
        }
    }

    /// `OnLongLinkNetworkError` — the listeners first, then the app, which is
    /// the order the C++ calls them in.
    pub fn on_long_link_network_error(
        &mut self,
        err_type: ErrCmdType,
        err_code: i32,
        ip: &str,
        port: u16,
    ) {
        for listener in self.long_link_errors.iter_mut() {
            listener(err_type, err_code, ip, port);
        }
        if let Some(app) = self.app.as_mut() {
            app.on_long_link_network_error(err_type, err_code, ip, port);
        }
    }

    /// `OnShortLinkNetworkError` — the listeners first, then the app.
    pub fn on_short_link_network_error(
        &mut self,
        err_type: ErrCmdType,
        err_code: i32,
        ip: &str,
        host: &str,
        port: u16,
    ) {
        for listener in self.short_link_errors.iter_mut() {
            listener(err_type, err_code, ip, host, port);
        }
        if let Some(app) = self.app.as_mut() {
            app.on_short_link_network_error(err_type, err_code, ip, host, port);
        }
    }

    /// `OnLongLinkStatusChange`.
    pub fn on_long_link_status_change(&mut self, status: LongLinkStatus) {
        if let Some(app) = self.app.as_mut() {
            app.on_long_link_status_change(status);
        }
    }

    /// `GetLonglinkIdentifyCheckBuffer` — [`IdentifyBuffer::next`] while there
    /// is no app, which is what the identify checker says too when it has
    /// nobody to ask: put the check off to the next connect.
    pub fn identify_check_buffer(&mut self, channel_id: &str, cmdid: u32) -> IdentifyBuffer {
        self.app.as_mut().map_or_else(
            || IdentifyBuffer::next(Vec::new()),
            |app| app.identify_check_buffer(channel_id, cmdid),
        )
    }

    /// `OnLonglinkIdentifyResponse` — `false` while there is no app, which is
    /// what the identify checker says too: a check nobody answered did not pass.
    pub fn identify_response(&mut self, channel_id: &str, response: &[u8], hash: &[u8]) -> bool {
        self.app
            .as_mut()
            .is_some_and(|app| app.identify_response(channel_id, response, hash))
    }

    /// `RequestSync`.
    pub fn request_sync(&mut self) {
        if let Some(app) = self.app.as_mut() {
            app.request_sync();
        }
    }

    /// `RequestNetCheckShortLinkHosts` — the hosts, which is the C++'s
    /// out-parameter as a value.
    pub fn net_check_shortlink_hosts(&mut self) -> Vec<String> {
        self.app
            .as_mut()
            .map_or_else(Vec::new, |app| app.net_check_shortlink_hosts())
    }

    /// `ReportTaskProfile`.
    pub fn report_task_profile(&mut self, profile: &TaskProfile) {
        if let Some(app) = self.app.as_mut() {
            app.report_task_profile(profile);
        }
    }

    /// `ReportTaskLimited` — `0` while there is no app, which is "go ahead".
    pub fn report_task_limited(&mut self, check_type: i32, task: &Task) -> u32 {
        self.app
            .as_mut()
            .map_or(0, |app| app.report_task_limited(check_type, task))
    }

    /// `ReportDnsProfile`.
    pub fn report_dns_profile(&mut self, profile: &DnsProfile) {
        if let Some(app) = self.app.as_mut() {
            app.report_dns_profile(profile);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Everything the bridge said to an app, as one value.
    #[derive(Debug, Default, Clone, PartialEq, Eq)]
    struct Said {
        authed: Vec<(String, String)>,
        traffic: Vec<(i64, i64)>,
        dns: Vec<String>,
        pushed: Vec<(String, u32, u32, Vec<u8>)>,
        encoded: Vec<(u32, i32, String, u16)>,
        decoded: Vec<(u32, Vec<u8>, i32)>,
        ended: Vec<(u32, ErrCmdType, i32, CgiProfile)>,
        status: Vec<(NetStatus, NetStatus)>,
        long_err: Vec<(ErrCmdType, i32, String, u16)>,
        short_err: Vec<(ErrCmdType, i32, String, String, u16)>,
        link_status: Vec<LongLinkStatus>,
        identify: Vec<(String, u32)>,
        identify_response: Vec<(String, Vec<u8>)>,
        sync: usize,
        limited: Vec<(i32, u32)>,
        profiles: usize,
        dns_profiles: Vec<String>,
    }

    /// The app's book, which the test reads.
    type Book = Arc<Mutex<Said>>;

    /// When the app was asked, which the test reads too.
    type Order = Arc<Mutex<Vec<&'static str>>>;

    /// An app that answers every question the same way and writes down what it
    /// was asked, and, if it was given one, the order it was asked in.
    struct Rec {
        said: Book,
        order: Order,
    }

    impl Rec {
        /// One whose book nobody else reads.
        fn new() -> Self {
            Self {
                said: Arc::new(Mutex::new(Said::default())),
                order: Arc::new(Mutex::new(Vec::new())),
            }
        }

        /// One whose book a test reads, and which says when it was asked.
        fn ordered(said: Book, order: Order) -> Self {
            Self { said, order }
        }
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
            _longlink_host: bool,
            _extra: &ExtraInfo,
        ) -> Vec<String> {
            vec![format!("{host}:1"), format!("{host}:2")]
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
            channel_select: i32,
            host: &str,
            sequence: u16,
        ) -> Result<Vec<u8>, i32> {
            self.said.lock().unwrap().encoded.push((
                taskid,
                channel_select,
                host.to_string(),
                sequence,
            ));
            Ok(b"body".to_vec())
        }

        fn buf2resp(
            &mut self,
            taskid: u32,
            _user_id: &str,
            body: &[u8],
            channel_select: i32,
        ) -> (i32, TaskFailHandleType) {
            self.said
                .lock()
                .unwrap()
                .decoded
                .push((taskid, body.to_vec(), channel_select));
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
            self.order.lock().unwrap().push("app");
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
            self.order.lock().unwrap().push("short app");
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
            self.said
                .lock()
                .unwrap()
                .dns_profiles
                .push(profile.host.clone());
        }
    }

    /// A bridge with the app in it, the app's own book, and the order the app
    /// was asked in.
    fn app_with_order() -> (StnCallbackBridge, Book, Order) {
        let said = Arc::new(Mutex::new(Said::default()));
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut bridge = StnCallbackBridge::new();
        bridge.set_callback(Rec::ordered(Arc::clone(&said), Arc::clone(&order)));
        (bridge, said, order)
    }

    /// The same, for a test that does not care about the order.
    fn app() -> (StnCallbackBridge, Book) {
        let (bridge, said, _order) = app_with_order();
        (bridge, said)
    }

    fn connect() -> ConnectProfile {
        let mut profile = ConnectProfile::new();
        profile.net_type = "wifi".to_string();
        profile.start_time = 100;
        profile.start_connect_time = 110;
        profile.connect_successful_time = 120;
        profile.start_send_packet_time = 130;
        profile.send_request_cost = 5;
        profile.start_read_packet_time = 140;
        profile.read_packet_finished_time = 150;
        profile.channel_type = Task::CHANNEL_SHORT;
        profile.transport_protocol = Task::TRANSPORT_PROTOCOL_TCP;
        profile.conn_rtt = 30;
        profile
    }

    #[test]
    fn a_bridge_with_no_app_answers_for_itself() {
        let mut bridge = StnCallbackBridge::new();
        assert!(!bridge.has_callback());

        // the answers STN needs when it has nobody to ask
        assert!(bridge.makesure_authed("host", "user"));
        assert_eq!(
            bridge.on_new_dns("host", true, &ExtraInfo::new()),
            Vec::<String>::new()
        );
        assert_eq!(
            bridge.req2buf(7, "user", Task::CHANNEL_SHORT, "host", 1),
            Err(LOCAL_START_TASK_FAIL)
        );
        assert_eq!(
            bridge.buf2resp(7, "user", b"body", Task::CHANNEL_SHORT),
            (0, TaskFailHandleType::Normal)
        );
        assert_eq!(
            bridge.on_task_end(7, "user", ErrCmdType::Ok, 0, &ConnectProfile::new()),
            0
        );
        assert_eq!(bridge.net_check_shortlink_hosts(), Vec::<String>::new());
        assert_eq!(bridge.report_task_limited(1, &Task::new(7, 12)), 0);
        assert!(!bridge.identify_response("channel", b"resp", b"hash"));
        // a check that nobody answered is put off, not failed
        assert_eq!(
            bridge.identify_check_buffer("channel", 6),
            IdentifyBuffer::next(Vec::new())
        );

        // and none of it panics
        bridge.traffic_data(1, 2);
        bridge.on_push("channel", 6, 7, b"push");
        bridge.report_connect_status(NetStatus::Connected, NetStatus::Connected);
        bridge.on_long_link_network_error(ErrCmdType::Ok, 0, "1.2.3.4", 80);
        bridge.on_short_link_network_error(ErrCmdType::Ok, 0, "1.2.3.4", "host", 80);
        bridge.on_long_link_status_change(LongLinkStatus::Connected);
        bridge.request_sync();
        bridge.report_task_profile(&TaskProfile::new(
            Task::new(7, 12),
            crate::PrepareProfile::new(),
        ));
        bridge.report_dns_profile(&DnsProfile::new("host"));
    }

    #[test]
    fn every_question_reaches_the_app() {
        let (mut bridge, said) = app();
        assert!(bridge.has_callback());

        assert!(bridge.makesure_authed("host", "user"));
        bridge.traffic_data(10, 20);
        assert_eq!(
            bridge.on_new_dns("host", true, &ExtraInfo::new()),
            vec!["host:1".to_string(), "host:2".to_string()]
        );
        bridge.on_push("channel", 6, 7, b"push");
        assert_eq!(
            bridge.req2buf(7, "user", Task::CHANNEL_SHORT, "host", 3),
            Ok(b"body".to_vec())
        );
        assert_eq!(
            bridge.buf2resp(7, "user", b"answer", Task::CHANNEL_SHORT),
            (-1, TaskFailHandleType::TaskTimeout)
        );
        assert_eq!(
            bridge.on_task_end(7, "user", ErrCmdType::Server, 500, &connect()),
            500
        );
        bridge.report_connect_status(NetStatus::Connected, NetStatus::Connected);
        bridge.on_long_link_network_error(ErrCmdType::Socket, -1, "1.2.3.4", 8080);
        bridge.on_short_link_network_error(ErrCmdType::Http, -2, "1.2.3.4", "host", 80);
        bridge.on_long_link_status_change(LongLinkStatus::Connecting);
        assert_eq!(
            bridge.identify_check_buffer("channel", 6),
            IdentifyBuffer::now(b"check".to_vec(), b"hash".to_vec(), 6)
        );
        assert!(bridge.identify_response("channel", b"resp", b"hash"));
        bridge.request_sync();
        assert_eq!(
            bridge.net_check_shortlink_hosts(),
            vec!["check.host".to_string()]
        );
        bridge.report_task_profile(&TaskProfile::new(
            Task::new(7, 12),
            crate::PrepareProfile::new(),
        ));
        assert_eq!(bridge.report_task_limited(2, &Task::new(7, 12)), 1);
        bridge.report_dns_profile(&DnsProfile::new("host"));

        let said = said.lock().unwrap().clone();
        assert_eq!(said.authed, vec![("host".to_string(), "user".to_string())]);
        assert_eq!(said.traffic, vec![(10, 20)]);
        assert_eq!(
            said.pushed,
            vec![("channel".to_string(), 6, 7, b"push".to_vec())]
        );
        assert_eq!(
            said.encoded,
            vec![(7, Task::CHANNEL_SHORT, "host".to_string(), 3)]
        );
        assert_eq!(
            said.decoded,
            vec![(7, b"answer".to_vec(), Task::CHANNEL_SHORT)]
        );
        assert_eq!(said.ended.len(), 1);
        assert_eq!(said.ended[0].0, 7);
        assert_eq!(said.ended[0].1, ErrCmdType::Server);
        assert_eq!(
            said.status,
            vec![(NetStatus::Connected, NetStatus::Connected)]
        );
        assert_eq!(
            said.long_err,
            vec![(ErrCmdType::Socket, -1, "1.2.3.4".to_string(), 8080)]
        );
        assert_eq!(
            said.short_err,
            vec![(
                ErrCmdType::Http,
                -2,
                "1.2.3.4".to_string(),
                "host".to_string(),
                80
            )]
        );
        assert_eq!(said.link_status, vec![LongLinkStatus::Connecting]);
        assert_eq!(said.identify, vec![("channel".to_string(), 6)]);
        assert_eq!(
            said.identify_response,
            vec![("channel".to_string(), b"resp".to_vec())]
        );
        assert_eq!(said.sync, 1);
        assert_eq!(said.limited, vec![(2, 1)]);
        assert_eq!(said.profiles, 1);
        assert_eq!(said.dns_profiles, vec!["host".to_string()]);
    }

    #[test]
    fn the_connect_the_task_ran_on_is_what_the_app_hears_about() {
        let (mut bridge, said) = app();
        bridge.on_task_end(7, "user", ErrCmdType::Ok, 0, &connect());

        let said = said.lock().unwrap().clone();
        assert_eq!(said.ended[0].3, CgiProfile::of(&connect()));
        assert_eq!(said.ended[0].3.start_time, 100);
        assert_eq!(said.ended[0].3.connect_successful_time, 120);
        // the app is told when the write was done, which STN keeps as a start
        // and a cost
        assert_eq!(said.ended[0].3.start_send_packet_time, 130);
        assert_eq!(said.ended[0].3.send_packet_finished_time, 135);
        assert_eq!(said.ended[0].3.read_packet_finished_time, 150);
        assert_eq!(said.ended[0].3.channel_type, Task::CHANNEL_SHORT);
        assert_eq!(said.ended[0].3.rtt, 30);
        assert_eq!(said.ended[0].3.nettype, "wifi");
    }

    #[test]
    fn an_error_is_handed_to_the_listeners_before_the_app() {
        let (mut bridge, said, order) = app_with_order();
        let heard = Arc::clone(&order);
        bridge.add_long_link_error_listener(move |_err_type, _code, _ip, _port| {
            heard.lock().unwrap().push("long listener");
        });
        let heard = Arc::clone(&order);
        bridge.add_short_link_error_listener(move |_err_type, _code, _ip, _host, _port| {
            heard.lock().unwrap().push("short listener");
        });
        assert_eq!(bridge.error_listener_count(), 2);

        bridge.on_long_link_network_error(ErrCmdType::Ok, 0, "1.2.3.4", 80);
        bridge.on_short_link_network_error(ErrCmdType::Ok, 0, "1.2.3.4", "host", 80);

        // the app was told both
        assert_eq!(said.lock().unwrap().long_err.len(), 1);
        assert_eq!(said.lock().unwrap().short_err.len(), 1);
        // and every listener heard it first, which is the C++'s order
        assert_eq!(
            order.lock().unwrap().clone(),
            vec!["long listener", "app", "short listener", "short app"]
        );
    }

    #[test]
    fn a_dns_question_that_did_not_come_back_is_a_failed_one() {
        let mut profile = DnsProfile::new_at(100, "host");
        assert_eq!(profile.start_time, 100);
        assert_eq!(profile.end_time, 0);
        assert_eq!(profile.err_type, ErrCmdType::Ok);
        assert_eq!(profile.dnstype, DnsType::NewDns);

        profile.end_time = 120;
        profile.failed();
        assert_eq!(profile.err_type, ErrCmdType::Local);
        assert_eq!(profile.err_code, -1);
    }

    #[test]
    fn an_app_can_be_handed_back_and_replaced() {
        let mut bridge = StnCallbackBridge::default();
        assert!(bridge.callback().is_none());

        bridge.set_callback(Rec::new());
        assert!(bridge.has_callback());
        assert!(bridge
            .callback()
            .is_some_and(|app| app.makesure_authed("host", "user")));

        // the second one is the one that answers
        let second = Arc::new(Mutex::new(Said::default()));
        bridge.set_callback(Rec::ordered(
            Arc::clone(&second),
            Arc::new(Mutex::new(Vec::new())),
        ));
        bridge.makesure_authed("host", "user");
        assert_eq!(
            second.lock().unwrap().authed,
            vec![("host".to_string(), "user".to_string())]
        );
    }

    #[test]
    fn the_debug_of_a_bridge_says_what_is_in_it() {
        let mut bridge = StnCallbackBridge::new();
        assert_eq!(
            format!("{bridge:?}"),
            "StnCallbackBridge { app: false, long_link_errors: 0, short_link_errors: 0 }"
        );

        bridge.set_callback(Rec::new());
        bridge.add_long_link_error_listener(|_, _, _, _| {});
        assert_eq!(
            format!("{bridge:?}"),
            "StnCallbackBridge { app: true, long_link_errors: 1, short_link_errors: 0 }"
        );
    }
}
