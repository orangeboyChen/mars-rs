//! `mars/stn/src/net_core.{h,cc}` — the net core: the two queues, the pieces
//! they share, and where a task that ended goes.
//!
//! The C++'s `NetCore` is the one value that owns the whole of STN — the net
//! source, the anti-avalanche check, the dynamic timeout, the two task queues,
//! the zombies, the timing sync and one `LongLinkMetaData` per long link — and
//! the wiring between them: which queue a task is started on, what a task that
//! ended is routed to, and who is told about an error. There is no logic of its
//! own in it that the slices before this one did not already carry, which is
//! what makes it the last piece: everything it does is *which* of the things
//! that exist talks to *which*.
//!
//! Three things the C++ gets from a platform are hooks here, which is what the
//! rest of the port does with them:
//!
//! * `context_->GetManager<StnManager>()->…` — `OnTaskEnd`, `OnPush`,
//!   `ReportConnectStatus`, `OnLongLinkNetworkError`, `OnShortLinkNetworkError`
//!   and `OnLongLinkStatusChange` are [`NetCore::set_on_task_end`] and friends.
//!   `user_context` — a `void*` the C++ hands straight back — is not ported, but
//!   `user_id` is: the C++ hands `task.user_id` to `OnTaskEnd` with the task,
//!   and an app that keeps one STN for several accounts needs it. The identify
//!   check's two questions are [`NetCore::set_identify_check_buffer`] and
//!   [`NetCore::set_identify_on_response`]: the C++'s checker pulls them out of
//!   the manager when it needs them, so every link the core keeps — and every
//!   one it makes afterwards — is wired to them.
//! * `ActiveLogic` — `IsForeground`, `LastForegroundChangeTime` and
//!   `IsActive` are [`NetCore::set_active`] plus a `NetInfo` hook; the
//!   `MakeSureConnected` a foreground task asks for is not ported, because a
//!   port has no notion of a foreground.
//! * the `MessageQueue` — the C++ `StartTask`, `RetryTasks` and the two
//!   network-error handlers post themselves to it, which is what lets a queue
//!   re-enter the net core from inside its own run. A `&mut` that a value
//!   handed out is not something Rust lets it use while another of its fields
//!   is the one running, so what those four ask for is collected in a queue
//!   ([`NetCore::run_pending`] is what the message queue thread would have
//!   done with it) — the same deferral, said in a type instead of in a thread.
//!   Everything else is answered where it is asked, which is what the C++
//!   does too: `fun_callback_` is not posted.
//!
//! Not ported: the wake lock and the android-only branches, the tls group
//! name, the `get_real_host` / handshake / intercept setters (`SetGetRealHostFunc`
//! and friends are one line each over a queue the port leaves to the host), the
//! minor long link (`AddMinorLongLink`, `IsMinorAvailable`, `FixMinorRealhost` —
//! a second link the C++ makes out of a host list the app hands in, which
//! nothing in the port makes), `__OnShortLinkResponse` (nothing but a log), and
//! `__ResetLongLink` (an `#ifdef __APPLE__` that is commented out).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use mars_comm::tickcount::gettickcount;

use crate::anti_avalanche::AntiAvalanche;
use crate::dynamic_timeout::{DynamicTimeout, NetworkKind};
use crate::long_link::LongLink;
use crate::longlink_identify_checker::{
    GetIdentifyCheckBuffer, IdentifyBuffer, OnIdentifyResponse,
};
use crate::net_source::NO_NET;
use crate::task_profile::{
    ConnectProfile, ErrCmdType, PrepareProfile, TaskFailHandleType, LOCAL_CHANNEL_SELECT,
    LOCAL_NO_NET, LOCAL_RESET, LOCAL_START_TASK_FAIL, LOCAL_TASK_PARAM,
};
use crate::{
    ChannelFactory, DisconnectInternalCode, LongLinkEncoder, LongLinkMetaData, LongLinkStatus,
    LongLinkTaskManager, LonglinkConfig, NetCheckLogic, NetSource, ShortLinkTaskManager, Task,
    TimingSync, ZombieTaskManager, DEFAULT_LONGLINK_GROUP,
};

/// `DEFAULT_LONGLINK_NAME` — the channel every task with an empty
/// `channel_name` is started on, which is what `stn.cc` fills in before the
/// net core sees the task.
pub const DEFAULT_LONGLINK_NAME: &str = "default-longlink";

/// `DEF_TASK_RETRY_COUNT` — what a task that asked for a negative
/// `retry_count` gets.
pub const DEF_TASK_RETRY_COUNT: i32 = 1;

/// `kFastSendUseLonglinkTaskCntLimit` — a `kChannelFastStrategy` task is only
/// put on the long link while no other task of that channel is out.
pub const FAST_SEND_LONGLINK_TASK_CNT_LIMIT: usize = 0;

/// `kShortlinkErrTime` — how many short-link errors in a row make the whole
/// connection `ServerFailed`.
pub const SHORTLINK_ERR_TIME: i32 = 3;

/// `kMobile` — one of the [`NetInfo`] answers, next to [`NO_NET`] and
/// [`crate::NET_TYPE_WIFI`].
pub const NET_TYPE_MOBILE: i32 = 2;

/// `enum { kCallFromLong, kCallFromShort, kCallFromZombie }` — which queue the
/// task that ended came from, which is what decides whether it can be saved as
/// a zombie: a zombie that is saved again would never end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallFrom {
    /// `kCallFromLong`.
    Long,
    /// `kCallFromShort`.
    Short,
    /// `kCallFromZombie`.
    Zombie,
}

/// `NetStatus` — the connection as the app is asked to see it: one answer for
/// "can STN reach anything at all" and one for the long link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetStatus {
    /// `kNetworkUnkown` — nothing has tried yet.
    #[default]
    Unknown = -1,
    /// `kNetworkUnavailable`.
    Unavailable = 0,
    /// `kGateWayFailed`.
    GatewayFailed = 1,
    /// `kServerFailed`.
    ServerFailed = 2,
    /// `kConnecting`.
    Connecting = 3,
    /// `kConnected`.
    Connected = 4,
    /// `kServerDown`.
    ServerDown = 5,
}

/// `task_process_hook_` — the last chance to change a task before it goes out.
pub type TaskProcess = dyn FnMut(&mut Task) + Send;

/// `task_callback_hook_` — a task that ended, and what the app says about it.
/// `0` is "the net core is done with it", which is what stops the task from
/// being handed to [`OnTaskEnd`] or saved as a zombie.
pub type TaskCallback =
    dyn FnMut(CallFrom, ErrCmdType, i32, TaskFailHandleType, &Task) -> i32 + Send;

/// `StnManager::OnTaskEnd` — a task that is over, and the user it was started
/// for: the C++ hands `task.user_id` along, so an app that keeps one STN for
/// several accounts still knows which of them the task was. The answer is the
/// code the task is remembered with, which for a short-link task that did go out
/// is the app's own return code.
pub type OnTaskEnd = dyn FnMut(u32, &str, ErrCmdType, i32, &ConnectProfile) -> i32 + Send;

/// `push_preprocess_signal_` — a push, before the app is given it.
pub type PushPreprocess = dyn FnMut(u32, &[u8]) + Send;

/// `StnManager::OnPush` — a push, as `(channel_id, cmdid, taskid, body)`. The
/// C++ hands an `AutoBuffer` extension over too, which is the app's own
/// decoder's.
pub type OnPush = dyn FnMut(&str, u32, u32, &[u8]) + Send;

/// `StnManager::ReportConnectStatus(all, longlink)`.
pub type ReportConnectStatus = dyn FnMut(NetStatus, NetStatus) + Send;

/// `StnManager::OnLongLinkNetworkError` — only the main link's errors are the
/// app's business.
pub type OnLongLinkNetworkError = dyn FnMut(ErrCmdType, i32, &str, u16) + Send;

/// `StnManager::OnShortLinkNetworkError`.
pub type OnShortLinkNetworkError = dyn FnMut(ErrCmdType, i32, &str, &str, u16) + Send;

/// `StnManager::OnLongLinkStatusChange`.
pub type OnLongLinkStatusChange = dyn FnMut(LongLinkStatus) + Send;

/// `getNetInfo()` — [`NO_NET`], [`crate::NET_TYPE_WIFI`] or
/// [`NET_TYPE_MOBILE`]. One hook the net core, the two queues, the net source,
/// the timing sync and the anti-avalanche check all read, which is why
/// [`NetCore::set_net_info`] is the one place to set it.
pub type NetInfo = dyn FnMut() -> i32 + Send;

/// `time(NULL)` — the unix second the two ip reports are stamped with. Unset
/// is the clock of the machine the port runs on.
pub type Clock = dyn FnMut() -> u64 + Send;

fn poisoned<T>(poisoned: PoisonError<T>) -> T {
    poisoned.into_inner()
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// What the C++ posts to its `MessageQueue`: a queue that is running cannot
/// re-enter the net core, so what it asks for is collected and
/// [`NetCore::run_pending`] does it.
#[derive(Debug)]
enum FollowUp {
    /// `NetCore::StartTask` — a zombie that is started again. Boxed because a
    /// `Task` is much bigger than the rest of these.
    Start(Box<Task>),
    /// `NetCore::RetryTasks` — an answer that could not be read, or a session
    /// timeout: every task of that user has to be looked at again.
    Retry {
        err_type: ErrCmdType,
        err_code: i32,
        handle: TaskFailHandleType,
        src_taskid: u32,
        user_id: String,
    },
    /// `NetCore::__OnLongLinkNetworkError`.
    LongLinkError {
        name: String,
        err_type: ErrCmdType,
        err_code: i32,
        ip: String,
        port: u16,
    },
    /// `NetCore::__OnShortLinkNetworkError`.
    ShortLinkError {
        err_type: ErrCmdType,
        err_code: i32,
        ip: String,
        host: String,
        port: u16,
    },
}

/// The hooks the two queues reach themselves, without the net core being asked:
/// a task that ended, and a push. Everything else the app is told comes from
/// [`NetCore`] itself.
#[derive(Default)]
struct Hooks {
    task_callback: Option<Box<TaskCallback>>,
    on_task_end: Option<Box<OnTaskEnd>>,
    push_preprocess: Option<Box<PushPreprocess>>,
    on_push: Option<Box<OnPush>>,
}

impl std::fmt::Debug for Hooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hooks")
            .field("task_callback", &self.task_callback.is_some())
            .field("on_task_end", &self.on_task_end.is_some())
            .field("push_preprocess", &self.push_preprocess.is_some())
            .field("on_push", &self.on_push.is_some())
            .finish()
    }
}

/// `NetCore`.
pub struct NetCore {
    /// `net_source_`.
    net_source: NetSource,
    /// `netcheck_logic_`.
    netcheck: NetCheckLogic,
    /// `dynamic_timeout_`.
    dynamic_timeout: DynamicTimeout,
    /// `shortlink_task_manager_`.
    shortlink: ShortLinkTaskManager,
    /// `longlink_task_manager_`.
    longlink: LongLinkTaskManager,
    /// `timing_sync_`.
    timing_sync: TimingSync,
    /// `longlink_metas_` — the C++ keeps these in the long-link queue; the port
    /// keeps the *names* there (a channel is a name and a bundle of hooks) and
    /// the links themselves here, because a link is what the multi-long-link
    /// APIs hand out.
    links: HashMap<String, LongLinkMetaData>,
    /// Which of [`NetCore::links`] is `Config().isMain()`: the C++ marks the
    /// config, and a port answers with the name instead of writing to a value
    /// the link was made from.
    default_link: Option<String>,
    /// `LongLinkChannelFactory` / `ShortLinkChannelFactory`.
    factory: ChannelFactory,

    /// `anti_avalanche_` — the one piece a queue reaches *while it is running*
    /// and that nothing else does, which is why it is the one that is shared
    /// instead of deferred.
    anti_avalanche: Arc<Mutex<AntiAvalanche>>,
    /// `zombie_task_manager_` — shared because the queues' `fun_callback_`
    /// saves to it.
    zombie: Arc<Mutex<ZombieTaskManager>>,
    /// what the C++ posts to its message queue.
    pending: Arc<Mutex<VecDeque<FollowUp>>>,
    /// `getNetInfo()`, shared with everything that asks.
    net_info: Arc<Mutex<Box<NetInfo>>>,
    /// What the two queues ask for themselves.
    hooks: Arc<Mutex<Hooks>>,

    /// `need_use_longlink_`.
    use_long_link: bool,
    /// `already_release_net_`.
    released: bool,
    /// `shortlink_error_count_`.
    shortlink_error_count: i32,
    /// `shortlink_try_flag_`.
    shortlink_try_flag: bool,
    /// `all_connect_status_`.
    all_status: NetStatus,
    /// `longlink_connect_status_`.
    longlink_status: NetStatus,
    /// `packer_encoder_version_`.
    packer_encoder_version: i32,
    /// `packer_encoder_name_`.
    packer_encoder_name: String,
    /// The encoder every long link is made with.
    encoder: LongLinkEncoder,

    /// `task_process_hook_`.
    task_process: Option<Box<TaskProcess>>,
    /// `StnManager::ReportConnectStatus`.
    report_connect_status: Option<Box<ReportConnectStatus>>,
    /// `StnManager::OnLongLinkNetworkError`.
    on_longlink_network_err: Option<Box<OnLongLinkNetworkError>>,
    /// `StnManager::OnShortLinkNetworkError`.
    on_shortlink_network_err: Option<Box<OnShortLinkNetworkError>>,
    /// `StnManager::OnLongLinkStatusChange`.
    on_longlink_status_change: Option<Box<OnLongLinkStatusChange>>,
    /// `GetLonglinkIdentifyCheckBuffer` — what every long link's identify
    /// checker asks the app for. One answer shared by every link, because the
    /// C++'s checker asks for it when it needs it rather than being handed it.
    identify_buffer: Arc<Mutex<Option<Box<GetIdentifyCheckBuffer>>>>,
    /// `OnLonglinkIdentifyResponse` — the app's verdict on the answer.
    identify_response: Arc<Mutex<Option<Box<OnIdentifyResponse>>>>,
    clock: Option<Box<Clock>>,
}

impl NetCore {
    /// `NetCore(...)` — the pieces, wired to each other, and one long link
    /// ([`DEFAULT_LONGLINK_NAME`]) made and marked main.
    pub fn new() -> Self {
        Self::new_at(gettickcount())
    }

    /// The same, with the reading the C++'s `gettickcount()` would hand out.
    pub fn new_at(now: u64) -> Self {
        Self::with_encoder_at(now, true, LongLinkEncoder::new())
    }

    /// `NetCore(..., _use_long_link, longlink_encoder, ...)`.
    pub fn with_encoder(use_long_link: bool, encoder: LongLinkEncoder) -> Self {
        Self::with_encoder_at(gettickcount(), use_long_link, encoder)
    }

    /// The same, with the reading handed in.
    pub fn with_encoder_at(now: u64, use_long_link: bool, encoder: LongLinkEncoder) -> Self {
        let mut core = Self {
            net_source: NetSource::new_at(now),
            netcheck: NetCheckLogic::new_at(now),
            dynamic_timeout: DynamicTimeout::new(),
            shortlink: ShortLinkTaskManager::new(),
            longlink: LongLinkTaskManager::new(),
            timing_sync: TimingSync::new_at(now),
            links: HashMap::new(),
            default_link: None,
            factory: ChannelFactory::new(),
            anti_avalanche: Arc::new(Mutex::new(AntiAvalanche::new_at(false, now))),
            zombie: Arc::new(Mutex::new(ZombieTaskManager::new_at(now))),
            pending: Arc::new(Mutex::new(VecDeque::new())),
            net_info: Arc::new(Mutex::new(Box::new(|| crate::NET_TYPE_WIFI))),
            hooks: Arc::new(Mutex::new(Hooks::default())),
            use_long_link,
            released: false,
            shortlink_error_count: 0,
            shortlink_try_flag: false,
            all_status: NetStatus::Unavailable,
            longlink_status: NetStatus::Unavailable,
            packer_encoder_version: 0,
            packer_encoder_name: String::new(),
            encoder,
            task_process: None,
            report_connect_status: None,
            on_longlink_network_err: None,
            on_shortlink_network_err: None,
            on_longlink_status_change: None,
            identify_buffer: Arc::new(Mutex::new(None)),
            identify_response: Arc::new(Mutex::new(None)),
            clock: None,
        };
        // `defaultConfig.longlink_encoder = default_longlink_encoder`: every
        // link the core makes is made with the encoder it was given, until the
        // app replaces the factory with its own
        let encoder = core.encoder;
        core.factory
            .set_create_longlink(move |config| LongLink::with_encoder(config.clone(), encoder));
        core.wire();

        if use_long_link {
            let mut config = LonglinkConfig::new(DEFAULT_LONGLINK_NAME);
            config.is_keep_alive = true;
            config.is_main = true;
            config.group = DEFAULT_LONGLINK_GROUP.to_string();
            core.create_long_link(config);
        }
        core
    }

    /// `__InitShortLink` + `__InitLongLink` — what the queues ask of the net
    /// core, and what the net core asks of the anti-avalanche check.
    fn wire(&mut self) {
        let pending = Arc::clone(&self.pending);
        self.shortlink.set_notify_retry_all_tasks(
            move |err_type, err_code, handle, src_taskid, user_id| {
                push(
                    &pending,
                    FollowUp::Retry {
                        err_type,
                        err_code,
                        handle,
                        src_taskid,
                        user_id: user_id.to_string(),
                    },
                );
            },
        );
        let pending = Arc::clone(&self.pending);
        self.shortlink
            .set_notify_network_err(move |err_type, err_code, ip, host, port| {
                push(
                    &pending,
                    FollowUp::ShortLinkError {
                        err_type,
                        err_code,
                        ip: ip.to_string(),
                        host: host.to_string(),
                        port,
                    },
                );
            });

        let pending = Arc::clone(&self.pending);
        self.longlink.set_notify_retry_all_tasks(
            move |err_type, err_code, handle, src_taskid, user_id| {
                push(
                    &pending,
                    FollowUp::Retry {
                        err_type,
                        err_code,
                        handle,
                        src_taskid,
                        user_id: user_id.to_string(),
                    },
                );
            },
        );
        let pending = Arc::clone(&self.pending);
        self.longlink
            .set_notify_network_err(move |name, err_type, err_code, ip, port| {
                push(
                    &pending,
                    FollowUp::LongLinkError {
                        name: name.to_string(),
                        err_type,
                        err_code,
                        ip: ip.to_string(),
                        port,
                    },
                );
            });

        let hooks = Arc::clone(&self.hooks);
        let zombie = Arc::clone(&self.zombie);
        let use_long_link = self.use_long_link;
        self.shortlink
            .set_callback(move |err_type, err_code, handle, task, cost, profile| {
                call_back(
                    &hooks,
                    &zombie,
                    use_long_link,
                    gettickcount(),
                    CallFrom::Short,
                    err_type,
                    err_code,
                    handle,
                    task,
                    cost,
                    profile,
                )
            });

        let hooks = Arc::clone(&self.hooks);
        let zombie = Arc::clone(&self.zombie);
        let use_long_link = self.use_long_link;
        self.longlink
            .set_callback(move |err_type, err_code, handle, task, cost, profile| {
                call_back(
                    &hooks,
                    &zombie,
                    use_long_link,
                    gettickcount(),
                    CallFrom::Long,
                    err_type,
                    err_code,
                    handle,
                    task,
                    cost,
                    profile,
                )
            });

        let hooks = Arc::clone(&self.hooks);
        self.longlink.set_on_push(move |name, cmdid, taskid, body| {
            let mut hooks = hooks.lock().unwrap_or_else(poisoned);
            if let Some(preprocess) = hooks.push_preprocess.as_mut() {
                preprocess(cmdid, body);
            }
            if let Some(on_push) = hooks.on_push.as_mut() {
                on_push(name, cmdid, taskid, body);
            }
        });

        let hooks = Arc::clone(&self.hooks);
        let zombie = Arc::clone(&self.zombie);
        self.zombie.lock().unwrap_or_else(poisoned).set_callback(
            move |err_type, err_code, handle, task, cost| {
                call_back(
                    &hooks,
                    &zombie,
                    true,
                    gettickcount(),
                    CallFrom::Zombie,
                    err_type,
                    err_code,
                    handle,
                    task,
                    cost,
                    &ConnectProfile::new(),
                );
            },
        );
        let pending = Arc::clone(&self.pending);
        self.zombie()
            .set_start_task(move |task| push(&pending, FollowUp::Start(Box::new(task.clone()))));

        let avalanche = Arc::clone(&self.anti_avalanche);
        let net_info = Arc::clone(&self.net_info);
        self.shortlink.set_anti_avalanche_check(move |task, body| {
            anti_avalanche_check(&avalanche, &net_info, task, body)
        });
        let avalanche = Arc::clone(&self.anti_avalanche);
        let net_info = Arc::clone(&self.net_info);
        self.longlink.set_anti_avalanche_check(move |task, body| {
            anti_avalanche_check(&avalanche, &net_info, task, body)
        });

        let net_info = Arc::clone(&self.net_info);
        self.net_source
            .set_net_info(move || net_info.lock().unwrap_or_else(poisoned)());
        let net_info = Arc::clone(&self.net_info);
        self.timing_sync
            .set_net_info(move || net_info.lock().unwrap_or_else(poisoned)());
        let net_info = Arc::clone(&self.net_info);
        self.shortlink.set_net_info(move || {
            if net_info.lock().unwrap_or_else(poisoned)() == NET_TYPE_MOBILE {
                NetworkKind::Mobile
            } else {
                NetworkKind::Wifi
            }
        });
        let net_info = Arc::clone(&self.net_info);
        self.longlink.set_net_info(move || {
            if net_info.lock().unwrap_or_else(poisoned)() == NET_TYPE_MOBILE {
                NetworkKind::Mobile
            } else {
                NetworkKind::Wifi
            }
        });
    }

    //===------------------------------------------------------------------===//
    // the app's own hooks
    //===------------------------------------------------------------------===//

    /// `task_process_hook_`.
    pub fn set_task_process(&mut self, process: impl FnMut(&mut Task) + Send + 'static) {
        self.task_process = Some(Box::new(process));
    }

    /// `task_callback_hook_`.
    pub fn set_task_callback(
        &mut self,
        callback: impl FnMut(CallFrom, ErrCmdType, i32, TaskFailHandleType, &Task) -> i32
            + Send
            + 'static,
    ) {
        self.hooks.lock().unwrap_or_else(poisoned).task_callback = Some(Box::new(callback));
    }

    /// `StnManager::OnTaskEnd`.
    pub fn set_on_task_end(
        &mut self,
        end: impl FnMut(u32, &str, ErrCmdType, i32, &ConnectProfile) -> i32 + Send + 'static,
    ) {
        self.hooks.lock().unwrap_or_else(poisoned).on_task_end = Some(Box::new(end));
    }

    /// `push_preprocess_signal_`.
    pub fn set_push_preprocess(&mut self, preprocess: impl FnMut(u32, &[u8]) + Send + 'static) {
        self.hooks.lock().unwrap_or_else(poisoned).push_preprocess = Some(Box::new(preprocess));
    }

    /// `StnManager::OnPush`.
    pub fn set_on_push(&mut self, push: impl FnMut(&str, u32, u32, &[u8]) + Send + 'static) {
        self.hooks.lock().unwrap_or_else(poisoned).on_push = Some(Box::new(push));
    }

    /// `StnManager::ReportConnectStatus`.
    pub fn set_report_connect_status(
        &mut self,
        report: impl FnMut(NetStatus, NetStatus) + Send + 'static,
    ) {
        self.report_connect_status = Some(Box::new(report));
    }

    /// `StnManager::OnLongLinkNetworkError`.
    pub fn set_on_longlink_network_err(
        &mut self,
        err: impl FnMut(ErrCmdType, i32, &str, u16) + Send + 'static,
    ) {
        self.on_longlink_network_err = Some(Box::new(err));
    }

    /// `StnManager::OnShortLinkNetworkError`.
    pub fn set_on_shortlink_network_err(
        &mut self,
        err: impl FnMut(ErrCmdType, i32, &str, &str, u16) + Send + 'static,
    ) {
        self.on_shortlink_network_err = Some(Box::new(err));
    }

    /// `StnManager::OnLongLinkStatusChange`.
    pub fn set_on_longlink_status_change(
        &mut self,
        change: impl FnMut(LongLinkStatus) + Send + 'static,
    ) {
        self.on_longlink_status_change = Some(Box::new(change));
    }

    /// `GetLonglinkIdentifyCheckBuffer` — the check a long link is asked for
    /// before it is used, and the app's answer to it.
    ///
    /// The C++'s `LongLinkIdentifyChecker` reaches the `StnManager` through its
    /// context whenever it needs the buffer, so a link made before the app
    /// answered is asked all the same: every link the net core keeps is wired to
    /// this, and so is every one it makes afterwards.
    pub fn set_identify_check_buffer(
        &mut self,
        check_buffer: impl FnMut(&str, u32) -> IdentifyBuffer + Send + 'static,
    ) {
        *self.identify_buffer.lock().unwrap_or_else(poisoned) = Some(Box::new(check_buffer));
        self.wire_identify();
    }

    /// `OnLonglinkIdentifyResponse` — the app's verdict on the answer: whether
    /// the link it came in on is one the app trusts.
    pub fn set_identify_on_response(
        &mut self,
        on_response: impl FnMut(&str, &[u8], &[u8]) -> bool + Send + 'static,
    ) {
        *self.identify_response.lock().unwrap_or_else(poisoned) = Some(Box::new(on_response));
        self.wire_identify();
    }

    /// Every link the core keeps, wired to what the app answered: a link that
    /// was made before is asked as well, which is what pulling the buffer out of
    /// the manager when it is needed amounts to.
    fn wire_identify(&mut self) {
        let names: Vec<String> = self.links.keys().cloned().collect();
        for name in names {
            self.wire_link_identify(&name);
        }
    }

    /// One link: the checker asks the app through the two slots, and a link
    /// nobody answered for gets [`IdentifyBuffer::never`] — the C++'s own
    /// `kCheckNever` — and a verdict of `false`.
    fn wire_link_identify(&mut self, name: &str) {
        let buffer = Arc::clone(&self.identify_buffer);
        let response = Arc::clone(&self.identify_response);
        let Some(meta) = self.links.get(name) else {
            return;
        };
        let mut link = meta.channel().lock().unwrap_or_else(poisoned);
        link.set_identify_check_buffer(move |channel_id, cmdid| {
            buffer.lock().unwrap_or_else(poisoned).as_mut().map_or_else(
                || IdentifyBuffer::never(Vec::new()),
                |ask| ask(channel_id, cmdid),
            )
        });
        link.set_identify_on_response(move |channel_id, answer, hash| {
            response
                .lock()
                .unwrap_or_else(poisoned)
                .as_mut()
                .is_some_and(|judge| judge(channel_id, answer, hash))
        });
    }

    /// `getNetInfo()` — one hook the net core, the two queues, the net source
    /// and the timing sync read.
    pub fn set_net_info(&mut self, net_info: impl FnMut() -> i32 + Send + 'static) {
        *self.net_info.lock().unwrap_or_else(poisoned) = Box::new(net_info);
    }

    /// `time(NULL)`.
    pub fn set_clock(&mut self, clock: impl FnMut() -> u64 + Send + 'static) {
        self.clock = Some(Box::new(clock));
    }

    fn now_secs(&mut self) -> u64 {
        match self.clock.as_mut() {
            Some(clock) => clock(),
            None => unix_secs(),
        }
    }

    fn net(&self) -> i32 {
        self.net_info.lock().unwrap_or_else(poisoned)()
    }

    //===------------------------------------------------------------------===//
    // the pieces
    //===------------------------------------------------------------------===//

    /// `shortlink_task_manager_`.
    pub fn shortlink(&mut self) -> &mut ShortLinkTaskManager {
        &mut self.shortlink
    }

    /// `longlink_task_manager_`.
    pub fn longlink(&mut self) -> &mut LongLinkTaskManager {
        &mut self.longlink
    }

    /// `zombie_task_manager_` — behind a lock because the queues save to it
    /// from their own callback.
    pub fn zombie(&self) -> MutexGuard<'_, ZombieTaskManager> {
        self.zombie.lock().unwrap_or_else(poisoned)
    }

    /// `net_source_`.
    pub fn net_source(&mut self) -> &mut NetSource {
        &mut self.net_source
    }

    /// `netcheck_logic_`.
    pub fn netcheck(&mut self) -> &mut NetCheckLogic {
        &mut self.netcheck
    }

    /// `dynamic_timeout_`.
    pub fn dynamic_timeout(&mut self) -> &mut DynamicTimeout {
        &mut self.dynamic_timeout
    }

    /// `timing_sync_`.
    pub fn timing_sync(&mut self) -> &mut TimingSync {
        &mut self.timing_sync
    }

    /// `anti_avalanche_`.
    pub fn anti_avalanche(&self) -> MutexGuard<'_, AntiAvalanche> {
        self.anti_avalanche.lock().unwrap_or_else(poisoned)
    }

    /// `LongLinkChannelFactory::Create` / `ShortLinkChannelFactory::Create`.
    pub fn factory(&mut self) -> &mut ChannelFactory {
        &mut self.factory
    }

    /// `__OnSignalActive(isactive)`.
    pub fn set_active(&mut self, is_active: bool) {
        self.anti_avalanche().on_signal_active(is_active);
    }

    /// `SetPackerEncoderVersion` / `SetPackerEncoderName` — carried for the
    /// app to read back; nothing in the port hands them to a channel.
    pub fn set_packer_encoder(&mut self, version: i32, name: impl Into<String>) {
        self.packer_encoder_version = version;
        self.packer_encoder_name = name.into();
    }

    /// `GetPackerEncoderVersion()`.
    pub fn packer_encoder_version(&self) -> i32 {
        self.packer_encoder_version
    }

    /// `GetPackerEncoderName()`.
    pub fn packer_encoder_name(&self) -> &str {
        &self.packer_encoder_name
    }

    //===------------------------------------------------------------------===//
    // the tasks
    //===------------------------------------------------------------------===//

    /// `StartTask(_task)` — which queue the task goes out on, and what happens
    /// when it cannot go out at all. `true` is a task a queue took.
    pub fn start_task(&mut self, task: Task) -> bool {
        self.start_task_at(gettickcount(), task)
    }

    /// The same, with the reading handed in.
    pub fn start_task_at(&mut self, now: u64, mut task: Task) -> bool {
        if self.released {
            return false;
        }

        let mut prepare = PrepareProfile::new_at(now);
        if !valid_and_init_default(&mut task) {
            self.end_task_at(
                &task,
                ErrCmdType::Local,
                LOCAL_TASK_PARAM,
                &ConnectProfile::new(),
            );
            return false;
        }

        prepare.begin_process_hosts_time = now;
        if let Some(process) = self.task_process.as_mut() {
            process(&mut task);
        }
        prepare.end_process_hosts_time = now;

        if task.channel_select == 0 {
            self.end_task_at(
                &task,
                ErrCmdType::Local,
                LOCAL_CHANNEL_SELECT,
                &ConnectProfile::new(),
            );
            return false;
        }

        if task.channel_name.is_empty() {
            task.channel_name = self.default_link.clone().unwrap_or_default();
        }

        if task.network_status_sensitive
            && self.net() == NO_NET
            && self.long_link_is_down(&task.channel_name)
        {
            self.end_task_at(
                &task,
                ErrCmdType::Local,
                LOCAL_NO_NET,
                &ConnectProfile::new(),
            );
            return false;
        }

        let channel = self.choose_channel(&task);
        let start_ok = match channel {
            Task::CHANNEL_MINOR_LONG | Task::CHANNEL_LONG => {
                self.longlink.start_task_at(now, task.clone(), channel)
            }
            Task::CHANNEL_SHORT => {
                let mut task = task.clone();
                // `task.shortlink_fallback_hostlist = task.shortlink_host_list;`
                // is the `kChannelShort` arm of the C++'s switch and nothing
                // else: the `!need_use_longlink_` path hands `StartTask` the
                // task as it came in, so what a retry of it goes out on is
                // whatever the app put there — for a task that put nothing,
                // nowhere at all
                if self.use_long_link {
                    task.shortlink_fallback_hostlist = task.shortlink_host_list.clone();
                }
                self.shortlink.start_task_at(now, task, prepare)
            }
            _ => false,
        };

        if !start_ok {
            self.end_task_at(
                &task,
                ErrCmdType::Local,
                LOCAL_START_TASK_FAIL,
                &ConnectProfile::new(),
            );
            return false;
        }
        if self.use_long_link {
            self.zombie().on_net_core_start_task_at(now);
        }
        true
    }

    /// `StopTask(_taskid)` — long link, then zombies, then short link.
    pub fn stop_task(&mut self, taskid: u32) -> bool {
        if self.longlink.stop_task(taskid) {
            return true;
        }
        let stopped = self
            .zombie
            .lock()
            .unwrap_or_else(poisoned)
            .stop_task(taskid);
        stopped || self.shortlink.stop_task(taskid)
    }

    /// `HasTask(_taskid)`.
    pub fn has_task(&self, taskid: u32) -> bool {
        let saved = self.zombie.lock().unwrap_or_else(poisoned).has_task(taskid);
        saved || self.longlink.has_task(taskid) || self.shortlink.has_task(taskid)
    }

    /// `ClearTasks()`.
    pub fn clear_tasks(&mut self) {
        self.longlink.clear_tasks();
        self.zombie().clear_tasks();
        self.shortlink.clear_tasks();
    }

    /// `RedoTasks()` — every task that was out is cancelled and tried again.
    pub fn redo_tasks(&mut self) {
        self.redo_tasks_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn redo_tasks_at(&mut self, now: u64) {
        self.net_source.clear_cache();
        if self.use_long_link {
            self.longlink.redo_tasks_at(now);
            self.zombie().redo_tasks_at(now);
        }
        self.shortlink.redo_tasks_at(now);
    }

    /// `TouchTasks()` — the queues' own timeouts, without a network change.
    pub fn touch_tasks(&mut self) {
        self.touch_tasks_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn touch_tasks_at(&mut self, now: u64) {
        self.longlink.touch_tasks_at(now);
        self.shortlink.touch_tasks_at(now);
    }

    /// `OnNetworkChange()` — the hosts the net source resolved are thrown away,
    /// the dynamic timeout starts over, and every task is looked at again.
    pub fn on_network_change(&mut self) {
        self.on_network_change_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn on_network_change_at(&mut self, now: u64) {
        self.net_source.clear_cache();
        self.dynamic_timeout.reset();
        // the two queues keep a timeout of their own — the C++ hands them the
        // *same* one the net core has — so the network they learned is theirs
        // to forget too
        self.shortlink.dynamic_timeout().reset();
        self.longlink.dynamic_timeout().reset();
        if self.use_long_link {
            self.timing_sync.on_network_change_at(now);
            self.longlink.on_network_change_at(now);
            self.zombie().redo_tasks_at(now);
        }
        self.shortlink.redo_tasks_at(now);
        self.shortlink_try_flag = false;
        self.shortlink_error_count = 0;
    }

    /// `RetryTasks(...)` — a session timeout, or an answer that could not be
    /// read: every task of that user is looked at again, on both queues.
    pub fn retry_tasks_at(
        &mut self,
        now: u64,
        err_type: ErrCmdType,
        err_code: i32,
        handle: TaskFailHandleType,
        src_taskid: u32,
        user_id: &str,
    ) {
        self.shortlink
            .retry_tasks_at(now, err_type, err_code, handle, src_taskid);
        if self.use_long_link {
            self.longlink
                .retry_tasks_at(now, err_type, err_code, handle, src_taskid, user_id);
        }
    }

    /// `GetConnectProfile(_taskid, _channel_select)` — the connect a task ran
    /// on, which is what the app's report reads.
    pub fn connect_profile(&mut self, taskid: u32, channel_select: i32) -> ConnectProfile {
        if channel_select == Task::CHANNEL_SHORT {
            return self
                .shortlink
                .connect_profile(taskid)
                .cloned()
                .unwrap_or_default();
        }
        if self.use_long_link
            && (channel_select == Task::CHANNEL_LONG
                || channel_select == Task::CHANNEL_MINOR_LONG
                || channel_select == Task::CHANNEL_BOTH)
        {
            return self.longlink.connect_profile(taskid);
        }
        ConnectProfile::new()
    }

    /// When the net core next has something to do: the earliest of the two
    /// queues, the zombie check and the timing sync's alarm.
    pub fn due_time(&mut self) -> Option<u64> {
        let zombies = self.zombie.lock().unwrap_or_else(poisoned).due_time();
        let mut due = min_due(self.shortlink.due_time(), self.longlink.due_time());
        due = min_due(due, zombies);
        min_due(due, self.timing_sync.due_time())
    }

    //===------------------------------------------------------------------===//
    // the follow-ups the C++ posts
    //===------------------------------------------------------------------===//

    /// Whether a queue has asked the net core for something it could not do
    /// while it was running.
    pub fn has_pending(&self) -> bool {
        !self.pending.lock().unwrap_or_else(poisoned).is_empty()
    }

    /// How many.
    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap_or_else(poisoned).len()
    }

    /// What the C++'s message queue thread does: one follow-up at a time, in
    /// the order they were posted. A zombie that is started again can fail at
    /// once, which is another follow-up, so this is a loop and not a `for`.
    pub fn run_pending(&mut self) {
        self.run_pending_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn run_pending_at(&mut self, now: u64) {
        loop {
            let next = self.pending.lock().unwrap_or_else(poisoned).pop_front();
            match next {
                None => return,
                Some(FollowUp::Start(task)) => {
                    self.start_task_at(now, *task);
                }
                Some(FollowUp::Retry {
                    err_type,
                    err_code,
                    handle,
                    src_taskid,
                    user_id,
                }) => {
                    self.retry_tasks_at(now, err_type, err_code, handle, src_taskid, &user_id);
                }
                Some(FollowUp::LongLinkError {
                    name,
                    err_type,
                    err_code,
                    ip,
                    port,
                }) => {
                    self.on_longlink_network_error_at(now, &name, err_type, err_code, &ip, port);
                }
                Some(FollowUp::ShortLinkError {
                    err_type,
                    err_code,
                    ip,
                    host,
                    port,
                }) => {
                    self.on_shortlink_network_error_at(now, err_type, err_code, &ip, &host, port);
                }
            }
        }
    }

    //===------------------------------------------------------------------===//
    // what a task that ended is routed to
    //===------------------------------------------------------------------===//

    /// `__CallBack(...)` — a task that ended, from one of the three queues:
    /// the app's own say on it first, then `OnTaskEnd` for a task that is
    /// really over, and the zombie queue for one that is not.
    ///
    /// This is the free function `call_back` with the net core's own pieces:
    /// a queue's `fun_callback_` cannot hand out a `&mut` to the core that
    /// installed it, so both of them go through the shared hooks and the
    /// shared zombie queue.
    #[allow(clippy::too_many_arguments)]
    pub fn call_back_at(
        &mut self,
        now: u64,
        from: CallFrom,
        err_type: ErrCmdType,
        err_code: i32,
        handle: TaskFailHandleType,
        task: &Task,
        cost: u32,
        profile: &ConnectProfile,
    ) -> i32 {
        call_back(
            &Arc::clone(&self.hooks),
            &Arc::clone(&self.zombie),
            self.use_long_link,
            now,
            from,
            err_type,
            err_code,
            handle,
            task,
            cost,
            profile,
        )
    }

    /// `StnManager::OnTaskEnd` — the answer is what the queue remembers the
    /// task's error code as.
    fn end_task_at(
        &mut self,
        task: &Task,
        err_type: ErrCmdType,
        err_code: i32,
        profile: &ConnectProfile,
    ) -> i32 {
        match self
            .hooks
            .lock()
            .unwrap_or_else(poisoned)
            .on_task_end
            .as_mut()
        {
            Some(end) => end(task.taskid, &task.user_id, err_type, err_code, profile),
            None => 0,
        }
    }

    /// `__OnLongLinkNetworkError(...)` — the diagnosis is told, the app is told
    /// when it is the main link, an answer that came back starts the zombies
    /// again, and the pair is reported to the net source.
    fn on_longlink_network_error_at(
        &mut self,
        now: u64,
        name: &str,
        err_type: ErrCmdType,
        err_code: i32,
        ip: &str,
        port: u16,
    ) {
        if !self.use_long_link || self.released {
            return;
        }
        let continuous_fail = self.longlink.tasks_continuous_fail_count();
        self.netcheck
            .update_long_link_info_at(now, continuous_fail, err_type == ErrCmdType::Ok);

        // the C++ asks the link's own `IsMain()`; the port's `is_main` is what
        // the config was made with, and the main link since is the one
        // [`NetCore::mark_main_longlink`] named — a task that is out on the
        // link the app marked is the one whose errors the app hears about
        let is_main = self.default_link.as_deref() == Some(name);
        if is_main {
            if let Some(report) = self.on_longlink_network_err.as_mut() {
                report(err_type, err_code, ip, port);
            }
        }

        if err_type == ErrCmdType::Ok {
            self.zombie().redo_tasks_at(now);
        }

        // `kEctDial`, `kEctHttp`, `kEctServer` and `kEctLocal` are not about
        // the pair the link was on, so the net source is not told about them
        if matches!(
            err_type,
            ErrCmdType::Dial | ErrCmdType::Http | ErrCmdType::Server | ErrCmdType::Local
        ) {
            return;
        }
        let now_secs = self.now_secs();
        self.net_source
            .report_long_ip_at(now, now_secs, err_type == ErrCmdType::Ok, ip, port);
    }

    /// `__OnShortLinkNetworkError(...)` — the same, plus the two counters the
    /// connect status is worked out from.
    fn on_shortlink_network_error_at(
        &mut self,
        now: u64,
        err_type: ErrCmdType,
        err_code: i32,
        ip: &str,
        host: &str,
        port: u16,
    ) {
        if self.released {
            return;
        }
        let continuous_fail = self.shortlink.tasks_continuous_fail_count();
        self.netcheck
            .update_short_link_info_at(now, continuous_fail, err_type == ErrCmdType::Ok);

        if let Some(report) = self.on_shortlink_network_err.as_mut() {
            report(err_type, err_code, ip, host, port);
        }

        self.shortlink_try_flag = true;
        if err_type == ErrCmdType::Ok {
            self.shortlink_error_count = 0;
        } else {
            self.shortlink_error_count = self.shortlink_error_count.saturating_add(1);
        }
        self.conn_status_call_back();

        if self.use_long_link && err_type == ErrCmdType::Ok {
            self.zombie().redo_tasks_at(now);
        }

        if matches!(
            err_type,
            ErrCmdType::Dial | ErrCmdType::NetMsgXp | ErrCmdType::Server | ErrCmdType::Local
        ) {
            return;
        }
        let now_secs = self.now_secs();
        self.net_source.report_short_ip_at(
            now,
            now_secs,
            err_type == ErrCmdType::Ok,
            ip,
            host,
            port,
        );
    }

    /// `__OnLongLinkConnStatusChange(_status, _channel_id)` — what the C++
    /// connects to `LongLink::SignalConnection`: the timing sync is told, a
    /// link that came up starts the zombies again, and the app is told.
    pub fn on_longlink_status_changed(&mut self, status: LongLinkStatus) {
        self.on_longlink_status_changed_at(gettickcount(), status)
    }

    /// The same, with the reading handed in.
    pub fn on_longlink_status_changed_at(&mut self, now: u64, status: LongLinkStatus) {
        if !self.use_long_link {
            return;
        }
        self.timing_sync.on_longlink_status_changed_at(now, status);
        if status == LongLinkStatus::Connected {
            self.zombie().redo_tasks_at(now);
        }
        self.conn_status_call_back();
        if let Some(change) = self.on_longlink_status_change.as_mut() {
            change(status);
        }
    }

    /// `__ConnStatusCallBack()` — the two answers the app is given, out of the
    /// long link's status and how many short-link errors there have been since
    /// the last one.
    fn conn_status_call_back(&mut self) {
        // a core that does not use the long link has nothing to ask: the app's
        // answer is the short link's, and the long link stays "nothing has
        // tried yet"
        if !self.use_long_link {
            let all = match self.shortlink_error_count {
                count if count >= SHORTLINK_ERR_TIME => NetStatus::ServerFailed,
                _ => NetStatus::Connected,
            };
            self.report_status(all, NetStatus::Unknown);
            return;
        }

        let Some(status) = self.default_link_status() else {
            return;
        };
        let (all, longlink) = match status {
            // a link that went down is not one the app is told about at all
            LongLinkStatus::DisConnected => return,
            LongLinkStatus::ConnectFailed => (self.shortlink_all_status(), NetStatus::ServerFailed),
            LongLinkStatus::ConnectIdle | LongLinkStatus::Connecting => {
                let all = if self.shortlink_try_flag {
                    match shortlink_status(self.shortlink_error_count) {
                        NetStatus::Unknown => NetStatus::Connecting,
                        other => other,
                    }
                } else {
                    NetStatus::Connecting
                };
                (all, NetStatus::Connecting)
            }
            LongLinkStatus::Connected => {
                self.shortlink_error_count = 0;
                self.shortlink_try_flag = false;
                (NetStatus::Connected, NetStatus::Connected)
            }
        };
        self.report_status(all, longlink);
    }

    /// How many short-link errors there have been, as the answer the app is
    /// given: `Unavailable` until one has tried.
    fn shortlink_all_status(&self) -> NetStatus {
        if self.shortlink_try_flag {
            shortlink_status(self.shortlink_error_count)
        } else {
            NetStatus::Unknown
        }
    }

    /// `ReportConnectStatus(all, longlink)` — the two answers, remembered so
    /// the C++'s log line can compare them.
    fn report_status(&mut self, all: NetStatus, longlink: NetStatus) {
        if all != self.all_status || longlink != self.longlink_status {
            self.all_status = all;
            self.longlink_status = longlink;
        }
        if let Some(report) = self.report_connect_status.as_mut() {
            report(all, longlink);
        }
    }

    //===------------------------------------------------------------------===//
    // the long links
    //===------------------------------------------------------------------===//

    /// `CreateLongLink(_config)` — the link, made by the factory, and the
    /// metadata around it. [`None`] is a core that does not use the long link,
    /// or a config the factory would not make one for.
    pub fn create_long_link(&mut self, config: LonglinkConfig) -> Option<Arc<Mutex<LongLink>>> {
        if !self.use_long_link {
            return None;
        }
        let name = config.name.clone();
        if !self.longlink.add_long_link(config.clone()) {
            return self.links.get(&name).map(|meta| Arc::clone(meta.channel()));
        }

        let link = self.factory.create_longlink(&config);
        let meta = LongLinkMetaData::new(config.clone(), link);
        self.links.insert(name.clone(), meta);
        if config.is_main() {
            self.default_link = Some(name.clone());
        }
        // the identify check is the app's, like every other question the links
        // ask — and it is asked of the app whenever a link needs it, which is
        // why a link made now is wired the same way the older ones are
        self.wire_link_identify(&name);
        self.links.get(&name).map(|meta| Arc::clone(meta.channel()))
    }

    /// `DestroyLongLink(_name)` — the channel is gone and every task that was
    /// going out on it is failed.
    pub fn destroy_long_link(&mut self, name: &str) -> bool {
        self.destroy_long_link_at(gettickcount(), name)
    }

    /// The same, with the reading handed in.
    pub fn destroy_long_link_at(&mut self, now: u64, name: &str) -> bool {
        if !self.use_long_link || !self.links.contains_key(name) {
            return false;
        }
        self.longlink.remove_long_link_at(now, name);
        self.links.remove(name);
        if self.default_link.as_deref() == Some(name) {
            self.default_link = None;
        }
        true
    }

    /// `MarkMainLonglink_ext(_name)` — `false` when there is no such link, or
    /// when it is already the main one.
    ///
    /// The C++ moves the link's `fun_network_report_` and the timing sync's and
    /// the keeper's signals over to the new link and flips `Config().isMain` on
    /// both; the port's hooks are the app's, installed once for every channel,
    /// so what moves here is the one thing that is asked about: which link's
    /// errors and status are the app's business.
    pub fn mark_main_longlink(&mut self, name: &str) -> bool {
        if !self.links.contains_key(name) || self.default_link.as_deref() == Some(name) {
            return false;
        }
        self.default_link = Some(name.to_string());
        true
    }

    /// `DefaultLongLink()` — the main link, which is the one the app is given
    /// when it asks for "the" long link.
    pub fn default_long_link(&self) -> Option<&Arc<Mutex<LongLink>>> {
        let name = self.default_link.as_deref()?;
        self.links.get(name).map(|meta| meta.channel())
    }

    /// `DefaultLongLink()` — its name.
    pub fn default_link(&self) -> Option<&str> {
        self.default_link.as_deref()
    }

    /// `GetLongLink(_name)` — the link of a channel, if there is one.
    pub fn long_link(&self, name: &str) -> Option<&Arc<Mutex<LongLink>>> {
        self.links.get(name).map(|meta| meta.channel())
    }

    /// `GetLongLink(_name)` — the metadata, which is the link and the three
    /// things that keep it up.
    pub fn long_link_meta(&mut self, name: &str) -> Option<&mut LongLinkMetaData> {
        self.links.get_mut(name)
    }

    /// `LongLinkIsConnected_ext(_name)`.
    pub fn is_long_link_connected(&self, name: &str) -> bool {
        self.long_link(name).is_some_and(|link| {
            link.lock().unwrap_or_else(poisoned).connect_status() == LongLinkStatus::Connected
        })
    }

    /// `MakeSureLongLinkConnect_ext(_name)` — the link is made, or a run is
    /// started for it.
    pub fn make_sure_long_link_connected(&mut self, name: &str) {
        if let Some(link) = self.long_link(name) {
            link.lock().unwrap_or_else(poisoned).make_sure_connected();
        }
    }

    /// `MakeSureLongLinkConnect()` — the main link's.
    pub fn make_sure_default_long_link_connected(&mut self) {
        if let Some(link) = self.default_long_link() {
            link.lock().unwrap_or_else(poisoned).make_sure_connected();
        }
    }

    /// `LongLinkIsConnected()`.
    pub fn is_default_long_link_connected(&self) -> bool {
        match self.default_link.as_deref() {
            Some(name) => self.is_long_link_connected(name),
            None => false,
        }
    }

    /// `KeepSignal()` — the main link's signalling keeper holds a mapping up.
    pub fn keep_signal(&mut self) {
        self.keep_signal_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn keep_signal_at(&mut self, now: u64) {
        if let Some(name) = self.default_link.clone() {
            if let Some(meta) = self.links.get_mut(&name) {
                meta.keeper().keep_at(now);
            }
        }
    }

    /// `StopSignal()`.
    pub fn stop_signal(&mut self) {
        if let Some(name) = self.default_link.clone() {
            if let Some(meta) = self.links.get_mut(&name) {
                meta.keeper().stop();
            }
        }
    }

    /// `DisconnectLongLinkByTaskId(...)`.
    pub fn disconnect_long_link_by_taskid(
        &mut self,
        taskid: u32,
        code: DisconnectInternalCode,
    ) -> bool {
        if !self.use_long_link {
            return false;
        }
        self.longlink.disconnect_by_taskid(taskid, code)
    }

    /// `SetDebugHost(_host)`.
    pub fn set_debug_host(&mut self, host: impl Into<String>) {
        self.shortlink.set_debug_host(host);
    }

    /// `ForbidLonglinkTlsHost(_host)`.
    pub fn forbid_longlink_tls_host(&mut self, hosts: &[String]) {
        self.longlink.add_forbid_tls_host(hosts);
    }

    /// `AddServerBan(_ip)`.
    pub fn add_server_ban(&mut self, ip: &str) {
        self.net_source.add_server_ban(ip);
    }

    /// `InitHistory2BannedList()`.
    pub fn init_history_to_banned_list(&mut self) {
        self.net_source.init_history_to_banned_list();
    }

    /// `SetIpConnectTimeout(...)`.
    pub fn set_ip_connect_timeout(&mut self, v4_timeout: u32, v6_timeout: u32) {
        self.net_source
            .set_ip_connect_timeout(v4_timeout, v6_timeout);
    }

    /// `SetNeedUseLongLink(flag)` — and the wiring that goes with it: the
    /// queues' callbacks are the ones that decide whether a task is saved as a
    /// zombie.
    pub fn set_need_use_long_link(&mut self, use_long_link: bool) {
        self.use_long_link = use_long_link;
        self.wire();
    }

    /// `UseLongLink()`.
    pub fn use_long_link(&self) -> bool {
        self.use_long_link
    }

    /// `ReleaseNet()` — the tasks are dropped and the links are gone.
    pub fn release(&mut self) {
        self.clear_tasks();
        self.links.clear();
        self.default_link = None;
        self.timing_sync.cancel();
        self.released = true;
    }

    /// `IsAlreadyRelease()`.
    pub fn is_released(&self) -> bool {
        self.released
    }

    /// `longlink && LongLink::kConnected != longlink->Channel()->ConnectStatus()`
    /// — a link the core has for the name, that is not up.
    ///
    /// `need_use_longlink_` is what makes the C++ look one up at all
    /// (`longlink` stays `nullptr` without it), and a name no link answers to
    /// is a `nullptr` too: in both cases the C++ asks no question of the
    /// network and lets the task out.
    fn long_link_is_down(&self, name: &str) -> bool {
        self.use_long_link
            && self.long_link(name).is_some_and(|link| {
                link.lock().unwrap_or_else(poisoned).connect_status() != LongLinkStatus::Connected
            })
    }

    /// `__ChooseChannel(...)` — long link, short link, or the channel the task
    /// asked for: a task that may use either is put on the long link while it
    /// is up, and a `kChannelFastStrategy` one only while nothing else of that
    /// channel is out.
    fn choose_channel(&self, task: &Task) -> i32 {
        let mut longlink_ok = self.is_long_link_connected(&task.channel_name);
        // `kFastSendUseLonglinkTaskCntLimit` is `0` in the C++, so "at most"
        // is "none": a fast task is only put on the long link while nothing
        // else of that channel is out
        if longlink_ok
            && task.channel_strategy == Task::CHANNEL_FAST_STRATEGY
            && self.longlink.task_count(&task.channel_name) > FAST_SEND_LONGLINK_TASK_CNT_LIMIT
        {
            longlink_ok = false;
        }

        if !self.use_long_link {
            return Task::CHANNEL_SHORT;
        }

        match task.channel_select {
            Task::CHANNEL_ALL => {
                if longlink_ok {
                    Task::CHANNEL_LONG
                } else {
                    Task::CHANNEL_SHORT
                }
            }
            Task::CHANNEL_NORMAL => Task::CHANNEL_SHORT,
            Task::CHANNEL_BOTH => {
                if longlink_ok {
                    Task::CHANNEL_LONG
                } else {
                    Task::CHANNEL_SHORT
                }
            }
            other => other,
        }
    }

    fn default_link_status(&self) -> Option<LongLinkStatus> {
        let name = self.default_link.as_deref()?;
        let link = self.links.get(name)?;
        Some(
            link.channel()
                .lock()
                .unwrap_or_else(poisoned)
                .connect_status(),
        )
    }
}

impl Default for NetCore {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for NetCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetCore")
            .field("use_long_link", &self.use_long_link)
            .field("released", &self.released)
            .field("default_link", &self.default_link)
            .field("links", &self.links.keys().collect::<Vec<_>>())
            .field("shortlink_tasks", &self.shortlink.len())
            .field("longlink_tasks", &self.longlink.len())
            .field(
                "zombies",
                &self.zombie.lock().unwrap_or_else(poisoned).len(),
            )
            .field("pending", &self.pending_count())
            .field("shortlink_error_count", &self.shortlink_error_count)
            .field("shortlink_try_flag", &self.shortlink_try_flag)
            .field("all_status", &self.all_status)
            .field("longlink_status", &self.longlink_status)
            .finish_non_exhaustive()
    }
}

/// `__ValidAndInitDefault(...)` — a task the net core will not start, and the
/// three things it fills in for one it will: a long-link task without a cmdid
/// is not a long-link task, a short-link task without a cgi is not a short-link
/// one, and a task that asked for a negative retry count is given
/// [`DEF_TASK_RETRY_COUNT`].
fn valid_and_init_default(task: &mut Task) -> bool {
    if task.server_process_cost > 2 * 60 * 1000 {
        return false;
    }
    if task.retry_count > 30 {
        return false;
    }
    if task.total_timeout > 10 * 60 * 1000 {
        return false;
    }
    if task.channel_select & Task::CHANNEL_LONG != 0 && task.cmdid == 0 {
        task.channel_select &= !Task::CHANNEL_LONG;
    }
    if task.channel_select & Task::CHANNEL_SHORT != 0 && task.cgi.is_empty() {
        task.channel_select &= !Task::CHANNEL_SHORT;
    }
    if task.retry_count < 0 {
        task.retry_count = DEF_TASK_RETRY_COUNT;
    }
    true
}

/// How many short-link errors in a row the app is told about.
fn shortlink_status(error_count: i32) -> NetStatus {
    if error_count >= SHORTLINK_ERR_TIME {
        NetStatus::ServerFailed
    } else if error_count == 0 {
        NetStatus::Connected
    } else {
        NetStatus::Unknown
    }
}

fn min_due(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(left), None) => Some(left),
        (None, right) => right,
    }
}

fn push(pending: &Arc<Mutex<VecDeque<FollowUp>>>, follow_up: FollowUp) {
    pending.lock().unwrap_or_else(poisoned).push_back(follow_up);
}

fn anti_avalanche_check(
    avalanche: &Arc<Mutex<AntiAvalanche>>,
    net_info: &Arc<Mutex<Box<NetInfo>>>,
    task: &Task,
    body: &[u8],
) -> bool {
    let mobile = net_info.lock().unwrap_or_else(poisoned)() == NET_TYPE_MOBILE;
    avalanche
        .lock()
        .unwrap_or_else(poisoned)
        .check(task, body, mobile)
        .is_ok()
}

/// `__CallBack(...)` as a free function, which is what lets the queues' own
/// `fun_callback_` and the net core's methods do the same thing: the hooks and
/// the zombie queue are shared, and nothing else is touched.
#[allow(clippy::too_many_arguments)]
fn call_back(
    hooks: &Arc<Mutex<Hooks>>,
    zombie: &Arc<Mutex<ZombieTaskManager>>,
    use_long_link: bool,
    now: u64,
    from: CallFrom,
    err_type: ErrCmdType,
    err_code: i32,
    handle: TaskFailHandleType,
    task: &Task,
    cost: u32,
    profile: &ConnectProfile,
) -> i32 {
    {
        let mut hooks = hooks.lock().unwrap_or_else(poisoned);
        if let Some(callback) = hooks.task_callback.as_mut() {
            if callback(from, err_type, err_code, handle, task) == 0 {
                return 0;
            }
        }
    }

    // `kEctLocal` / `kEctLocalReset` — a task that is over because the net core
    // itself is gone
    if err_type == ErrCmdType::Local && err_code == LOCAL_RESET {
        return end_task(hooks, task, err_type, err_code, &ConnectProfile::new());
    }
    // a task that answered, or one the app said not to try again: the app is
    // given the connect it ran on
    if err_type == ErrCmdType::Ok || handle == TaskFailHandleType::TaskEnd {
        return end_task(hooks, task, err_type, err_code, profile);
    }
    // a zombie that ends is over: it is not saved again
    if from == CallFrom::Zombie {
        return end_task(hooks, task, err_type, err_code, &ConnectProfile::new());
    }
    if use_long_link
        && zombie
            .lock()
            .unwrap_or_else(poisoned)
            .save_task_at(now, task, cost)
    {
        return 0;
    }
    end_task(hooks, task, err_type, err_code, &ConnectProfile::new())
}

fn end_task(
    hooks: &Arc<Mutex<Hooks>>,
    task: &Task,
    err_type: ErrCmdType,
    err_code: i32,
    profile: &ConnectProfile,
) -> i32 {
    match hooks.lock().unwrap_or_else(poisoned).on_task_end.as_mut() {
        Some(end) => end(task.taskid, &task.user_id, err_type, err_code, profile),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::longlink_task_manager::Response as LongAnswer;
    use crate::shortlink_task_manager::Response as ShortAnswer;
    use crate::task_profile::LOCAL_LONG_LINK_RELEASED;
    use crate::{DynamicTimeoutStatus, RespHandle, RunId, TaskFailHandleType, NET_TYPE_WIFI};

    const NOW: u64 = 100 * 1000;
    const MAIN: &str = DEFAULT_LONGLINK_NAME;
    const SHORT_HOST: &str = "short.weixin.qq.com";

    /// `(taskid, user_id, err_type, err_code, profile.ip)` — the user a task
    /// was started for goes out with the end, like the C++ hands
    /// `task.user_id` to `OnTaskEnd`.
    type Ended = Vec<(u32, String, ErrCmdType, i32, String)>;
    /// `(channel, taskid, cgi)`.
    type Sent = Vec<(String, u32, String)>;
    /// `(all, longlink)`.
    type Status = Vec<(NetStatus, NetStatus)>;
    /// `(err_type, err_code, ip, port)`.
    type Pairs = Vec<(ErrCmdType, i32, String, u16)>;
    /// `(cmdid, body)`.
    type Pushed = Vec<(u32, String)>;
    /// `(from, err_type, err_code)`.
    type Callbacks = Vec<(CallFrom, ErrCmdType, i32)>;

    /// Everything the app and the two queues were told, as one value a test
    /// reads: the net core's job is *who* is told *what*, so a test is a list
    /// of the answers it handed out.
    #[derive(Clone, Default)]
    struct Rec {
        ended: Arc<Mutex<Ended>>,
        sent: Arc<Mutex<Sent>>,
        status: Arc<Mutex<Status>>,
        long_err: Arc<Mutex<Pairs>>,
        short_err: Arc<Mutex<Pairs>>,
        pushed: Arc<Mutex<Pushed>>,
        preprocessed: Arc<Mutex<Vec<u32>>>,
        /// The `retry_count` of every task, as the task process saw it.
        processed: Arc<Mutex<Vec<i32>>>,
        callbacks: Arc<Mutex<Callbacks>>,
    }

    fn drain<T>(cell: &Arc<Mutex<Vec<T>>>) -> Vec<T> {
        std::mem::take(&mut *cell.lock().unwrap_or_else(poisoned))
    }

    impl Rec {
        fn ended(&self) -> Ended {
            drain(&self.ended)
        }

        fn sent(&self) -> Sent {
            drain(&self.sent)
        }

        fn status(&self) -> Status {
            drain(&self.status)
        }

        fn long_err(&self) -> Pairs {
            drain(&self.long_err)
        }

        fn short_err(&self) -> Pairs {
            drain(&self.short_err)
        }

        fn pushed(&self) -> Pushed {
            drain(&self.pushed)
        }
    }

    /// A core with the default long link, both queues wired to a recorder, and
    /// a wifi the net source can resolve hosts on.
    fn wired() -> (NetCore, Rec) {
        let mut core = NetCore::new_at(NOW);
        let rec = Rec::default();
        core.set_net_info(|| NET_TYPE_WIFI);

        let sent = rec.sent.clone();
        core.shortlink().set_start_run(move |task, _request| {
            sent.lock().unwrap_or_else(poisoned).push((
                "short".to_string(),
                task.taskid,
                task.cgi.clone(),
            ));
            Some(RunId(u64::from(task.taskid)))
        });
        let sent = rec.sent.clone();
        core.longlink().set_send(move |name, task, _body| {
            sent.lock().unwrap_or_else(poisoned).push((
                name.to_string(),
                task.taskid,
                task.cgi.clone(),
            ));
            Some(RunId(u64::from(task.taskid)))
        });
        core.shortlink()
            .set_req2buf(|task| Ok(task.cgi.as_bytes().to_vec()));
        core.longlink()
            .set_req2buf(|task| Ok(task.cgi.as_bytes().to_vec()));
        core.shortlink()
            .set_buf2resp(|_task, _body| (0, TaskFailHandleType::Normal));
        core.longlink()
            .set_buf2resp(|_task, _body| (0, TaskFailHandleType::Normal));
        core.longlink().set_make_sure_connected(|_name| true);
        core.longlink().set_channel_profile(|name| {
            let mut profile = ConnectProfile::new();
            profile.host = name.to_string();
            profile.ip = "10.0.0.1".to_string();
            profile.port = 8080;
            profile
        });

        let ended = rec.ended.clone();
        core.set_on_task_end(move |taskid, user_id, err_type, err_code, profile| {
            ended.lock().unwrap_or_else(poisoned).push((
                taskid,
                user_id.to_string(),
                err_type,
                err_code,
                profile.ip.clone(),
            ));
            err_code
        });
        let status = rec.status.clone();
        core.set_report_connect_status(move |all, longlink| {
            status.lock().unwrap_or_else(poisoned).push((all, longlink));
        });
        let long_err = rec.long_err.clone();
        core.set_on_longlink_network_err(move |err_type, err_code, ip, port| {
            long_err.lock().unwrap_or_else(poisoned).push((
                err_type,
                err_code,
                ip.to_string(),
                port,
            ));
        });
        let short_err = rec.short_err.clone();
        core.set_on_shortlink_network_err(move |err_type, err_code, ip, _host, port| {
            short_err.lock().unwrap_or_else(poisoned).push((
                err_type,
                err_code,
                ip.to_string(),
                port,
            ));
        });
        let pushed = rec.pushed.clone();
        core.set_on_push(move |_name, cmdid, _taskid, body| {
            pushed
                .lock()
                .unwrap_or_else(poisoned)
                .push((cmdid, String::from_utf8_lossy(body).to_string()));
        });
        let preprocessed = rec.preprocessed.clone();
        core.set_push_preprocess(move |cmdid, _body| {
            preprocessed.lock().unwrap_or_else(poisoned).push(cmdid);
        });
        let processed = rec.processed.clone();
        core.set_task_process(move |task| {
            processed
                .lock()
                .unwrap_or_else(poisoned)
                .push(task.retry_count);
        });
        (core, rec)
    }

    fn task(taskid: u32) -> Task {
        let mut task = Task::new(taskid, 12);
        task.cgi = format!("/cgi-bin/{taskid}");
        task.channel_select = Task::CHANNEL_ALL;
        task.shortlink_host_list = vec![SHORT_HOST.to_string()];
        // big enough that the time the task already spent is not the whole of
        // it, which is what lets it be kept as a zombie
        task.total_timeout = 10 * 60 * 1000;
        task.retry_count = 1;
        task.user_id = "user".to_string();
        task
    }

    /// What the host would have done: the link is up, or is not.
    fn up(core: &NetCore, status: LongLinkStatus) {
        let link = Arc::clone(core.long_link(MAIN).expect("the default link"));
        link.lock().unwrap_or_else(poisoned).set_status(status);
    }

    fn short_answer(profile: ConnectProfile) -> ShortAnswer {
        ShortAnswer {
            err_type: ErrCmdType::Ok,
            status: 200,
            body: b"ok".to_vec(),
            cancel_retry: false,
            profile,
        }
    }

    fn long_answer(taskid: u32, profile: ConnectProfile) -> LongAnswer {
        LongAnswer {
            name: MAIN.to_string(),
            err_type: ErrCmdType::Ok,
            err_code: 0,
            cmdid: 12,
            taskid,
            body: Vec::new(),
            profile,
        }
    }

    fn profile_of(ip: &str) -> ConnectProfile {
        let mut profile = ConnectProfile::new();
        profile.ip = ip.to_string();
        profile
    }

    #[test]
    fn a_task_that_may_use_everything_goes_on_the_long_link_while_it_is_up() {
        let (mut core, rec) = wired();
        up(&core, LongLinkStatus::Connected);

        assert!(core.start_task_at(NOW, task(7)));
        assert!(core.longlink().has_task(7));
        assert!(!core.shortlink().has_task(7));
        assert_eq!(
            rec.sent(),
            vec![(MAIN.to_string(), 7, "/cgi-bin/7".to_string())]
        );
    }

    #[test]
    fn a_task_that_may_use_everything_goes_on_the_short_link_while_the_long_link_is_down() {
        let (mut core, rec) = wired();

        assert!(core.start_task_at(NOW, task(7)));
        assert!(core.shortlink().has_task(7));
        assert_eq!(
            rec.sent(),
            vec![("short".to_string(), 7, "/cgi-bin/7".to_string())]
        );
    }

    #[test]
    fn an_empty_channel_name_is_the_default_link() {
        let (mut core, rec) = wired();
        up(&core, LongLinkStatus::Connected);

        let mut task = task(7);
        task.channel_name = String::new();
        assert!(core.start_task_at(NOW, task));

        assert_eq!(core.longlink().task_count(MAIN), 1);
        assert_eq!(
            rec.sent(),
            vec![(MAIN.to_string(), 7, "/cgi-bin/7".to_string())]
        );
    }

    #[test]
    fn a_task_that_may_use_no_channel_is_failed_at_once() {
        let (mut core, rec) = wired();
        let mut task = task(7);
        task.channel_select = 0;

        assert!(!core.start_task_at(NOW, task));
        assert_eq!(
            rec.ended(),
            vec![(
                7,
                "user".to_string(),
                ErrCmdType::Local,
                LOCAL_CHANNEL_SELECT,
                String::new()
            )]
        );
        assert!(!core.has_task(7));
    }

    #[test]
    fn a_task_the_server_may_not_take_that_long_is_not_started() {
        let (mut core, rec) = wired();
        let mut task = task(7);
        task.server_process_cost = 2 * 60 * 1000 + 1;

        assert!(!core.start_task_at(NOW, task));
        assert_eq!(
            rec.ended(),
            vec![(
                7,
                "user".to_string(),
                ErrCmdType::Local,
                LOCAL_TASK_PARAM,
                String::new()
            )]
        );
    }

    #[test]
    fn a_task_that_asked_for_more_than_thirty_tries_is_not_started() {
        let (mut core, rec) = wired();
        let mut task = task(7);
        task.retry_count = 31;

        assert!(!core.start_task_at(NOW, task));
        assert_eq!(
            rec.ended(),
            vec![(
                7,
                "user".to_string(),
                ErrCmdType::Local,
                LOCAL_TASK_PARAM,
                String::new()
            )]
        );
    }

    #[test]
    fn a_task_with_a_deadline_over_ten_minutes_is_not_started() {
        let (mut core, rec) = wired();
        let mut task = task(7);
        task.total_timeout = 10 * 60 * 1000 + 1;

        assert!(!core.start_task_at(NOW, task));
        assert_eq!(
            rec.ended(),
            vec![(
                7,
                "user".to_string(),
                ErrCmdType::Local,
                LOCAL_TASK_PARAM,
                String::new()
            )]
        );
    }

    #[test]
    fn a_long_link_task_without_a_cmdid_loses_the_long_link() {
        let (mut core, rec) = wired();
        up(&core, LongLinkStatus::Connected);
        let mut task = task(7);
        task.cmdid = 0;
        task.channel_select = Task::CHANNEL_BOTH;

        assert!(core.start_task_at(NOW, task));
        assert!(!core.longlink().has_task(7));
        assert!(core.shortlink().has_task(7));
        assert_eq!(rec.sent().len(), 1);
    }

    #[test]
    fn a_short_link_task_without_a_cgi_loses_the_short_link() {
        let (mut core, _rec) = wired();
        up(&core, LongLinkStatus::Connected);
        let mut task = task(7);
        task.cgi = String::new();
        task.channel_select = Task::CHANNEL_BOTH;

        assert!(core.start_task_at(NOW, task));
        assert!(core.longlink().has_task(7));
        assert!(!core.shortlink().has_task(7));
    }

    #[test]
    fn a_task_that_asked_for_a_negative_retry_count_is_given_the_default_one() {
        let (mut core, rec) = wired();
        let mut task = task(7);
        task.retry_count = -1;

        assert!(core.start_task_at(NOW, task));
        assert_eq!(drain(&rec.processed), vec![DEF_TASK_RETRY_COUNT]);
    }

    #[test]
    fn a_task_the_network_cannot_take_is_failed_when_the_task_is_sensitive() {
        let (mut core, rec) = wired();
        core.set_net_info(|| NO_NET);
        let mut sensitive = task(7);
        sensitive.network_status_sensitive = true;

        assert!(!core.start_task_at(NOW, sensitive));
        assert_eq!(
            rec.ended(),
            vec![(
                7,
                "user".to_string(),
                ErrCmdType::Local,
                LOCAL_NO_NET,
                String::new()
            )]
        );

        // ... and a link that is up is a network the task can be given to
        up(&core, LongLinkStatus::Connected);
        let mut later = task(8);
        later.network_status_sensitive = true;
        assert!(core.start_task_at(NOW, later));
        assert!(rec.ended().is_empty());
    }

    /// `longlink` is `nullptr` in the C++ when nothing was looked up — no long
    /// link at all, or a channel no link answers to — and a `nullptr` asks no
    /// question of the network: the task goes out.
    #[test]
    fn a_sensitive_task_with_no_link_to_ask_about_is_not_failed_for_the_network() {
        // the long link is off: `need_use_longlink_` is false, so the C++
        // never looks one up
        let (mut core, rec) = wired();
        core.set_need_use_long_link(false);
        core.set_net_info(|| NO_NET);
        let mut off = task(7);
        off.network_status_sensitive = true;

        assert!(
            core.start_task_at(NOW, off),
            "with no long link there is nothing to ask"
        );
        assert!(core.shortlink().has_task(7));
        assert!(rec.ended().is_empty());

        // ... and the long link is on, but the task names a channel the core
        // has no link for
        let (mut core, rec) = wired();
        core.set_net_info(|| NO_NET);
        let mut named = task(7);
        named.network_status_sensitive = true;
        named.channel_name = "no-such-link".to_string();

        assert!(core.start_task_at(NOW, named));
        assert!(core.shortlink().has_task(7));
        assert!(rec.ended().is_empty());
    }

    /// `case Task::kChannelShort: task.shortlink_fallback_hostlist =
    /// task.shortlink_host_list;` — the arm of the C++'s switch, which a core
    /// that does not use the long link never reaches: its `StartTask` gets the
    /// task as it came in, and a retry of it goes out on the hosts the app
    /// asked for a fallback to.
    #[test]
    fn a_core_with_no_long_link_leaves_the_fallback_hosts_alone() {
        let (mut core, _rec) = wired();
        core.set_need_use_long_link(false);
        let mut short = task(7);
        short.channel_select = Task::CHANNEL_SHORT;

        assert!(core.start_task_at(NOW, short));
        assert!(
            core.shortlink().tasks()[0]
                .task
                .shortlink_fallback_hostlist
                .is_empty(),
            "the C++ does not fill the fallback hosts in on this path"
        );

        // ... and with a long link, the short arm is the one that runs
        let (mut core, _rec) = wired();
        let mut short = task(7);
        short.channel_select = Task::CHANNEL_SHORT;

        assert!(core.start_task_at(NOW, short));
        assert_eq!(
            core.shortlink().tasks()[0].task.shortlink_fallback_hostlist,
            vec![SHORT_HOST.to_string()]
        );
    }

    #[test]
    fn a_task_that_is_only_sent_is_not_one_a_short_link_can_start() {
        let (mut core, rec) = wired();
        let mut task = task(7);
        task.send_only = true;

        assert!(!core.start_task_at(NOW, task));
        assert_eq!(
            rec.ended(),
            vec![(
                7,
                "user".to_string(),
                ErrCmdType::Local,
                LOCAL_START_TASK_FAIL,
                String::new()
            )]
        );
    }

    #[test]
    fn a_fast_task_gives_the_long_link_up_while_one_of_the_channel_is_out() {
        let (mut core, _rec) = wired();
        up(&core, LongLinkStatus::Connected);
        let mut first = task(7);
        first.channel_strategy = Task::CHANNEL_FAST_STRATEGY;
        let mut second = task(8);
        second.channel_strategy = Task::CHANNEL_FAST_STRATEGY;

        assert!(core.start_task_at(NOW, first));
        assert!(core.start_task_at(NOW, second));

        assert_eq!(core.longlink().task_count(MAIN), 1);
        assert!(core.shortlink().has_task(8));
    }

    #[test]
    fn a_task_that_asked_for_the_short_link_stays_on_it_while_the_long_link_is_up() {
        let (mut core, rec) = wired();
        up(&core, LongLinkStatus::Connected);
        for (taskid, channel_select) in [(7, Task::CHANNEL_SHORT), (8, Task::CHANNEL_NORMAL)] {
            let mut task = task(taskid);
            task.channel_select = channel_select;
            assert!(core.start_task_at(NOW, task));
        }

        assert_eq!(core.longlink().len(), 0);
        assert_eq!(core.shortlink().len(), 2);
        assert_eq!(rec.sent().len(), 2);
    }

    #[test]
    fn an_answer_that_came_back_ends_the_task_with_the_connect_it_ran_on() {
        let (mut core, rec) = wired();
        assert!(core.start_task_at(NOW, task(7)));

        let handle = core.shortlink().on_response_at(
            NOW + 100,
            RunId(7),
            short_answer(profile_of("1.2.3.4")),
        );
        assert_eq!(handle, Some(RespHandle::Ended));

        assert_eq!(
            rec.ended(),
            vec![(
                7,
                "user".to_string(),
                ErrCmdType::Ok,
                0,
                "1.2.3.4".to_string()
            )]
        );
        assert!(!core.has_task(7));
    }

    #[test]
    fn a_task_the_app_says_nothing_about_is_not_ended() {
        let (mut core, rec) = wired();
        let callbacks = rec.callbacks.clone();
        core.set_task_callback(move |from, err_type, err_code, _handle, _task| {
            callbacks
                .lock()
                .unwrap_or_else(poisoned)
                .push((from, err_type, err_code));
            0
        });

        assert!(core.start_task_at(NOW, task(7)));
        core.shortlink()
            .on_response_at(NOW + 100, RunId(7), short_answer(profile_of("1.2.3.4")));

        assert_eq!(
            drain(&rec.callbacks),
            vec![(CallFrom::Short, ErrCmdType::Ok, 0)]
        );
        assert!(rec.ended().is_empty());
        assert!(!core.has_task(7));
    }

    #[test]
    fn a_long_link_task_that_heard_nothing_is_saved_as_a_zombie() {
        let (mut core, rec) = wired();
        up(&core, LongLinkStatus::Connected);
        core.longlink()
            .set_buf2resp(|_task, _body| (9, TaskFailHandleType::TaskTimeout));

        assert!(core.start_task_at(NOW, task(7)));
        core.longlink()
            .on_response_at(NOW + 100, long_answer(7, profile_of("1.2.3.4")));

        assert_eq!(core.zombie().len(), 1);
        assert!(rec.ended().is_empty());
        assert!(!core.longlink().has_task(7));
        // the whole channel failed, which the host drains into the app's
        // report: the C++ posts it and the message queue thread runs it
        core.run_pending_at(NOW + 100);
        assert_eq!(
            rec.long_err(),
            vec![(
                ErrCmdType::EnDecode,
                TaskFailHandleType::TaskTimeout as i32,
                "1.2.3.4".to_string(),
                0
            )]
        );
    }

    #[test]
    fn a_zombie_is_started_again_when_the_link_comes_back() {
        let (mut core, rec) = wired();
        up(&core, LongLinkStatus::Connected);
        core.longlink()
            .set_buf2resp(|_task, _body| (9, TaskFailHandleType::TaskTimeout));
        assert!(core.start_task_at(NOW, task(7)));
        core.longlink()
            .on_response_at(NOW + 100, long_answer(7, profile_of("1.2.3.4")));
        assert_eq!(core.zombie().len(), 1);

        core.on_longlink_status_changed_at(NOW + 500, LongLinkStatus::Connected);
        assert!(core.has_pending());
        core.run_pending_at(NOW + 500);

        assert_eq!(core.zombie().len(), 0);
        assert!(core.longlink().has_task(7));
        assert_eq!(rec.sent().len(), 2);
    }

    #[test]
    fn a_zombie_that_ends_is_not_saved_again() {
        let (mut core, rec) = wired();
        let task = task(7);

        let code = core.call_back_at(
            NOW,
            CallFrom::Zombie,
            ErrCmdType::EnDecode,
            9,
            TaskFailHandleType::TaskTimeout,
            &task,
            100,
            &profile_of("1.2.3.4"),
        );
        assert_eq!(code, 9);
        assert_eq!(core.zombie().len(), 0);
        assert_eq!(
            rec.ended(),
            vec![(
                7,
                "user".to_string(),
                ErrCmdType::EnDecode,
                9,
                String::new()
            )]
        );
    }

    #[test]
    fn a_task_that_is_over_because_the_core_is_gone_is_ended_with_nothing() {
        let (mut core, rec) = wired();
        let task = task(7);

        core.call_back_at(
            NOW,
            CallFrom::Short,
            ErrCmdType::Local,
            LOCAL_RESET,
            TaskFailHandleType::Default,
            &task,
            100,
            &profile_of("1.2.3.4"),
        );
        assert_eq!(
            rec.ended(),
            vec![(
                7,
                "user".to_string(),
                ErrCmdType::Local,
                LOCAL_RESET,
                String::new()
            )]
        );
        assert_eq!(core.zombie().len(), 0);
    }

    #[test]
    fn a_core_that_does_not_use_the_long_link_fails_the_task_instead_of_keeping_it() {
        let (mut core, rec) = wired();
        core.shortlink()
            .set_buf2resp(|_task, _body| (9, TaskFailHandleType::TaskTimeout));

        assert!(core.start_task_at(NOW, task(7)));
        core.set_need_use_long_link(false);
        core.shortlink()
            .on_response_at(NOW + 100, RunId(7), short_answer(profile_of("1.2.3.4")));

        assert_eq!(core.zombie().len(), 0);
        assert_eq!(
            rec.ended(),
            vec![(
                7,
                "user".to_string(),
                ErrCmdType::EnDecode,
                9,
                String::new()
            )]
        );
    }

    #[test]
    fn a_session_timeout_is_a_follow_up_the_host_drains() {
        let (mut core, _rec) = wired();
        core.shortlink()
            .set_buf2resp(|_task, _body| (0, TaskFailHandleType::SessionTimeout));

        assert!(core.start_task_at(NOW, task(7)));
        let handle = core.shortlink().on_response_at(
            NOW + 100,
            RunId(7),
            short_answer(profile_of("1.2.3.4")),
        );
        assert_eq!(handle, Some(RespHandle::Deferred));

        assert_eq!(core.pending_count(), 1);
        assert!(core.has_pending());
        core.run_pending_at(NOW + 200);
        assert!(!core.has_pending());
        // the answer was not one the task is over with: it is still in the queue
        assert!(core.has_task(7));
    }

    #[test]
    fn three_short_link_errors_in_a_row_are_a_server_the_app_cannot_reach() {
        let (mut core, rec) = wired();
        up(&core, LongLinkStatus::ConnectFailed);

        for i in 0..SHORTLINK_ERR_TIME {
            core.on_shortlink_network_error_at(
                NOW + u64::try_from(i).unwrap(),
                ErrCmdType::Socket,
                -1,
                "1.2.3.4",
                SHORT_HOST,
                80,
            );
        }

        assert_eq!(
            rec.status(),
            vec![
                (NetStatus::Unknown, NetStatus::ServerFailed),
                (NetStatus::Unknown, NetStatus::ServerFailed),
                (NetStatus::ServerFailed, NetStatus::ServerFailed),
            ]
        );
        assert_eq!(rec.short_err().len(), SHORTLINK_ERR_TIME as usize);
    }

    #[test]
    fn a_short_link_answer_that_came_back_starts_the_zombies_again() {
        let (mut core, rec) = wired();
        up(&core, LongLinkStatus::Connected);
        core.longlink()
            .set_buf2resp(|_task, _body| (9, TaskFailHandleType::TaskTimeout));
        assert!(core.start_task_at(NOW, task(7)));
        core.longlink()
            .on_response_at(NOW + 100, long_answer(7, profile_of("1.2.3.4")));
        assert_eq!(core.zombie().len(), 1);

        core.on_shortlink_network_error_at(NOW + 200, ErrCmdType::Ok, 0, "1.2.3.4", SHORT_HOST, 80);
        assert!(core.has_pending());
        core.run_pending_at(NOW + 200);

        assert_eq!(core.zombie().len(), 0);
        assert!(core.longlink().has_task(7));
        assert_eq!(rec.sent().len(), 2);
    }

    #[test]
    fn a_long_link_error_is_the_apps_only_when_it_is_the_main_link() {
        let (mut core, rec) = wired();
        core.create_long_link(LonglinkConfig::new("second"));

        core.on_longlink_network_error_at(NOW, "second", ErrCmdType::Socket, -1, "5.6.7.8", 443);
        assert!(rec.long_err().is_empty());

        core.on_longlink_network_error_at(NOW, MAIN, ErrCmdType::Socket, -1, "1.2.3.4", 443);
        assert_eq!(
            rec.long_err(),
            vec![(ErrCmdType::Socket, -1, "1.2.3.4".to_string(), 443)]
        );
    }

    #[test]
    fn the_errors_the_app_hears_about_are_the_ones_of_the_link_it_marked_main() {
        let (mut core, rec) = wired();
        core.create_long_link(LonglinkConfig::new("second"));
        assert!(core.mark_main_longlink("second"));

        // the link the app marked is the main one now, so its errors are the
        // app's and the first link's are not
        core.on_longlink_network_error_at(NOW, MAIN, ErrCmdType::Socket, -1, "1.2.3.4", 443);
        assert!(rec.long_err().is_empty());

        core.on_longlink_network_error_at(NOW, "second", ErrCmdType::Socket, -1, "5.6.7.8", 443);
        assert_eq!(
            rec.long_err(),
            vec![(ErrCmdType::Socket, -1, "5.6.7.8".to_string(), 443)]
        );
    }

    #[test]
    fn a_network_change_forgets_the_network_every_queue_learned() {
        let (mut core, _rec) = wired();

        // a network the two queues learned on their own: one that answered
        // slowly enough to be called `Bad`
        core.shortlink().dynamic_timeout().record_at(
            crate::dynamic_timeout::NetworkKind::Mobile,
            1,
            10_000,
            NOW,
        );
        core.longlink().dynamic_timeout().record_at(
            crate::dynamic_timeout::NetworkKind::Mobile,
            1,
            10_000,
            NOW,
        );
        core.dynamic_timeout().record_at(
            crate::dynamic_timeout::NetworkKind::Mobile,
            1,
            10_000,
            NOW,
        );

        core.on_network_change_at(NOW + 1);

        // the C++ resets the one timeout all three share, which is what a
        // change of network means: what was learned about the old one is gone
        assert_eq!(
            core.shortlink().dynamic_timeout().status(),
            DynamicTimeoutStatus::Evaluating
        );
        assert_eq!(
            core.longlink().dynamic_timeout().status(),
            DynamicTimeoutStatus::Evaluating
        );
        assert_eq!(
            core.dynamic_timeout().status(),
            DynamicTimeoutStatus::Evaluating
        );
    }

    #[test]
    fn a_long_link_error_that_is_not_about_a_pair_is_not_reported_to_the_net_source() {
        let (mut core, rec) = wired();

        // `kEctDial` is a pair the link never got to: the net source is not
        // told, but the app still is
        core.on_longlink_network_error_at(NOW, MAIN, ErrCmdType::Dial, -1, "1.2.3.4", 443);
        assert_eq!(
            rec.long_err(),
            vec![(ErrCmdType::Dial, -1, "1.2.3.4".to_string(), 443)]
        );
    }

    #[test]
    fn the_app_is_given_the_two_answers_of_a_link_that_came_up() {
        let (mut core, rec) = wired();
        up(&core, LongLinkStatus::Connected);

        core.on_longlink_status_changed_at(NOW, LongLinkStatus::Connected);
        assert_eq!(
            rec.status(),
            vec![(NetStatus::Connected, NetStatus::Connected)]
        );

        // a link that went down is not one the app is told about at all
        up(&core, LongLinkStatus::DisConnected);
        core.on_longlink_status_changed_at(NOW, LongLinkStatus::DisConnected);
        assert!(rec.status().is_empty());
    }

    #[test]
    fn a_core_without_a_long_link_reports_the_short_links_own_answer() {
        let (mut core, rec) = wired();
        core.set_need_use_long_link(false);

        core.on_shortlink_network_error_at(NOW, ErrCmdType::Socket, -1, "1.2.3.4", SHORT_HOST, 80);
        assert_eq!(
            rec.status(),
            vec![(NetStatus::Connected, NetStatus::Unknown)]
        );
        // with no long link the status change is not one the core looks at
        core.on_longlink_status_changed_at(NOW, LongLinkStatus::Connected);
        assert!(rec.status().is_empty());
    }

    #[test]
    fn a_network_change_throws_the_hosts_away_and_looks_at_every_task_again() {
        let (mut core, rec) = wired();
        // a link that is not up is not one a task may go out on
        up(&core, LongLinkStatus::ConnectFailed);
        assert!(core.start_task_at(NOW, task(7)));

        // three short-link errors: the app is told the server is down
        for _ in 0..SHORTLINK_ERR_TIME {
            core.on_shortlink_network_error_at(
                NOW,
                ErrCmdType::Socket,
                -1,
                "1.2.3.4",
                SHORT_HOST,
                80,
            );
        }
        assert_eq!(
            rec.status().last(),
            Some(&(NetStatus::ServerFailed, NetStatus::ServerFailed))
        );

        core.on_network_change_at(NOW + 100);

        // the task that was out is cancelled and looked at again, which is not
        // one the app is told about
        assert!(core.shortlink().has_task(7));
        assert!(rec.ended().is_empty());
        // ... and the counters the connect status is worked out from start
        // over, so the next error is the first one again
        core.on_shortlink_network_error_at(
            NOW + 100,
            ErrCmdType::Socket,
            -1,
            "1.2.3.4",
            SHORT_HOST,
            80,
        );
        assert_eq!(
            rec.status().last(),
            Some(&(NetStatus::Unknown, NetStatus::ServerFailed))
        );
    }

    #[test]
    fn a_push_is_preprocessed_before_the_app_is_given_it() {
        let (mut core, rec) = wired();

        core.longlink().on_response_at(
            NOW,
            LongAnswer {
                name: MAIN.to_string(),
                err_type: ErrCmdType::Ok,
                err_code: 0,
                cmdid: 6,
                taskid: Task::INVALID_TASK_ID,
                body: b"push".to_vec(),
                profile: ConnectProfile::new(),
            },
        );

        assert_eq!(rec.pushed(), vec![(6, "push".to_string())]);
        assert_eq!(drain(&rec.preprocessed), vec![6]);
    }

    #[test]
    fn a_second_long_link_can_be_made_marked_main_and_destroyed() {
        let (mut core, _rec) = wired();
        assert_eq!(core.default_link(), Some(MAIN));
        assert!(core.default_long_link().is_some());
        assert!(core.long_link_meta(MAIN).is_some());
        assert!(!core.is_long_link_connected("second"));

        let second = core.create_long_link(LonglinkConfig::new("second"));
        assert!(second.is_some());
        // `is_main` is not set, so the default link is still the first one
        assert_eq!(core.default_link(), Some(MAIN));

        assert!(core.mark_main_longlink("second"));
        assert_eq!(core.default_link(), Some("second"));
        // already the main one, and a link that does not exist
        assert!(!core.mark_main_longlink("second"));
        assert!(!core.mark_main_longlink("nope"));

        up(&core, LongLinkStatus::Connected);
        let second = Arc::clone(core.long_link("second").expect("the second link"));
        second
            .lock()
            .unwrap_or_else(poisoned)
            .set_status(LongLinkStatus::Connected);
        assert!(core.is_default_long_link_connected());

        assert!(core.destroy_long_link("second"));
        assert_eq!(core.default_link(), None);
        assert!(!core.destroy_long_link("second"));
        assert!(!core.is_default_long_link_connected());
    }

    #[test]
    fn a_core_that_does_not_use_the_long_link_makes_none() {
        let mut core = NetCore::with_encoder_at(NOW, false, crate::LongLinkEncoder::new());
        assert!(!core.use_long_link());
        assert!(core
            .create_long_link(LonglinkConfig::new("second"))
            .is_none());
        assert!(core.long_link(MAIN).is_none());
        // the same config twice does not make two links
        let mut core = NetCore::new_at(NOW);
        let first = core.create_long_link(LonglinkConfig::new("second"));
        let again = core.create_long_link(LonglinkConfig::new("second"));
        assert_eq!(
            first.map(|link| link.lock().unwrap_or_else(poisoned).connect_status()),
            again.map(|link| link.lock().unwrap_or_else(poisoned).connect_status())
        );
        assert_eq!(core.longlink().channels().len(), 2);
    }

    #[test]
    fn a_released_core_starts_nothing_and_reports_nothing() {
        let (mut core, rec) = wired();
        core.release();

        assert!(core.is_released());
        assert_eq!(core.default_link(), None);
        assert!(!core.start_task_at(NOW, task(7)));
        core.on_shortlink_network_error_at(NOW, ErrCmdType::Socket, -1, "1.2.3.4", SHORT_HOST, 80);
        core.on_longlink_network_error_at(NOW, MAIN, ErrCmdType::Socket, -1, "1.2.3.4", 443);
        assert!(rec.status().is_empty());
        assert!(rec.long_err().is_empty());
        assert!(rec.short_err().is_empty());
    }

    #[test]
    fn a_task_can_be_stopped_and_all_of_them_cleared() {
        let (mut core, _rec) = wired();
        up(&core, LongLinkStatus::Connected);
        assert!(core.start_task_at(NOW, task(7)));
        let mut short = task(8);
        short.channel_select = Task::CHANNEL_SHORT;
        assert!(core.start_task_at(NOW, short));

        assert!(core.has_task(7));
        assert!(core.has_task(8));
        assert!(core.stop_task(7));
        assert!(!core.has_task(7));
        assert!(!core.stop_task(7));

        core.clear_tasks();
        assert!(!core.has_task(8));
        assert_eq!(core.shortlink().len(), 0);
        assert_eq!(core.longlink().len(), 0);
    }

    #[test]
    fn a_task_of_a_destroyed_channel_is_failed() {
        let (mut core, rec) = wired();
        up(&core, LongLinkStatus::Connected);
        assert!(core.start_task_at(NOW, task(7)));

        assert!(core.destroy_long_link_at(NOW + 100, MAIN));
        assert!(!core.has_task(7));

        let ended = rec.ended();
        assert_eq!(
            ended,
            vec![(
                7,
                "user".to_string(),
                ErrCmdType::Local,
                LOCAL_LONG_LINK_RELEASED,
                "10.0.0.1".to_string()
            )]
        );
    }

    #[test]
    fn the_identify_check_the_app_answered_is_the_one_every_link_asks() {
        let (mut core, _rec) = wired();
        let asked: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = asked.clone();
        core.set_identify_check_buffer(move |channel_id, _cmdid| {
            recorder
                .lock()
                .unwrap_or_else(poisoned)
                .push(channel_id.to_string());
            IdentifyBuffer::now(b"check".to_vec(), b"hash".to_vec(), 99)
        });
        let judged: Arc<Mutex<Vec<(String, bool)>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = judged.clone();
        core.set_identify_on_response(move |channel_id, response, hash| {
            let accepted = response == hash;
            recorder
                .lock()
                .unwrap_or_else(poisoned)
                .push((channel_id.to_string(), accepted));
            accepted
        });

        // a link made *after* the app answered is asked the same way the one
        // that was already there is: the C++'s checker reaches the manager
        // whenever it needs the buffer, not when the link is made
        core.create_long_link(LonglinkConfig::new("second"));

        for name in [MAIN, "second"] {
            let link = Arc::clone(core.long_link(name).expect("the link"));
            let mut link = link.lock().unwrap_or_else(poisoned);
            link.set_status(LongLinkStatus::Connected);
            assert!(link.noop_req_at(NOW, false));
            assert!(link.noop_resp_at(
                NOW + 100,
                99,
                Task::LONG_LINK_IDENTIFY_CHECKER_TASK_ID,
                b"hash"
            ));
        }
        assert_eq!(
            *asked.lock().unwrap_or_else(poisoned),
            vec![MAIN.to_string(), "second".to_string()]
        );
        assert_eq!(
            *judged.lock().unwrap_or_else(poisoned),
            vec![(MAIN.to_string(), true), ("second".to_string(), true)]
        );
    }

    #[test]
    fn the_due_time_is_the_earliest_of_the_queues_and_the_zombies() {
        let (mut core, _rec) = wired();
        assert_eq!(core.due_time(), core.timing_sync().due_time());

        up(&core, LongLinkStatus::Connected);
        assert!(core.start_task_at(NOW, task(7)));
        let due = core.due_time();
        assert!(due.is_some());
        assert!(due <= Some(NOW + 60 * 1000));

        let mut task = task(9);
        task.network_status_sensitive = false;
        assert!(core.zombie().save_task_at(NOW, &task, 0));
        assert_eq!(core.due_time(), Some(NOW + 3000));
    }

    #[test]
    fn the_connect_profile_of_a_task_is_the_one_of_the_channel_it_went_out_on() {
        let (mut core, _rec) = wired();
        up(&core, LongLinkStatus::Connected);
        assert!(core.start_task_at(NOW, task(7)));

        let profile = core.connect_profile(7, Task::CHANNEL_LONG);
        assert_eq!(profile.ip, "10.0.0.1");
        assert_eq!(profile.port, 8080);
        assert_eq!(profile.host, MAIN);
        // the short link has no such task, and neither has a channel-select
        // that is not the long link's
        assert_eq!(core.connect_profile(7, Task::CHANNEL_SHORT).ip, "");
        assert_eq!(core.connect_profile(7, Task::CHANNEL_NORMAL).ip, "");
    }

    #[test]
    fn the_setters_reach_the_pieces_they_are_for() {
        let (mut core, _rec) = wired();
        core.set_debug_host(SHORT_HOST);
        core.forbid_longlink_tls_host(&[SHORT_HOST.to_string()]);
        core.add_server_ban("1.2.3.4");
        core.init_history_to_banned_list();
        core.set_ip_connect_timeout(1000, 2000);
        core.set_packer_encoder(3, "encoder");
        assert_eq!(core.packer_encoder_version(), 3);
        assert_eq!(core.packer_encoder_name(), "encoder");

        up(&core, LongLinkStatus::Connected);
        assert!(core.start_task_at(NOW, task(7)));
        assert!(core.disconnect_long_link_by_taskid(7, DisconnectInternalCode::Reset));
        core.make_sure_long_link_connected(MAIN);
        core.make_sure_default_long_link_connected();
        core.keep_signal_at(NOW);
        core.stop_signal();
        core.keep_signal();
        core.touch_tasks_at(NOW + 100);
        core.redo_tasks_at(NOW + 100);
    }

    #[test]
    fn the_debug_is_the_wiring_and_not_the_pieces() {
        let (core, _rec) = wired();
        let debug = format!("{core:?}");
        assert!(debug.contains("NetCore"));
        assert!(debug.contains(MAIN));
        assert!(debug.contains("use_long_link: true"));
    }

    #[test]
    fn the_two_answers_of_a_short_link_error_count() {
        assert_eq!(shortlink_status(0), NetStatus::Connected);
        assert_eq!(shortlink_status(1), NetStatus::Unknown);
        assert_eq!(shortlink_status(SHORTLINK_ERR_TIME - 1), NetStatus::Unknown);
        assert_eq!(
            shortlink_status(SHORTLINK_ERR_TIME),
            NetStatus::ServerFailed
        );
    }

    #[test]
    fn the_earliest_of_two_due_times_wins() {
        assert_eq!(min_due(None, None), None);
        assert_eq!(min_due(Some(5), None), Some(5));
        assert_eq!(min_due(None, Some(5)), Some(5));
        assert_eq!(min_due(Some(5), Some(3)), Some(3));
        assert_eq!(min_due(Some(3), Some(5)), Some(3));
    }

    #[test]
    fn a_task_the_net_core_will_not_start() {
        assert!(valid_and_init_default(&mut task(7)));

        let mut slow = task(7);
        slow.server_process_cost = 2 * 60 * 1000 + 1;
        assert!(!valid_and_init_default(&mut slow));

        let mut trying = task(7);
        trying.retry_count = 31;
        assert!(!valid_and_init_default(&mut trying));

        let mut late = task(7);
        late.total_timeout = 10 * 60 * 1000 + 1;
        assert!(!valid_and_init_default(&mut late));

        let mut no_cmdid = task(7);
        no_cmdid.channel_select = Task::CHANNEL_LONG;
        no_cmdid.cmdid = 0;
        assert!(valid_and_init_default(&mut no_cmdid));
        assert_eq!(no_cmdid.channel_select, 0);

        let mut no_cgi = task(7);
        no_cgi.channel_select = Task::CHANNEL_SHORT;
        no_cgi.cgi = String::new();
        assert!(valid_and_init_default(&mut no_cgi));
        assert_eq!(no_cgi.channel_select, 0);

        let mut negative = task(7);
        negative.retry_count = -5;
        assert!(valid_and_init_default(&mut negative));
        assert_eq!(negative.retry_count, DEF_TASK_RETRY_COUNT);

        // `Task::new` hands out `retry_count = -1`, which is the whole reason a
        // task the caller never configured has a try to come back for: `0`
        // would be "do not retry" and there would be none.
        let mut plain = Task::new(7, 12);
        assert!(valid_and_init_default(&mut plain));
        assert_eq!(plain.retry_count, DEF_TASK_RETRY_COUNT);
    }

    #[test]
    fn the_default_of_the_status_is_nothing_has_tried_yet() {
        assert_eq!(NetStatus::default(), NetStatus::Unknown);
        assert_eq!(NetStatus::Unknown as i32, -1);
        assert_eq!(NetStatus::ServerDown as i32, 5);
    }
}
