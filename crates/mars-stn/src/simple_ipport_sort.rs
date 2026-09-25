//! `mars/stn/src/simple_ipport_sort.cc` — which ip/port pair a task is tried
//! on first.
//!
//! Every attempt at an ip/port pair is one bit in a history: `Update` pushes a
//! `0` for a success and a `1` for a failure, and [`SimpleIpPortSort`] decides
//! from those bits
//!
//! * **whether the pair is used at all** — [`BAN_FAIL_COUNT`] failures in the
//!   history ban it for [`BAN_TIME`], which grows by another [`BAN_TIME`] per
//!   further failure in a row up to [`MAX_BAN_TIME`], and `AddServerBan` keeps
//!   an ip out for [`SERVER_BAN_TIME`] outright;
//! * **in which order the pairs are tried** — the ones with a history first,
//!   least failed first, and the ones with none shuffled in between
//!   ([`SimpleIpPortSort::sort_and_filter`]).
//!
//! Three things the C++ gets from its platform are handed to the port:
//!
//! * the `ipportrecords2.xml` in the app's own directory — the port has no
//!   filesystem, so the host loads the records in
//!   ([`SimpleIpPortSort::load_records`]) and persists what
//!   [`SimpleIpPortSort::save_records`] hands back;
//! * `getCurrNetLabel` — [`SimpleIpPortSort::set_net_label`], whose [`None`] is
//!   the C++'s `kNoNet`, which is also what an unset callback means;
//! * both clocks: `gettickcount()` for the ban times, and the unix second the
//!   C++ takes from `gettimeofday` for the `time` attribute of a record, so
//!   [`SimpleIpPortSort::update_at`] takes both.
//!
//! `mars::comm::random_shuffle` and `rand()` are the platform's rng in the C++.
//! Both are one callback here ([`SimpleIpPortSort::set_random`]), which is what
//! makes the shuffle a test can pin down; what it is by default is a xorshift
//! seeded from the tick count.

use std::collections::{HashMap, VecDeque};

use crate::task::Task;
use crate::Random;

/// `kRecordTimeout` — a record older than this, in seconds, is gone the next
/// time the file is read or written.
pub const RECORD_TIMEOUT: u64 = 60 * 60 * 24;

/// `kBanTime` — how long an ip/port pair with [`BAN_FAIL_COUNT`] failures in
/// its history stays out, in milliseconds.
pub const BAN_TIME: u64 = 6 * 60 * 1000;
/// `kMaxBanTime` — the ceiling the ban time grows to with every further
/// failure in a row.
pub const MAX_BAN_TIME: u64 = 30 * 60 * 1000;
/// `kServerBanTime` — how long `AddServerBan` keeps an ip out.
pub const SERVER_BAN_TIME: u64 = 30 * 60 * 1000;
/// `kBanFailCount` — failures in the history before the pair is banned.
pub const BAN_FAIL_COUNT: u32 = 3;
/// `kSuccessUpdateInterval` — a success closer than this to the last one is
/// not written down.
pub const SUCCESS_UPDATE_INTERVAL: u64 = 10 * 1000;
/// `kFailUpdateInterval` — the same for a failure.
pub const FAIL_UPDATE_INTERVAL: u64 = 10 * 1000;

/// `IPSourceType` of `mars/stn/stn.h` — where an ip came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IpSourceType {
    /// `kIPSourceNULL`
    #[default]
    Null = 0,
    /// `kIPSourceDebug`
    Debug,
    /// `kIPSourceDNS`
    Dns,
    /// `kIPSourceNewDns`
    NewDns,
    /// `kIPSourceProxy`
    Proxy,
    /// `kIPSourceBackup`
    Backup,
}

/// `IPPortItem` of `mars/stn/stn.h` — one candidate to connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpPortItem {
    /// `str_ip`
    pub ip: String,
    /// `port`
    pub port: u16,
    /// `source_type`
    pub source_type: IpSourceType,
    /// `str_host` — the host the ip was resolved from.
    pub host: String,
    /// `transport_protocol` — one of the `Task::kTransportProtocol*`.
    pub transport_protocol: i32,
    /// `from_source`
    pub from_source: u32,
}

impl IpPortItem {
    /// An item with the defaults the C++ gives it: no source, no host, tcp.
    pub fn new(ip: impl Into<String>, port: u16) -> Self {
        Self {
            ip: ip.into(),
            port,
            source_type: IpSourceType::Null,
            host: String::new(),
            transport_protocol: Task::TRANSPORT_PROTOCOL_TCP,
            from_source: 0,
        }
    }

    /// Whether the ip is an IPv6 one, the way `__IsV6Ip` decides it: an IPv4
    /// address has a dot in it and an IPv6 one does not.
    fn is_v6(&self) -> bool {
        !self.ip.contains('.')
    }
}

/// `<item>` — the history the C++ keeps in `historyresult`: one **bit** per
/// attempt, newest in the low bit, which is what `Update` writes. What
/// `InitHistory2BannedList` reads out of it is one bit per *byte*, so the two
/// do not agree — see [`SimpleIpPortSort::load_records`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordItem {
    /// `ip`
    pub ip: String,
    /// `port`
    pub port: u16,
    /// `historyresult`
    pub history_result: u64,
}

/// `<record>` — what was learned on one network.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Record {
    /// `netinfo`
    pub net_info: String,
    /// `time` — the unix second the record was written at. [`None`] is a
    /// record without the attribute, which `__RemoveTimeoutXml` drops.
    pub time: Option<u64>,
    /// The `<item>`s.
    pub items: Vec<RecordItem>,
}

/// `struct BanItem` — the port of the C++: the history of one pair and the
/// readings the last attempt on it was written down at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BanItem {
    /// `ip`
    pub ip: String,
    /// `port`
    pub port: u16,
    /// `records` — the last eight attempts, `1` for a failure.
    pub records: u8,
    /// `last_fail_time` — [`None`] for "never", which is a pair the ban list
    /// knows only from the history the host handed in.
    pub last_fail_time: Option<u64>,
    /// `last_suc_time`, the same.
    pub last_suc_time: Option<u64>,
}

/// `tickcount_t::gettickspan()` — how long ago, in milliseconds.
///
/// A `tickcount_t` the C++ has not written into is `0`, and one it has is
/// `sg_tick_init` — two billion, about twenty-three days — on top of the tick
/// count, so "never" answers a span longer than any ban and longer than
/// either update interval. That is [`None`] here, and it is what keeps a pair
/// restored from the host's history from looking like one that failed a
/// moment ago.
fn tickspan(now: u64, at: Option<u64>) -> u64 {
    at.map_or(NEVER_SPAN, |at| now.saturating_sub(at))
}

/// `sg_tick_init` — what the C++'s `tickcount_t` starts a real reading at, and
/// so what a span from a "never" one is.
const NEVER_SPAN: u64 = 2_000_000_000;

/// `getCurrNetLabel` — the label of the network the app is on, or [`None`] for
/// `kNoNet`.
pub type NetLabel = dyn FnMut() -> Option<String> + Send;

/// `SimpleIPPortSort`.
pub struct SimpleIpPortSort {
    /// `recordsxml_` — the `<record>`s, in the order they were written in.
    records: Vec<Record>,
    /// `_ban_fail_list_`
    ban_fail_list: Vec<BanItem>,
    /// `_server_bans_`
    server_bans: HashMap<String, u64>,
    net_label: Option<Box<NetLabel>>,
    random: Box<Random>,
}

/// `SET_BIT(SET, RECORDS)` — the newest attempt goes in the low bit and the
/// oldest falls off the end.
fn set_bit(set: bool, records: u64) -> u64 {
    (records << 1) | u64::from(set)
}

/// `CAL_BIT_COUNT` — how many of the last eight attempts failed.
fn bit_count(records: u8) -> u32 {
    records.count_ones()
}

/// `CAL_LAST_CONTINUOUS_BIT_COUNT` — how many attempts in a row failed, most
/// recent first: the C++ walks up from the low bit while it is set.
fn last_continuous_bit_count(mut records: u8) -> u32 {
    let mut count = 0;
    while records & 0x1 != 0 {
        count += 1;
        records >>= 1;
    }
    count
}

/// The 64 `historyresult` bits of an xml item, "8 in 1": the C++ pushes one
/// byte at a time, low byte first, so the high byte is the newest attempt.
///
/// The two ends of the xml disagree in the C++ and disagree here: `Update`
/// writes one *bit* per attempt into the 64-bit attribute, and this reads one
/// *bit* per *byte* out of it, so eight attempts that failed come back as one
/// byte that is not `0` — a single failure. The port keeps the disagreement
/// because what the C++ does with a restored pair is sort it last, and one
/// failure is enough for that.
fn history_to_records(mut history: u64) -> u8 {
    let mut records = 0u8;
    for _ in 0..8 {
        records = set_bit(history & 0xFF != 0, u64::from(records)) as u8;
        history >>= 8;
    }
    records
}

impl SimpleIpPortSort {
    /// `SimpleIPPortSort(context)` — the C++ reads `ipportrecords2.xml` here,
    /// which in the port is [`SimpleIpPortSort::load_records`], and seeds its
    /// rng from the tick count.
    pub fn new() -> Self {
        Self::seeded(mars_comm::tickcount::gettickcount())
    }

    /// The same, with the rng seeded from `seed` — which is what makes the
    /// shuffle the same every time.
    pub fn seeded(seed: u64) -> Self {
        Self {
            records: Vec::new(),
            ban_fail_list: Vec::new(),
            server_bans: HashMap::new(),
            net_label: None,
            random: Box::new(crate::xorshift(seed)),
        }
    }

    /// `getCurrNetLabel`.
    pub fn set_net_label(&mut self, net_label: impl FnMut() -> Option<String> + Send + 'static) {
        self.net_label = Some(Box::new(net_label));
    }

    /// `getCurrNetLabel` answering `kNoNet` again.
    pub fn clear_net_label(&mut self) {
        self.net_label = None;
    }

    /// `rand()`, for a host that has an rng of its own.
    pub fn set_random(&mut self, random: impl FnMut(usize) -> usize + Send + 'static) {
        self.random = Box::new(random);
    }

    /// `recordsxml_` — the records as they are now.
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// `_ban_fail_list_`.
    pub fn ban_list(&self) -> &[BanItem] {
        &self.ban_fail_list
    }

    /// `__LoadXml` — the records the host read out of its own storage, with the
    /// ones whose `time` is missing, in the future or older than
    /// [`RECORD_TIMEOUT`] dropped, which is `__RemoveTimeoutXml`.
    pub fn load_records(&mut self, records: Vec<Record>, now_secs: u64) {
        self.records = records;
        self.remove_timeout_records(now_secs);
    }

    /// `__SaveXml` — what the host writes back: the records with the timed-out
    /// ones dropped, which is what the C++ does before `SaveFile`.
    pub fn save_records(&mut self, now_secs: u64) -> Vec<Record> {
        self.remove_timeout_records(now_secs);
        self.records.clone()
    }

    /// `InitHistory2BannedList(false)` — throw the ban list away and rebuild
    /// it from the record of the network the app is on.
    ///
    /// The C++ takes a `_savexml` flag for the callers that want the file
    /// written first; the port saves with
    /// [`SimpleIpPortSort::save_records`].
    pub fn init_history_to_banned_list(&mut self) {
        self.ban_fail_list.clear();

        let Some(net_info) = self.net_label() else {
            return;
        };
        let Some(record) = self
            .records
            .iter()
            .find(|record| record.net_info == net_info)
        else {
            return;
        };

        // the ban items the C++ builds here have no times: their `tickcount_t`
        // stays at `0`, which is why a pair that is only known from history is
        // never banned, only sorted
        self.ban_fail_list = record
            .items
            .iter()
            .map(|item| BanItem {
                ip: item.ip.clone(),
                port: item.port,
                records: history_to_records(item.history_result),
                last_fail_time: None,
                last_suc_time: None,
            })
            .collect();
    }

    /// `RemoveBannedList(_ip)` — every port of the ip leaves the ban list.
    pub fn remove_banned_list(&mut self, ip: &str) {
        self.ban_fail_list.retain(|item| item.ip != ip);
    }

    /// `Update(_ip, _port, _is_success)` — with the unix second the C++ takes
    /// from `gettimeofday` handed in; the tick count comes from the clock.
    pub fn update(&mut self, now_secs: u64, ip: &str, port: u16, is_success: bool) {
        self.update_at(
            mars_comm::tickcount::gettickcount(),
            now_secs,
            ip,
            port,
            is_success,
        )
    }

    /// The same, with both readings handed in.
    ///
    /// Nothing at all happens while the app has no network, and neither does
    /// an attempt that came too soon after the last one of its kind
    /// (`__CanUpdate`).
    pub fn update_at(&mut self, now: u64, now_secs: u64, ip: &str, port: u16, is_success: bool) {
        let Some(net_info) = self.net_label() else {
            return;
        };

        if !self.can_update(now, ip, port, is_success) {
            return;
        }
        self.update_ban_list(now, ip, port, is_success);

        if !self
            .records
            .iter()
            .any(|record| record.net_info == net_info)
        {
            self.records.push(Record {
                net_info: net_info.clone(),
                time: Some(now_secs),
                items: Vec::new(),
            });
        }
        let record = self
            .records
            .iter_mut()
            .find(|record| record.net_info == net_info)
            .expect("the record was just pushed");

        if !record
            .items
            .iter()
            .any(|item| item.ip == ip && item.port == port)
        {
            record.items.push(RecordItem {
                ip: ip.to_string(),
                port,
                history_result: 0,
            });
        }
        let item = record
            .items
            .iter_mut()
            .find(|item| item.ip == ip && item.port == port)
            .expect("the item was just pushed");
        item.history_result = set_bit(!is_success, item.history_result);
    }

    /// `AddServerBan(_ip)` — the tick count comes from the clock.
    pub fn add_server_ban(&mut self, ip: &str) {
        self.add_server_ban_at(mars_comm::tickcount::gettickcount(), ip);
    }

    /// The same, with the reading handed in. An empty ip is ignored.
    pub fn add_server_ban_at(&mut self, now: u64, ip: &str) {
        if ip.is_empty() {
            return;
        }
        self.server_bans.insert(ip.to_string(), now);
    }

    /// `SortandFilter(_items, _needcount, _use_IPv6)` — the tick count comes
    /// from the clock.
    ///
    /// The candidates go in and the ones worth trying come back: the C++ takes
    /// `std::vector<IPPortItem>&` and reorders it where the caller left it,
    /// which a `&mut Vec` out-parameter here would only pretend to be — the
    /// sort *replaces* the list (banned pairs are dropped, the rest are
    /// reordered), so it hands the new one back.
    pub fn sort_and_filter(
        &mut self,
        items: impl IntoIterator<Item = IpPortItem>,
        need_count: usize,
        use_ipv6: bool,
    ) -> Vec<IpPortItem> {
        self.sort_and_filter_at(
            mars_comm::tickcount::gettickcount(),
            items,
            need_count,
            use_ipv6,
        )
    }

    /// The same, with the reading handed in: drop the pairs that are banned,
    /// put the rest in the order they should be tried in, and keep at most
    /// `need_count` of them.
    ///
    /// [`usize::MAX`] keeps them all, which is the only reading `_needcount`
    /// can have here: the C++'s `int` is `resize`d to, and a negative one is a
    /// length a vector cannot have.
    pub fn sort_and_filter_at(
        &mut self,
        now: u64,
        items: impl IntoIterator<Item = IpPortItem>,
        need_count: usize,
        use_ipv6: bool,
    ) -> Vec<IpPortItem> {
        let mut items = self.filter_by_banned(now, items.into_iter().collect());
        self.sort_by_banned(&mut items, use_ipv6);
        items.truncate(need_count);
        items
    }

    /// Whether the pair is out right now: `__IsBanned`, plus what
    /// [`SimpleIpPortSort::add_server_ban`] may have said about its ip — which
    /// is what `__FilterbyBanned` asks, so a caller that decides on its own
    /// whether to connect cannot pick an ip the server took out.
    pub fn is_banned_at(&self, now: u64, ip: &str, port: u16) -> bool {
        if self.is_server_banned_at(now, ip) {
            return true;
        }

        let Some(item) = self.find_banned(ip, port) else {
            return false;
        };

        if bit_count(item.records) < BAN_FAIL_COUNT {
            return false;
        }

        let mut ban_time = BAN_TIME;
        let continuous = last_continuous_bit_count(item.records);
        if continuous > BAN_FAIL_COUNT {
            ban_time += u64::from(continuous - BAN_FAIL_COUNT) * BAN_TIME;
            ban_time = ban_time.min(MAX_BAN_TIME);
        }

        tickspan(now, item.last_fail_time) < ban_time
    }

    /// `__IsServerBan` without the `erase` — whether the ip is still inside
    /// `kServerBanTime`. [`SimpleIpPortSort::sort_and_filter`] is what forgets
    /// one whose ban has run out, the way the C++ does.
    pub fn is_server_banned_at(&self, now: u64, ip: &str) -> bool {
        self.server_bans
            .get(ip)
            .is_some_and(|&banned_at| now.saturating_sub(banned_at) < SERVER_BAN_TIME)
    }

    /// `__RemoveTimeoutXml`.
    fn remove_timeout_records(&mut self, now_secs: u64) {
        self.records.retain(|record| match record.time {
            // a record without a `time`, one from the future, or one whose
            // `kRecordTimeout` has run out
            None => false,
            Some(time) => time <= now_secs && now_secs - time < RECORD_TIMEOUT,
        });
    }

    /// `getCurrNetLabel`, whose `kNoNet` the unset callback answers too.
    fn net_label(&mut self) -> Option<String> {
        match self.net_label.as_mut() {
            Some(net_label) => net_label(),
            None => None,
        }
    }

    /// `rand()`.
    fn random(&mut self, bound: usize) -> usize {
        (self.random)(bound)
    }

    fn find_banned(&self, ip: &str, port: u16) -> Option<&BanItem> {
        self.ban_fail_list
            .iter()
            .find(|item| item.ip == ip && item.port == port)
    }

    fn find_banned_mut(&mut self, ip: &str, port: u16) -> Option<&mut BanItem> {
        self.ban_fail_list
            .iter_mut()
            .find(|item| item.ip == ip && item.port == port)
    }

    /// `__CanUpdate` — an attempt that came too soon after the last one of its
    /// kind is not written down, so one flapping pair cannot fill the history
    /// on its own.
    fn can_update(&self, now: u64, ip: &str, port: u16, is_success: bool) -> bool {
        let Some(item) = self.find_banned(ip, port) else {
            return true;
        };
        if is_success {
            SUCCESS_UPDATE_INTERVAL < tickspan(now, item.last_suc_time)
        } else {
            FAIL_UPDATE_INTERVAL < tickspan(now, item.last_fail_time)
        }
    }

    /// `__UpdateBanList`.
    fn update_ban_list(&mut self, now: u64, ip: &str, port: u16, is_success: bool) {
        if let Some(item) = self.find_banned_mut(ip, port) {
            item.records = set_bit(!is_success, u64::from(item.records)) as u8;
            if is_success {
                item.last_suc_time = Some(now);
            } else {
                item.last_fail_time = Some(now);
            }
            return;
        }

        self.ban_fail_list.push(BanItem {
            ip: ip.to_string(),
            port,
            records: u8::from(!is_success),
            last_fail_time: if is_success { None } else { Some(now) },
            last_suc_time: if is_success { Some(now) } else { None },
        });
    }

    /// `__FilterbyBanned` — the pairs that are out, dropped out of the list
    /// instead of `erase`d out of the one the caller handed in.
    fn filter_by_banned(&mut self, now: u64, mut items: Vec<IpPortItem>) -> Vec<IpPortItem> {
        items.retain(|item| {
            !self.is_banned_at(now, &item.ip, item.port) && !self.is_server_ban(now, &item.ip)
        });
        items
    }

    /// `__IsServerBan` — and an ip whose ban has run out is forgotten, which is
    /// what the C++'s `erase` does.
    fn is_server_ban(&mut self, now: u64, ip: &str) -> bool {
        let Some(&banned_at) = self.server_bans.get(ip) else {
            return false;
        };
        if now.saturating_sub(banned_at) < SERVER_BAN_TIME {
            return true;
        }
        self.server_bans.remove(ip);
        false
    }

    /// `__SortbyBanned`: shuffle, pull the pairs that share an ip apart, put
    /// the ones with a history — least failed first — in front, and shuffle
    /// the ones without one in between.
    fn sort_by_banned(&mut self, items: &mut Vec<IpPortItem>, use_ipv6: bool) {
        // `mars::comm::random_shuffle`
        for i in (1..items.len()).rev() {
            let j = self.random(i + 1);
            items.swap(i, j);
        }

        // pull the pairs that share an ip apart, so one host does not take all
        // the first tries
        let count = items.len();
        let mut i = 1;
        while i + 1 < count {
            if items[i].ip != items[i - 1].ip {
                i += 1;
                continue;
            }
            let Some(at) = (i + 1..count).find(|at| items[*at].ip != items[i - 1].ip) else {
                break;
            };
            items.swap(i, at);
            i += 1;
        }

        // separate the ones with a history from the ones without one
        let mut history: VecDeque<IpPortItem> = VecDeque::new();
        let mut fresh: VecDeque<IpPortItem> = VecDeque::new();
        for item in items.drain(..) {
            if self.find_banned(&item.ip, item.port).is_some() {
                history.push_back(item);
            } else {
                fresh.push_back(item);
            }
        }

        // sort the history: least failed first, then the one that failed longer
        // ago, then the one that succeeded more recently
        history.make_contiguous().sort_by(|left, right| {
            let (Some(left), Some(right)) = (
                self.find_banned(&left.ip, left.port),
                self.find_banned(&right.ip, right.port),
            ) else {
                return std::cmp::Ordering::Equal;
            };
            if bit_count(left.records) != bit_count(right.records) {
                return bit_count(left.records).cmp(&bit_count(right.records));
            }
            // `None` is the C++'s `0`, which is smaller than any reading
            if left.last_fail_time != right.last_fail_time {
                return left.last_fail_time.cmp(&right.last_fail_time);
            }
            if left.last_suc_time != right.last_suc_time {
                return right.last_suc_time.cmp(&left.last_suc_time);
            }
            std::cmp::Ordering::Equal
        });

        if !use_ipv6 {
            while !history.is_empty() || !fresh.is_empty() {
                self.pick_random(items, &mut history, &mut fresh);
            }
            return;
        }

        // ... and when IPv6 is in play, one of each family in turn
        let mut fresh_v6: VecDeque<IpPortItem> = VecDeque::new();
        let mut fresh_v4: VecDeque<IpPortItem> = VecDeque::new();
        for item in fresh {
            if item.is_v6() {
                fresh_v6.push_back(item);
            } else {
                fresh_v4.push_back(item);
            }
        }
        let mut history_v6: VecDeque<IpPortItem> = VecDeque::new();
        let mut history_v4: VecDeque<IpPortItem> = VecDeque::new();
        for item in history {
            if item.is_v6() {
                history_v6.push_back(item);
            } else {
                history_v4.push_back(item);
            }
        }

        let mut history: VecDeque<IpPortItem> = VecDeque::new();
        let mut v6 = history_v6.into_iter();
        let mut v4 = history_v4.into_iter();
        for item in v6.by_ref() {
            history.push_back(item);
            if let Some(item) = v4.next() {
                history.push_back(item);
            }
        }
        history.extend(v4);

        let mut pick_v6 = true;
        while !history.is_empty() || !fresh_v6.is_empty() || !fresh_v4.is_empty() {
            if pick_v6 {
                if history.front().is_some_and(IpPortItem::is_v6) {
                    self.pick_random(items, &mut history, &mut fresh_v6);
                } else if let Some(item) = fresh_v6.pop_front() {
                    items.push(item);
                }
            } else if history.front().is_some_and(|item| !item.is_v6()) {
                self.pick_random(items, &mut history, &mut fresh_v4);
            } else if let Some(item) = fresh_v4.pop_front() {
                items.push(item);
            }
            pick_v6 = !pick_v6;
        }
    }

    /// `__PickIpItemRandom` — `rand() % (history + new)`: which of the two
    /// queues the next item comes from.
    fn pick_random(
        &mut self,
        items: &mut Vec<IpPortItem>,
        history: &mut VecDeque<IpPortItem>,
        fresh: &mut VecDeque<IpPortItem>,
    ) {
        let total = history.len() + fresh.len();
        if total == 0 {
            return;
        }
        if self.random(total) < history.len() {
            if let Some(item) = history.pop_front() {
                items.push(item);
            }
        } else if let Some(item) = fresh.pop_front() {
            items.push(item);
        }
    }
}

impl Default for SimpleIpPortSort {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SimpleIpPortSort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleIpPortSort")
            .field("records", &self.records.len())
            .field("ban_fail_list", &self.ban_fail_list.len())
            .field("server_bans", &self.server_bans.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sort on one network, whose clock the tests move by hand.
    fn a_sort(net_info: &str) -> SimpleIpPortSort {
        let mut sort = SimpleIpPortSort::seeded(1);
        sort.set_net_label({
            let net_info = net_info.to_string();
            move || Some(net_info.clone())
        });
        sort
    }

    /// A failure on `ip:port`, `spaced` after the one before it so
    /// `__CanUpdate` lets it through.
    fn fail(sort: &mut SimpleIpPortSort, now: u64, ip: &str, port: u16) {
        sort.update_at(now, now / 1000, ip, port, false);
    }

    #[test]
    fn the_history_is_one_bit_per_attempt_newest_first() {
        // three failures and then a success, newest first: `0b1110`
        let mut records = 0;
        for is_success in [false, false, false, true] {
            records = set_bit(!is_success, records);
        }
        assert_eq!(records, 0b1110);
        assert_eq!(bit_count(records as u8), 3);
        // the success is the newest bit, so there is no run to count
        assert_eq!(last_continuous_bit_count(records as u8), 0);

        // one more failure starts a new one
        records = set_bit(true, records);
        assert_eq!(last_continuous_bit_count(records as u8), 1);
        // ... and the oldest attempt is the one that falls off the end: the
        // newest goes in where the `1`s of the run are counted from
        assert_eq!(set_bit(false, 0xFF), 0x1FE);
        assert_eq!(bit_count(set_bit(false, 0xFF) as u8), 7);
        assert_eq!(bit_count(set_bit(true, 0xFF) as u8), 8, "a `1` for a `1`");
    }

    #[test]
    fn a_pair_is_banned_after_three_failures_and_forgiven_later() {
        let mut sort = a_sort("wifi");
        for at in [0, 11_000, 22_000] {
            fail(&mut sort, at, "1.2.3.4", 80);
        }
        assert_eq!(sort.ban_list()[0].records, 0b111);
        assert!(sort.is_banned_at(22_000, "1.2.3.4", 80));

        // `kBanTime` later it is back
        assert!(!sort.is_banned_at(22_000 + BAN_TIME, "1.2.3.4", 80));
        // ... and so is a pair that never failed, or failed twice
        assert!(!sort.is_banned_at(22_000, "1.2.3.4", 443));
    }

    #[test]
    fn the_ban_grows_with_every_failure_in_a_row_and_stops_at_the_ceiling() {
        let mut sort = a_sort("wifi");
        let mut now = 0;
        for _ in 0..4 {
            fail(&mut sort, now, "1.2.3.4", 80);
            now += 11_000;
        }
        // four in a row: `kBanTime` plus one more, counted from the last
        // failure (`now` has moved on by another interval since)
        let last_fail = 3 * 11_000;
        assert!(sort.is_banned_at(last_fail + BAN_TIME, "1.2.3.4", 80));
        assert!(!sort.is_banned_at(last_fail + 2 * BAN_TIME, "1.2.3.4", 80));

        // eight in a row is `6 + 5 * 6`, which `kMaxBanTime` cuts to 30
        let mut sort = a_sort("wifi");
        for at in (0..8).map(|index| index * 11_000) {
            fail(&mut sort, at, "1.2.3.4", 80);
        }
        let last_fail = 7 * 11_000;
        assert_eq!(last_continuous_bit_count(sort.ban_list()[0].records), 8);
        assert!(sort.is_banned_at(last_fail + MAX_BAN_TIME - 1, "1.2.3.4", 80));
        assert!(!sort.is_banned_at(last_fail + MAX_BAN_TIME, "1.2.3.4", 80));
    }

    #[test]
    fn an_attempt_too_soon_after_the_last_one_of_its_kind_is_dropped() {
        let mut sort = a_sort("wifi");
        fail(&mut sort, 0, "1.2.3.4", 80);
        // one millisecond later: inside `kFailUpdateInterval`
        fail(&mut sort, 1, "1.2.3.4", 80);
        assert_eq!(sort.ban_list()[0].records, 0b1);
        assert_eq!(sort.records()[0].items[0].history_result, 0b1);

        // a success is a different kind of attempt, and this pair has never
        // had one, so nothing holds it back
        sort.update_at(2, 0, "1.2.3.4", 80, true);
        assert_eq!(sort.ban_list()[0].records, 0b10);
        assert_eq!(sort.ban_list()[0].last_suc_time, Some(2));

        // ... and a second one inside `kSuccessUpdateInterval` is dropped
        sort.update_at(3, 0, "1.2.3.4", 80, true);
        assert_eq!(sort.ban_list()[0].records, 0b10);
        sort.update_at(2 + SUCCESS_UPDATE_INTERVAL + 1, 0, "1.2.3.4", 80, true);
        assert_eq!(sort.ban_list()[0].records, 0b100);
        assert_eq!(
            sort.ban_list()[0].last_suc_time,
            Some(2 + SUCCESS_UPDATE_INTERVAL + 1)
        );
    }

    #[test]
    fn a_pair_restored_from_history_is_neither_banned_nor_held_back() {
        let mut sort = a_sort("wifi");
        sort.load_records(
            vec![Record {
                net_info: "wifi".to_string(),
                time: Some(1),
                items: vec![RecordItem {
                    ip: "1.2.3.4".to_string(),
                    port: 80,
                    // three bytes that are not `0`: three failures, which is
                    // `kBanFailCount`
                    history_result: 0x00_00_00_00_00_01_01_01,
                }],
            }],
            1,
        );
        sort.init_history_to_banned_list();

        // the pair has no `last_fail_time` — the C++'s `tickcount_t` is `0`
        // until something is written into it, and a span from a `0` is about
        // twenty-three days — so a history of failures alone never bans it
        assert_eq!(sort.ban_list()[0].last_fail_time, None);
        assert!(!sort.is_banned_at(0, "1.2.3.4", 80));
        assert!(!sort.is_banned_at(BAN_TIME - 1, "1.2.3.4", 80));

        // ... and it is not "inside an update interval" either, so the first
        // attempt on it goes through right away
        fail(&mut sort, 1, "1.2.3.4", 80);
        // the new failure goes in the low bit and the oldest of the eight
        // falls off the end, so two of the three restored ones are left
        assert_eq!(sort.ban_list()[0].records, 0b1100_0001);
        assert_eq!(sort.ban_list()[0].last_fail_time, Some(1));
        // from then on the interval does hold it back
        fail(&mut sort, 2, "1.2.3.4", 80);
        assert_eq!(sort.ban_list()[0].records, 0b1100_0001);
    }

    #[test]
    fn a_success_stamps_its_own_time_and_a_failure_the_other_one() {
        let mut sort = a_sort("wifi");
        sort.update_at(1_000, 1, "1.2.3.4", 80, true);
        assert_eq!(sort.ban_list()[0].last_suc_time, Some(1_000));
        assert_eq!(sort.ban_list()[0].last_fail_time, None, "never failed");

        sort.update_at(1_000, 1, "1.2.3.4", 443, false);
        assert_eq!(sort.ban_list()[1].last_fail_time, Some(1_000));
        assert_eq!(sort.ban_list()[1].last_suc_time, None, "never succeeded");
    }

    #[test]
    fn what_was_learned_is_written_to_the_record_of_the_network() {
        let mut sort = a_sort("wifi");
        fail(&mut sort, 0, "1.2.3.4", 80);
        fail(&mut sort, 11_000, "1.2.3.4", 80);
        sort.update_at(22_000, 22, "1.2.3.4", 80, true);

        assert_eq!(sort.records().len(), 1);
        let record = &sort.records()[0];
        assert_eq!(record.net_info, "wifi");
        assert_eq!(record.time, Some(0), "stamped when the record was made");
        assert_eq!(record.items.len(), 1);
        assert_eq!(record.items[0].history_result, 0b110);

        // another pair, another item
        fail(&mut sort, 33_000, "1.2.3.4", 443);
        assert_eq!(sort.records()[0].items.len(), 2);
        // ... and another network, another record
        fail(&mut sort, 44_000, "1.2.3.4", 80);
        let mut other = a_sort("4g");
        fail(&mut other, 0, "1.2.3.4", 80);
        assert_eq!(other.records().len(), 1);
        assert_eq!(other.records()[0].net_info, "4g");
    }

    #[test]
    fn with_no_network_nothing_is_learned() {
        let mut sort = SimpleIpPortSort::seeded(1);
        fail(&mut sort, 0, "1.2.3.4", 80);
        assert!(sort.records().is_empty());
        assert!(sort.ban_list().is_empty());

        // ... and neither does the history come back
        sort.load_records(
            vec![Record {
                net_info: "wifi".to_string(),
                time: Some(0),
                items: vec![RecordItem {
                    ip: "1.2.3.4".to_string(),
                    port: 80,
                    history_result: 0xFF,
                }],
            }],
            0,
        );
        sort.init_history_to_banned_list();
        assert!(sort.ban_list().is_empty(), "kNoNet");

        // a network with no record of its own learns nothing either
        sort.set_net_label(|| Some("4g".to_string()));
        sort.init_history_to_banned_list();
        assert!(sort.ban_list().is_empty());
    }

    #[test]
    fn the_history_of_the_network_rebuilds_the_ban_list() {
        let mut sort = a_sort("wifi");
        sort.load_records(
            vec![
                Record {
                    net_info: "4g".to_string(),
                    time: Some(0),
                    items: vec![RecordItem {
                        ip: "9.9.9.9".to_string(),
                        port: 80,
                        history_result: 0xFF,
                    }],
                },
                Record {
                    net_info: "wifi".to_string(),
                    time: Some(0),
                    items: vec![
                        RecordItem {
                            ip: "1.2.3.4".to_string(),
                            port: 80,
                            // one byte per attempt, the low one the oldest
                            history_result: 0x01,
                        },
                        RecordItem {
                            ip: "1.2.3.4".to_string(),
                            port: 443,
                            history_result: 0x01_01,
                        },
                    ],
                },
            ],
            0,
        );
        sort.init_history_to_banned_list();

        assert_eq!(sort.ban_list().len(), 2, "only the wifi record");
        // eight attempts ago the one failure there ever was: `0b1000_0000`
        assert_eq!(sort.ban_list()[0].ip, "1.2.3.4");
        assert_eq!(sort.ban_list()[0].records, 0b1000_0000);
        assert_eq!(sort.ban_list()[1].records, 0b1100_0000);
        // the times stay at "never", so history alone never bans a pair
        assert_eq!(sort.ban_list()[0].last_fail_time, None);
        assert!(!sort.is_banned_at(0, "1.2.3.4", 80));

        // ... and a second call starts from nothing again
        sort.init_history_to_banned_list();
        assert_eq!(sort.ban_list().len(), 2);
    }

    #[test]
    fn removing_an_ip_takes_every_port_of_it_out_of_the_ban_list() {
        let mut sort = a_sort("wifi");
        fail(&mut sort, 0, "1.2.3.4", 80);
        fail(&mut sort, 0, "1.2.3.4", 443);
        fail(&mut sort, 0, "5.6.7.8", 80);
        assert_eq!(sort.ban_list().len(), 3);

        sort.remove_banned_list("1.2.3.4");
        assert_eq!(sort.ban_list().len(), 1);
        assert_eq!(sort.ban_list()[0].ip, "5.6.7.8");
        // the records are not touched
        assert_eq!(sort.records()[0].items.len(), 3);

        sort.remove_banned_list("9.9.9.9");
        assert_eq!(sort.ban_list().len(), 1);
    }

    #[test]
    fn a_server_ban_takes_an_ip_out_and_runs_out() {
        let mut sort = a_sort("wifi");
        sort.add_server_ban_at(1_000, "1.2.3.4");
        // an empty ip is ignored
        sort.add_server_ban_at(1_000, "");
        assert_eq!(sort.server_bans.len(), 1);

        let items = vec![
            IpPortItem::new("1.2.3.4", 80),
            IpPortItem::new("5.6.7.8", 80),
        ];
        let items = sort.sort_and_filter_at(1_000, items, 10, false);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].ip, "5.6.7.8");

        // `kServerBanTime` later the ip is back, and forgotten — which only
        // happens if it is offered again, the way `__IsServerBan` erases it
        let items = vec![
            IpPortItem::new("1.2.3.4", 80),
            IpPortItem::new("5.6.7.8", 80),
        ];
        let items = sort.sort_and_filter_at(1_000 + SERVER_BAN_TIME, items, 10, false);
        assert_eq!(items.len(), 2);
        assert!(sort.server_bans.is_empty());
    }

    #[test]
    fn the_banned_pairs_are_filtered_out_and_the_count_is_kept() {
        let mut sort = a_sort("wifi");
        for at in [0, 11_000, 22_000] {
            fail(&mut sort, at, "1.2.3.4", 80);
        }
        let items = vec![
            IpPortItem::new("1.2.3.4", 80),
            IpPortItem::new("1.2.3.4", 443),
            IpPortItem::new("5.6.7.8", 80),
        ];
        let items = sort.sort_and_filter_at(22_000, items, 10, false);
        assert_eq!(items.len(), 2, "the banned pair is gone");
        assert!(items
            .iter()
            .all(|item| item.port != 80 || item.ip != "1.2.3.4"));

        // `_needcount` keeps the first ones only
        let items: Vec<IpPortItem> = (0..5)
            .map(|port| IpPortItem::new("5.6.7.8", port))
            .collect();
        let items = sort.sort_and_filter_at(22_000, items, 2, false);
        assert_eq!(items.len(), 2);
        // ... and `usize::MAX` keeps them all, which is more than the C++'s
        // `resize` of a negative number could do
        let items: Vec<IpPortItem> = (0..5)
            .map(|port| IpPortItem::new("5.6.7.8", port))
            .collect();
        let items = sort.sort_and_filter_at(22_000, items, usize::MAX, false);
        assert_eq!(items.len(), 5);
    }

    #[test]
    fn the_pairs_that_failed_least_come_first() {
        let mut sort = a_sort("wifi");
        // two failures on :80 and one on :443 — below `kBanFailCount`, so both
        // survive, and the shuffle cannot move the order the history is sorted
        // into
        fail(&mut sort, 0, "1.2.3.4", 80);
        fail(&mut sort, 11_000, "1.2.3.4", 80);
        fail(&mut sort, 22_000, "5.6.7.8", 443);

        let items = vec![
            IpPortItem::new("1.2.3.4", 80),
            IpPortItem::new("5.6.7.8", 443),
        ];
        let items = sort.sort_and_filter_at(33_000, items, 10, false);
        assert_eq!(
            items.iter().map(|item| item.port).collect::<Vec<_>>(),
            vec![443, 80],
            "one failure before two"
        );
        assert_eq!(sort.ban_list().len(), 2);
    }

    #[test]
    fn the_two_families_are_tried_in_turn_when_ipv6_is_used() {
        let mut sort = a_sort("wifi");
        let items = vec![
            IpPortItem::new("1.2.3.4", 80),
            IpPortItem::new("1.2.3.4", 443),
            IpPortItem::new("2001:db8::1", 80),
            IpPortItem::new("2001:db8::2", 80),
        ];
        let items = sort.sort_and_filter_at(0, items, 10, true);
        assert_eq!(items.len(), 4);
        // one of each, back and forth
        let v6: Vec<bool> = items.iter().map(|item| item.is_v6()).collect();
        assert_eq!(v6, vec![true, false, true, false]);

        // ... and with nothing but one family left, they all come back
        let items = vec![
            IpPortItem::new("1.2.3.4", 80),
            IpPortItem::new("1.2.3.4", 443),
        ];
        let items = sort.sort_and_filter_at(0, items, 10, true);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn a_host_with_an_rng_of_its_own_decides_the_shuffle() {
        let mut sort = a_sort("wifi");
        // always the first of the `0..bound` the C++'s `rand() % bound` picks
        // from
        sort.set_random(|_bound| 0);
        let items: Vec<IpPortItem> = (0..4)
            .map(|port| IpPortItem::new("5.6.7.8", port))
            .collect();
        let items = sort.sort_and_filter_at(0, items, 10, false);
        assert_eq!(
            items.iter().map(|item| item.port).collect::<Vec<_>>(),
            vec![1, 2, 3, 0],
            "the shuffle moved the last one to the front and then left it there"
        );
    }

    #[test]
    fn the_records_that_timed_out_are_gone() {
        let mut sort = a_sort("wifi");
        sort.load_records(
            vec![
                Record {
                    net_info: "wifi".to_string(),
                    time: Some(0),
                    items: Vec::new(),
                },
                Record {
                    net_info: "4g".to_string(),
                    time: None,
                    items: Vec::new(),
                },
                Record {
                    net_info: "5g".to_string(),
                    time: Some(RECORD_TIMEOUT),
                    items: Vec::new(),
                },
            ],
            100,
        );
        assert_eq!(sort.records().len(), 1, "the 4g one has no `time`");
        assert_eq!(sort.records()[0].net_info, "wifi");

        // a record from the future is dropped too
        sort.load_records(
            vec![Record {
                net_info: "wifi".to_string(),
                time: Some(RECORD_TIMEOUT),
                items: Vec::new(),
            }],
            0,
        );
        assert!(sort.records().is_empty());

        // ... and so is one that is a day old when the host saves
        sort.load_records(
            vec![Record {
                net_info: "wifi".to_string(),
                time: Some(0),
                items: Vec::new(),
            }],
            0,
        );
        assert_eq!(sort.save_records(RECORD_TIMEOUT - 1).len(), 1);
        assert!(sort.save_records(RECORD_TIMEOUT).is_empty());
        assert!(sort.records().is_empty(), "saving dropped it");
    }

    #[test]
    fn the_default_sort_has_no_network_and_a_seeded_rng() {
        let mut sort = SimpleIpPortSort::default();
        assert!(sort.records().is_empty());
        assert!(sort.ban_list().is_empty());
        assert!(format!("{sort:?}").contains("SimpleIpPortSort"));
        // and it is usable without a host at all
        let items = vec![IpPortItem::new("1.2.3.4", 80)];
        let items = sort.sort_and_filter(items, 10, false);
        assert_eq!(items.len(), 1);
        sort.update(0, "1.2.3.4", 80, true);
        assert!(sort.records().is_empty(), "no network, nothing learned");
        sort.add_server_ban("1.2.3.4");
        assert_eq!(sort.server_bans.len(), 1);
    }
}
