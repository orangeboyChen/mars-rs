//! `mars/stn/src/net_check_logic.cc` — when a run of failed tasks is a
//! network worth diagnosing.
//!
//! Every task that finishes pushes one bit into a window of the last thirty-two
//! of its link ([`NetCheckLogic::update_long_link_info`] and
//! [`NetCheckLogic::update_short_link_info`], `1` for a success). A link is
//! "broken" when fewer than [`CHECK_IF_BELOW_COUNT`] of its eight most recent
//! tasks succeeded, and the diagnosis is started when a link is broken now but
//! was fine in the eight before that, with two limits on top: the checks it
//! starts are at least [`MIN_CHECK_TIME_SPAN`] apart and that span grows by
//! [`CHECK_TIME_SPAN_INCREMENT_STEP`] every time, and
//! [`CommFrequencyLimit`](mars_comm::frequency_limit::FrequencyLimit) caps how
//! many of them get through per [`LIMIT_TIME_SPAN`]. That cap is the C++'s
//! `touch_times_.size() <= count_`, which is [`LIMIT_COUNT`] **plus one** — two
//! checks in the first hour, then one every [`LIMIT_TIME_SPAN`] — and the port
//! keeps it rather than "fixing" it; the growing span is what keeps wait apart,
//! not the limit.
//!
//! Everything the C++ reads from `NetSource`, `DnsUtil`, `StnManager` and
//! `sdt::SdtManager` is a callback here:
//!
//! * the hosts, ports and dns answers a check would run against
//!   ([`NetCheckLogic::set_long_link_hosts`] and friends), unset answering
//!   nothing at all, which is what makes `__StartNetCheck` give up;
//! * `sdt::SdtManager::StartActiveCheck` —
//!   [`NetCheckLogic::set_start_active_check`], whose arguments are the hosts
//!   of the two links and the mode the C++ always asks for,
//!   [`NET_CHECK_MODE`];
//! * the tick count is handed in by the `*_at` methods, and the ones without
//!   it ask [`gettickcount`];
//! * `last_netcheck_time_` is not written anywhere in the C++ — the span it
//!   measures is the age of the process — and the port keeps it that way, at
//!   `0`, rather than "fixing" it;
//! * `IS_LAST_N_BIT_ZERO`, the one macro the C++ defines and never uses, is
//!   not ported.

use mars_comm::frequency_limit::FrequencyLimit;
use mars_comm::tickcount::gettickcount;
use mars_sdt::sdt::{CheckIPPort, CheckIPPorts};
use mars_sdt::NET_CHECK_BASIC;
use mars_sdt::NET_CHECK_LONG;
use mars_sdt::NET_CHECK_SHORT;

/// `NET_CHECK_BASIC | NET_CHECK_LONG | NET_CHECK_SHORT` — the mode
/// `__StartNetCheck` always asks for.
pub const NET_CHECK_MODE: i32 = NET_CHECK_BASIC | NET_CHECK_LONG | NET_CHECK_SHORT;
/// `kLimitTimeSpan` — the window of the frequency limit, in milliseconds.
pub const LIMIT_TIME_SPAN: u64 = 60 * 60 * 1000;
/// `kLimitCount` — the count the frequency limit is built with. Its `Check()`
/// passes while `touch_times.size() <= count`, so the limit it gives is
/// `LIMIT_COUNT + 1` checks per [`LIMIT_TIME_SPAN`], which is the C++'s own
/// arithmetic and is what [`NetCheckLogic`] keeps.
pub const LIMIT_COUNT: usize = 1;
/// `kMinCheckTimeSpan` — how long apart two checks are, to start with.
pub const MIN_CHECK_TIME_SPAN: u64 = 5 * 60 * 1000;
/// `kCheckTimeSpanIncrementStep` — how much longer the wait gets every time.
pub const CHECK_TIME_SPAN_INCREMENT_STEP: u64 = 10 * 60 * 1000;
/// `kValidBitsFilter` — the window is the last thirty-two tasks.
pub const VALID_BITS_FILTER: u32 = 0xFFFF_FFFF;
/// `kMostRecentTaskStartN` — where the eight most recent tasks start, and how
/// many of them there are.
pub const MOST_RECENT_TASK_START_N: [u32; 2] = [24, 8];
/// `kSecondRecentTaskStartN` — the eight before those.
pub const SECOND_RECENT_TASK_START_N: [u32; 2] = [16, 8];
/// `kCheckifBelowCount` — fewer successes than this and the link is broken.
pub const CHECK_IF_BELOW_COUNT: u32 = 3;
/// `kCheckifAboveCount` — more than this and the link is fine.
pub const CHECK_IF_ABOVE_COUNT: u32 = 5;

/// `GetLongLinkHosts` / `GetLonglinkPorts` — what the long link is reachable
/// at. Unset answers nothing, which is what makes the check give up.
pub type Hosts = dyn FnMut() -> Vec<String> + Send;
/// `GetShortLinkPort`.
pub type ShortLinkPort = dyn FnMut() -> u16 + Send;
/// `RequestNetCheckShortLinkHosts` — the C++ hands the vector in for the app
/// to fill.
pub type RequestHosts = dyn FnMut(&mut Vec<String>) + Send;
/// `DnsUtil::GetNewDNS().GetHostByName` and `GetDNS().GetHostByName` — the
/// second is only asked when the first answered nothing.
pub type Dns = dyn FnMut(&str) -> Vec<String> + Send;
/// `sdt::SdtManager::StartActiveCheck`, without the timeout the C++ always
/// hands in as `UNUSE_TIMEOUT`.
pub type StartActiveCheck = dyn FnMut(&CheckIPPorts, &CheckIPPorts, i32) + Send;

/// `SET_BIT(IS_TRUE, RECORDS, VALID_BITS)` — the task goes in at the low end
/// and the oldest of the thirty-two falls off.
fn set_bit(is_true: bool, records: u32, valid_bits: u32) -> u32 {
    let mut records = (records << 1) & valid_bits;
    if is_true {
        records |= 1;
    } else {
        records &= !1;
    }
    records
}

/// `EXTRACT_N_BITS` — `n` bits out of `records`, starting at `start_pos`.
///
/// The C++ shifts the window it wants down into the low bits, and asks for
/// `32 - n` back, so a start position past the end gives an `n` of `0` and a
/// shift of thirty-two that C++ leaves undefined: the port answers `0`.
fn extract_n_bits(records: u32, start_pos: u32, n: u32) -> u32 {
    let mut n = n;
    let mut start_pos = start_pos;
    if !(1..=32).contains(&start_pos) {
        start_pos = 1;
    }
    if start_pos + n > 32 {
        n = 32 - start_pos;
    }
    if n == 0 {
        return 0;
    }
    (records << (start_pos - 1)) >> (32 - n)
}

/// `struct NetTaskStatusItem` — the window of one link and the reading of its
/// last failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NetTaskStatusItem {
    /// `NetTaskStatusItem() : records(0xFFFFFFFF)` — a link that has not been
    /// used yet is one that never failed.
    records: u32,
    last_failedtime: u64,
}

impl Default for NetTaskStatusItem {
    fn default() -> Self {
        Self {
            records: VALID_BITS_FILTER,
            last_failedtime: 0,
        }
    }
}

/// `NetCheckLogic`.
pub struct NetCheckLogic {
    longlink_taskstatus_item: NetTaskStatusItem,
    shortlink_taskstatus_item: NetTaskStatusItem,
    /// `last_netcheck_time_` — nothing in the C++ ever writes it, so the span
    /// `gettickspan(last_netcheck_time_)` measures is the age of the process.
    last_netcheck_time: u64,
    /// The `static int increment_steps` of `__ShouldNetCheck`: how many times
    /// the wait has grown.
    increment_steps: u32,
    frequency_limit: FrequencyLimit,

    long_link_hosts: Option<Box<Hosts>>,
    long_link_ports: Option<Box<dyn FnMut() -> Vec<u16> + Send>>,
    short_link_port: Option<Box<ShortLinkPort>>,
    request_short_link_hosts: Option<Box<RequestHosts>>,
    /// `GetNewDNS()`.
    new_dns: Option<Box<Dns>>,
    /// `GetDNS()` — the fallback.
    dns: Option<Box<Dns>>,
    start_active_check: Option<Box<StartActiveCheck>>,
    cancel_active_check: Option<Box<dyn FnMut() + Send>>,
}

impl Default for NetCheckLogic {
    /// `NetCheckLogic(…)`, at the tick count the clock gives.
    fn default() -> Self {
        Self::new()
    }
}

impl NetCheckLogic {
    /// `NetCheckLogic(…)`.
    pub fn new() -> Self {
        Self::new_at(gettickcount())
    }

    /// The same, with the reading the C++'s `gettickcount()` would hand out.
    pub fn new_at(_now: u64) -> Self {
        Self {
            longlink_taskstatus_item: NetTaskStatusItem::default(),
            shortlink_taskstatus_item: NetTaskStatusItem::default(),
            last_netcheck_time: 0,
            increment_steps: 0,
            frequency_limit: FrequencyLimit::new(LIMIT_COUNT, LIMIT_TIME_SPAN),
            long_link_hosts: None,
            long_link_ports: None,
            short_link_port: None,
            request_short_link_hosts: None,
            new_dns: None,
            dns: None,
            start_active_check: None,
            cancel_active_check: None,
        }
    }

    /// `net_source_->GetLongLinkHosts()`.
    pub fn set_long_link_hosts(&mut self, hosts: impl FnMut() -> Vec<String> + Send + 'static) {
        self.long_link_hosts = Some(Box::new(hosts));
    }

    /// `net_source_->GetLonglinkPorts(_portlist)`.
    pub fn set_long_link_ports(&mut self, ports: impl FnMut() -> Vec<u16> + Send + 'static) {
        self.long_link_ports = Some(Box::new(ports));
    }

    /// `net_source_->GetShortLinkPort()`.
    pub fn set_short_link_port(&mut self, port: impl FnMut() -> u16 + Send + 'static) {
        self.short_link_port = Some(Box::new(port));
    }

    /// `RequestNetCheckShortLinkHosts(_hostlist)` — the app fills the vector
    /// in.
    pub fn set_request_short_link_hosts(
        &mut self,
        request: impl FnMut(&mut Vec<String>) + Send + 'static,
    ) {
        self.request_short_link_hosts = Some(Box::new(request));
    }

    /// `dns_util_.GetNewDNS().GetHostByName(_host, _iplist)` — tried first.
    pub fn set_new_dns(&mut self, dns: impl FnMut(&str) -> Vec<String> + Send + 'static) {
        self.new_dns = Some(Box::new(dns));
    }

    /// `dns_util_.GetDNS().GetHostByName(_host, _iplist)` — asked only when
    /// [`NetCheckLogic::set_new_dns`] answered nothing.
    pub fn set_dns(&mut self, dns: impl FnMut(&str) -> Vec<String> + Send + 'static) {
        self.dns = Some(Box::new(dns));
    }

    /// `sdt::SdtManager::StartActiveCheck(…, mode, UNUSE_TIMEOUT)`.
    pub fn set_start_active_check(
        &mut self,
        start: impl FnMut(&CheckIPPorts, &CheckIPPorts, i32) + Send + 'static,
    ) {
        self.start_active_check = Some(Box::new(start));
    }

    /// `sdt::SdtManager::CancelActiveCheck()` — what the C++'s destructor
    /// calls, and what a host that owns an [`NetCheckLogic`] calls when it
    /// lets it go.
    pub fn set_cancel_active_check(&mut self, cancel: impl FnMut() + Send + 'static) {
        self.cancel_active_check = Some(Box::new(cancel));
    }

    /// The window of the long link, `1` for a success and the newest in the
    /// low bit: the C++ keeps it private, and the port shows it because it is
    /// what decides whether a check starts.
    pub fn longlink_records(&self) -> u32 {
        self.longlink_taskstatus_item.records
    }

    /// The same for the short link.
    pub fn shortlink_records(&self) -> u32 {
        self.shortlink_taskstatus_item.records
    }

    /// `longlink_taskstatus_item_.last_failedtime`.
    pub fn longlink_last_failed_time(&self) -> u64 {
        self.longlink_taskstatus_item.last_failedtime
    }

    /// `shortlink_taskstatus_item_.last_failedtime`.
    pub fn shortlink_last_failed_time(&self) -> u64 {
        self.shortlink_taskstatus_item.last_failedtime
    }

    /// How many times the wait between two checks has grown.
    pub fn increment_steps(&self) -> u32 {
        self.increment_steps
    }

    /// `CancelActiveCheck()`.
    pub fn cancel_active_check(&mut self) {
        if let Some(cancel) = self.cancel_active_check.as_mut() {
            cancel();
        }
    }

    /// `UpdateLongLinkInfo(_continues_fail_count, _task_succ)` — the count is
    /// only written to the log in the C++, so the port takes it and ignores
    /// it.
    pub fn update_long_link_info(&mut self, continues_fail_count: u32, task_succ: bool) {
        self.update_long_link_info_at(gettickcount(), continues_fail_count, task_succ)
    }

    /// The same, with the reading handed in.
    pub fn update_long_link_info_at(
        &mut self,
        now: u64,
        _continues_fail_count: u32,
        task_succ: bool,
    ) {
        Self::update(&mut self.longlink_taskstatus_item, now, task_succ);
        if self.should_net_check_at(now) {
            self.start_net_check();
        }
    }

    /// `UpdateShortLinkInfo(_continue_fail_count, _task_succ)`.
    pub fn update_short_link_info(&mut self, continues_fail_count: u32, task_succ: bool) {
        self.update_short_link_info_at(gettickcount(), continues_fail_count, task_succ)
    }

    /// The same, with the reading handed in.
    pub fn update_short_link_info_at(
        &mut self,
        now: u64,
        _continues_fail_count: u32,
        task_succ: bool,
    ) {
        Self::update(&mut self.shortlink_taskstatus_item, now, task_succ);
        if self.should_net_check_at(now) {
            self.start_net_check();
        }
    }

    /// What both `Update*Info` do to the window of their link.
    fn update(item: &mut NetTaskStatusItem, now: u64, task_succ: bool) {
        if !task_succ {
            item.last_failedtime = now;
        }
        item.records = set_bit(task_succ, item.records, VALID_BITS_FILTER);
    }

    /// `__ShouldNetCheck`.
    fn should_net_check_at(&mut self, now: u64) -> bool {
        let shortlink_status = extract_n_bits(
            self.shortlink_taskstatus_item.records,
            MOST_RECENT_TASK_START_N[0],
            MOST_RECENT_TASK_START_N[1],
        );
        let mut succ_count = shortlink_status.count_ones();
        let is_shortlink_bad = succ_count < CHECK_IF_BELOW_COUNT;
        let is_shortlink_good = succ_count > CHECK_IF_ABOVE_COUNT;

        let mut shortlink_shouldcheck = false;
        if is_shortlink_bad {
            let second_status = extract_n_bits(
                self.shortlink_taskstatus_item.records,
                SECOND_RECENT_TASK_START_N[0],
                SECOND_RECENT_TASK_START_N[1],
            );
            succ_count = second_status.count_ones();
            shortlink_shouldcheck = succ_count > CHECK_IF_ABOVE_COUNT;
        }

        let longlink_status = extract_n_bits(
            self.longlink_taskstatus_item.records,
            MOST_RECENT_TASK_START_N[0],
            MOST_RECENT_TASK_START_N[1],
        );
        let mut succ_count = longlink_status.count_ones();
        let is_longlink_bad = succ_count < CHECK_IF_BELOW_COUNT;
        let is_longlink_good = succ_count > CHECK_IF_ABOVE_COUNT;

        let mut longlink_shouldcheck = false;
        if is_longlink_bad {
            let second_status = extract_n_bits(
                self.longlink_taskstatus_item.records,
                SECOND_RECENT_TASK_START_N[0],
                SECOND_RECENT_TASK_START_N[1],
            );
            succ_count = second_status.count_ones();
            longlink_shouldcheck = succ_count > CHECK_IF_ABOVE_COUNT;
        }

        let mut ret = longlink_shouldcheck || shortlink_shouldcheck;

        if ret {
            let span = MIN_CHECK_TIME_SPAN
                .saturating_add(u64::from(self.increment_steps) * CHECK_TIME_SPAN_INCREMENT_STEP);
            if now.saturating_sub(self.last_netcheck_time) < span {
                ret = false;
            } else {
                self.increment_steps = self.increment_steps.saturating_add(1);
            }
        }
        // the network is stable now, so the wait starts over
        if is_shortlink_good && is_longlink_good {
            self.increment_steps = 0;
        }

        if ret && !self.frequency_limit.check_at(now) {
            return false;
        }
        ret
    }

    /// `__StartNetCheck` — the hosts of the two links, resolved through the
    /// dns, handed to `sdt::SdtManager::StartActiveCheck`.
    fn start_net_check(&mut self) {
        let longlink_hosts = self.long_link_hosts();
        if longlink_hosts.is_empty() {
            return;
        }
        let longlink_ports = self.long_link_ports();
        if longlink_ports.is_empty() {
            return;
        }

        let mut longlink_check_items = CheckIPPorts::new();
        for host in &longlink_hosts {
            let ips = self.resolve(host);
            if ips.is_empty() {
                continue;
            }
            // the C++ walks the ports outside and the ips inside
            let mut check_ipport_list = Vec::new();
            for port in &longlink_ports {
                for ip in &ips {
                    check_ipport_list.push(CheckIPPort::new(ip.as_str(), *port));
                }
            }
            longlink_check_items.insert(host.clone(), check_ipport_list);
        }

        let mut shortlink_check_items = CheckIPPorts::new();
        let mut shortlink_hostlist = Vec::new();
        if let Some(request) = self.request_short_link_hosts.as_mut() {
            request(&mut shortlink_hostlist);
        }
        let shortlink_port = self.short_link_port();
        for host in &shortlink_hostlist {
            let ips = self.resolve(host);
            if ips.is_empty() {
                continue;
            }
            let check_ipport_list = ips
                .iter()
                .map(|ip| CheckIPPort::new(ip.as_str(), shortlink_port))
                .collect();
            shortlink_check_items.insert(host.clone(), check_ipport_list);
        }

        if longlink_check_items.is_empty() && shortlink_check_items.is_empty() {
            return;
        }
        if let Some(start) = self.start_active_check.as_mut() {
            start(
                &longlink_check_items,
                &shortlink_check_items,
                NET_CHECK_MODE,
            );
        }
    }

    /// `GetNewDNS()`, then `GetDNS()` when the first answered nothing.
    fn resolve(&mut self, host: &str) -> Vec<String> {
        if let Some(new_dns) = self.new_dns.as_mut() {
            let ips = new_dns(host);
            if !ips.is_empty() {
                return ips;
            }
        }
        match self.dns.as_mut() {
            Some(dns) => dns(host),
            None => Vec::new(),
        }
    }

    fn long_link_hosts(&mut self) -> Vec<String> {
        match self.long_link_hosts.as_mut() {
            Some(hosts) => hosts(),
            None => Vec::new(),
        }
    }

    fn long_link_ports(&mut self) -> Vec<u16> {
        match self.long_link_ports.as_mut() {
            Some(ports) => ports(),
            None => Vec::new(),
        }
    }

    fn short_link_port(&mut self) -> u16 {
        match self.short_link_port.as_mut() {
            Some(port) => port(),
            None => 0,
        }
    }
}

impl std::fmt::Debug for NetCheckLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetCheckLogic")
            .field("longlink_records", &self.longlink_taskstatus_item.records)
            .field("shortlink_records", &self.shortlink_taskstatus_item.records)
            .field("increment_steps", &self.increment_steps)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// What `StartActiveCheck` was handed: the (ip, port) pairs of the two
    /// links and the mode.
    type Started = Arc<Mutex<Vec<(Vec<(String, u16)>, Vec<(String, u16)>, i32)>>>;

    /// Every host and port of a `CheckIPPorts`, in the order they went in.
    fn pairs(items: &CheckIPPorts) -> Vec<(String, u16)> {
        items
            .values()
            .flat_map(|list| list.iter().map(|item| (item.ip.clone(), item.port)))
            .collect()
    }

    /// A logic whose long link is reachable at one host and two ports, whose
    /// short link is at one host of its own, and whose new dns only knows the
    /// long link's host.
    fn a_logic() -> NetCheckLogic {
        let mut logic = NetCheckLogic::new_at(0);
        logic.set_long_link_hosts(|| vec!["long.example".to_string()]);
        logic.set_long_link_ports(|| vec![80, 443]);
        logic.set_short_link_port(|| 8080);
        logic.set_request_short_link_hosts(|hosts| hosts.push("short.example".to_string()));
        logic.set_new_dns(|host| {
            if host.starts_with("long") {
                vec!["1.2.3.4".to_string()]
            } else {
                Vec::new()
            }
        });
        logic.set_dns(|_host| vec!["5.6.7.8".to_string()]);
        logic
    }

    /// Sixteen successes and then `failures` failures on the long link: broken
    /// now, and fine in the eight tasks before.
    fn broken_longlink(logic: &mut NetCheckLogic, now: u64, failures: usize) {
        for _ in 0..16 {
            logic.update_long_link_info_at(now, 0, true);
        }
        for _ in 0..failures {
            logic.update_long_link_info_at(now, 0, false);
        }
    }

    #[test]
    fn the_window_starts_full_of_successes_and_the_newest_goes_in_the_low_bit() {
        let mut logic = NetCheckLogic::new_at(0);
        assert_eq!(logic.longlink_records(), VALID_BITS_FILTER);

        // a window that is all successes stays that way when one more comes in
        logic.update_long_link_info_at(1_000, 0, true);
        assert_eq!(logic.longlink_records(), VALID_BITS_FILTER);

        logic.update_long_link_info_at(2_000, 0, false);
        assert_eq!(
            logic.longlink_records(),
            0b1111_1111_1111_1111_1111_1111_1111_1110
        );
        assert_eq!(logic.longlink_last_failed_time(), 2_000);
        assert_eq!(
            logic.shortlink_last_failed_time(),
            0,
            "the other link is not touched"
        );

        // ... and the next success pushes the failure up and takes the low bit
        logic.update_long_link_info_at(3_000, 0, true);
        assert_eq!(
            logic.longlink_records(),
            0b1111_1111_1111_1111_1111_1111_1111_1101
        );
        assert_eq!(
            logic.longlink_last_failed_time(),
            2_000,
            "a success stamps the other time"
        );
    }

    #[test]
    fn the_eight_most_recent_tasks_come_out_of_the_window() {
        // `1` in the bits 8..1, which is what `kMostRecentTaskStartN` asks for
        let records = 0b0000_0000_0000_0000_0000_0001_1111_1110;
        assert_eq!(extract_n_bits(records, 24, 8), 0b1111_1111);
        // ... and the eight before those are the bits 16..9
        assert_eq!(extract_n_bits(records, 16, 8), 0);

        // a start position past the end of the window leaves nothing, which is
        // the shift of thirty-two the C++ leaves undefined
        assert_eq!(extract_n_bits(records, 32, 8), 0);
        assert_eq!(extract_n_bits(records, 0, 8), extract_n_bits(records, 1, 8));
    }

    #[test]
    fn a_link_that_was_fine_and_then_broke_starts_a_check() {
        let mut logic = a_logic();
        let started: Started = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&started);
        logic.set_start_active_check(move |longlink, shortlink, mode| {
            record.lock().unwrap_or_else(|e| e.into_inner()).push((
                pairs(longlink),
                pairs(shortlink),
                mode,
            ));
        });

        // sixteen successes, then the seven failures it takes for the eight
        // most recent tasks to be mostly failures
        broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN, 7);
        let started = started.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert_eq!(started.len(), 1, "the check started");
        let (longlink, shortlink, mode) = &started[0];
        assert_eq!(*mode, NET_CHECK_MODE);
        assert_eq!(
            *longlink,
            vec![("1.2.3.4".to_string(), 80), ("1.2.3.4".to_string(), 443)],
            "one ip on each port, and the ports walk outside the ips"
        );
        // the short link's host is the one only the old dns knows
        assert_eq!(*shortlink, vec![("5.6.7.8".to_string(), 8080)]);

        // ... and the eighth failure does not start another one: the wait has
        // grown to fifteen minutes and the reading is still the same
        logic.update_long_link_info_at(MIN_CHECK_TIME_SPAN, 0, false);
        assert_eq!(started.len(), 1);
    }

    #[test]
    fn the_wait_grows_with_every_check_that_gets_through() {
        // no hosts, so `__StartNetCheck` has nothing to resolve against: only
        // the decision itself is observable here
        let mut logic = NetCheckLogic::new_at(0);
        assert!(logic.long_link_hosts.is_none());

        // six failures: the most recent eight tasks are not broken enough yet
        broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN - 1, 6);
        assert_eq!(logic.increment_steps(), 0);

        // the seventh is, and the process is younger than `kMinCheckTimeSpan`
        logic.update_long_link_info_at(MIN_CHECK_TIME_SPAN - 1, 0, false);
        assert_eq!(logic.increment_steps(), 0, "too soon");

        // five minutes old, the wait is met and grows
        logic.update_long_link_info_at(MIN_CHECK_TIME_SPAN, 0, false);
        assert_eq!(logic.increment_steps(), 1);
        // ... and fifteen minutes is what the next one needs
        logic.update_long_link_info_at(MIN_CHECK_TIME_SPAN, 0, false);
        assert_eq!(logic.increment_steps(), 1);
        logic.update_long_link_info_at(
            MIN_CHECK_TIME_SPAN + CHECK_TIME_SPAN_INCREMENT_STEP,
            0,
            false,
        );
        assert_eq!(logic.increment_steps(), 2);
        logic.update_long_link_info_at(
            MIN_CHECK_TIME_SPAN + 2 * CHECK_TIME_SPAN_INCREMENT_STEP,
            0,
            false,
        );
        assert_eq!(logic.increment_steps(), 3, "twenty-five minutes");
    }

    #[test]
    fn the_frequency_limit_lets_one_more_through_than_its_count() {
        let mut logic = a_logic();
        let started: Started = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&started);
        logic.set_start_active_check(move |_, _, _| {
            record
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((Vec::new(), Vec::new(), 0));
        });

        // the first check, five minutes in
        broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN, 7);
        assert_eq!(started.lock().unwrap_or_else(|e| e.into_inner()).len(), 1);

        // fifteen minutes: `LIMIT_COUNT` is one, but the C++'s `Check()` passes
        // while `touch_times_.size() <= count_`, so a second one goes through
        // in the same hour
        logic.update_long_link_info_at(
            MIN_CHECK_TIME_SPAN + CHECK_TIME_SPAN_INCREMENT_STEP,
            0,
            false,
        );
        assert_eq!(started.lock().unwrap_or_else(|e| e.into_inner()).len(), 2);

        // twenty-five minutes is refused, and the hour is what it takes
        logic.update_long_link_info_at(
            MIN_CHECK_TIME_SPAN + 2 * CHECK_TIME_SPAN_INCREMENT_STEP,
            0,
            false,
        );
        assert_eq!(
            started.lock().unwrap_or_else(|e| e.into_inner()).len(),
            2,
            "the third is refused"
        );
        logic.update_long_link_info_at(MIN_CHECK_TIME_SPAN + LIMIT_TIME_SPAN + 1, 0, false);
        assert_eq!(started.lock().unwrap_or_else(|e| e.into_inner()).len(), 3);
    }

    #[test]
    fn a_link_that_is_fine_again_resets_the_wait() {
        let mut logic = NetCheckLogic::new_at(0);
        broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN, 7);
        assert_eq!(logic.increment_steps(), 1);

        // seven successes in a row and both links are fine again
        for _ in 0..7 {
            logic.update_long_link_info_at(MIN_CHECK_TIME_SPAN, 0, true);
        }
        assert_eq!(logic.increment_steps(), 0);
    }

    #[test]
    fn a_link_without_hosts_or_ports_or_ips_is_not_checked() {
        let started = Arc::new(Mutex::new(0usize));

        // nothing at all: the hosts are the first thing the C++ gives up on
        let mut logic = NetCheckLogic::new_at(0);
        let record = Arc::clone(&started);
        logic.set_start_active_check(move |_, _, _| {
            *record.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        });
        broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN, 7);
        assert_eq!(*started.lock().unwrap_or_else(|e| e.into_inner()), 0);

        // hosts but no ports
        let mut logic = a_logic();
        logic.set_long_link_ports(Vec::new);
        let record = Arc::clone(&started);
        logic.set_start_active_check(move |_, _, _| {
            *record.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        });
        broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN, 7);
        assert_eq!(
            *started.lock().unwrap_or_else(|e| e.into_inner()),
            0,
            "no ports"
        );

        // hosts and ports, but a dns that knows neither
        let mut logic = a_logic();
        logic.set_new_dns(|_| Vec::new());
        logic.set_dns(|_| Vec::new());
        let record = Arc::clone(&started);
        logic.set_start_active_check(move |_, _, _| {
            *record.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        });
        broken_longlink(&mut logic, MIN_CHECK_TIME_SPAN, 7);
        assert_eq!(
            *started.lock().unwrap_or_else(|e| e.into_inner()),
            0,
            "no ips, so nothing to check"
        );
    }

    #[test]
    fn the_short_link_decides_on_its_own() {
        let mut logic = NetCheckLogic::new_at(0);
        assert_eq!(logic.shortlink_records(), VALID_BITS_FILTER);

        for _ in 0..16 {
            logic.update_short_link_info_at(0, 0, true);
        }
        for _ in 0..7 {
            logic.update_short_link_info_at(MIN_CHECK_TIME_SPAN, 0, false);
        }
        assert_eq!(logic.increment_steps(), 1, "the short link decided it");
        assert_eq!(logic.longlink_records(), VALID_BITS_FILTER);
    }

    #[test]
    fn without_a_host_nothing_is_asked_for() {
        let mut logic = NetCheckLogic::default();
        logic.update_long_link_info(0, false);
        logic.update_short_link_info(0, false);
        assert_eq!(
            logic.longlink_records(),
            0b1111_1111_1111_1111_1111_1111_1111_1110
        );

        let cancelled = Arc::new(Mutex::new(0usize));
        let record = Arc::clone(&cancelled);
        logic.set_cancel_active_check(move || {
            *record.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        });
        // what the C++'s destructor does
        logic.cancel_active_check();
        assert_eq!(*cancelled.lock().unwrap_or_else(|e| e.into_inner()), 1);
        assert!(format!("{logic:?}").contains("NetCheckLogic"));
    }
}
