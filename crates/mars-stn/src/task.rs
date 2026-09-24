//! `mars/stn/stn.h` — the task the caller hands to STN.
//!
//! The C++ `Task` is a plain struct with a constructor that fills in the
//! defaults; the port keeps the same field names (snake case) and the same
//! defaults, and gives the channel/priority/protocol constants as associated
//! constants like the C++ statics.

use std::collections::BTreeMap;

/// `HostRedirectType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostRedirectType {
    /// `kHostRedirectNone`
    #[default]
    None,
    /// `kHostRedirectBareToHttps`
    BareToHttps,
    /// `kHostRedirectHttpToHttps`
    HttpToHttps,
    /// `kHostRedirectNewHost`
    NewHost,
}

/// One unit of work: a request STN sends over a long link or a short link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// Required: the id STN identifies the task by.
    pub taskid: u32,
    /// Required: the command id of the request.
    pub cmdid: u32,
    /// Which channels the task may use; see the `CHANNEL_*` constants.
    pub channel_select: i32,
    /// `kTransportProtocol*`.
    pub transport_protocol: i32,
    /// The CGI path.
    pub cgi: String,

    /// Send without waiting for a response.
    pub send_only: bool,
    /// Whether the task needs a logged-in session.
    pub need_authed: bool,
    /// Whether [`crate::FlowLimit`] applies.
    pub limit_flow: bool,
    /// Whether [`crate::FrequencyLimit`] applies.
    pub limit_frequency: bool,
    /// Whether the task is dropped when the network is down.
    pub network_status_sensitive: bool,
    /// `kChannelNormalStrategy` / `kChannelFastStrategy` /
    /// `kChannelDisasterRecoveryStategy`.
    pub channel_strategy: i32,
    /// `kTaskPriority*` — lower is more urgent.
    pub priority: i32,
    /// How many times STN retries the task.
    pub retry_count: i32,
    /// Expected server processing time, in milliseconds.
    pub server_process_cost: i32,
    /// Overall deadline, in milliseconds.
    pub total_timeout: i32,
    /// Whether the task is a long poll.
    pub long_polling: bool,
    /// Long-poll deadline, in milliseconds.
    pub long_polling_timeout: i32,

    /// Extra argument carried into the report.
    pub report_arg: String,
    /// The long link channel name.
    pub channel_name: String,
    /// Selects the decode method.
    pub group_name: String,
    /// Identifies the user of a multi-user long link.
    pub user_id: String,
    /// The protocol of the payload.
    pub protocol: i32,
    /// HTTP headers of a short-link task.
    pub headers: BTreeMap<String, String>,
    /// Hosts of the short link, in use order.
    pub shortlink_host_list: Vec<String>,
    /// Hosts to fall back to.
    pub shortlink_fallback_hostlist: Vec<String>,
    /// Hosts of the long link.
    pub longlink_host_list: Vec<String>,
    /// Hosts of the minor long link.
    pub minorlong_host_list: Vec<String>,
    /// Hosts of the QUIC link.
    pub quic_host_list: Vec<String>,
    /// How many minor long links may be open at once.
    pub max_minorlinks: i32,
    /// The function name, for reporting.
    pub function: String,
    /// Prefix of the CGI, for reporting.
    pub cgi_prefix: String,
    /// How the host was redirected.
    pub redirect_type: HostRedirectType,
    /// Sequence id that ties the task to the server-side report.
    pub client_sequence_id: u16,
}

impl Task {
    /// Short link only.
    pub const CHANNEL_SHORT: i32 = 0x1;
    /// Long link only.
    pub const CHANNEL_LONG: i32 = 0x2;
    /// Either link.
    pub const CHANNEL_BOTH: i32 = 0x3;
    /// A secondary long link.
    pub const CHANNEL_MINOR_LONG: i32 = 0x4;
    /// The normal long link.
    pub const CHANNEL_NORMAL: i32 = 0x5;
    /// Everything.
    pub const CHANNEL_ALL: i32 = 0x7;

    /// Pick the first available channel.
    pub const CHANNEL_NORMAL_STRATEGY: i32 = 0;
    /// Race the channels and keep the fastest.
    pub const CHANNEL_FAST_STRATEGY: i32 = 1;
    /// Fall back to another channel on failure.
    pub const CHANNEL_DISASTER_RECOVERY_STRATEGY: i32 = 2;

    /// Whatever the channel supports.
    pub const TRANSPORT_PROTOCOL_DEFAULT: i32 = 0;
    /// TCP.
    pub const TRANSPORT_PROTOCOL_TCP: i32 = 1;
    /// QUIC.
    pub const TRANSPORT_PROTOCOL_QUIC: i32 = 2;
    /// TCP or QUIC.
    pub const TRANSPORT_PROTOCOL_MIXED: i32 = 3;

    /// Highest and lowest priority, and the levels in between.
    pub const TASK_PRIORITY_HIGHEST: i32 = 0;
    pub const TASK_PRIORITY_0: i32 = 0;
    pub const TASK_PRIORITY_1: i32 = 1;
    pub const TASK_PRIORITY_2: i32 = 2;
    pub const TASK_PRIORITY_3: i32 = 3;
    pub const TASK_PRIORITY_NORMAL: i32 = 3;
    pub const TASK_PRIORITY_4: i32 = 4;
    pub const TASK_PRIORITY_5: i32 = 5;
    pub const TASK_PRIORITY_LOWEST: i32 = 5;

    /// `Task::kInvalidTaskID`.
    pub const INVALID_TASK_ID: u32 = 0;
    /// `Task::kNoopTaskID` — the heartbeat.
    pub const NOOP_TASK_ID: u32 = 0xffff_ffff;
    /// `Task::kLongLinkIdentifyCheckerTaskID`.
    pub const LONG_LINK_IDENTIFY_CHECKER_TASK_ID: u32 = 0xffff_fffe;
    /// `Task::kSignallingKeeperTaskID`.
    pub const SIGNALLING_KEEPER_TASK_ID: u32 = 0xffff_fffd;
    /// Command ids of the minor long link carry this mask.
    pub const MINOR_LONGLINK_CMD_MASK: u32 = 0xff00_0000;

    /// `Task()` — the defaults of the C++ constructor.
    pub fn new(taskid: u32, cmdid: u32) -> Self {
        Self {
            taskid,
            cmdid,
            channel_select: Self::CHANNEL_BOTH,
            transport_protocol: Self::TRANSPORT_PROTOCOL_DEFAULT,
            cgi: String::new(),
            send_only: false,
            need_authed: true,
            limit_flow: true,
            limit_frequency: true,
            network_status_sensitive: false,
            channel_strategy: Self::CHANNEL_NORMAL_STRATEGY,
            priority: Self::TASK_PRIORITY_NORMAL,
            retry_count: 0,
            server_process_cost: 0,
            total_timeout: 0,
            long_polling: false,
            long_polling_timeout: 0,
            report_arg: String::new(),
            channel_name: String::new(),
            group_name: String::new(),
            user_id: String::new(),
            protocol: 0,
            headers: BTreeMap::new(),
            shortlink_host_list: Vec::new(),
            shortlink_fallback_hostlist: Vec::new(),
            longlink_host_list: Vec::new(),
            minorlong_host_list: Vec::new(),
            quic_host_list: Vec::new(),
            max_minorlinks: 0,
            function: String::new(),
            cgi_prefix: String::new(),
            redirect_type: HostRedirectType::None,
            client_sequence_id: 0,
        }
    }
}
