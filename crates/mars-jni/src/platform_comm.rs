//! `io.github.orangeboychen.marsrs.comm.PlatformComm$C2Java` — what the platform is, as
//! `com/tencent/mars/comm/PlatformComm.java` answers it.
//!
//! `PlatformComm.java` declares no `native` method: `C2Java` is the set of
//! statics the C++ calls through `mars/comm/jni/platform_comm.cc` to ask
//! Android about the network, the proxy, the SIM card and the signal. This
//! module is that question and its answer: what a host set here is what the
//! port answers with, and what it did not set is asked of Java — one
//! [`Question`], one [`Answer`], which is what makes the nine C2Java calls one
//! seam.
//!
//! The two platform services it owns outright are not among them: the alarm and
//! the wake lock delegate to [`crate::alarm`] and [`crate::wakerlock`], which
//! is what the C++'s `startAlarm`/`stopAlarm`/`wakeupLock_new` are here — a
//! timer and a lock this process keeps, not something Java is asked for.
//!
//! Not ported: `getifaddrs_ipv4_hotspot` (the C++ reads it out of `getifaddrs`,
//! and nothing in the port has an interface list); the `_force_refresh` and
//! `realtime` arguments of `getCurWifiInfo` and `getCurSIMInfo`, which the
//! C++'s own C2Java calls do not hand over either.
//!
//! Everything the JVM touches lives in [`crate::jni_bridge`]; what is here is
//! plain Rust and is covered by `cargo test`.

use std::sync::{Mutex, OnceLock};

use crate::alarm::{alarm_start_impl, alarm_stop_impl};
use crate::wakerlock::{wakerlock_new_impl, WakerLockHandle};

/// One of the nine questions `mars/comm/jni/platform_comm.cc` asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Question {
    /// `getNetInfo()I` — the network the device is on.
    NetInfo,
    /// `getStatisticsNetType()I` — the network type as the statistics report
    /// wants it.
    StatisticsNetType,
    /// `getProxyInfo(Ljava/lang/StringBuffer;)I` — the port Java answers with,
    /// and the host it writes into the buffer it was handed.
    ProxyInfo,
    /// `getCurWifiInfo()Lcom/tencent/mars/comm/PlatformComm$WifiInfo;`.
    WifiInfo,
    /// `getCurSIMInfo()Lcom/tencent/mars/comm/PlatformComm$SIMInfo;`.
    SimInfo,
    /// `getAPNInfo()Lcom/tencent/mars/comm/PlatformComm$APNInfo;`.
    ApnInfo,
    /// `getCurRadioAccessNetworkInfo()I`.
    RadioAccessNetwork,
    /// `getSignal(Z)J` — the signal strength, asked for the wifi or for the
    /// gsm radio.
    Signal {
        /// `isWifi`
        wifi: bool,
    },
    /// `isNetworkConnected()Z`.
    NetworkConnected,
}

/// What Android answered, in the terms the port asks in.
///
/// A question nobody answered — no VM to attach to, a call that could not be
/// made — is [`Answer::Nothing`], and every answer read out of it is the
/// platform's own default: offline, no proxy, no wifi, no SIM, no access point,
/// no signal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Answer {
    /// `getNetInfo`.
    NetInfo(NetInfo),
    /// `getStatisticsNetType`.
    StatisticsNetType(NetType),
    /// `getProxyInfo` — the port, and the host; a port of `-1` means there is
    /// no proxy.
    Proxy {
        /// what the call answered.
        port: i32,
        /// what Java wrote into the `StringBuffer`.
        host: String,
    },
    /// `getCurWifiInfo` — [`None`] when the device is not on wifi.
    Wifi(Option<WifiInfo>),
    /// `getCurSIMInfo` — [`None`] when there is no SIM.
    Sim(Option<SimInfo>),
    /// `getAPNInfo` — [`None`] when there is no active network.
    Apn(Option<ApnInfo>),
    /// `getCurRadioAccessNetworkInfo`.
    RadioAccessNetwork(i32),
    /// `getSignal`.
    Signal(i64),
    /// `isNetworkConnected`.
    Connected(bool),
    /// Java that was not asked at all.
    #[default]
    Nothing,
}

impl Answer {
    /// `getNetInfo` — offline when the answer is not this one.
    pub fn net_info(&self) -> NetInfo {
        match self {
            Self::NetInfo(net_info) => *net_info,
            _ => NetInfo::NoNet,
        }
    }

    /// `getStatisticsNetType` — `NETTYPE_NON` when the answer is not this one.
    pub fn statistics_net_type(&self) -> NetType {
        match self {
            Self::StatisticsNetType(net_type) => *net_type,
            _ => NetType::Non,
        }
    }

    /// `getProxyInfo` — `(-1, "")` when the answer is not this one, which is
    /// the C++'s own "there is no proxy".
    pub fn proxy(&self) -> (i32, String) {
        match self {
            Self::Proxy { port, host } => (*port, host.clone()),
            _ => (-1, String::new()),
        }
    }

    /// `getCurWifiInfo` — [`None`] when the answer is not this one.
    pub fn wifi(&self) -> Option<WifiInfo> {
        match self {
            Self::Wifi(wifi) => wifi.clone(),
            _ => None,
        }
    }

    /// `getCurSIMInfo` — [`None`] when the answer is not this one.
    pub fn sim(&self) -> Option<SimInfo> {
        match self {
            Self::Sim(sim) => sim.clone(),
            _ => None,
        }
    }

    /// `getAPNInfo` — [`None`] when the answer is not this one.
    pub fn apn(&self) -> Option<ApnInfo> {
        match self {
            Self::Apn(apn) => apn.clone(),
            _ => None,
        }
    }

    /// `getCurRadioAccessNetworkInfo` — `UNKNOWN` when the answer is not this
    /// one.
    pub fn radio_access_network(&self) -> i32 {
        match self {
            Self::RadioAccessNetwork(network) => *network,
            _ => RADIO_ACCESS_NETWORK_UNKNOWN,
        }
    }

    /// `getSignal` — `0` when the answer is not this one.
    pub fn signal(&self) -> i64 {
        match self {
            Self::Signal(signal) => *signal,
            _ => 0,
        }
    }

    /// `isNetworkConnected` — `false` when the answer is not this one.
    pub fn connected(&self) -> bool {
        match self {
            Self::Connected(connected) => *connected,
            _ => false,
        }
    }
}

/// What the platform is asked with: [`Ask::jvm`] asks Java, and a host hands
/// over its own with [`set_ask`] — which is what the C++'s own platform
/// (`platform_comm_base.cc`) is.
pub struct Ask {
    ask: Box<dyn FnMut(Question) -> Answer + Send>,
}

impl Ask {
    /// The platform whose answers come from Java.
    pub fn jvm() -> Self {
        Self::new(crate::jni_bridge::ask_platform_comm)
    }

    /// The same, with another answerer: a host that is not Java, or a test.
    pub fn new(ask: impl FnMut(Question) -> Answer + Send + 'static) -> Self {
        Self { ask: Box::new(ask) }
    }

    fn ask(&mut self, question: Question) -> Answer {
        (self.ask)(question)
    }
}

impl std::fmt::Debug for Ask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ask").finish_non_exhaustive()
    }
}

fn asker() -> &'static Mutex<Ask> {
    static ASK: OnceLock<Mutex<Ask>> = OnceLock::new();
    ASK.get_or_init(|| Mutex::new(Ask::jvm()))
}

/// The platform is asked through this instead of through the JVM — the C++'s
/// `SetNetworkInfoCallback`.
pub fn set_ask(ask: Ask) {
    let mut asked = asker().lock().unwrap_or_else(poisoned);
    *asked = ask;
}

fn ask_platform(question: Question) -> Answer {
    asker().lock().unwrap_or_else(poisoned).ask(question)
}

fn poisoned<T>(error: std::sync::PoisonError<T>) -> T {
    error.into_inner()
}

/// `PlatformComm.ENoNet` / `EWifi` / `EMobile` / `EOtherNet` — what
/// `getNetInfo()` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetInfo {
    /// `ENoNet`
    NoNet = -1,
    /// `EWifi`
    Wifi = 1,
    /// `EMobile`
    Mobile = 2,
    /// `EOtherNet` — and what an unknown number falls back to.
    OtherNet = 3,
}

impl NetInfo {
    /// The number Java answered; anything else is `EOtherNet`, which is what
    /// `getNetInfo()` returns from its `default` branch.
    pub fn of(value: i32) -> Self {
        match value {
            -1 => Self::NoNet,
            1 => Self::Wifi,
            2 => Self::Mobile,
            _ => Self::OtherNet,
        }
    }

    /// The number to hand back to the network layer.
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// `PlatformComm.NETTYPE_*` — what `getStatisticsNetType()` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetType {
    /// `NETTYPE_NON` — no network, and what an unknown number falls back to.
    Non = -1,
    /// `NETTYPE_NOT_WIFI`
    NotWifi = 0,
    /// `NETTYPE_WIFI`
    Wifi = 1,
    /// `NETTYPE_WAP`
    Wap = 2,
    /// `NETTYPE_2G`
    G2 = 3,
    /// `NETTYPE_3G`
    G3 = 4,
    /// `NETTYPE_4G`
    G4 = 5,
    /// `NETTYPE_UNKNOWN`
    Unknown = 6,
}

impl NetType {
    /// The number Java answered; anything else is `NETTYPE_NON`, which is what
    /// `getStatisticsNetType()` returns when the context is gone.
    pub fn of(value: i32) -> Self {
        match value {
            0 => Self::NotWifi,
            1 => Self::Wifi,
            2 => Self::Wap,
            3 => Self::G2,
            4 => Self::G3,
            5 => Self::G4,
            6 => Self::Unknown,
            _ => Self::Non,
        }
    }

    /// The number to report.
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// `TelephonyManager.NETWORK_TYPE_UNKNOWN` — what
/// `getCurRadioAccessNetworkInfo()` answers when there is no telephony manager.
pub const RADIO_ACCESS_NETWORK_UNKNOWN: i32 = 0;

/// `kMarsDefaultNetLabel` — the label of a network that is neither a wifi one
/// nor a mobile one.
pub const DEFAULT_NET_LABEL: &str = "default";
/// `kMarsWifiNetLabelPrefix`.
pub const WIFI_NET_LABEL_PREFIX: &str = "wifi_";
/// `kMarsMobileNetLabelPrefix`.
pub const MOBILE_NET_LABEL_PREFIX: &str = "mobile_";

/// `PlatformComm.WifiInfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WifiInfo {
    /// The SSID.
    pub ssid: String,
    /// The BSSID.
    pub bssid: String,
}

/// `PlatformComm.SIMInfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimInfo {
    /// The operator code, as a string — the Java writes `"" + ispCode`.
    pub isp_code: String,
    /// The operator name.
    pub isp_name: String,
}

/// `PlatformComm.APNInfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApnInfo {
    /// `NetworkInfo.getType()`.
    pub net_type: i32,
    /// `NetworkInfo.getSubtype()`.
    pub sub_net_type: i32,
    /// The APN name, or the SSID when the network is a wifi one.
    pub extra_info: String,
}

/// What was answered about a question whose answer may be "there is none":
/// [`None`] is a question nobody has answered, and `Some(None)` is an answer of
/// "there is none" — no wifi, no SIM, no active network.
type Answered<T> = Option<Option<T>>;

/// What the platform answered in Rust: a host that hands these over is what
/// the C++'s own platform is, and what is set here is what the port answers
/// with instead of asking Java.
#[derive(Debug, Clone, Default)]
struct PlatformState {
    net_info: Option<NetInfo>,
    statistics_net_type: Option<NetType>,
    proxy: Option<(i32, String)>,
    wifi: Answered<WifiInfo>,
    sim: Answered<SimInfo>,
    apn: Answered<ApnInfo>,
    radio_access_network: Option<i32>,
    wifi_signal: Option<i64>,
    gsm_signal: Option<i64>,
    network_connected: Option<bool>,
}

fn state() -> &'static Mutex<PlatformState> {
    static STATE: OnceLock<Mutex<PlatformState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(PlatformState::default()))
}

fn with_state<R>(f: impl FnOnce(&mut PlatformState) -> R) -> R {
    let mut state = state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut state)
}

/// `C2Java.getNetInfo()` — what a host answered, which is what the port answers
/// with from here on.
pub fn set_net_info_impl(value: i32) {
    with_state(|state| state.net_info = Some(NetInfo::of(value)))
}

/// The network the device is on: what a host set, or what Java answered.
pub fn net_info_impl() -> NetInfo {
    match with_state(|state| state.net_info) {
        Some(net_info) => net_info,
        None => ask_platform(Question::NetInfo).net_info(),
    }
}

/// `C2Java.getStatisticsNetType()`.
pub fn set_statistics_net_type_impl(value: i32) {
    with_state(|state| state.statistics_net_type = Some(NetType::of(value)))
}

/// The network type as the statistics report wants it.
pub fn statistics_net_type_impl() -> NetType {
    match with_state(|state| state.statistics_net_type) {
        Some(net_type) => net_type,
        None => ask_platform(Question::StatisticsNetType).statistics_net_type(),
    }
}

/// `C2Java.getProxyInfo(StringBuffer)`.
pub fn set_proxy_info_impl(host: &str, port: i32) {
    with_state(|state| state.proxy = Some((port, host.to_owned())))
}

/// The proxy, `(port, host)`; a port of `-1` means there is none.
pub fn proxy_info_impl() -> (i32, String) {
    match with_state(|state| state.proxy.clone()) {
        Some(proxy) => proxy,
        None => ask_platform(Question::ProxyInfo).proxy(),
    }
}

/// `C2Java.getCurWifiInfo()` — `None` when the device is not on wifi.
pub fn set_wifi_info_impl(wifi: Option<WifiInfo>) {
    with_state(|state| state.wifi = Some(wifi))
}

/// The wifi the device is on.
pub fn wifi_info_impl() -> Option<WifiInfo> {
    match with_state(|state| state.wifi.clone()) {
        Some(wifi) => wifi,
        None => ask_platform(Question::WifiInfo).wifi(),
    }
}

/// `C2Java.getCurSIMInfo()` — `None` when there is no SIM.
pub fn set_sim_info_impl(sim: Option<SimInfo>) {
    with_state(|state| state.sim = Some(sim))
}

/// `comm::getCurrNetLabel()` — the network the device is on, as the net type and
/// the label mars keeps the history of a host under: `wifi_<ssid>`,
/// `mobile_<isp code>`, or [`DEFAULT_NET_LABEL`] for a network that is neither.
///
/// This is the one `mars-stn` asks for (`NetSource::set_net_label` and
/// `SimpleIpPortSort::set_net_label`), and here it is built out of what
/// [`net_info_impl`], [`wifi_info_impl`] and [`sim_info_impl`] answered, the way
/// the C++ builds it out of `getNetInfo()`, `getCurWifiInfo()` and
/// `getCurSIMInfo()`. A wifi or a SIM the platform never answered leaves the
/// prefix and nothing behind it, which is what a default-constructed `WifiInfo`
/// and `SimInfo` read as in the C++.
///
/// Not ported: `getRealtimeNetLabel()`, whose only difference is asking the
/// platform again with `realtime = true` — nothing here caches an answer to
/// refresh, so it would be this same call; and `getNetworkIDLabel()`, which is
/// the app's own wifi id (`SetWiFiIdCallBack`) in front of this same fallback.
pub fn net_label_impl() -> (NetInfo, String) {
    let net_info = net_info_impl();
    let label = match net_info {
        NetInfo::Wifi => {
            let ssid = wifi_info_impl().map_or(String::new(), |wifi| wifi.ssid);
            format!("{WIFI_NET_LABEL_PREFIX}{ssid}")
        }
        NetInfo::Mobile => {
            let isp_code = sim_info_impl().map_or(String::new(), |sim| sim.isp_code);
            format!("{MOBILE_NET_LABEL_PREFIX}{isp_code}")
        }
        NetInfo::NoNet | NetInfo::OtherNet => DEFAULT_NET_LABEL.to_owned(),
    };
    (net_info, label)
}

/// The SIM in the device.
pub fn sim_info_impl() -> Option<SimInfo> {
    match with_state(|state| state.sim.clone()) {
        Some(sim) => sim,
        None => ask_platform(Question::SimInfo).sim(),
    }
}

/// `C2Java.getAPNInfo()` — `None` when there is no active network.
pub fn set_apn_info_impl(apn: Option<ApnInfo>) {
    with_state(|state| state.apn = Some(apn))
}

/// The access point the device is on.
pub fn apn_info_impl() -> Option<ApnInfo> {
    match with_state(|state| state.apn.clone()) {
        Some(apn) => apn,
        None => ask_platform(Question::ApnInfo).apn(),
    }
}

/// `C2Java.getCurRadioAccessNetworkInfo()`.
pub fn set_radio_access_network_info_impl(value: i32) {
    with_state(|state| state.radio_access_network = Some(value))
}

/// The radio access network.
pub fn radio_access_network_info_impl() -> i32 {
    match with_state(|state| state.radio_access_network) {
        Some(network) => network,
        None => ask_platform(Question::RadioAccessNetwork).radio_access_network(),
    }
}

/// `C2Java.getSignal(isWifi)`.
pub fn set_signal_impl(is_wifi: bool, signal: i64) {
    with_state(|state| {
        if is_wifi {
            state.wifi_signal = Some(signal);
        } else {
            state.gsm_signal = Some(signal);
        }
    })
}

/// The signal strength; `0` when the platform never answered.
pub fn signal_impl(is_wifi: bool) -> i64 {
    let answered = with_state(|state| {
        if is_wifi {
            state.wifi_signal
        } else {
            state.gsm_signal
        }
    });
    match answered {
        Some(signal) => signal,
        None => ask_platform(Question::Signal { wifi: is_wifi }).signal(),
    }
}

/// `C2Java.isNetworkConnected()`.
pub fn set_network_connected_impl(connected: bool) {
    with_state(|state| state.network_connected = Some(connected))
}

/// Whether the device has a network at all.
pub fn is_network_connected_impl() -> bool {
    match with_state(|state| state.network_connected) {
        Some(connected) => connected,
        None => ask_platform(Question::NetworkConnected).connected(),
    }
}

/// `C2Java.startAlarm(type, id, after)` — the timer is [`crate::alarm`].
pub fn start_alarm_impl(_type: i32, id: i64, after: i64, curtime: u64) -> bool {
    alarm_start_impl(id, after, curtime)
}

/// `C2Java.stopAlarm(id)`.
pub fn stop_alarm_impl(id: i64) -> bool {
    alarm_stop_impl(id)
}

/// `C2Java.wakeupLock_new()` — the lock is [`crate::wakerlock`].
pub fn wakeup_lock_new_impl() -> WakerLockHandle {
    wakerlock_new_impl()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn isolated<R>(f: impl FnOnce() -> R) -> R {
        let guard = crate::test_lock();
        reset();
        let result = f();
        reset();
        drop(guard);
        result
    }

    fn reset() {
        with_state(|state| *state = PlatformState::default());
        set_ask(Ask::new(|_| Answer::Nothing));
    }

    /// A platform that writes down what it was asked and answers `answer` —
    /// i.e. what the JVM hook would have been handed.
    fn asking(answer: Answer) -> Arc<Mutex<Vec<Question>>> {
        let questions = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&questions);
        set_ask(Ask::new(move |question| {
            recorded.lock().unwrap_or_else(poisoned).push(question);
            answer.clone()
        }));
        questions
    }

    fn asked(questions: &Arc<Mutex<Vec<Question>>>) -> Vec<Question> {
        questions.lock().unwrap_or_else(poisoned).clone()
    }

    #[test]
    fn a_platform_that_answered_nothing_is_offline() {
        isolated(|| {
            assert_eq!(net_info_impl(), NetInfo::NoNet);
            assert_eq!(statistics_net_type_impl(), NetType::Non);
            assert_eq!(proxy_info_impl(), (-1, String::new()));
            assert_eq!(wifi_info_impl(), None);
            assert_eq!(sim_info_impl(), None);
            assert_eq!(apn_info_impl(), None);
            assert_eq!(
                radio_access_network_info_impl(),
                RADIO_ACCESS_NETWORK_UNKNOWN
            );
            assert_eq!(signal_impl(true), 0);
            assert_eq!(signal_impl(false), 0);
            assert!(!is_network_connected_impl());
        })
    }

    #[test]
    fn a_platform_a_host_said_nothing_about_is_asked() {
        isolated(|| {
            let questions = asking(Answer::Nothing);

            let _ = net_info_impl();
            let _ = statistics_net_type_impl();
            let _ = proxy_info_impl();
            let _ = wifi_info_impl();
            let _ = sim_info_impl();
            let _ = apn_info_impl();
            let _ = radio_access_network_info_impl();
            let _ = signal_impl(true);
            let _ = signal_impl(false);
            let _ = is_network_connected_impl();

            assert_eq!(
                asked(&questions),
                vec![
                    Question::NetInfo,
                    Question::StatisticsNetType,
                    Question::ProxyInfo,
                    Question::WifiInfo,
                    Question::SimInfo,
                    Question::ApnInfo,
                    Question::RadioAccessNetwork,
                    Question::Signal { wifi: true },
                    Question::Signal { wifi: false },
                    Question::NetworkConnected,
                ]
            );
        })
    }

    #[test]
    fn what_a_host_set_is_what_the_port_reads() {
        isolated(|| {
            set_net_info_impl(1);
            set_statistics_net_type_impl(5);
            set_proxy_info_impl("proxy.example.com", 8080);
            set_wifi_info_impl(Some(WifiInfo {
                ssid: "MarsWifi".to_owned(),
                bssid: "00:11:22:33:44:55".to_owned(),
            }));
            set_sim_info_impl(Some(SimInfo {
                isp_code: "46001".to_owned(),
                isp_name: "China Unicom".to_owned(),
            }));
            set_apn_info_impl(Some(ApnInfo {
                net_type: 1,
                sub_net_type: 0,
                extra_info: "MarsWifi".to_owned(),
            }));
            set_radio_access_network_info_impl(13);
            set_signal_impl(true, -55);
            set_signal_impl(false, -70);
            set_network_connected_impl(true);

            assert_eq!(net_info_impl(), NetInfo::Wifi);
            assert_eq!(statistics_net_type_impl(), NetType::G4);
            assert_eq!(proxy_info_impl(), (8080, "proxy.example.com".to_owned()));
            assert_eq!(wifi_info_impl().unwrap().ssid, "MarsWifi");
            assert_eq!(sim_info_impl().unwrap().isp_name, "China Unicom");
            assert_eq!(apn_info_impl().unwrap().extra_info, "MarsWifi");
            assert_eq!(radio_access_network_info_impl(), 13);
            // the two signals are kept apart
            assert_eq!(signal_impl(true), -55);
            assert_eq!(signal_impl(false), -70);
            assert!(is_network_connected_impl());
        })
    }

    #[test]
    fn what_a_host_answered_is_what_java_is_never_asked() {
        isolated(|| {
            let questions = asking(Answer::Connected(true));

            // an answer of "there is none" is an answer, not a question
            set_wifi_info_impl(None);
            set_sim_info_impl(None);
            set_apn_info_impl(None);

            assert_eq!(wifi_info_impl(), None);
            assert_eq!(sim_info_impl(), None);
            assert_eq!(apn_info_impl(), None);

            assert!(asked(&questions).is_empty());
        })
    }

    #[test]
    fn what_java_answered_is_what_the_port_reads() {
        isolated(|| {
            set_ask(Ask::new(|question| match question {
                Question::NetInfo => Answer::NetInfo(NetInfo::Mobile),
                Question::StatisticsNetType => Answer::StatisticsNetType(NetType::G3),
                Question::ProxyInfo => Answer::Proxy {
                    port: 8080,
                    host: "proxy.example.com".to_owned(),
                },
                Question::WifiInfo => Answer::Wifi(Some(WifiInfo {
                    ssid: "MarsWifi".to_owned(),
                    bssid: "00:11:22:33:44:55".to_owned(),
                })),
                Question::SimInfo => Answer::Sim(Some(SimInfo {
                    isp_code: "46001".to_owned(),
                    isp_name: "China Unicom".to_owned(),
                })),
                Question::ApnInfo => Answer::Apn(Some(ApnInfo {
                    net_type: 1,
                    sub_net_type: 0,
                    extra_info: "MarsWifi".to_owned(),
                })),
                Question::RadioAccessNetwork => Answer::RadioAccessNetwork(13),
                Question::Signal { wifi } => Answer::Signal(if wifi { -55 } else { -70 }),
                Question::NetworkConnected => Answer::Connected(true),
            }));

            assert_eq!(net_info_impl(), NetInfo::Mobile);
            assert_eq!(statistics_net_type_impl(), NetType::G3);
            assert_eq!(proxy_info_impl(), (8080, "proxy.example.com".to_owned()));
            assert_eq!(wifi_info_impl().unwrap().ssid, "MarsWifi");
            assert_eq!(sim_info_impl().unwrap().isp_name, "China Unicom");
            assert_eq!(apn_info_impl().unwrap().extra_info, "MarsWifi");
            assert_eq!(radio_access_network_info_impl(), 13);
            assert_eq!(signal_impl(true), -55);
            assert_eq!(signal_impl(false), -70);
            assert!(is_network_connected_impl());
        })
    }

    #[test]
    fn the_label_of_a_network_is_its_ssid_or_its_isp_code() {
        isolated(|| {
            set_net_info_impl(NetInfo::Wifi.as_i32());
            set_wifi_info_impl(Some(WifiInfo {
                ssid: "home".to_owned(),
                bssid: "00:11:22:33:44:55".to_owned(),
            }));
            assert_eq!(net_label_impl(), (NetInfo::Wifi, "wifi_home".to_owned()));

            set_net_info_impl(NetInfo::Mobile.as_i32());
            set_sim_info_impl(Some(SimInfo {
                isp_code: "46000".to_owned(),
                isp_name: "China Mobile".to_owned(),
            }));
            assert_eq!(
                net_label_impl(),
                (NetInfo::Mobile, "mobile_46000".to_owned())
            );
        })
    }

    #[test]
    fn a_network_that_is_neither_wifi_nor_mobile_keeps_the_default_label() {
        isolated(|| {
            // `default:` of the C++'s switch, which leaves `netInfo` as it was
            // set at the top of the function
            set_net_info_impl(NetInfo::NoNet.as_i32());
            assert_eq!(
                net_label_impl(),
                (NetInfo::NoNet, DEFAULT_NET_LABEL.to_owned())
            );

            set_net_info_impl(NetInfo::OtherNet.as_i32());
            assert_eq!(
                net_label_impl(),
                (NetInfo::OtherNet, DEFAULT_NET_LABEL.to_owned())
            );
        })
    }

    #[test]
    fn a_network_the_platform_never_answered_is_labelled_after_it_anyway() {
        isolated(|| {
            // a wifi with no `WifiInfo` is the prefix and nothing behind it,
            // the way a default-constructed one reads in the C++
            set_net_info_impl(NetInfo::Wifi.as_i32());
            set_wifi_info_impl(None);
            assert_eq!(net_label_impl(), (NetInfo::Wifi, "wifi_".to_owned()));

            set_net_info_impl(NetInfo::Mobile.as_i32());
            set_sim_info_impl(None);
            assert_eq!(net_label_impl(), (NetInfo::Mobile, "mobile_".to_owned()));
        })
    }

    #[test]
    fn the_label_is_built_out_of_the_answers_and_not_asked_of_java() {
        isolated(|| {
            let questions = asking(Answer::Nothing);
            set_net_info_impl(NetInfo::Wifi.as_i32());
            set_wifi_info_impl(Some(WifiInfo {
                ssid: "MarsWifi".to_owned(),
                bssid: String::new(),
            }));

            assert_eq!(
                net_label_impl(),
                (NetInfo::Wifi, "wifi_MarsWifi".to_owned())
            );
            assert!(asked(&questions).is_empty());
        })
    }

    #[test]
    fn a_platform_that_answered_nothing_is_labelled_default() {
        isolated(|| {
            // what `mars-stn` gets when there is no JVM: `kNoNet`, and the
            // label of a network that is neither
            assert_eq!(
                net_label_impl(),
                (NetInfo::NoNet, DEFAULT_NET_LABEL.to_owned())
            );
        })
    }

    #[test]
    fn an_answer_read_as_another_question_is_the_platforms_default() {
        let wifi = Answer::Wifi(Some(WifiInfo::default()));
        assert_eq!(wifi.net_info(), NetInfo::NoNet);
        assert_eq!(wifi.statistics_net_type(), NetType::Non);
        assert_eq!(wifi.proxy(), (-1, String::new()));
        assert_eq!(wifi.sim(), None);
        assert_eq!(wifi.apn(), None);
        assert_eq!(wifi.radio_access_network(), RADIO_ACCESS_NETWORK_UNKNOWN);
        assert_eq!(wifi.signal(), 0);
        assert!(!wifi.connected());

        // and nothing is every default at once
        assert_eq!(Answer::Nothing.net_info(), NetInfo::NoNet);
        assert_eq!(Answer::Nothing.statistics_net_type(), NetType::Non);
        assert_eq!(Answer::Nothing.proxy(), (-1, String::new()));
        assert_eq!(Answer::Nothing.wifi(), None);
        assert_eq!(Answer::Nothing.sim(), None);
        assert_eq!(Answer::Nothing.apn(), None);
        assert_eq!(
            Answer::Nothing.radio_access_network(),
            RADIO_ACCESS_NETWORK_UNKNOWN
        );
        assert_eq!(Answer::Nothing.signal(), 0);
        assert!(!Answer::Nothing.connected());
        assert_eq!(Answer::default(), Answer::Nothing);
    }

    #[test]
    fn without_a_vm_java_answers_nothing() {
        isolated(|| {
            // `Ask::jvm` is what the port starts with: a test has no VM, so the
            // platform is asked and answers nothing
            set_ask(Ask::jvm());

            assert_eq!(net_info_impl(), NetInfo::NoNet);
            assert!(!is_network_connected_impl());
            assert_eq!(signal_impl(true), 0);
        })
    }

    #[test]
    fn an_answerer_that_panicked_does_not_lock_the_port_out() {
        isolated(|| {
            set_ask(Ask::new(|_| panic!("the answerer panicked")));
            let panicked =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(is_network_connected_impl));
            assert!(panicked.is_err());

            // the ask is poisoned, and handing the port another one still works
            set_ask(Ask::new(|_| Answer::Nothing));
            assert!(!is_network_connected_impl());
        })
    }

    #[test]
    fn a_platform_a_panic_poisoned_keeps_answering() {
        isolated(|| {
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                with_state(|_| panic!("the platform panicked"))
            }));
            assert!(panicked.is_err());

            set_net_info_impl(1);
            assert_eq!(net_info_impl(), NetInfo::Wifi);
        })
    }

    #[test]
    fn the_ask_is_debug_without_the_answerer_it_holds() {
        let ask = Ask::new(|_| Answer::Nothing);
        assert!(format!("{ask:?}").contains("Ask"));
    }

    #[test]
    fn the_net_info_numbers_are_the_java_constants() {
        assert_eq!(NetInfo::of(-1), NetInfo::NoNet);
        assert_eq!(NetInfo::of(1), NetInfo::Wifi);
        assert_eq!(NetInfo::of(2), NetInfo::Mobile);
        // the `default` branch of the Java switch
        assert_eq!(NetInfo::of(3), NetInfo::OtherNet);
        assert_eq!(NetInfo::of(99), NetInfo::OtherNet);
        assert_eq!(NetInfo::of(0), NetInfo::OtherNet);

        assert_eq!(NetInfo::NoNet.as_i32(), -1);
        assert_eq!(NetInfo::Wifi.as_i32(), 1);
        assert_eq!(NetInfo::Mobile.as_i32(), 2);
        assert_eq!(NetInfo::OtherNet.as_i32(), 3);
    }

    #[test]
    fn the_net_type_numbers_are_the_java_constants() {
        for (number, expected) in [
            (0, NetType::NotWifi),
            (1, NetType::Wifi),
            (2, NetType::Wap),
            (3, NetType::G2),
            (4, NetType::G3),
            (5, NetType::G4),
            (6, NetType::Unknown),
        ] {
            assert_eq!(NetType::of(number), expected, "net type {number}");
            assert_eq!(expected.as_i32(), number);
        }
        // no context, and every number the Java does not name
        assert_eq!(NetType::of(-1), NetType::Non);
        assert_eq!(NetType::of(7), NetType::Non);
    }

    #[test]
    fn the_alarm_and_the_wake_lock_are_the_platform_services() {
        isolated(|| {
            // `C2Java.startAlarm` and `stopAlarm` are the alarm module
            assert!(start_alarm_impl(0, 42, 100, 1_000));
            assert!(!start_alarm_impl(0, 42, 100, 1_000), "already waiting");
            assert!(stop_alarm_impl(42));
            assert!(!stop_alarm_impl(42));

            // `C2Java.wakeupLock_new` hands back a lock handle
            let handle = wakeup_lock_new_impl();
            assert_ne!(handle, 0);
        })
    }
}
