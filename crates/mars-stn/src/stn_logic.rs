//! `mars/stn/stn_logic.{h,cc}` and `mars/stn/stn_manager.{h,cc}` — the app's
//! api to STN, and the one value behind it.
//!
//! `stn_logic.h` is a list of function pointers — `StartTask`, `StopTask`,
//! `RedoTasks`, `SetLonglinkSvrAddr`, `MakesureLonglinkConnected`, … — and
//! every one of them does the same two things: ask a boot framework for the one
//! `StnManager` there is, and call the method of the same name on it.
//! `StnManager` is the two halves of that in one object: it owns the `NetCore`
//! and the `StnCallbackBridge`, it forwards the app's api calls to the core
//! (`net_core_ ? net_core_->… : xwarn2("net core is empty")`), and it forwards
//! the core's callbacks to the bridge.
//!
//! Rust has no boot framework to ask, so there is no split to keep: [`StnLogic`]
//! is one value that owns an [`Option<NetCore>`] and the bridge, and every call
//! the C++ writes twice is written once. `Option` is the C++'s
//! `if (net_core_)`: nothing the app asks for before [`StnLogic::create`] has a
//! core to be asked of, and the answer it gets is the one the C++ warns its way
//! to — `false`, no task, an empty list.
//!
//! The wiring the C++ does in `StnManager`'s own callbacks is what
//! [`StnLogic::create`] does once: every hook the net core, the two queues, the
//! net source and the timing sync ask for is answered by the bridge, which is
//! what makes the app the thing STN talks to.
//!
//! Not ported: `SetStnCallbackBridge` and `GetStnCallbackBridge` (a `dyn App`
//! is already what the C++ needs a bridge for — its other half calls into
//! Java); `GetAllLonglink_ext` (the C++ answers an empty vector);
//! `ProxyIsAvailable` (the port's [`crate::ProxyTest`] is a value the host
//! makes with the sockets it wants it on, not something STN owns); `OnAlarm`
//! (android's alarm dispatch); `OnSingalCrash` / `OnExceptionCrash` (they close
//! the xlog appender, which is `mars_appender`'s own business, not STN's);
//! `OnNetworkDataChange`'s `XLOGGER_TAG` comparison (the tag is a build-time
//! define the port has no equivalent of — the host calls
//! [`StnLogic::traffic_data`] for the traffic it wants counted);
//! `getNoopTaskID` (a constant: [`Task::NOOP_TASK_ID`]); and
//! `SetDefaultShortLinkTlsGroup` (the tls group is not ported).

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use mars_comm::tickcount::gettickcount;

use crate::net_core::NetCore;
use crate::signalling_keeper::set_strategy;
use crate::stn_callback_bridge::{App, StnCallbackBridge};
use crate::task_profile::ConnectProfile;
use crate::{xorshift, LongLink, LongLinkEncoder, LongLinkStatus, LonglinkConfig, NetStatus, Task};

/// `kReservedTaskIDStart` — the id the counter is put back to `1` at, so a
/// generated id is never one of the four the C++ keeps for itself (`kNoopTaskID`
/// and friends).
pub const RESERVED_TASK_ID_START: u32 = 0xffff_fff0;

/// `gs_taskid` — one counter for the whole process, like the C++'s `static
/// uint32_t`.
fn task_ids() -> &'static Mutex<u32> {
    static TASK_IDS: OnceLock<Mutex<u32>> = OnceLock::new();
    TASK_IDS.get_or_init(|| Mutex::new(1))
}

/// `GenTaskID()` — the id a task is started with.
///
/// The C++'s `atomic_inc32` answers the value *after* the increment, so the
/// first id handed out is `2`; and it is put back to `1` at
/// [`RESERVED_TASK_ID_START`], which is what keeps the ids the C++ named
/// (`kNoopTaskID`, the identify checker's, the signalling keeper's) out of the
/// way.
pub fn gen_task_id() -> u32 {
    let mut next = task_ids()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if *next >= RESERVED_TASK_ID_START {
        *next = 1;
    }
    *next += 1;
    *next
}

/// `gs_sequence_id` — the C++'s `std::mt19937 mt_seed(rd())`, which is one seed
/// the process keeps turning. A [`random_device`][rd] is the clock here, which
/// is the same thing said in the port's own words.
///
/// [rd]: https://en.cppreference.com/w/cpp/numeric/random/random_device
fn sequences() -> &'static Mutex<Box<crate::Random>> {
    static SEQUENCES: OnceLock<Mutex<Box<crate::Random>>> = OnceLock::new();
    SEQUENCES.get_or_init(|| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_nanos() as u64)
            .unwrap_or(1);
        Mutex::new(Box::new(xorshift(now)))
    })
}

/// `GenSequenceId()` — the sequence that ties a task to the server's report of
/// it, which is the C++'s `uniform_int_distribution(0, 65535)`.
///
/// A new one is drawn for every task that is encoded, so a retry is not taken
/// for the request it is a retry of.
pub fn gen_sequence_id() -> u16 {
    let mut sequences = sequences()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    sequences(u16::MAX as usize + 1) as u16
}

/// The app's api to STN, and the net core behind it.
pub struct StnLogic {
    core: Option<NetCore>,
    bridge: Arc<Mutex<StnCallbackBridge>>,
    encoder: LongLinkEncoder,
    encoder_version: i32,
    encoder_name: String,
}

impl Default for StnLogic {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for StnLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StnLogic")
            .field("core", &self.core.is_some())
            .field(
                "bridge",
                &self
                    .bridge
                    .lock()
                    .map(|bridge| bridge.has_callback())
                    .unwrap_or(false),
            )
            .field("encoder_version", &self.encoder_version)
            .field("encoder_name", &self.encoder_name)
            .finish()
    }
}

impl StnLogic {
    /// `StnManager::StnManager` — one with no net core in it yet: `OnCreate` is
    /// what makes the core, and everything the app asks for before that is
    /// answered the way the C++ warns its way to.
    pub fn new() -> Self {
        Self {
            core: None,
            bridge: Arc::new(Mutex::new(StnCallbackBridge::new())),
            encoder: LongLinkEncoder::new(),
            encoder_version: 0,
            encoder_name: String::new(),
        }
    }

    /// The bridge the app's answers go through, which is what a host asks when
    /// it wants to ask the app something of its own.
    pub fn bridge(&self) -> &Arc<Mutex<StnCallbackBridge>> {
        &self.bridge
    }

    /// The net core, which is what a host wires its channels on and runs its
    /// loops through — [`None`] until [`StnLogic::create`].
    pub fn net_core(&mut self) -> Option<&mut NetCore> {
        self.core.as_mut()
    }

    /// Whether there is a net core: `OnCreate` has been through, and `OnDestroy`
    /// has not.
    pub fn is_created(&self) -> bool {
        self.core.is_some()
    }

    //===------------------------------------------------------------------===//
    // the boot events
    //===------------------------------------------------------------------===//

    /// `OnInitConfigBeforeOnCreate` — the encoder version the net core is made
    /// with, which is set before there is a core to hand it to.
    pub fn on_init_config_before_on_create(&mut self, version: i32) {
        self.encoder.set_encoder_version(version);
        self.encoder_version = version;
    }

    /// `OnInitConfigBeforeOnCreateV2` — the same, and the encoder's name.
    pub fn on_init_config_before_on_create_v2(&mut self, version: i32, name: impl Into<String>) {
        self.encoder.set_encoder_version(version);
        self.encoder_version = version;
        self.encoder_name = name.into();
    }

    /// `SetDefaultLongLinkEncoder` — the encoder every long link is made with.
    pub fn set_default_longlink_encoder(&mut self, encoder: LongLinkEncoder) {
        self.encoder = encoder;
    }

    /// `OnCreate` — makes the net core and wires every question in it to the
    /// app. `false` when there is one already, which is the C++'s
    /// `if (!net_core_)`.
    pub fn create(&mut self) -> bool {
        self.create_at(gettickcount())
    }

    /// The same, with the reading the C++'s `gettickcount()` would hand out.
    pub fn create_at(&mut self, now: u64) -> bool {
        if self.core.is_some() {
            return false;
        }
        let mut core = NetCore::with_encoder_at(now, true, self.encoder);
        core.set_packer_encoder(self.encoder_version, self.encoder_name.clone());
        Self::wire(&mut core, &self.bridge);
        self.core = Some(core);
        true
    }

    /// `OnDestroy` — the net core is dropped, which is the C++'s
    /// `NetCore::__Release`. `false` when there was none, which is the C++'s
    /// "net core is nullptr. ignore destroy".
    pub fn destroy(&mut self) -> bool {
        self.core.take().is_some()
    }

    /// `Reset` — a net core made again from nothing, which is what the C++ does
    /// when the app has moved to another account: the one before it is dropped
    /// with everything in it.
    pub fn reset(&mut self) {
        self.reset_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn reset_at(&mut self, now: u64) {
        self.core = None;
        self.create_at(now);
    }

    /// `ResetAndInitEncoderVersion` — a net core made again, with the encoder
    /// the app asked for.
    pub fn reset_with_encoder(&mut self, version: i32, name: impl Into<String>) {
        self.reset_with_encoder_at(gettickcount(), version, name)
    }

    /// The same, with the reading handed in.
    pub fn reset_with_encoder_at(&mut self, now: u64, version: i32, name: impl Into<String>) {
        self.on_init_config_before_on_create_v2(version, name);
        self.reset_at(now);
    }

    /// `SetCallback` — the app STN talks to, which is what the bridge answers
    /// with from then on.
    pub fn set_callback(&mut self, app: impl App + 'static) {
        locked(&self.bridge).set_callback(app);
    }

    /// `OnNetworkChange(pre_change)` — the host's own change first, then the
    /// net core's, which is the order the C++ binds them in. Neither runs when
    /// there is no core, which is the C++'s `if (net_core_ && !released)`.
    pub fn on_network_change(&mut self, pre_change: impl FnOnce()) {
        if let Some(core) = self.core.as_mut() {
            pre_change();
            core.on_network_change();
        }
    }

    /// `ActiveLogic` — whether the app is in the foreground, which is what the
    /// anti-avalanche check and the timing sync ask.
    pub fn set_active(&mut self, is_active: bool) {
        if let Some(core) = self.core.as_mut() {
            core.set_active(is_active);
        }
    }

    //===------------------------------------------------------------------===//
    // the tasks
    //===------------------------------------------------------------------===//

    /// `StartTask(_task)` — `true` is a task a queue took.
    pub fn start_task(&mut self, task: Task) -> bool {
        self.start_task_at(gettickcount(), task)
    }

    /// The same, with the reading handed in.
    pub fn start_task_at(&mut self, now: u64, task: Task) -> bool {
        self.core
            .as_mut()
            .is_some_and(|core| core.start_task_at(now, task))
    }

    /// `StopTask(_taskid)`.
    pub fn stop_task(&mut self, taskid: u32) -> bool {
        self.core
            .as_mut()
            .is_some_and(|core| core.stop_task(taskid))
    }

    /// `HasTask(_taskid)`.
    pub fn has_task(&self, taskid: u32) -> bool {
        self.core.as_ref().is_some_and(|core| core.has_task(taskid))
    }

    /// `RedoTasks` — every task is started again, which is what a network change
    /// asks for.
    pub fn redo_tasks(&mut self) {
        self.redo_tasks_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn redo_tasks_at(&mut self, now: u64) {
        if let Some(core) = self.core.as_mut() {
            core.redo_tasks_at(now);
        }
    }

    /// `TouchTasks` — every task is looked at again, which is what a timer asks
    /// for.
    pub fn touch_tasks(&mut self) {
        self.touch_tasks_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn touch_tasks_at(&mut self, now: u64) {
        if let Some(core) = self.core.as_mut() {
            core.touch_tasks_at(now);
        }
    }

    /// `ClearTasks` — every task is thrown away, which is what the app asks for
    /// when it has moved to another account.
    pub fn clear_tasks(&mut self) {
        if let Some(core) = self.core.as_mut() {
            core.clear_tasks();
        }
    }

    /// `DisableLongLink` — no task is put on a long link again.
    pub fn disable_long_link(&mut self) {
        if let Some(core) = self.core.as_mut() {
            core.set_need_use_long_link(false);
        }
    }

    /// `NetCore::GetNextHeartbeatTime` / the C++'s message queue thread: when
    /// the host's run loop is to wake, and what it is to do when it does.
    pub fn due_time(&mut self) -> Option<u64> {
        self.core.as_mut().and_then(NetCore::due_time)
    }

    /// Whether there is a follow-up waiting, which is the C++'s message queue
    /// having something in it.
    pub fn has_pending(&self) -> bool {
        self.core.as_ref().is_some_and(NetCore::has_pending)
    }

    /// How many follow-ups are waiting.
    pub fn pending_count(&self) -> usize {
        self.core.as_ref().map_or(0, NetCore::pending_count)
    }

    /// What the C++'s message queue thread would have done with them.
    pub fn run_pending(&mut self) {
        self.run_pending_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn run_pending_at(&mut self, now: u64) {
        if let Some(core) = self.core.as_mut() {
            core.run_pending_at(now);
        }
    }

    //===------------------------------------------------------------------===//
    // the long links
    //===------------------------------------------------------------------===//

    /// `CreateLonglink_ext(_config)` — a long link the app named, which is
    /// [`None`] while there is no net core.
    pub fn create_long_link(&mut self, config: LonglinkConfig) -> Option<Arc<Mutex<LongLink>>> {
        self.core.as_mut()?.create_long_link(config)
    }

    /// `DestroyLonglink_ext(_name)`.
    pub fn destroy_long_link(&mut self, name: &str) -> bool {
        self.core
            .as_mut()
            .is_some_and(|core| core.destroy_long_link(name))
    }

    /// The same, with the reading handed in.
    pub fn destroy_long_link_at(&mut self, now: u64, name: &str) -> bool {
        self.core
            .as_mut()
            .is_some_and(|core| core.destroy_long_link_at(now, name))
    }

    /// `MarkMainLonglink_ext(_name)` — the long link whose errors and status the
    /// app is told about.
    pub fn mark_main_longlink(&mut self, name: &str) -> bool {
        self.core
            .as_mut()
            .is_some_and(|core| core.mark_main_longlink(name))
    }

    /// `LongLinkIsConnected_ext(_name)`.
    pub fn is_long_link_connected(&self, name: &str) -> bool {
        self.core
            .as_ref()
            .is_some_and(|core| core.is_long_link_connected(name))
    }

    /// `MakesureLonglinkConnected_ext(_name)`.
    pub fn make_sure_long_link_connected(&mut self, name: &str) {
        if let Some(core) = self.core.as_mut() {
            core.make_sure_long_link_connected(name);
        }
    }

    /// `MakesureLonglinkConnected` — the default long link's.
    pub fn make_sure_default_long_link_connected(&mut self) {
        if let Some(core) = self.core.as_mut() {
            core.make_sure_default_long_link_connected();
        }
    }

    /// `LongLinkIsConnected`.
    pub fn is_default_long_link_connected(&self) -> bool {
        self.core
            .as_ref()
            .is_some_and(NetCore::is_default_long_link_connected)
    }

    /// `DefaultLongLink()` — the long link every task with an empty
    /// `channel_name` goes out on.
    pub fn default_long_link(&self) -> Option<Arc<Mutex<LongLink>>> {
        self.core
            .as_ref()
            .and_then(NetCore::default_long_link)
            .cloned()
    }

    /// `NetCore::default_link` — the name of the long link
    /// [`StnLogic::default_long_link`] answers.
    pub fn default_link(&self) -> Option<&str> {
        self.core.as_ref().and_then(NetCore::default_link)
    }

    //===------------------------------------------------------------------===//
    // the signalling
    //===------------------------------------------------------------------===//

    /// `KeepSignalling` — the mapping is kept open while the app is waiting for
    /// something.
    pub fn keep_signalling(&mut self) {
        if let Some(core) = self.core.as_mut() {
            core.keep_signal();
        }
    }

    /// `StopSignalling`.
    pub fn stop_signalling(&mut self) {
        if let Some(core) = self.core.as_mut() {
            core.stop_signal();
        }
    }

    /// `SetSignallingStrategy(_period, _keepTime)` — for every keeper in the
    /// process, which is why it is a free function and not a method: the C++'s
    /// `g_period` and `g_keepTime` are `static`s of `SignallingKeeper`, and what
    /// `StnManager` does with them is call it.
    pub fn set_signalling_strategy(&self, period: u64, keep_time: u64) {
        set_strategy(period, keep_time);
    }

    //===------------------------------------------------------------------===//
    // the net source
    //===------------------------------------------------------------------===//

    /// `SetLonglinkSvrAddr(host, ports, debugip)` — the long link's hosts, which
    /// is one host the C++ turns into a list of one.
    pub fn set_longlink_svr_addr(&mut self, host: &str, ports: Vec<u16>, debugip: &str) {
        let hosts = if host.is_empty() {
            Vec::new()
        } else {
            vec![host.to_string()]
        };
        if let Some(core) = self.core.as_mut() {
            core.net_source().set_longlink(hosts, ports, debugip);
        }
    }

    /// `SetShortlinkSvrAddr(port, debugip)`.
    pub fn set_shortlink_svr_addr(&mut self, port: u16, debugip: &str) {
        if let Some(core) = self.core.as_mut() {
            core.net_source().set_shortlink(port, debugip);
        }
    }

    /// `SetDebugIP(host, ip)` — a host that is reached without asking dns.
    pub fn set_debug_ip(&mut self, host: &str, ip: &str) {
        if let Some(core) = self.core.as_mut() {
            core.net_source().set_debug_ip(host, ip);
        }
    }

    /// `SetBackupIPs(host, iplist)` — the pairs a host falls back to.
    pub fn set_backup_ips(&mut self, host: &str, ips: Vec<String>) {
        if let Some(core) = self.core.as_mut() {
            core.net_source().set_backup_ips(host, ips);
        }
    }

    /// `GetLongLinkHosts()` — the hosts the long link goes out on.
    pub fn long_link_hosts(&mut self) -> Vec<String> {
        self.core
            .as_mut()
            .map_or_else(Vec::new, |core| core.net_source().longlink_hosts().to_vec())
    }

    //===------------------------------------------------------------------===//
    // what the app is told
    //===------------------------------------------------------------------===//

    /// `TrafficData(_send, _recv)` — how much went out and came in. The C++
    /// counts the log's own tag only, which it knows from a build-time define;
    /// here the tag is the host's business, because the host is the one whose
    /// traffic it is.
    pub fn traffic_data(&mut self, send: i64, recv: i64) {
        locked(&self.bridge).traffic_data(send, recv);
    }

    //===------------------------------------------------------------------===//
    // the wiring
    //===------------------------------------------------------------------===//

    /// Every question STN asks, answered by the app: the two queues' encode and
    /// decode, whether the app is logged in, the sequence a task is tied to its
    /// report by, and everything the net core, the net source, the network
    /// check and the timing sync ask the app about.
    fn wire(core: &mut NetCore, bridge: &Arc<Mutex<StnCallbackBridge>>) {
        let wired = Arc::clone(bridge);
        core.longlink().set_req2buf(move |task| {
            locked(&wired).req2buf(
                task.taskid,
                &task.user_id,
                task.channel_select,
                host_of(&task.longlink_host_list),
                task.client_sequence_id,
            )
        });
        let wired = Arc::clone(bridge);
        core.longlink().set_buf2resp(move |task, body| {
            locked(&wired).buf2resp(task.taskid, &task.user_id, body, task.channel_select)
        });
        let wired = Arc::clone(bridge);
        core.longlink().set_make_sure_authed(move |host, user_id| {
            locked(&wired).makesure_authed(host, user_id)
        });
        // the sequence is drawn here, not when the task is made, so a retry is
        // not taken for the request it is a retry of
        core.longlink().set_gen_sequence_id(gen_sequence_id);

        let wired = Arc::clone(bridge);
        core.shortlink().set_req2buf(move |task| {
            locked(&wired).req2buf(
                task.taskid,
                &task.user_id,
                task.channel_select,
                host_of(&task.shortlink_host_list),
                task.client_sequence_id,
            )
        });
        let wired = Arc::clone(bridge);
        core.shortlink().set_buf2resp(move |task, body| {
            locked(&wired).buf2resp(task.taskid, &task.user_id, body, task.channel_select)
        });
        let wired = Arc::clone(bridge);
        core.shortlink().set_make_sure_authed(move |host, user_id| {
            locked(&wired).makesure_authed(host, user_id)
        });

        let wired = Arc::clone(bridge);
        core.set_on_task_end(
            move |taskid, err_type, err_code, profile: &ConnectProfile| {
                locked(&wired).on_task_end(taskid, "", err_type, err_code, profile)
            },
        );
        let wired = Arc::clone(bridge);
        core.set_on_push(move |channel, cmdid, taskid, body| {
            locked(&wired).on_push(channel, cmdid, taskid, body)
        });
        let wired = Arc::clone(bridge);
        core.set_report_connect_status(move |all: NetStatus, longlink: NetStatus| {
            locked(&wired).report_connect_status(all, longlink)
        });
        let wired = Arc::clone(bridge);
        core.set_on_longlink_network_err(move |err_type, err_code, ip, port| {
            locked(&wired).on_long_link_network_error(err_type, err_code, ip, port)
        });
        let wired = Arc::clone(bridge);
        core.set_on_shortlink_network_err(move |err_type, err_code, ip, host, port| {
            locked(&wired).on_short_link_network_error(err_type, err_code, ip, host, port)
        });
        let wired = Arc::clone(bridge);
        core.set_on_longlink_status_change(move |status: LongLinkStatus| {
            locked(&wired).on_long_link_status_change(status)
        });

        let wired = Arc::clone(bridge);
        core.net_source()
            .set_new_dns(move |host, is_longlink, extra| {
                locked(&wired).on_new_dns(host, is_longlink, extra)
            });
        let wired = Arc::clone(bridge);
        core.netcheck()
            .set_request_short_link_hosts(move || locked(&wired).net_check_shortlink_hosts());
        let wired = Arc::clone(bridge);
        core.timing_sync()
            .set_request_sync(move || locked(&wired).request_sync());
    }
}

/// The bridge, without letting a panic in one thread take every thread with it.
fn locked(bridge: &Arc<Mutex<StnCallbackBridge>>) -> MutexGuard<'_, StnCallbackBridge> {
    bridge.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The host a task is going out on, which is the C++'s
/// `task.longlink_host_list.front()` / `task.shortlink_host_list.front()`: the
/// first of the hosts the app named, or nothing when it named none.
fn host_of(hosts: &[String]) -> &str {
    hosts.first().map_or("", String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_profile::TaskFailHandleType;
    use crate::DEFAULT_LONGLINK_NAME;
    use std::sync::mpsc;

    /// An app that writes down what it was asked and answers the way a sample
    /// wants.
    struct Rec {
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl App for Rec {
        fn makesure_authed(&mut self, host: &str, _user_id: &str) -> bool {
            self.asked.lock().unwrap().push(format!("authed {host}"));
            true
        }

        fn req2buf(
            &mut self,
            taskid: u32,
            _user_id: &str,
            _channel_select: i32,
            host: &str,
            _sequence: u16,
        ) -> Result<Vec<u8>, i32> {
            self.asked
                .lock()
                .unwrap()
                .push(format!("req2buf {taskid} {host}"));
            Ok(b"body".to_vec())
        }

        fn buf2resp(
            &mut self,
            taskid: u32,
            _user_id: &str,
            _body: &[u8],
            _channel_select: i32,
        ) -> (i32, TaskFailHandleType) {
            self.asked
                .lock()
                .unwrap()
                .push(format!("buf2resp {taskid}"));
            (0, TaskFailHandleType::Normal)
        }

        fn on_new_dns(
            &mut self,
            host: &str,
            _longlink_host: bool,
            _extra: &crate::net_source::ExtraInfo,
        ) -> Vec<String> {
            self.asked.lock().unwrap().push(format!("newdns {host}"));
            vec!["10.0.0.1".to_string()]
        }

        fn net_check_shortlink_hosts(&mut self) -> Vec<String> {
            self.asked.lock().unwrap().push("hosts".to_string());
            vec!["check.host".to_string()]
        }

        fn request_sync(&mut self) {
            self.asked.lock().unwrap().push("sync".to_string());
        }
    }

    /// A logic with a core in it and an app that answers.
    fn logic() -> (StnLogic, Arc<Mutex<Vec<String>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let mut logic = StnLogic::new();
        logic.set_callback(Rec {
            asked: Arc::clone(&asked),
        });
        assert!(logic.create_at(1000));
        (logic, asked)
    }

    fn asked_of(cell: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        cell.lock().unwrap().clone()
    }

    #[test]
    fn a_logic_with_no_core_answers_the_way_the_cpp_warns() {
        let mut logic = StnLogic::new();
        assert!(!logic.is_created());
        assert!(logic.net_core().is_none());

        // every call the C++ answers with a warning and a `false`
        assert!(!logic.start_task(Task::new(7, 12)));
        assert!(!logic.stop_task(7));
        assert!(!logic.has_task(7));
        assert!(!logic.destroy_long_link("link"));
        assert!(!logic.mark_main_longlink("link"));
        assert!(!logic.is_long_link_connected("link"));
        assert!(!logic.is_default_long_link_connected());
        assert_eq!(logic.due_time(), None);
        assert!(!logic.has_pending());
        assert_eq!(logic.pending_count(), 0);
        assert_eq!(logic.long_link_hosts(), Vec::<String>::new());
        assert!(logic.default_long_link().is_none());
        assert!(logic
            .create_long_link(LonglinkConfig::new("link"))
            .is_none());
        assert!(!logic.destroy());

        // and none of these panics
        logic.redo_tasks();
        logic.touch_tasks();
        logic.clear_tasks();
        logic.disable_long_link();
        logic.run_pending();
        logic.keep_signalling();
        logic.stop_signalling();
        logic.make_sure_default_long_link_connected();
        logic.set_longlink_svr_addr("host", vec![80], "");
        logic.set_shortlink_svr_addr(8080, "");
        logic.set_debug_ip("host", "1.2.3.4");
        logic.set_backup_ips("host", vec!["1.2.3.4".to_string()]);
        logic.set_active(true);
        let mut changed = false;
        logic.on_network_change(|| changed = true);
        assert!(!changed, "the host's own change runs after the core's");
    }

    #[test]
    fn a_core_is_made_once_and_destroyed_once() {
        let mut logic = StnLogic::new();
        assert!(logic.create_at(1000));
        assert!(!logic.create_at(2000), "the second create does nothing");
        assert!(logic.is_created());

        assert!(logic.destroy());
        assert!(!logic.is_created());
        assert!(!logic.destroy(), "the second destroy does nothing");

        // `Reset` makes one whether there was one or not
        logic.reset_at(3000);
        assert!(logic.is_created());
    }

    #[test]
    fn the_encoder_the_core_is_made_with_is_the_one_the_app_set() {
        let mut logic = StnLogic::new();
        logic.on_init_config_before_on_create_v2(2, "encoder");
        assert!(logic.create_at(1000));

        let core = logic.net_core().expect("no core");
        assert_eq!(core.packer_encoder_version(), 2);
        assert_eq!(core.packer_encoder_name(), "encoder");
    }

    #[test]
    fn a_network_change_runs_the_hosts_own_change_first() {
        let (mut logic, _asked) = logic();
        let (sent, heard) = mpsc::channel();
        logic.on_network_change(move || sent.send("host").unwrap());
        assert_eq!(heard.try_recv().unwrap(), "host");
    }

    #[test]
    fn the_app_is_the_one_the_core_asks() {
        let (mut logic, asked) = logic();
        let core = logic.net_core().expect("no core");
        core.net_source()
            .set_longlink(vec!["long.host".to_string()], vec![80], "");

        // a dns question the app answers, and the sync the timing sync is due
        // for
        let items = core.net_source().get_longlink_items_at(
            1000,
            &LonglinkConfig::new("link"),
            &crate::net_source::ExtraInfo::new(),
        );
        assert!(
            items.iter().any(|item| item.ip == "10.0.0.1"),
            "the app's own ip is the one the long link is tried on"
        );
        core.timing_sync().on_alarm_at(1000);

        assert_eq!(
            asked_of(&asked),
            vec!["newdns long.host".to_string(), "sync".to_string()]
        );
    }

    #[test]
    fn an_address_the_app_set_is_the_one_the_net_source_keeps() {
        let mut logic = StnLogic::new();
        assert!(logic.create_at(1000));

        logic.set_longlink_svr_addr("long.host", vec![80, 443], "1.2.3.4");
        logic.set_shortlink_svr_addr(8080, "");
        logic.set_debug_ip("short.host", "5.6.7.8");
        logic.set_backup_ips("short.host", vec!["9.9.9.9".to_string()]);

        assert_eq!(logic.long_link_hosts(), vec!["long.host".to_string()]);
        let core = logic.net_core().expect("no core");
        assert_eq!(core.net_source().longlink_ports(), vec![80, 443]);
        assert_eq!(core.net_source().shortlink_port(), 8080);
        assert_eq!(core.net_source().longlink_debug_ip(), "1.2.3.4");
        assert_eq!(
            core.net_source().backup_ips("short.host"),
            vec!["9.9.9.9".to_string()]
        );
    }

    #[test]
    fn task_ids_are_handed_out_one_after_the_other() {
        // the counter is one for the whole process
        let _guard = crate::test_lock();
        let first = gen_task_id();
        let second = gen_task_id();
        assert_eq!(second, first + 1);
        assert!(second < RESERVED_TASK_ID_START);
    }

    #[test]
    fn a_sequence_id_is_a_different_one_every_time() {
        let _guard = crate::test_lock();
        let first = gen_sequence_id();
        let second = gen_sequence_id();
        assert_ne!(first, second, "a retry is not the request it retries");
    }

    #[test]
    fn what_a_core_is_asked_through_the_logic_is_asked_of_the_core() {
        let (mut logic, asked) = logic();

        // everything here is one line over the core in the C++ too, so what a
        // sample pins down is that none of it is lost on the way
        logic.set_active(true);
        logic.redo_tasks_at(1000);
        logic.touch_tasks_at(1000);
        logic.make_sure_long_link_connected(DEFAULT_LONGLINK_NAME);
        logic.make_sure_default_long_link_connected();
        logic.keep_signalling();
        logic.stop_signalling();
        logic.run_pending_at(1000);
        logic.clear_tasks();
        assert!(logic.due_time().is_some(), "the heartbeat is due");
        assert!(!logic.has_pending());
        assert_eq!(logic.pending_count(), 0);

        // the signalling strategy is a `static` of the keeper's own, so the
        // logic does not even have to have a core to set it
        StnLogic::new().set_signalling_strategy(1000, 2000);

        assert!(!logic.has_task(7));
        assert!(!logic.stop_task(7));
        assert_eq!(asked_of(&asked), Vec::<String>::new());
    }

    #[test]
    fn an_encoder_the_app_set_is_the_one_the_next_core_is_made_with() {
        let mut logic = StnLogic::default();
        assert!(!logic.is_created(), "a default logic is one with no core");

        // the version alone, then the version and the name
        logic.on_init_config_before_on_create(3);
        assert_eq!(logic.encoder_version, 3);
        logic.reset_with_encoder_at(1000, 4, "encoder");
        let core = logic.net_core().expect("no core");
        assert_eq!(core.packer_encoder_version(), 4);
        assert_eq!(core.packer_encoder_name(), "encoder");

        // and an encoder of the app's own, which `Reset` keeps
        let mut encoder = LongLinkEncoder::new();
        encoder.set_encoder_version(5);
        logic.set_default_longlink_encoder(encoder);
        logic.reset_at(2000);
        let core = logic.net_core().expect("no core");
        assert_eq!(
            core.packer_encoder_version(),
            4,
            "the name's, not the encoder's"
        );
    }

    #[test]
    fn the_app_is_the_one_the_network_check_asks_for_its_hosts() {
        let (mut logic, asked) = logic();
        let core = logic.net_core().expect("no core");

        // a diagnosis is worth starting: sixteen tasks that answered and then
        // eight that did not, which is what the C++'s window says is broken now
        core.netcheck()
            .set_long_link_hosts(|| vec!["long.example".to_string()]);
        core.netcheck().set_long_link_ports(|| vec![80]);
        core.netcheck().set_short_link_port(|| 8080);
        for _ in 0..16 {
            core.netcheck().update_long_link_info_at(400_000, 0, true);
        }
        for _ in 0..8 {
            core.netcheck().update_long_link_info_at(400_000, 0, false);
        }

        // and the short hosts it runs on are the app's
        assert_eq!(asked_of(&asked), vec!["hosts".to_string()]);
    }

    #[test]
    fn a_traffic_report_goes_to_the_app_through_the_bridge() {
        let mut logic = StnLogic::new();
        logic.set_callback(Rec {
            asked: Arc::new(Mutex::new(Vec::new())),
        });

        // the C++ counts its own log tag only, so what is here is the report
        // itself, which the app is the one that answers
        logic.traffic_data(100, 200);
    }

    #[test]
    fn the_debug_of_a_logic_says_whether_there_is_a_core() {
        let mut logic = StnLogic::new();
        assert_eq!(
            format!("{logic:?}"),
            "StnLogic { core: false, bridge: false, encoder_version: 0, encoder_name: \"\" }"
        );

        logic.set_callback(Rec {
            asked: Arc::new(Mutex::new(Vec::new())),
        });
        logic.on_init_config_before_on_create_v2(2, "encoder");
        assert!(logic.create_at(1000));
        assert_eq!(
            format!("{logic:?}"),
            "StnLogic { core: true, bridge: true, encoder_version: 2, encoder_name: \"encoder\" }"
        );
    }
}
