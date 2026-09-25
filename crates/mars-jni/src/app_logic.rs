//! `io.github.marsrs.app.AppLogic` — what the app is, as the network layer
//! needs it.
//!
//! `com/tencent/mars/app/AppLogic.java` declares no `native` method: it is the
//! class the C++ asks, through `mars/app/jni/com_tencent_mars_app_AppLogic_C2Java.cc`,
//! for the app's directory, its account, its client version and its device.
//! This module is that question and its answer: what a host set here is what
//! the port answers with, and what it did not set is asked of Java — one
//! [`Question`], one [`Answer`], which is what makes the four C2Java functions
//! one seam.
//!
//! Not ported: `GetProxyInfo`, which the C++ answers with an empty proxy of its
//! own rather than asking the app at all.
//!
//! Everything the JVM touches lives in [`crate::jni_bridge`]; what is here is
//! plain Rust and is covered by `cargo test`.

use std::sync::{Mutex, OnceLock};

/// One of the four questions `com_tencent_mars_app_AppLogic_C2Java.cc` asks.
///
/// `GetAppUserName` and `GetRecentUserName` are not among them: both read the
/// `userName` of [`Question::AccountInfo`], which is what the C++ does too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Question {
    /// `getAppFilePath()Ljava/lang/String;` — the directory STN stores its
    /// configuration in.
    AppFilePath,
    /// `getAccountInfo()Lcom/tencent/mars/app/AppLogic$AccountInfo;` — the
    /// account, which is how STN knows whether anybody is logged in and lowers
    /// its connection rate when nobody is.
    AccountInfo,
    /// `getClientVersion()I` — the version that tells one client's stored
    /// network policies from another's.
    ClientVersion,
    /// `getDeviceType()Lcom/tencent/mars/app/AppLogic$DeviceInfo;` — the device,
    /// which sorts the app into a statistics bucket.
    DeviceInfo,
}

/// What the app answered, in the terms the port asks in.
///
/// A question nobody answered — no VM to attach to, a call that could not be
/// made — is [`Answer::Nothing`], and every answer read out of it is what the
/// app's own defaults are: no directory, nobody logged in, no version, no
/// device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Answer {
    /// `getAppFilePath`.
    Path(String),
    /// `getAccountInfo`.
    Account(AccountInfo),
    /// `getClientVersion`.
    Version(i32),
    /// `getDeviceType`.
    Device(DeviceInfo),
    /// Java that was not asked at all.
    #[default]
    Nothing,
}

impl Answer {
    /// `getAppFilePath` — empty when the answer is not this one, which is the
    /// C++'s `""` for a Java call that returned `null`.
    pub fn path(&self) -> String {
        match self {
            Self::Path(path) => path.clone(),
            _ => String::new(),
        }
    }

    /// `getAccountInfo` — nobody when the answer is not this one.
    pub fn account(&self) -> AccountInfo {
        match self {
            Self::Account(account) => account.clone(),
            _ => AccountInfo::default(),
        }
    }

    /// `getClientVersion` — `0` when the answer is not this one.
    pub fn version(&self) -> i32 {
        match self {
            Self::Version(version) => *version,
            _ => 0,
        }
    }

    /// `getDeviceType` — nothing when the answer is not this one.
    pub fn device(&self) -> DeviceInfo {
        match self {
            Self::Device(device) => device.clone(),
            _ => DeviceInfo::default(),
        }
    }
}

/// What the app is asked with: [`Ask::jvm`] asks Java, and a host hands over
/// its own with [`set_ask`] — which is what the C++'s `NATIVE_CALLBACK` build
/// is.
pub struct Ask {
    ask: Box<dyn FnMut(Question) -> Answer + Send>,
}

impl Ask {
    /// The app whose answers come from Java.
    pub fn jvm() -> Self {
        Self::new(crate::jni_bridge::ask_app_logic)
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

/// `SetAppLogicNativeCallback` — the app is asked through this instead of
/// through the JVM.
pub fn set_ask(ask: Ask) {
    let mut asked = asker().lock().unwrap_or_else(poisoned);
    *asked = ask;
}

fn ask_app(question: Question) -> Answer {
    asker().lock().unwrap_or_else(poisoned).ask(question)
}

fn poisoned<T>(error: std::sync::PoisonError<T>) -> T {
    error.into_inner()
}

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

/// What the app answered in Rust: a host that hands these over is what the
/// C++'s `NATIVE_CALLBACK` build is, and what is set here is what the port
/// answers with instead of asking Java.
#[derive(Debug, Clone, Default)]
struct AppState {
    account: Option<AccountInfo>,
    device: Option<DeviceInfo>,
    client_version: Option<i32>,
    app_file_path: Option<String>,
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

/// `AppLogic.getAccountInfo()` — what a host answered, which is what the port
/// answers with from here on.
pub fn set_account_info_impl(uin: i64, user_name: &str) {
    with_state(|state| state.account = Some(AccountInfo::new(uin, user_name)))
}

/// The account the app is logged in as: what a host set, or what Java answered.
///
/// Java is asked every time, which is what makes a login the port was not told
/// about visible — the C++ asked on every call too.
pub fn account_info_impl() -> AccountInfo {
    match with_state(|state| state.account.clone()) {
        Some(account) => account,
        None => ask_app(Question::AccountInfo).account(),
    }
}

/// The `userName` of [`account_info_impl`] — the C++'s `GetAppUserName`, which
/// is what `GetRecentUserName` answers with as well.
pub fn user_name_impl() -> String {
    account_info_impl().user_name
}

/// `AppLogic.getDeviceType()`.
pub fn set_device_info_impl(devicename: &str, devicetype: &str) {
    with_state(|state| state.device = Some(DeviceInfo::new(devicename, devicetype)))
}

/// The device the app runs on: what a host set, or what Java answered.
pub fn device_info_impl() -> DeviceInfo {
    match with_state(|state| state.device.clone()) {
        Some(device) => device,
        None => ask_app(Question::DeviceInfo).device(),
    }
}

/// `AppLogic.getClientVersion()`.
pub fn set_client_version_impl(version: i32) {
    with_state(|state| state.client_version = Some(version))
}

/// The client version: what a host set, or what Java answered.
pub fn client_version_impl() -> i32 {
    match with_state(|state| state.client_version) {
        Some(version) => version,
        None => ask_app(Question::ClientVersion).version(),
    }
}

/// `AppLogic.getAppFilePath()`.
pub fn set_app_file_path_impl(path: &str) {
    with_state(|state| state.app_file_path = Some(path.to_owned()))
}

/// The app directory: what a host set, or what Java answered; empty when
/// neither did.
pub fn app_file_path_impl() -> String {
    match with_state(|state| state.app_file_path.clone()) {
        Some(path) => path,
        None => ask_app(Question::AppFilePath).path(),
    }
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
        with_state(|state| *state = AppState::default());
        set_ask(Ask::new(|_| Answer::Nothing));
    }

    /// An app that writes down what it was asked and answers `answer` — i.e.
    /// what the JVM hook would have been handed.
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
    fn a_fresh_app_has_answered_nothing() {
        isolated(|| {
            assert_eq!(account_info_impl(), AccountInfo::default());
            assert_eq!(device_info_impl(), DeviceInfo::default());
            assert_eq!(client_version_impl(), 0);
            assert!(app_file_path_impl().is_empty());
        })
    }

    #[test]
    fn what_a_host_set_is_what_the_port_reads() {
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

    #[test]
    fn what_java_answered_is_what_the_port_reads() {
        isolated(|| {
            // one answer per question, which is what four C2Java calls are
            set_ask(Ask::new(|question| match question {
                Question::AppFilePath => {
                    Answer::Path("/data/data/io.github.marsrs/app_mars".to_owned())
                }
                Question::AccountInfo => Answer::Account(AccountInfo::new(100_001, "alice")),
                Question::ClientVersion => Answer::Version(0x0102_0304),
                Question::DeviceInfo => Answer::Device(DeviceInfo::new("Pixel", "phone")),
            }));

            assert_eq!(app_file_path_impl(), "/data/data/io.github.marsrs/app_mars");
            assert_eq!(account_info_impl(), AccountInfo::new(100_001, "alice"));
            assert_eq!(client_version_impl(), 0x0102_0304);
            assert_eq!(device_info_impl(), DeviceInfo::new("Pixel", "phone"));
        })
    }

    #[test]
    fn an_app_a_host_said_nothing_about_is_asked() {
        isolated(|| {
            let questions = asking(Answer::Nothing);

            assert!(account_info_impl().user_name.is_empty());
            let _ = device_info_impl();
            assert_eq!(client_version_impl(), 0);
            assert!(app_file_path_impl().is_empty());

            // one question each, in the order the port asked them
            assert_eq!(
                asked(&questions),
                vec![
                    Question::AccountInfo,
                    Question::DeviceInfo,
                    Question::ClientVersion,
                    Question::AppFilePath,
                ]
            );
        })
    }

    #[test]
    fn what_a_host_answered_is_what_java_is_never_asked() {
        isolated(|| {
            let questions = asking(Answer::Account(AccountInfo::new(9, "not-alice")));
            set_account_info_impl(100_001, "alice");
            set_device_info_impl("Pixel", "phone");
            set_client_version_impl(0x0102_0304);
            set_app_file_path_impl("/data/data/io.github.marsrs/app_mars");

            assert_eq!(account_info_impl(), AccountInfo::new(100_001, "alice"));
            assert_eq!(device_info_impl(), DeviceInfo::new("Pixel", "phone"));
            assert_eq!(client_version_impl(), 0x0102_0304);
            assert_eq!(app_file_path_impl(), "/data/data/io.github.marsrs/app_mars");

            // a host that answered is the C++'s `NATIVE_CALLBACK` build
            assert!(asked(&questions).is_empty());
        })
    }

    #[test]
    fn java_is_asked_on_every_call() {
        isolated(|| {
            let questions = asking(Answer::Account(AccountInfo::new(100_002, "bob")));

            // a login the port was not told about is visible the next time
            assert_eq!(user_name_impl(), "bob");
            assert_eq!(user_name_impl(), "bob");

            assert_eq!(
                asked(&questions),
                vec![Question::AccountInfo, Question::AccountInfo]
            );
        })
    }

    #[test]
    fn the_user_name_is_the_accounts() {
        isolated(|| {
            set_account_info_impl(100_001, "alice");
            // `GetAppUserName` and `GetRecentUserName` read the same field
            assert_eq!(user_name_impl(), "alice");
            assert_eq!(account_info_impl().user_name, "alice");
        })
    }

    #[test]
    fn an_answer_read_as_another_question_is_the_apps_default() {
        let path = Answer::Path("/data".to_owned());
        assert_eq!(path.account(), AccountInfo::default());
        assert_eq!(path.device(), DeviceInfo::default());
        assert_eq!(path.version(), 0);

        assert!(Answer::Version(3).path().is_empty());
        assert_eq!(Answer::Account(AccountInfo::new(1, "alice")).version(), 0);
        assert!(Answer::Device(DeviceInfo::new("Pixel", "phone"))
            .path()
            .is_empty());

        // and nothing is every default at once
        assert!(Answer::Nothing.path().is_empty());
        assert_eq!(Answer::Nothing.account(), AccountInfo::default());
        assert_eq!(Answer::Nothing.device(), DeviceInfo::default());
        assert_eq!(Answer::Nothing.version(), 0);
        assert_eq!(Answer::default(), Answer::Nothing);
    }

    #[test]
    fn without_a_vm_java_answers_nothing() {
        isolated(|| {
            // `Ask::jvm` is what the port starts with: a test has no VM, so the
            // app is asked and answers nothing
            set_ask(Ask::jvm());

            assert!(app_file_path_impl().is_empty());
            assert_eq!(client_version_impl(), 0);
            assert_eq!(account_info_impl(), AccountInfo::default());
            assert_eq!(device_info_impl(), DeviceInfo::default());
            assert!(user_name_impl().is_empty());
        })
    }

    #[test]
    fn an_answerer_that_panicked_does_not_lock_the_port_out() {
        isolated(|| {
            set_ask(Ask::new(|_| panic!("the answerer panicked")));
            let panicked =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(app_file_path_impl));
            assert!(panicked.is_err());

            // the ask is poisoned, and handing the port another one still works
            set_ask(Ask::new(|_| Answer::Nothing));
            assert!(app_file_path_impl().is_empty());
        })
    }

    #[test]
    fn an_app_a_panic_poisoned_keeps_answering() {
        isolated(|| {
            // a host that panicked while changing the app must not lock the
            // port out of it
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                with_state(|_| panic!("the app panicked"))
            }));
            assert!(panicked.is_err());

            set_account_info_impl(100_001, "alice");
            assert_eq!(user_name_impl(), "alice");
        })
    }

    #[test]
    fn the_ask_is_debug_without_the_answerer_it_holds() {
        let ask = Ask::new(|_| Answer::Nothing);
        assert!(format!("{ask:?}").contains("Ask"));
    }
}
