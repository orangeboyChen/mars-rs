//! STN — the task pipeline of mars — ported from `mars/stn`.
//!
//! The first slice is the part that has no sockets and no callbacks: the task
//! model and the three policies that decide whether a task may go out at all
//! (anti-avalanche: frequency, flow, and the dynamic timeout status). They are
//! pure logic over [`mars_comm::tickcount::gettickcount`], which is what makes
//! them testable here.
//!
//! The second slice is the long link: its wire format ([`longlink`]) and the
//! heartbeat interval that keeps it alive ([`smart_heartbeat`]). Neither needs
//! a socket either — the packer is bytes in, bytes out, and the heartbeat
//! takes the answers as arguments — but the long link that *uses* them does,
//! so what is here is the format and the computation, not the connection.
//!
//! The third slice is what a task that ran leaves behind ([`task_profile`]) and
//! the logic that calls the network weak ([`weak_network`]): the error a task
//! failed with and how far it got decide whether the network is weak, and the
//! report the C++ hands to the app is a `(key, value)` pair the port hands to a
//! callback instead.
//!
//! The fourth slice is what keeps the long link trusted and the mapping alive:
//! the identify check the app answers before the connection is used
//! ([`longlink_identify_checker`]) and the signalling that keeps a mapping up
//! while the app waits ([`signalling_keeper`]).
//!
//! The fifth slice is the task that outlived the link it was started on
//! ([`zombie_task_manager`]): it is saved instead of failed, and started again
//! when the link comes back or when its periodic check decides it waited long
//! enough.
//!
//! The sixth slice is which ip/port pair a task is tried on
//! ([`simple_ipport_sort`]): one bit per attempt decides whether a pair is used
//! at all and in which order, and the history behind it is what survives
//! between runs — the port has no filesystem, so the host loads and saves it.
//!
//! The seventh slice is how soon the long link is tried again
//! ([`longlink_connect_monitor`]): an interval out of a table, and a ladder it
//! walks while the app is in the background.
//!
//! The eighth slice is when a run of failed tasks is a network worth
//! diagnosing ([`net_check_logic`]): a window of the last thirty-two tasks of
//! each link, and two limits on how often the diagnosis may start.
//!
//! The ninth slice is the periodic sync the app is asked for
//! ([`timing_sync`]) and the answer of a task the app answered itself
//! ([`task_intercept`]): one alarm restarted by every change, and one table
//! of answers that are good for a minute.
//!
//! The tenth slice is the check that gets the long link off a backup ip
//! ([`netsource_timercheck`]): a periodic post that resolves the host the link
//! is on and asks, at most so many times an hour, whether one of the other
//! pairs is reachable now.
//!
//! The eleventh slice is where an ip/port pair comes from ([`net_source`]): the
//! hosts, ports, debug ips and backup ips the app set, and the history that
//! says which pairs failed — at most five pairs per host list, sorted by their
//! history when they came from dns and shuffled when they came from the backup
//! list.
//!
//! The twelfth slice is the race a link runs between its pairs
//! ([`longlink_speed_test`]): the same noop goes out on all of them at once,
//! and the first one whose answer comes back is the one the link is made on.
//!
//! The thirteenth slice is the socket layer under the links
//! ([`socket_operator`] and [`socket_pool`]): what a link asks of a socket,
//! which is the host's to answer, and the pool that keeps the socket one task
//! is done with for the next task that wants the same ip, port and host — for
//! a while, and never again once one taken from it turned out to be no good.
//!
//! The fourteenth slice is the long link itself ([`long_link`]): the status a
//! task asks about, the connect that fills the profile the slices before this
//! one were written to fill in, and the proxy and the debug ip that say which
//! of its pairs it is made on.
//!
//! The fifteenth slice is the heartbeat that keeps it up ([`long_link`]): the
//! two alarms a host's run reads instead of waiting on, the noop that goes out
//! when the interval is up, the identify check that rides on the first one, and
//! what a heartbeat that answered — or did not — leaves on the profile.
//!
//! The sixteenth slice is the run that reads and writes it ([`long_link`]): the
//! four steps a host's loop calls — the connect, one write, one read, and what
//! the C++ does when the run is over — and what a read hands back: the answer
//! to a task, or the answer to the heartbeat that was out.
//!
//! The seventeenth slice is what a short-link task goes out as
//! ([`shortlink`]) and what comes back for it: a `POST` of the cgi with the
//! five fields mars writes for itself, and an answer that is the body when the
//! status is 200 and `kEctHttp` with the status when it is not.
//!
//! The eighteenth slice is the short link itself, and the connect it is made on
//! ([`short_link`]): the pairs a task may go out on, the proxy it goes through
//! when it goes through one, the socket the pool still has for one of them, and
//! the profile the connect writes as it goes.
//!
//! The nineteenth slice is the run on the socket that connect made
//! ([`short_link`]): the request the task goes out as, one read at a time of the
//! answer that comes back, the fork on its status, and the socket itself —
//! handed back to the pool when both the task and the server asked for it, and
//! closed when one of them did not.
//!
//! The twentieth slice is whether a proxy is one a task can go through
//! ([`proxy_test`]): the test connects the way the long link does, writes one
//! `GET` and judges the status that comes back — which is what the C++ asks of
//! a proxy before it routes anything through it.
//!
//! What still needs the app callbacks (`net_core`, `longlink_task_manager`, the
//! task managers that own the tasks, `stn_logic`) comes later.

/// The `static`s of this crate are one value for the whole process —
/// `sg_client_version` in [`longlink`], `outer_setted_heart_` in
/// [`smart_heartbeat`], and `g_period` / `g_keepTime` in
/// [`signalling_keeper`] — so the unit tests that move them need **one** lock
/// for the crate, not one per module.
/// `rand()` — the port's own, so a test does not have to care which numbers the
/// platform would have handed out.
///
/// The modules that need one take it as a [`FnMut`] a host can replace, and
/// this is what they start with: a xorshift over `seed`.
pub(crate) fn xorshift(seed: u64) -> impl FnMut(usize) -> usize + Send + 'static {
    let mut state = if seed == 0 {
        0x9E37_79B9_7F4A_7C15
    } else {
        seed
    };
    move |bound: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        if bound == 0 {
            0
        } else {
            (state % bound as u64) as usize
        }
    }
}

#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `rand()` — what the platform hands out in the C++: a number in `0..bound`.
///
/// Both [`simple_ipport_sort`] and [`longlink_connect_monitor`] take one as a
/// callback, which is what makes the shuffle and the jitter a test can pin
/// down.
pub type Random = dyn FnMut(usize) -> usize + Send;

pub mod anti_avalanche;
pub mod config;
pub mod dynamic_timeout;
pub mod flow_limit;
pub mod frequency_limit;
pub mod long_link;
pub mod longlink;
pub mod longlink_connect_monitor;
pub mod longlink_identify_checker;
pub mod longlink_speed_test;
pub mod net_check_logic;
pub mod net_source;
pub mod netsource_timercheck;
pub mod proxy_test;
pub mod short_link;
pub mod shortlink;
pub mod signalling_keeper;
pub mod simple_ipport_sort;
pub mod smart_heartbeat;
pub mod socket_operator;
pub mod socket_pool;
pub mod task;
pub mod task_intercept;
pub mod task_profile;
pub mod timing_sync;
pub mod weak_network;
pub mod zombie_task_manager;

pub use anti_avalanche::{AntiAvalanche, LimitKind};
pub use dynamic_timeout::{DynamicTimeout, DynamicTimeoutStatus};
pub use flow_limit::FlowLimit;
pub use frequency_limit::FrequencyLimit;
pub use long_link::{
    AlarmStatus, Answer, ConnectFail, DisconnectInternalCode, LongLink, MakeSure, NoopAlarm,
    RunEnd, SendData, Written, EBADMSG, ECT_DNS_MAKE_SOCKET_PREPARED,
    ECT_NET_MSG_XP_HANDLE_BUFFER_ERR, ECT_SOCKET_MAKE_SOCKET_PREPARED, ECT_SOCKET_RECV_ERR,
    ECT_SOCKET_SHUTDOWN, ECT_SOCKET_USER_BREAK, NET_TYPE_WIFI, NOOP_ACTIVE_TIMEOUT,
    NOOP_LATE_TOO_MUCH, NOOP_TIMEOUT, RECV_BUFFER_LEN, SELECT_TIMEOUT,
};
pub use longlink::{LongLinkEncoder, Unpacked};
pub use longlink_connect_monitor::{
    ActiveState, ConnectType, LongLinkConnectMonitor, LongLinkStatus, INACTIVE_BUFFER,
    INTERNAL_MAX_INDEX, INTERVALS, NO_ACCOUNT_INFO_INACTIVE_INTERVAL, NO_ACCOUNT_INFO_SALT_RATE,
    NO_ACCOUNT_INFO_SALT_RISE, NO_NET_SALT_RATE, NO_NET_SALT_RISE, RECONNECT_INTERVAL,
    START_CHECK_PERIOD, TIME_CHECK_PERIOD, UP_OR_DOWN_THRESHOLD, WAKE_ALARM_INTERVAL,
};
pub use longlink_identify_checker::{IdentifyBuffer, IdentifyMode, LongLinkIdentifyChecker};
pub use longlink_speed_test::{
    Fastest, LongLinkSpeedTest, Need, Socket, SocketEvent, SpeedTestItem, SpeedTestState, Stop,
    Watch, MAX_RETRIES, OUT_OF_BAND_CMDID,
};
pub use net_check_logic::{
    NetCheckLogic, CHECK_IF_ABOVE_COUNT, CHECK_IF_BELOW_COUNT, CHECK_TIME_SPAN_INCREMENT_STEP,
    LIMIT_COUNT, LIMIT_TIME_SPAN, MIN_CHECK_TIME_SPAN, MOST_RECENT_TASK_START_N, NET_CHECK_MODE,
    SECOND_RECENT_TASK_START_N, VALID_BITS_FILTER,
};
pub use net_source::{
    ExtraInfo, LonglinkConfig, NetSource, TimeoutSource, CGI_DEBUG_DEFAULT_PORT,
    DEFAULT_LONGLINK_GROUP, DEFAULT_QUIC_CONNECT_TIMEOUT_MS, DEFAULT_QUIC_RW_TIMEOUT_MS,
    DISABLE_QUIC_SECONDS, ITEM_DELIMITER, NUM_MAKE_COUNT,
};
pub use netsource_timercheck::{NetSourceTimerCheck, INTERVAL_TIME, MAX_SPEED_TEST_COUNT, TIMEOUT};
pub use proxy_test::{ProxyTest, Verdict, BUFFER_SIZE, READ_TIMEOUT_MS, TEST_PORT};
// `ConnectFail` is not here: [`long_link`] already has one of that name, so the
// short link's is [`short_link::ConnectFail`].
pub use short_link::{
    NetworkLabel, Read, RunFail, ShortLink, DEFAULT_CONNECT_TIMEOUT_MS, DEFAULT_RW_TIMEOUT_MS,
    ECT_HTTP_PARSE_STATUS_LINE, ECT_HTTP_SPLIT_HTTP_HEAD_AND_BODY, ECT_SOCKET_READ_ONCE,
    ECT_SOCKET_WRITEN_WITH_NON_BLOCK, ENOTCONN, ETIMEDOUT, K_BUFFER_SIZE, K_MOBILE, K_WIFI,
};
pub use shortlink::{
    default_packer, is_keep_alive, keep_alive, pack, request_headers, request_url, Headers,
    KeepAlive, Packer,
};
pub use signalling_keeper::{SignallingKeeper, DEFAULT_KEEP_TIME, DEFAULT_PERIOD};
pub use simple_ipport_sort::{
    BanItem, IpPortItem, IpSourceType, Record, RecordItem, SimpleIpPortSort, BAN_FAIL_COUNT,
    BAN_TIME, FAIL_UPDATE_INTERVAL, MAX_BAN_TIME, RECORD_TIMEOUT, SERVER_BAN_TIME,
    SUCCESS_UPDATE_INTERVAL,
};
pub use smart_heartbeat::{
    NetHeartbeatInfo, SmartHeartBeatAction, SmartHeartBeatType, SmartHeartbeat,
};
pub use socket_operator::{
    contain_ipv6, is_impatient, tcp_identify, OpBreaker, SocketFd, SocketOperator, SocketProfile,
};
pub use socket_pool::{CachedSocket, SocketPool, BAN_INTERVAL, DEFAULT_MAX_KEEPALIVE_TIME};
pub use task::{HostRedirectType, Task};
pub use task_intercept::{TaskIntercept, TaskInterceptInfo, INTERCEPT_TIMEOUT};
pub use task_profile::{
    ConnectProfile, ErrCmdType, NoopProfile, TaskFailHandleType, TaskFailStep, TaskOutcome,
};
pub use timing_sync::{
    alarm_time, TimingSync, ACTIVE_SYNC_INTERVAL, INACTIVE_SYNC_INTERVAL, NONET_SALT_RATE,
    UNLOGIN_SYNC_INTERVAL,
};
pub use weak_network::{ReportWeak, WeakKey, WeakNetworkLogic};
pub use zombie_task_manager::ZombieTaskManager;
