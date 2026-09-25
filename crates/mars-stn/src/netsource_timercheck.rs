//! `mars/stn/src/netsource_timercheck.cc` — the check that gets the long link
//! off a backup ip.
//!
//! While the app is active the C++ posts a check every [`TIME_CHECK_PERIOD`].
//! Each one asks three questions before it does anything: is the long link on a
//! backup ip ([`IpSourceType::Backup`]), is no check already running, and did
//! fewer than [`MAX_SPEED_TEST_COUNT`] of them happen in the last
//! [`INTERVAL_TIME`]? Then it resolves the host the link is on, picks one ip
//! and one port out of what dns and `NetSource` give it at random, and asks
//! whether that pair can be reached. A pair that can is one whose ban is
//! lifted, and the app is told — which is what makes the long link drop and
//! connect again on a better ip.
//!
//! What the C++ does with a socket and a thread is two callbacks here: the
//! speed test itself ([`SpeedTest`], the `LongLinkSpeedTestItem` and the
//! `SocketSelect` loop around it, whose [`TIMEOUT`] the host honours inside it)
//! and the thread, which is a `testing` flag the port sets while the test runs
//! because nothing here runs concurrently. The periodic post is a due time
//! ([`NetSourceTimerCheck::period_due`]) the way the rest of the crate models
//! one.
//!
//! The C++ starts the check from its constructor when the app is active; the
//! port cannot, because the host hands the callbacks in afterwards, so
//! [`NetSourceTimerCheck::on_active_changed_at`] is what starts it.

use crate::simple_ipport_sort::IpSourceType;
use crate::Random;
use mars_comm::frequency_limit::FrequencyLimit;
use mars_comm::tickcount::gettickcount;

/// `kTimeCheckPeriod` — how often the check is posted, in milliseconds.
pub const TIME_CHECK_PERIOD: u64 = 150 * 1000;
/// `kTimeout` — what the C++ hands `SocketSelect::Select`, in milliseconds:
/// the speed test is a callback here, and this is the timeout the host is
/// expected to give it.
pub const TIMEOUT: u64 = 10 * 1000;
/// `kMaxSpeedTestCount` — the count the frequency limit is built with. Its
/// `Check()` passes while `touch_times.size() <= count`, so it is one more than
/// this many tests per [`INTERVAL_TIME`].
pub const MAX_SPEED_TEST_COUNT: usize = 30;
/// `kIntervalTime` — the window of the frequency limit, in milliseconds.
pub const INTERVAL_TIME: u64 = 60 * 60 * 1000;

/// `longlink_.Profile().ip_type`.
pub type IpType = dyn FnMut() -> IpSourceType + Send;
/// `longlink_.Profile().host` and `.ip` — one string each.
pub type Profile = dyn FnMut() -> String + Send;
/// `DnsUtil::GetNewDNS().GetHostByName` and `GetDNS().GetHostByName` — the
/// second is only asked when the first answered nothing.
pub type Dns = dyn FnMut(&str) -> Vec<String> + Send;
/// `NetSource::GetLonglinkPorts`.
pub type Ports = dyn FnMut() -> Vec<u16> + Send;
/// `NetSource::RemoveLongBanIP`.
pub type RemoveBanIp = dyn FnMut(&str) + Send;
/// `LongLinkSpeedTestItem` and the `SocketSelect` loop around it: `true` for
/// `kLongLinkSpeedTestSuc`, `false` for a fail or a timeout.
pub type SpeedTest = dyn FnMut(&str, u16) -> bool + Send;
/// `fun_time_check_suc_` — who is told a pair was reachable.
pub type OnTimeCheckSuc = dyn FnMut() + Send;

/// `NetSourceTimerCheck`.
pub struct NetSourceTimerCheck {
    /// `asyncpost_` — [`None`] for `MessageQueue::KNullPost`, which is a check
    /// that was never started or was stopped.
    period_due: Option<u64>,
    /// `thread_.isruning()`.
    testing: bool,
    /// `frequency_limit_`.
    frequency_limit: FrequencyLimit,

    ip_type: Option<Box<IpType>>,
    host: Option<Box<Profile>>,
    ip: Option<Box<Profile>>,
    new_dns: Option<Box<Dns>>,
    dns: Option<Box<Dns>>,
    longlink_ports: Option<Box<Ports>>,
    remove_long_ban_ip: Option<Box<RemoveBanIp>>,
    speed_test: Option<Box<SpeedTest>>,
    on_time_check_suc: Option<Box<OnTimeCheckSuc>>,
    random: Box<Random>,
}

impl Default for NetSourceTimerCheck {
    /// `NetSourceTimerCheck(…)`, at the tick count the clock gives.
    fn default() -> Self {
        Self::new()
    }
}

impl NetSourceTimerCheck {
    /// `NetSourceTimerCheck(…)`.
    pub fn new() -> Self {
        Self::new_at(gettickcount())
    }

    /// The same, with the reading the C++'s `gettickcount()` would hand out.
    pub fn new_at(_now: u64) -> Self {
        Self {
            period_due: None,
            testing: false,
            frequency_limit: FrequencyLimit::new(MAX_SPEED_TEST_COUNT, INTERVAL_TIME),
            ip_type: None,
            host: None,
            ip: None,
            new_dns: None,
            dns: None,
            longlink_ports: None,
            remove_long_ban_ip: None,
            speed_test: None,
            on_time_check_suc: None,
            random: Box::new(crate::xorshift(mars_comm::tickcount::gettickcount())),
        }
    }

    /// `longlink_.Profile().ip_type` — unset answers [`IpSourceType::Null`],
    /// which is a link the check has nothing to say about.
    pub fn set_ip_type(&mut self, ip_type: impl FnMut() -> IpSourceType + Send + 'static) {
        self.ip_type = Some(Box::new(ip_type));
    }

    /// `longlink_.Profile().host` — unset answers nothing, which is a host no
    /// dns can resolve.
    pub fn set_host(&mut self, host: impl FnMut() -> String + Send + 'static) {
        self.host = Some(Box::new(host));
    }

    /// `longlink_.Profile().ip`.
    pub fn set_ip(&mut self, ip: impl FnMut() -> String + Send + 'static) {
        self.ip = Some(Box::new(ip));
    }

    /// `DnsUtil::GetNewDNS()` — unset answers nothing.
    pub fn set_new_dns(&mut self, new_dns: impl FnMut(&str) -> Vec<String> + Send + 'static) {
        self.new_dns = Some(Box::new(new_dns));
    }

    /// `DnsUtil::GetDNS()` — the fallback, asked when the new dns answered
    /// nothing. Unset answers nothing.
    pub fn set_dns(&mut self, dns: impl FnMut(&str) -> Vec<String> + Send + 'static) {
        self.dns = Some(Box::new(dns));
    }

    /// `NetSource::GetLonglinkPorts` — unset answers nothing, which is what
    /// makes `__TryConnnect` give up.
    pub fn set_longlink_ports(&mut self, ports: impl FnMut() -> Vec<u16> + Send + 'static) {
        self.longlink_ports = Some(Box::new(ports));
    }

    /// `NetSource::RemoveLongBanIP` — what a test that succeeded does.
    pub fn set_remove_long_ban_ip(&mut self, remove: impl FnMut(&str) + Send + 'static) {
        self.remove_long_ban_ip = Some(Box::new(remove));
    }

    /// `LongLinkSpeedTestItem` and the select loop around it — unset answers
    /// `false`, a test that did not succeed.
    pub fn set_speed_test(&mut self, speed_test: impl FnMut(&str, u16) -> bool + Send + 'static) {
        self.speed_test = Some(Box::new(speed_test));
    }

    /// `fun_time_check_suc_` — what a test that succeeded asks for.
    pub fn set_on_time_check_suc(&mut self, on_suc: impl FnMut() + Send + 'static) {
        self.on_time_check_suc = Some(Box::new(on_suc));
    }

    /// `srand(gettickcount())` and `rand()` — which ip and which port the test
    /// is made on.
    pub fn set_random(&mut self, random: impl FnMut(usize) -> usize + Send + 'static) {
        self.random = Box::new(random);
    }

    /// When the check is posted next, [`None`] for one that is not running.
    pub fn period_due(&self) -> Option<u64> {
        self.period_due
    }

    /// `thread_.isruning()` — a test that is in flight.
    pub fn is_testing(&self) -> bool {
        self.testing
    }

    /// `__StartCheck()` — the C++ posts one and only one: a second call while
    /// the first is posted does nothing.
    pub fn start_check(&mut self) {
        self.start_check_at(gettickcount())
    }

    /// The same, with the reading handed in.
    pub fn start_check_at(&mut self, now: u64) {
        if self.period_due.is_some() {
            return;
        }
        self.period_due = Some(now.saturating_add(TIME_CHECK_PERIOD));
    }

    /// `__StopCheck()` — a check that is not running is not stopped: the C++
    /// returns before it clears `asyncpost_`, so the post keeps coming, and the
    /// port keeps that.
    pub fn stop_check(&mut self) {
        if self.period_due.is_none() || !self.testing {
            return;
        }
        self.testing = false;
        self.period_due = None;
    }

    /// `CancelConnect()` — break the pipe and let the test give up; the post
    /// itself stays.
    pub fn cancel_connect(&mut self) {
        if !self.testing {
            return;
        }
        self.testing = false;
    }

    /// `__Check()` — [`None`] when no test was made: the link is not on a
    /// backup ip, a test is already running, the frequency limit refused, or
    /// the check was never started. [`Some`] is what the test answered, and a
    /// `true` is what asks the app to reset the link.
    pub fn check(&mut self) -> Option<bool> {
        self.check_at(gettickcount())
    }

    /// The same, with the reading handed in. The post is periodic, so whatever
    /// it answered it is armed again from `now`.
    pub fn check_at(&mut self, now: u64) -> Option<bool> {
        self.period_due?;
        self.period_due = Some(now.saturating_add(TIME_CHECK_PERIOD));

        if self.ip_type() != IpSourceType::Backup {
            return None;
        }
        if self.testing {
            return None;
        }
        if !self.frequency_limit.check_at(now) {
            return None;
        }

        // what the C++ does on its own thread, and the port inside this call
        self.testing = true;
        let suc = self.try_connect_at(now);
        self.testing = false;

        if suc {
            if let Some(on_suc) = self.on_time_check_suc.as_mut() {
                on_suc();
            }
        }
        Some(suc)
    }

    /// `__Run` and `__TryConnnect` — resolve the host, pick an ip and a port,
    /// and test them. `true` is `kLongLinkSpeedTestSuc`, which is when the ban
    /// on the ip is lifted.
    pub fn try_connect_at(&mut self, _now: u64) -> bool {
        let host = self.host();
        let mut ips = self.new_dns(&host);
        if ips.is_empty() {
            ips = self.dns(&host);
        }
        if ips.is_empty() {
            return false;
        }

        // the ip the link is on already is not worth testing
        let linked_ip = self.ip();
        if ips.contains(&linked_ip) {
            return false;
        }

        let ports = self.longlink_ports();
        if ports.is_empty() {
            return false;
        }

        let ip_index = self.random(ips.len());
        let port_index = self.random(ports.len());
        let ip = ips[ip_index].clone();
        let port = ports[port_index];

        if !self.speed_test(&ip, port) {
            return false;
        }
        if let Some(remove) = self.remove_long_ban_ip.as_mut() {
            remove(&ip);
        }
        true
    }

    /// `__OnActiveChanged(_is_active)` — the app coming to the foreground
    /// starts the check and leaving it stops one.
    pub fn on_active_changed(&mut self, is_active: bool) {
        self.on_active_changed_at(gettickcount(), is_active)
    }

    /// The same, with the reading handed in.
    pub fn on_active_changed_at(&mut self, now: u64, is_active: bool) {
        if is_active {
            self.start_check_at(now);
        } else {
            self.stop_check();
        }
    }

    fn ip_type(&mut self) -> IpSourceType {
        match self.ip_type.as_mut() {
            Some(ip_type) => ip_type(),
            None => IpSourceType::Null,
        }
    }

    fn host(&mut self) -> String {
        match self.host.as_mut() {
            Some(host) => host(),
            None => String::new(),
        }
    }

    fn ip(&mut self) -> String {
        match self.ip.as_mut() {
            Some(ip) => ip(),
            None => String::new(),
        }
    }

    fn new_dns(&mut self, host: &str) -> Vec<String> {
        match self.new_dns.as_mut() {
            Some(new_dns) => new_dns(host),
            None => Vec::new(),
        }
    }

    fn dns(&mut self, host: &str) -> Vec<String> {
        match self.dns.as_mut() {
            Some(dns) => dns(host),
            None => Vec::new(),
        }
    }

    fn longlink_ports(&mut self) -> Vec<u16> {
        match self.longlink_ports.as_mut() {
            Some(ports) => ports(),
            None => Vec::new(),
        }
    }

    fn speed_test(&mut self, ip: &str, port: u16) -> bool {
        match self.speed_test.as_mut() {
            Some(speed_test) => speed_test(ip, port),
            None => false,
        }
    }

    fn random(&mut self, bound: usize) -> usize {
        (self.random)(bound)
    }
}

impl std::fmt::Debug for NetSourceTimerCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetSourceTimerCheck")
            .field("period_due", &self.period_due)
            .field("testing", &self.testing)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// What the check asked for: the pairs it tested, the bans it lifted, and
    /// how many times the app was told to reset the link.
    #[derive(Default)]
    struct Seen {
        tested: Vec<(String, u16)>,
        unbanned: Vec<String>,
        sucs: usize,
    }

    type Recorder = Arc<Mutex<Seen>>;
    /// What [`a_check`] hands back: the check and the record of what it asked
    /// for, in one name so the signature says what it is.
    type ACheck = (NetSourceTimerCheck, Recorder);

    /// A check whose long link is on a backup ip at `1.2.3.4`, whose new dns
    /// knows two other ips, and whose `NetSource` offers two ports.
    fn a_check() -> ACheck {
        let mut check = NetSourceTimerCheck::new_at(0);
        check.set_ip_type(|| IpSourceType::Backup);
        check.set_host(|| "long.example".to_string());
        check.set_ip(|| "1.2.3.4".to_string());
        check.set_new_dns(|_host| vec!["5.6.7.8".to_string(), "9.10.11.12".to_string()]);
        check.set_longlink_ports(|| vec![80, 443]);
        check.set_random(|_bound| 0);

        let seen: Recorder = Arc::new(Mutex::new(Seen::default()));
        let record = Arc::clone(&seen);
        check.set_speed_test(move |ip, port| {
            record
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .tested
                .push((ip.to_string(), port));
            true
        });
        let record = Arc::clone(&seen);
        check.set_remove_long_ban_ip(move |ip| {
            record
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .unbanned
                .push(ip.to_string());
        });
        let record = Arc::clone(&seen);
        check.set_on_time_check_suc(move || {
            record.lock().unwrap_or_else(|e| e.into_inner()).sucs += 1;
        });
        (check, seen)
    }

    #[test]
    fn a_check_that_was_never_started_never_runs() {
        let (mut check, seen) = a_check();
        assert_eq!(check.period_due(), None);
        assert_eq!(check.check_at(TIME_CHECK_PERIOD), None);
        assert!(seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tested
            .is_empty());

        // starting it posts one, and a second start does not move it
        check.start_check_at(0);
        assert_eq!(check.period_due(), Some(TIME_CHECK_PERIOD));
        check.start_check_at(1_000);
        assert_eq!(check.period_due(), Some(TIME_CHECK_PERIOD));
    }

    #[test]
    fn a_link_that_is_not_on_a_backup_ip_is_not_checked() {
        let (mut check, seen) = a_check();
        check.set_ip_type(|| IpSourceType::NewDns);
        check.start_check_at(0);
        assert_eq!(check.check_at(TIME_CHECK_PERIOD), None);
        assert!(seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tested
            .is_empty());
    }

    #[test]
    fn a_successful_test_lifts_the_ban_and_tells_the_app() {
        let (mut check, seen) = a_check();
        check.start_check_at(0);
        assert_eq!(check.check_at(TIME_CHECK_PERIOD), Some(true));
        let seen = seen.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            seen.tested,
            vec![("5.6.7.8".to_string(), 80)],
            "the first ip and the first port, the random pick pinned to 0"
        );
        assert_eq!(seen.unbanned, vec!["5.6.7.8".to_string()]);
        assert_eq!(seen.sucs, 1);
        drop(seen);
        assert!(!check.is_testing(), "the test is over");
        // ... and the post is armed again
        assert_eq!(check.period_due(), Some(2 * TIME_CHECK_PERIOD));
    }

    #[test]
    fn a_test_that_fails_tells_nobody() {
        let (mut check, seen) = a_check();
        check.set_speed_test(|_, _| false);
        check.start_check_at(0);
        assert_eq!(check.check_at(TIME_CHECK_PERIOD), Some(false));
        let seen = seen.lock().unwrap_or_else(|e| e.into_inner());
        assert!(seen.unbanned.is_empty());
        assert_eq!(seen.sucs, 0);
    }

    #[test]
    fn a_host_no_dns_knows_is_not_tested() {
        let (mut check, seen) = a_check();
        check.set_new_dns(|_| Vec::new());
        check.start_check_at(0);
        assert_eq!(check.check_at(TIME_CHECK_PERIOD), Some(false));
        assert!(seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tested
            .is_empty());

        // the fallback dns is what answers when the new one does not
        let (mut check, seen) = a_check();
        check.set_new_dns(|_| Vec::new());
        check.set_dns(|_| vec!["13.14.15.16".to_string()]);
        check.start_check_at(0);
        assert_eq!(check.check_at(TIME_CHECK_PERIOD), Some(true));
        assert_eq!(
            seen.lock().unwrap_or_else(|e| e.into_inner()).tested,
            vec![("13.14.15.16".to_string(), 80)]
        );
    }

    #[test]
    fn the_ip_the_link_is_on_already_is_not_tested() {
        let (mut check, seen) = a_check();
        // the second of the two ips the new dns knows is the one in use
        check.set_ip(|| "9.10.11.12".to_string());
        check.start_check_at(0);
        assert_eq!(check.check_at(TIME_CHECK_PERIOD), Some(false));
        assert!(
            seen.lock()
                .unwrap_or_else(|e| e.into_inner())
                .tested
                .is_empty(),
            "a dns answer that holds the ip in use rules the host out"
        );
    }

    #[test]
    fn without_ports_there_is_nothing_to_test() {
        let (mut check, seen) = a_check();
        check.set_longlink_ports(Vec::new);
        check.start_check_at(0);
        assert_eq!(check.check_at(TIME_CHECK_PERIOD), Some(false));
        assert!(seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tested
            .is_empty());
    }

    #[test]
    fn the_frequency_limit_is_what_stops_a_burst() {
        let (mut check, seen) = a_check();
        check.start_check_at(0);
        // `size() <= count`, so it is one more than `MAX_SPEED_TEST_COUNT` —
        // and the hour is what closes the window, not the `2.5` minutes the
        // post itself waits, which is only 24 tests an hour
        for index in 0..MAX_SPEED_TEST_COUNT + 1 {
            assert_eq!(check.check_at(1_000 * (index as u64 + 1)), Some(true));
        }
        assert_eq!(
            seen.lock().unwrap_or_else(|e| e.into_inner()).tested.len(),
            MAX_SPEED_TEST_COUNT + 1
        );
        assert_eq!(
            check.check_at(1_000 * (MAX_SPEED_TEST_COUNT as u64 + 2)),
            None,
            "the rest of the hour is refused"
        );
        // the oldest of them has to be more than an hour old, not an hour
        assert_eq!(
            check.check_at(INTERVAL_TIME + 2_000),
            Some(true),
            "an hour after the first one the window opens again"
        );
    }

    #[test]
    fn the_app_going_to_the_background_stops_a_running_check_and_only_that() {
        let (mut check, _seen) = a_check();
        check.on_active_changed_at(0, true);
        assert_eq!(check.period_due(), Some(TIME_CHECK_PERIOD));

        // nothing is in flight, so the C++ leaves the post alone
        check.on_active_changed_at(0, false);
        assert_eq!(check.period_due(), Some(TIME_CHECK_PERIOD));

        // a test in flight is what makes stopping it work
        check.testing = true;
        check.on_active_changed(false);
        assert_eq!(check.period_due(), None);
        assert!(!check.is_testing());
    }

    #[test]
    fn a_check_that_is_running_can_be_cancelled() {
        let (mut check, _seen) = a_check();
        check.start_check_at(0);
        // in flight, so the next post is turned away
        check.testing = true;
        assert_eq!(check.check_at(TIME_CHECK_PERIOD), None);
        check.cancel_connect();
        assert!(!check.is_testing());
        assert_eq!(
            check.check_at(2 * TIME_CHECK_PERIOD),
            Some(true),
            "and the post itself was never cancelled"
        );
    }

    #[test]
    fn without_a_host_nothing_is_asked_for() {
        let mut check = NetSourceTimerCheck::default();
        check.start_check();
        assert_eq!(check.check(), None, "no host: not on a backup ip");
        assert!(!check.try_connect_at(0));
        check.cancel_connect();
        check.stop_check();
        assert!(format!("{check:?}").contains("NetSourceTimerCheck"));
    }
}
