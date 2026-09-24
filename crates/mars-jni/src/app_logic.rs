//! `io.github.marsrs.app.AppLogic` — what the app is, as the network layer
//! needs it.
//!
//! `com/tencent/mars/app/AppLogic.java` declares no `native` method: it is the
//! class the C++ asks, through `platform_comm.cc`, for the app's directory, its
//! account, its client version and its device. This module is that answer,
//! kept in Rust — the JNI bridge fills it in from Java, and the port reads it.
//!
//! Everything the JVM touches lives in [`crate::jni_bridge`]; what is here is
//! plain Rust and is covered by `cargo test`.

use std::sync::{Mutex, OnceLock};

/// `AppLogic.AccountInfo` — `uin` and `userName`, both empty by default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountInfo {
    /// The account id; `0` means nobody is logged in.
    pub uin: i64,
    /// The user name.
    pub user_name: String,
}

impl AccountInfo {
    /// `AccountInfo(uin, userName)`.
    pub fn new(uin: i64, user_name: impl Into<String>) -> Self {
        Self {
            uin,
            user_name: user_name.into(),
        }
    }

    /// Whether the account looks logged in, which is how STN lowers its
    /// connection rate for a user who is not.
    pub fn is_logged_in(&self) -> bool {
        self.uin != 0
    }
}

/// `AppLogic.DeviceInfo` — `devicename` and `devicetype`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The device name.
    pub devicename: String,
    /// The device type, which sorts the device into a statistics bucket.
    pub devicetype: String,
}

impl DeviceInfo {
    /// `DeviceInfo(devicename, devicetype)`.
    pub fn new(devicename: impl Into<String>, devicetype: impl Into<String>) -> Self {
        Self {
            devicename: devicename.into(),
            devicetype: devicetype.into(),
        }
    }
}

/// What the app answered.
#[derive(Debug, Clone, Default)]
struct AppState {
    account: AccountInfo,
    device: DeviceInfo,
    client_version: i32,
    app_file_path: String,
}

fn state() -> &'static Mutex<AppState> {
    static STATE: OnceLock<Mutex<AppState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(AppState::default()))
}

fn with_state<R>(f: impl FnOnce(&mut AppState) -> R) -> R {
    let mut state = state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut state)
}

/// `AppLogic.getAccountInfo()` — what the JNI read from Java.
pub fn set_account_info_impl(uin: i64, user_name: &str) {
    with_state(|state| state.account = AccountInfo::new(uin, user_name))
}

/// The account the app is logged in as.
pub fn account_info_impl() -> AccountInfo {
    with_state(|state| state.account.clone())
}

/// `AppLogic.getDeviceType()`.
pub fn set_device_info_impl(devicename: &str, devicetype: &str) {
    with_state(|state| state.device = DeviceInfo::new(devicename, devicetype))
}

/// The device the app runs on.
pub fn device_info_impl() -> DeviceInfo {
    with_state(|state| state.device.clone())
}

/// `AppLogic.getClientVersion()` — the version that tells one client's stored
/// network policies from another's.
pub fn set_client_version_impl(version: i32) {
    with_state(|state| state.client_version = version)
}

/// The client version.
pub fn client_version_impl() -> i32 {
    with_state(|state| state.client_version)
}

/// `AppLogic.getAppFilePath()` — the directory STN stores its configuration in.
pub fn set_app_file_path_impl(path: &str) {
    with_state(|state| state.app_file_path = path.to_owned())
}

/// The app directory; empty when the app never answered.
pub fn app_file_path_impl() -> String {
    with_state(|state| state.app_file_path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated<R>(f: impl FnOnce() -> R) -> R {
        let guard = crate::test_lock();
        let result = f();
        with_state(|state| *state = AppState::default());
        drop(guard);
        result
    }

    #[test]
    fn a_fresh_app_has_answered_nothing() {
        isolated(|| {
            assert_eq!(account_info_impl(), AccountInfo::default());
            assert_eq!(device_info_impl(), DeviceInfo::default());
            assert_eq!(client_version_impl(), 0);
            assert!(app_file_path_impl().is_empty());
        })
    }

    #[test]
    fn what_java_answered_is_what_the_port_reads() {
        isolated(|| {
            set_account_info_impl(100_001, "alice");
            set_device_info_impl("Pixel", "phone");
            set_client_version_impl(0x0102_0304);
            set_app_file_path_impl("/data/data/io.github.marsrs/app_mars");

            assert_eq!(account_info_impl(), AccountInfo::new(100_001, "alice"));
            assert_eq!(device_info_impl(), DeviceInfo::new("Pixel", "phone"));
            assert_eq!(client_version_impl(), 0x0102_0304);
            assert_eq!(app_file_path_impl(), "/data/data/io.github.marsrs/app_mars");
        })
    }

    #[test]
    fn only_a_nonzero_uin_is_a_logged_in_account() {
        let anonymous = AccountInfo::default();
        assert!(!anonymous.is_logged_in());
        assert!(AccountInfo::new(1, "alice").is_logged_in());
        // a user name without a uin is not a session
        assert!(!AccountInfo::new(0, "alice").is_logged_in());
    }
}
