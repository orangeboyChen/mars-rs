//! `mars/sdt/constants.h` — the tunables and the literals of the diagnosis.

/// `NET_CHECK_BASIC`: ping and DNS.
pub const NET_CHECK_BASIC: i32 = 1;
/// `NET_CHECK_LONG`: TCP against the long-link hosts.
pub const NET_CHECK_LONG: i32 = 1 << 1;
/// `NET_CHECK_SHORT`: HTTP against the net-check CGI.
pub const NET_CHECK_SHORT: i32 = 1 << 2;

/// `ERR_SEQ` — the sequence number of a result that carries no sequence.
pub const ERR_SEQ: i32 = -1;

/// `MODE_BASIC(mode)`.
pub const fn mode_basic(mode: i32) -> bool {
    mode & NET_CHECK_BASIC != 0
}

/// `MODE_LONG(mode)`.
pub const fn mode_long(mode: i32) -> bool {
    mode & NET_CHECK_LONG != 0
}

/// `MODE_SHORT(mode)`.
pub const fn mode_short(mode: i32) -> bool {
    mode & NET_CHECK_SHORT != 0
}

/// `DEFAULT_HTTP_HOST`.
pub const DEFAULT_HTTP_HOST: &str = "www.qq.com";
/// `DEFAULT_PING_HOST`.
pub const DEFAULT_PING_HOST: &str = "www.qq.com";
/// `DUMMY_HOST` — what a check that has nothing to report reports against.
pub const DUMMY_HOST: &str = "DUMMY HOST";

/// `NET_CHECK_TAG`.
pub const NET_CHECK_TAG: &str = "NET_CHECK";
/// `CHECK_SUC`.
pub const CHECK_SUC: &str = "check success";
/// `CHECK_FAIL`.
pub const CHECK_FAIL: &str = "check failed";

/// `NoNetType`.
pub const NO_NET_TYPE: &str = "NoNet";
/// `WifiType`.
pub const WIFI_TYPE: &str = "Wifi Net";
/// `MobileType`.
pub const MOBILE_TYPE: &str = "Mobile Net";
/// `OtherNetType`.
pub const OTHER_NET_TYPE: &str = "Other Net";

/// `HTTP_DEFAULT_TIMEOUT`, in milliseconds.
pub const HTTP_DEFAULT_TIMEOUT: u32 = 5 * 1000;
/// `HTTP_DUMMY_RECV_DATA_SIZE` — how much of the HTTP answer is read.
pub const HTTP_DUMMY_RECV_DATA_SIZE: usize = 1;

/// `DEFAULT_PING_TIMEOUT`, in seconds.
pub const DEFAULT_PING_TIMEOUT: u32 = 4;
/// `DEFAULT_PING_COUNT` — how many pings one ping check sends.
pub const DEFAULT_PING_COUNT: u32 = 2;
/// `DEFAULT_PING_INTERVAL`, in seconds.
pub const DEFAULT_PING_INTERVAL: u32 = 1;

/// `DEFAULT_TCP_CONN_TIMEOUT`, in milliseconds.
pub const DEFAULT_TCP_CONN_TIMEOUT: u32 = 5000;
/// `DEFAULT_TCP_RECV_TIMEOUT`, in milliseconds.
pub const DEFAULT_TCP_RECV_TIMEOUT: u32 = 5 * 1000;

/// `DEFAULT_DNS_TIMEOUT`, in milliseconds.
pub const DEFAULT_DNS_TIMEOUT: u32 = 3 * 1000;

/// `UNUSE_TIMEOUT` — the timeout of a check that has none, i.e. `INT_MAX`.
pub const UNUSE_TIMEOUT: u32 = i32::MAX as u32;

/// `USER_AGENT`, which `constants.h` picks per platform.
#[cfg(target_os = "android")]
pub const USER_AGENT: &str =
    "Mozilla/5.0  (Linux; Android 4.1.1; Nexus 7 Build/JRO03S) AppleWebKit/535.19 (KHTML,  like Gecko) Chrome/18.0.1025.166 Safari/535.19";

/// `USER_AGENT`, which `constants.h` picks per platform.
#[cfg(target_vendor = "apple")]
pub const USER_AGENT: &str =
    "Mozilla/5.0  (iPhone; CPU iPhone OS 6_0 like Mac OS X) AppleWebKit/536.26 (KHTML,  like Gecko) Version/6.0 Mobile/10A403 Safari/8536.25";

/// `USER_AGENT`, which `constants.h` picks per platform.
#[cfg(not(any(target_os = "android", target_vendor = "apple")))]
pub const USER_AGENT: &str = "Mozilla/5.0 (compatible; MSIE9.0 Windows NT 6.1; WOW64; Trident/5.0)";
