//! `mars/stn/src/longlink_metadata.cc` — a long link, and the three things that
//! keep it up, in one value.
//!
//! The C++'s `LongLinkMetaData` is not logic: it is the bundle a `NetCore` keeps
//! one of per long link — the link itself, the monitor that decides when it is
//! made again, the timer check that gets it off a backup ip, and the keeper that
//! holds a mapping up while the app waits — plus the one thing of its own it
//! does, `on_timer_check_suc`: a check that found a better pair takes the
//! link down, but only when the pair it is on is a backup one.
//!
//! Which is to say it is the *wiring*: the three helpers ask the link things,
//! and in the C++ they hold a reference to it. A reference is not something a
//! Rust value can hold on to while the link is also a field of it, and the
//! callbacks the port's helpers take are `Send`, so the link is one
//! [`Arc<Mutex<LongLink>>`] the metadata and all three of its helpers share.
//!
//! The wiring the C++ does with the *net source* and the *active logic* is not
//! here: those are the app's, and the host wires them on the helpers it gets
//! from [`LongLinkMetaData::monitor`], [`LongLinkMetaData::checker`] and
//! [`LongLinkMetaData::keeper`].

use std::sync::{Arc, Mutex};

use crate::long_link::{DisconnectInternalCode, LongLink, MakeSure};
use crate::{
    IpSourceType, LongLinkConnectMonitor, LongLinkStatus, LonglinkConfig, NetSourceTimerCheck,
    SignallingKeeper, Task,
};

/// `LongLinkMetaData` — one long link and the three things that keep it up.
pub struct LongLinkMetaData {
    /// `longlink_` — shared with the three helpers below, which ask it things
    /// through the callbacks this wired for them.
    link: Arc<Mutex<LongLink>>,
    /// `longlink_monitor_`.
    monitor: LongLinkConnectMonitor,
    /// `netsource_checker_`.
    checker: NetSourceTimerCheck,
    /// `signal_keeper_`.
    keeper: SignallingKeeper,
    /// `config_`.
    config: LonglinkConfig,
}

impl LongLinkMetaData {
    /// `LongLinkMetaData(…, _config, …)` — the link, and the three helpers wired
    /// to it.
    ///
    /// The link is the one the app's own factory made, which is
    /// [`crate::ChannelFactory::create_longlink`]: the C++ calls
    /// `LongLinkChannelFactory::Create` from this constructor, and the port
    /// leaves the making of it to whoever has the factory.
    pub fn new(config: LonglinkConfig, link: LongLink) -> Self {
        let link = Arc::new(Mutex::new(link));
        let is_keep_alive = config.is_keep_alive;

        let mut monitor = LongLinkConnectMonitor::new(is_keep_alive);
        {
            let link = Arc::clone(&link);
            monitor.set_is_svr_trig_off(move || {
                link.lock()
                    .unwrap_or_else(poisoned)
                    .is_server_triggered_off()
            });
        }
        {
            let link = Arc::clone(&link);
            monitor
                .set_connect_status(move || link.lock().unwrap_or_else(poisoned).connect_status());
        }
        {
            let link = Arc::clone(&link);
            monitor.set_dns_time(move || link.lock().unwrap_or_else(poisoned).profile().dns_time);
        }
        {
            let link = Arc::clone(&link);
            monitor.set_make_sure_connected(move || {
                // `MakeSureConnected(&newone)`: the C++ reads the `bool` it
                // answers with and throws away the `newone` it wrote
                matches!(
                    link.lock().unwrap_or_else(poisoned).make_sure_connected(),
                    MakeSure::Connected
                )
            });
        }
        {
            let link = Arc::clone(&link);
            monitor.set_disconnect(move || {
                link.lock()
                    .unwrap_or_else(poisoned)
                    .disconnect(DisconnectInternalCode::NetworkChange)
            });
        }

        let mut checker = NetSourceTimerCheck::new();
        {
            let link = Arc::clone(&link);
            checker.set_ip_type(move || link.lock().unwrap_or_else(poisoned).profile().ip_type);
        }
        {
            let link = Arc::clone(&link);
            checker.set_host(move || link.lock().unwrap_or_else(poisoned).profile().host.clone());
        }
        {
            let link = Arc::clone(&link);
            checker.set_ip(move || link.lock().unwrap_or_else(poisoned).profile().ip.clone());
        }
        {
            let link = Arc::clone(&link);
            checker.set_on_time_check_suc(move || on_timer_check_suc(&link));
        }

        let mut keeper = SignallingKeeper::new();
        {
            let link = Arc::clone(&link);
            keeper.set_send(move |cmdid| {
                // `LongLink::SendWhenNoData(_1, _2, _3,
                // Task::kSignallingKeeperTaskID)`: both buffers are the empty
                // `KNullAtuoBuffer`, and what it answers with
                // `__SendSignallingBuffer` throws away
                link.lock().unwrap_or_else(poisoned).send_when_no_data(
                    cmdid,
                    Task::SIGNALLING_KEEPER_TASK_ID,
                    &[],
                );
                0
            });
        }

        Self {
            link,
            monitor,
            checker,
            keeper,
            config,
        }
    }

    /// `Channel()` — the link, shared with the three helpers: what the C++'s
    /// `std::shared_ptr<LongLink>` is.
    pub fn channel(&self) -> &Arc<Mutex<LongLink>> {
        &self.link
    }

    /// `Monitor()`.
    pub fn monitor(&mut self) -> &mut LongLinkConnectMonitor {
        &mut self.monitor
    }

    /// `Checker()`.
    pub fn checker(&mut self) -> &mut NetSourceTimerCheck {
        &mut self.checker
    }

    /// `SignalKeeper()`.
    pub fn keeper(&mut self) -> &mut SignallingKeeper {
        &mut self.keeper
    }

    /// `Config()`.
    pub fn config(&self) -> &LonglinkConfig {
        &self.config
    }

    /// `IsConnected()` — whether the link is up, which is the one question the
    /// C++ asks of it without going through a helper.
    pub fn is_connected(&self) -> bool {
        self.link.lock().unwrap_or_else(poisoned).connect_status() == LongLinkStatus::Connected
    }
}

impl std::fmt::Debug for LongLinkMetaData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LongLinkMetaData")
            .field("name", &self.config.name)
            .field("is_keep_alive", &self.config.is_keep_alive)
            .finish()
    }
}

/// A link whose lock a panic left poisoned is still the link: nothing here holds
/// it across a call that could panic twice.
fn poisoned<T>(poisoned: std::sync::PoisonError<T>) -> T {
    poisoned.into_inner()
}

/// `__OnTimerCheckSuc` — the timer check found a pair that answers, so the link
/// is taken down to be made on it again.
///
/// Only a link on a *backup* pair is: one on a pair dns gave it is where the
/// check wanted it anyway, and the C++ leaves it alone.
fn on_timer_check_suc(link: &Arc<Mutex<LongLink>>) {
    let mut link = link.lock().unwrap_or_else(poisoned);
    if link.profile().ip_type != IpSourceType::Backup {
        return;
    }
    link.disconnect(DisconnectInternalCode::TimeCheckSucc);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IpPortItem;

    /// The two readings the tests use: the link asked for its ips at `50_000`,
    /// and the monitor is asked at `60_000` — ten seconds later, which is the
    /// first rung of the reconnect ladder and therefore a connect that is due.
    const DNS_AT: u64 = 50 * 1000;
    const NOW: u64 = 60 * 1000;

    fn config() -> LonglinkConfig {
        LonglinkConfig::new("long.weixin.qq.com")
    }

    fn meta() -> LongLinkMetaData {
        let config = config();
        LongLinkMetaData::new(config.clone(), LongLink::new(config))
    }

    /// `MakeSureConnected` on the link the metadata shares: what leaves a run in
    /// flight, which is what [`LongLink::disconnect`] needs to write a scene.
    fn start_a_run(meta: &LongLinkMetaData) {
        let _ = meta.channel().lock().unwrap().make_sure_connected();
    }

    /// The link, connected onto `ip` as a pair of the given kind — which is what
    /// the profile's `ip_type`, `ip` and `host` are afterwards.
    fn connect_on(meta: &LongLinkMetaData, ip: &str, source: IpSourceType) {
        let ip = ip.to_string();
        let mut link = meta.channel().lock().unwrap();
        link.set_longlink_items(move |_config| {
            vec![IpPortItem {
                ip: ip.clone(),
                port: 80,
                source_type: source,
                host: "long.weixin.qq.com".to_string(),
                transport_protocol: Task::TRANSPORT_PROTOCOL_TCP,
                from_source: 0,
            }]
        });
        // no socket operator, so the connect fails — but the pair it was given
        // is the one the profile names
        let _ = link.connect_at(DNS_AT);
        assert_eq!(link.profile().ip_type, source, "the link is on the pair");
    }

    #[test]
    fn a_link_that_is_not_up_is_not_one_the_metadata_calls_connected() {
        let meta = meta();
        assert!(!meta.is_connected());
        assert_eq!(meta.config().name, "long.weixin.qq.com");
        assert!(!meta.config().is_keep_alive);
    }

    #[test]
    fn the_monitor_is_made_the_way_the_config_asks() {
        let mut config = config();
        config.is_keep_alive = true;
        let mut meta = LongLinkMetaData::new(config.clone(), LongLink::new(config));
        assert!(meta.monitor().is_keep_alive());
        assert!(meta.config().is_keep_alive);
    }

    #[test]
    fn a_monitor_that_waited_long_enough_makes_the_link_it_shares_connected() {
        let mut meta = meta();
        assert!(
            !meta.channel().lock().unwrap().is_running(),
            "no run was started yet"
        );

        assert!(!meta.monitor().make_sure_connected_at(NOW));
        assert!(
            meta.channel().lock().unwrap().is_running(),
            "the monitor asked the link it shares for a run"
        );
    }

    #[test]
    fn a_link_that_asked_for_its_ips_a_moment_ago_is_not_made_again_yet() {
        let mut meta = meta();
        start_a_run(&meta);
        connect_on(&meta, "1.1.1.1", IpSourceType::Dns);

        assert!(!meta.monitor().make_sure_connected_at(NOW));
        // `60s - 50s` is under the threshold, so the ladder went *up* and the
        // monitor is waiting out a rung instead of connecting
        assert!(meta.monitor().current_interval_index() > 0);
    }

    #[test]
    fn a_monitor_reads_the_status_of_the_link_off_the_link_itself() {
        let mut meta = meta();
        assert!(!meta.monitor().make_sure_connected_at(NOW));

        meta.channel()
            .lock()
            .unwrap()
            .set_status(LongLinkStatus::Connected);
        assert!(
            meta.monitor().make_sure_connected_at(NOW),
            "the link is up, which is what the monitor asked it"
        );
        assert!(meta.is_connected());
    }

    #[test]
    fn a_network_change_takes_the_link_the_monitor_shares_down() {
        let mut meta = meta();
        start_a_run(&meta);

        let _ = meta.monitor().network_change_at(NOW);
        assert_eq!(
            meta.channel().lock().unwrap().disconnect_code(),
            DisconnectInternalCode::NetworkChange,
            "the monitor took the link down on the change"
        );
    }

    #[test]
    fn a_timer_check_that_succeeded_takes_a_backup_link_down() {
        let mut meta = meta();
        start_a_run(&meta);
        connect_on(&meta, "1.1.1.1", IpSourceType::Backup);

        // the host's own: which pairs there are, and whether one answers
        meta.checker().set_dns(|_host| vec!["2.2.2.2".to_string()]);
        meta.checker().set_longlink_ports(|| vec![80]);
        meta.checker().set_speed_test(|_ip, _port| true);
        meta.checker().start_check();

        assert_eq!(meta.checker().check(), Some(true));
        assert_eq!(
            meta.channel().lock().unwrap().disconnect_code(),
            DisconnectInternalCode::TimeCheckSucc,
            "the link was taken down for the pair the check found"
        );
    }

    #[test]
    fn a_timer_check_leaves_a_link_that_is_not_on_a_backup_pair_up() {
        let mut meta = meta();
        start_a_run(&meta);
        connect_on(&meta, "1.1.1.1", IpSourceType::Dns);

        meta.checker().set_dns(|_host| vec!["2.2.2.2".to_string()]);
        meta.checker().set_longlink_ports(|| vec![80]);
        meta.checker().set_speed_test(|_ip, _port| true);
        meta.checker().start_check();

        assert_eq!(
            meta.checker().check(),
            None,
            "a check of a link that is where dns wanted it finds nothing"
        );
        assert_eq!(
            meta.channel().lock().unwrap().disconnect_code(),
            DisconnectInternalCode::None,
            "and it leaves the link alone"
        );
    }

    #[test]
    fn the_keeper_sends_on_the_link_it_shares_with_the_metadata() {
        let mut meta = meta();
        meta.channel()
            .lock()
            .unwrap()
            .set_status(LongLinkStatus::Connected);

        meta.keeper().keep();
        assert_eq!(meta.keeper().sent(), 1, "one buffer went out");
        // and it went out on the link, which queued it
        assert!(meta.channel().lock().unwrap().has_data_to_send());
    }

    #[test]
    fn the_metadata_is_the_link_and_the_config_it_was_made_with() {
        let meta = meta();
        assert!(format!("{meta:?}").contains("long.weixin.qq.com"));
        assert_eq!(
            meta.channel().lock().unwrap().config().name,
            meta.config().name
        );
    }

    #[test]
    fn the_task_the_keeper_sends_with_is_the_one_mars_keeps_for_it() {
        assert_eq!(Task::SIGNALLING_KEEPER_TASK_ID, 0xffff_fffd);
    }
}
