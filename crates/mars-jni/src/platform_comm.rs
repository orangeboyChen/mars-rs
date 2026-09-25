//! `io.github.marsrs.comm.PlatformComm$C2Java` — what the platform is, as
//! `com/tencent/mars/comm/PlatformComm.java` answers it.
//!
//! `PlatformComm.java` declares no `native` method: `C2Java` is the set of
//! statics the C++ calls through `platform_comm.cc` to ask Android about the
//! network, the proxy, the SIM card and the signal. This module is that answer
//! in Rust — the JNI bridge fills it in from Java, and the port reads it — and
//! the two platform services it owns outright, the alarm and the wake lock,
//! which delegate to [`crate::alarm`] and [`crate::wakerlock`].
//!
//! Everything the JVM touches lives in [`crate::jni_bridge`]; what is here is
//! plain Rust and is covered by `cargo test`.

use std::sync::{Mutex, OnceLock};

use crate::alarm::{alarm_start_impl, alarm_stop_impl};
use crate::wakerlock::{wakerlock_new_impl, WakerLockHandle};

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

/// What the platform answered.
#[derive(Debug, Clone)]
struct PlatformState {
    net_info: NetInfo,
    statistics_net_type: NetType,
    proxy_host: String,
    proxy_port: i32,
    wifi: Option<WifiInfo>,
    sim: Option<SimInfo>,
    apn: Option<ApnInfo>,
    radio_access_network: i32,
    wifi_signal: i64,
    gsm_signal: i64,
    network_connected: bool,
}

impl Default for PlatformState {
    fn default() -> Self {
        Self {
            net_info: NetInfo::NoNet,
            statistics_net_type: NetType::Non,
            proxy_host: String::new(),
            // `getProxyInfo` answers -1 when the proxy is off
            proxy_port: -1,
            wifi: None,
            sim: None,
            apn: None,
            radio_access_network: RADIO_ACCESS_NETWORK_UNKNOWN,
            wifi_signal: 0,
            gsm_signal: 0,
            network_connected: false,
        }
    }
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

/// `C2Java.getNetInfo()` — what the JNI read from Java.
pub fn set_net_info_impl(value: i32) {
    with_state(|state| state.net_info = NetInfo::of(value))
}

/// The network the device is on.
pub fn net_info_impl() -> NetInfo {
    with_state(|state| state.net_info)
}

/// `C2Java.getStatisticsNetType()`.
pub fn set_statistics_net_type_impl(value: i32) {
    with_state(|state| state.statistics_net_type = NetType::of(value))
}

/// The network type as the statistics report wants it.
pub fn statistics_net_type_impl() -> NetType {
    with_state(|state| state.statistics_net_type)
}

/// `C2Java.getProxyInfo(StringBuffer)`.
pub fn set_proxy_info_impl(host: &str, port: i32) {
    with_state(|state| {
        state.proxy_host = host.to_owned();
        state.proxy_port = port;
    })
}

/// The proxy, `(port, host)`; a port of `-1` means there is none.
pub fn proxy_info_impl() -> (i32, String) {
    with_state(|state| (state.proxy_port, state.proxy_host.clone()))
}

/// `C2Java.getCurWifiInfo()` — `None` when the device is not on wifi.
pub fn set_wifi_info_impl(wifi: Option<WifiInfo>) {
    with_state(|state| state.wifi = wifi)
}

/// The wifi the device is on.
pub fn wifi_info_impl() -> Option<WifiInfo> {
    with_state(|state| state.wifi.clone())
}

/// `C2Java.getCurSIMInfo()` — `None` when there is no SIM.
pub fn set_sim_info_impl(sim: Option<SimInfo>) {
    with_state(|state| state.sim = sim)
}

/// The SIM in the device.
pub fn sim_info_impl() -> Option<SimInfo> {
    with_state(|state| state.sim.clone())
}

/// `C2Java.getAPNInfo()` — `None` when there is no active network.
pub fn set_apn_info_impl(apn: Option<ApnInfo>) {
    with_state(|state| state.apn = apn)
}

/// The access point the device is on.
pub fn apn_info_impl() -> Option<ApnInfo> {
    with_state(|state| state.apn.clone())
}

/// `C2Java.getCurRadioAccessNetworkInfo()`.
pub fn set_radio_access_network_info_impl(value: i32) {
    with_state(|state| state.radio_access_network = value)
}

/// The radio access network.
pub fn radio_access_network_info_impl() -> i32 {
    with_state(|state| state.radio_access_network)
}

/// `C2Java.getSignal(isWifi)`.
pub fn set_signal_impl(is_wifi: bool, signal: i64) {
    with_state(|state| {
        if is_wifi {
            state.wifi_signal = signal;
        } else {
            state.gsm_signal = signal;
        }
    })
}

/// The signal strength; `0` when the platform never answered.
pub fn signal_impl(is_wifi: bool) -> i64 {
    with_state(|state| {
        if is_wifi {
            state.wifi_signal
        } else {
            state.gsm_signal
        }
    })
}

/// `C2Java.isNetworkConnected()`.
pub fn set_network_connected_impl(connected: bool) {
    with_state(|state| state.network_connected = connected)
}

/// Whether the device has a network at all.
pub fn is_network_connected_impl() -> bool {
    with_state(|state| state.network_connected)
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

    fn isolated<R>(f: impl FnOnce() -> R) -> R {
        let guard = crate::test_lock();
        with_state(|state| *state = PlatformState::default());
        let result = f();
        with_state(|state| *state = PlatformState::default());
        drop(guard);
        result
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
    fn what_java_answered_is_what_the_port_reads() {
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
