//! `mars/stn/src/longlink.cc` — the long link and its connect, through the
//! public api.
//!
//! The samples are what the C++ answers for the same calls: a first candidate
//! on 443 that never answered and a second on 80 that did (`tried_443port` is
//! the one the C++ reports), the `OnResponse` a connect with no candidates
//! gets, a link the app took down itself, which is the one connect failure
//! that is *not* answered, and a minor long link on a debug ip, which is the
//! one that never goes through a proxy.
//!
//! The heartbeat is the same: the noop that goes out when the interval is up
//! and the eight seconds it has to answer in, the one the app asks for, which
//! is not the one the interval asked for, the identify check the first of them
//! carries, and the two [`NoopProfile`]s a run that answered twice leaves on
//! the profile.

use std::sync::{Arc, Mutex};

use mars_comm::local_ipstack::LocalIpStack;
use mars_comm::{ProxyInfo, ProxyType, SocketAddress};
use mars_stn::{
    AlarmStatus, ConnectFail, DisconnectInternalCode, ErrCmdType, IdentifyBuffer, IpPortItem,
    IpSourceType, LongLink, LongLinkStatus, LonglinkConfig, MakeSure, NoopProfile, OpBreaker,
    SmartHeartbeat, SocketFd, SocketOperator, SocketProfile, Task, ECT_DNS_MAKE_SOCKET_PREPARED,
    ECT_SOCKET_MAKE_SOCKET_PREPARED,
};

/// A host's `OPBreaker`: the port has nothing blocking to give up.
struct Breaker {
    broken: bool,
}

impl OpBreaker for Breaker {
    fn is_break(&mut self) -> bool {
        self.broken
    }

    fn break_(&mut self) -> bool {
        self.broken = true;
        true
    }
}

/// What the host was asked for: the addresses and the proxy of every connect,
/// what went out on the socket, and what its answers were.
#[derive(Default)]
struct Seen {
    addresses: Vec<Vec<String>>,
    proxies: Vec<Option<ProxyInfo>>,
    sent: Vec<Vec<u8>>,
}

type Recorder = Arc<Mutex<Seen>>;

/// A host's `TcpSocketOperator`: the sockets are the numbers it hands out, the
/// answer of a connect is the [`SocketProfile`] it was given, and everything it
/// reads is the [`Seen::replies`] it was told to answer with.
struct Host {
    record: Recorder,
    next: i64,
    profile: SocketProfile,
    replies: Vec<Vec<u8>>,
    breaker: Breaker,
}

impl Host {
    fn new(record: Recorder) -> Self {
        Self {
            record,
            next: 3,
            profile: SocketProfile::default(),
            replies: Vec::new(),
            breaker: Breaker { broken: false },
        }
    }
}

impl SocketOperator for Host {
    fn connect(&mut self, addresses: &[SocketAddress], proxy: &ProxyInfo) -> SocketFd {
        let mut seen = self.record.lock().unwrap_or_else(|e| e.into_inner());
        seen.addresses.push(
            addresses
                .iter()
                .map(|address| address.url().to_string())
                .collect(),
        );
        seen.proxies.push(if proxy.kind.is_none() {
            None
        } else {
            Some(proxy.clone())
        });
        if self.profile.error_code != 0 {
            return SocketFd::INVALID;
        }
        let socket = SocketFd(self.next);
        self.next += 1;
        socket
    }

    fn send(&mut self, _socket: SocketFd, buffer: &[u8], _timeout_ms: i32) -> Result<usize, i32> {
        self.record
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sent
            .push(buffer.to_vec());
        Ok(buffer.len())
    }

    fn recv(
        &mut self,
        _socket: SocketFd,
        _max_size: usize,
        _timeout_ms: i32,
        _wait_full_size: bool,
    ) -> Result<Vec<u8>, i32> {
        // what the C++ reads off a socket: the head of what is left
        Ok(self.replies.first().cloned().unwrap_or_default())
    }

    fn close(&mut self, _socket: SocketFd) {}

    fn identify(&self, socket: SocketFd) -> String {
        mars_stn::tcp_identify(socket)
    }

    fn protocol(&self) -> i32 {
        Task::TRANSPORT_PROTOCOL_TCP
    }

    fn error_desc(&self, error_code: i32) -> String {
        format!("error {error_code}")
    }

    fn profile(&self) -> SocketProfile {
        self.profile
    }

    fn breaker(&mut self) -> &mut dyn OpBreaker {
        &mut self.breaker
    }

    fn create_stream(&mut self, socket: SocketFd) -> SocketFd {
        SocketFd(socket.0 + 100)
    }

    fn set_ip_connection_timeout(&mut self, _v4: u32, _v6: u32) {}
}

fn item(ip: &str, port: u16, host: &str) -> IpPortItem {
    IpPortItem {
        ip: ip.to_string(),
        port,
        host: host.to_string(),
        source_type: IpSourceType::Dns,
        ..IpPortItem::new(ip, port)
    }
}

/// A long link with two candidates — `1.1.1.1:443` and `2.2.2.2:80` — and the
/// host's record of what it does.
fn a_longlink() -> (LongLink, Recorder) {
    let record: Recorder = Arc::new(Mutex::new(Seen::default()));
    let mut link = LongLink::new(LonglinkConfig::new("long.example"));
    link.set_longlink_items(|_| {
        vec![
            item("1.1.1.1", 443, "long.example"),
            item("2.2.2.2", 80, "long.example"),
        ]
    });
    link.set_local_ip_stack(|| LocalIpStack::IPv4);
    link.set_net_label(|| "wifi".to_string());
    link.set_socket_operator(Host::new(Arc::clone(&record)));
    (link, record)
}

#[test]
fn a_connect_is_made_on_the_candidate_that_answered() {
    let (mut link, record) = a_longlink();
    link.set_socket_operator(Host {
        // the second candidate answered, in 30ms, after 90ms of trying
        profile: SocketProfile {
            rtt: 30,
            index: 1,
            total_cost: 90,
            ..SocketProfile::default()
        },
        ..Host::new(Arc::clone(&record))
    });
    link.set_local_address(|_| SocketAddress::new("10.0.0.1", 40_000));

    let said: Arc<Mutex<Vec<(LongLinkStatus, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let record_status = Arc::clone(&said);
    link.set_on_connection(move |status, name| {
        record_status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((status, name.to_string()))
    });

    assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: true });
    assert_eq!(link.connect_at(1_000), Ok(SocketFd(3)));
    assert_eq!(link.connect_status(), LongLinkStatus::Connected);

    let profile = link.profile();
    assert_eq!(profile.ip, "2.2.2.2");
    assert_eq!(profile.port, 80);
    assert_eq!(profile.host, "long.example");
    assert_eq!(profile.ip_index, 1);
    // the candidate that lost was on 443
    assert_eq!(profile.tried_443port, 1);
    assert_eq!(profile.tried_80port, 0);
    assert_eq!(profile.local_ip, "10.0.0.1");
    assert_eq!(profile.local_port, 40_000);
    assert_eq!(profile.conn_rtt, 30);
    assert_eq!(profile.conn_cost, 90);
    assert_eq!(profile.net_type, "wifi");
    assert_eq!(profile.ip_items.len(), 2);
    assert_eq!(profile.start_time, 1_000);
    assert_eq!(profile.dns_time, 1_000);
    assert_eq!(profile.dns_endtime, 1_000);
    assert!(!profile.nat64);

    // and both candidates were handed to the host, in order
    let seen = record.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(seen.addresses[0], vec!["1.1.1.1:443", "2.2.2.2:80"]);
    drop(seen);
    assert_eq!(
        *said.lock().unwrap_or_else(|e| e.into_inner()),
        vec![
            (LongLinkStatus::Connecting, "long.example".to_string()),
            (LongLinkStatus::Connected, "long.example".to_string()),
        ]
    );
}

#[test]
fn a_link_that_is_up_is_not_made_again() {
    let (mut link, _) = a_longlink();
    // one run: the first call starts it, and the rest are told to carry on
    assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: true });
    assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: false });
    assert!(link.connect_at(1_000).is_ok());

    assert_eq!(link.make_sure_connected(), MakeSure::Connected);

    // and a run that is over starts a new one from the beginning: the link it
    // was made on is gone, so the next one is a connect
    link.set_status(LongLinkStatus::ConnectFailed);
    link.end_run();
    assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: true });
    assert_eq!(link.connect_status(), LongLinkStatus::ConnectIdle);
    assert_eq!(link.profile().ip, "", "the profile was reset");
    assert_eq!(link.disconnect_code(), DisconnectInternalCode::None);
}

#[test]
fn a_connect_with_no_candidate_is_answered_with_dns() {
    let mut link = LongLink::new(LonglinkConfig::new("long.example"));
    let responses: Arc<Mutex<Vec<(ErrCmdType, i32)>>> = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&responses);
    link.set_on_response(move |_name, err_type, err_code, _profile| {
        record
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((err_type, err_code))
    });

    assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: true });
    assert_eq!(link.connect_at(1_000), Err(ConnectFail::NoAddress));
    assert_eq!(link.connect_status(), LongLinkStatus::ConnectFailed);
    assert_eq!(
        *responses.lock().unwrap_or_else(|e| e.into_inner()),
        vec![(ErrCmdType::Dns, ECT_DNS_MAKE_SOCKET_PREPARED)]
    );
}

#[test]
fn a_link_the_app_took_down_does_not_answer_the_connect() {
    let (mut link, record) = a_longlink();
    link.set_socket_operator(Host {
        profile: SocketProfile {
            error_code: -10087,
            ..SocketProfile::default()
        },
        ..Host::new(Arc::clone(&record))
    });
    let responses: Arc<Mutex<Vec<(ErrCmdType, i32)>>> = Arc::new(Mutex::new(Vec::new()));
    let said = Arc::clone(&responses);
    link.set_on_response(move |_name, err_type, err_code, _profile| {
        said.lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((err_type, err_code))
    });

    assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: true });
    // `kNetworkChange` is the app's own: the connect is not answered
    link.disconnect(DisconnectInternalCode::NetworkChange);
    assert_eq!(
        link.connect_at(1_000),
        Err(ConnectFail::Connect { error_code: -10087 })
    );
    assert_eq!(link.connect_status(), LongLinkStatus::ConnectFailed);
    assert!(responses
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty());

    // ... and one nobody took down is
    link.end_run();
    assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: true });
    assert_eq!(
        link.connect_at(2_000),
        Err(ConnectFail::Connect { error_code: -10087 })
    );
    assert_eq!(
        *responses.lock().unwrap_or_else(|e| e.into_inner()),
        vec![(ErrCmdType::Socket, ECT_SOCKET_MAKE_SOCKET_PREPARED)]
    );
}

#[test]
fn a_proxy_is_used_unless_the_link_is_a_minor_one_on_a_debug_ip() {
    let proxy = || ProxyInfo::new(ProxyType::Socks5, "", "10.0.0.2", 1080, "", "");

    let (mut link, record) = a_longlink();
    link.set_proxy(proxy);
    link.make_sure_connected();
    assert!(link.connect_at(1_000).is_ok());
    assert_eq!(
        record.lock().unwrap_or_else(|e| e.into_inner()).proxies[0]
            .as_ref()
            .unwrap()
            .ip,
        "10.0.0.2"
    );

    // a minor long link on a debug ip goes where the debug ip says
    let mut config = LonglinkConfig::new("minor.example");
    config.link_type = Task::CHANNEL_MINOR_LONG;
    let record: Recorder = Arc::new(Mutex::new(Seen::default()));
    let mut link = LongLink::new(config);
    link.set_longlink_items(|_| vec![item("1.1.1.1", 80, "minor.example")]);
    link.set_socket_operator(Host::new(Arc::clone(&record)));
    link.set_proxy(proxy);
    link.set_minorlong_debug_ip(|| "9.9.9.9".to_string());
    link.make_sure_connected();
    assert!(link.connect_at(1_000).is_ok());
    assert!(record.lock().unwrap_or_else(|e| e.into_inner()).proxies[0].is_none());
}

#[test]
fn the_heartbeat_is_told_the_network_the_link_came_up_on() {
    let (mut link, _) = a_longlink();
    link.set_smart_heartbeat(SmartHeartbeat::new());
    link.set_net_type(|| 1);
    link.make_sure_connected();
    assert!(link.connect_at(1_000).is_ok());

    let heartbeat = link.heartbeat().unwrap();
    assert_eq!(heartbeat.info().net_detail, "wifi");
    assert_eq!(heartbeat.info().net_type, 1);
}

#[test]
fn a_link_that_was_released_will_not_run_again() {
    let (mut link, _) = a_longlink();
    assert_eq!(link.make_sure_connected(), MakeSure::Run { new_one: true });
    link.release();
    assert_eq!(
        link.disconnect_code(),
        DisconnectInternalCode::ObjectDestruct
    );
    assert_eq!(link.make_sure_connected(), MakeSure::Released);
    // ... and a link nobody asked to go away is not
    assert!(!DisconnectInternalCode::None.is_set());
}

#[test]
fn the_verification_of_a_connect_is_a_noop_the_server_answers() {
    let (mut link, record) = a_longlink();
    link.set_socket_operator(Host {
        // what the server answers to a noop: the same package back
        replies: vec![mars_stn::longlink::longlink_pack(
            mars_stn::longlink::NOOP_CMDID,
            Task::NOOP_TASK_ID,
            &[],
        )],
        ..Host::new(Arc::clone(&record))
    });
    assert!(link.verify(SocketFd(3)));

    let seen = record.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        seen.sent[0],
        mars_stn::longlink::longlink_pack(mars_stn::longlink::NOOP_CMDID, Task::NOOP_TASK_ID, &[])
    );
    drop(seen);

    // and an answer that is not a package is not an answer
    link.set_socket_operator(Host::new(Arc::clone(&record)));
    assert!(!link.verify(SocketFd(4)));
}

/// A link that is up, which is what every heartbeat starts from.
fn a_connected_longlink() -> (LongLink, Recorder) {
    let (mut link, record) = a_longlink();
    link.make_sure_connected();
    assert!(link.connect_at(1_000).is_ok());
    (link, record)
}

/// What the host's run does with what it wrote: the whole of what is at the
/// head of the queue has gone out.
fn wrote(link: &mut LongLink) {
    let len = link
        .queued()
        .front()
        .map(|data| data.buffer.len())
        .unwrap_or(0);
    link.wrote(len);
}

#[test]
fn the_heartbeat_goes_out_on_the_socket_and_its_answer_ends_it() {
    let (mut link, _) = a_connected_longlink();

    assert!(link.send_heartbeat_at(1_000, false, false));
    // the noop: `kNoopTaskID` with `kNoopCmdID`, waiting for the host's run to
    // write it
    assert_eq!(
        link.queued()[0].buffer,
        mars_stn::longlink::longlink_pack(mars_stn::longlink::NOOP_CMDID, Task::NOOP_TASK_ID, &[])
    );
    assert!(link.is_nooping());
    // and it has eight seconds to answer
    assert_eq!(link.noop_timeout_due(), Some(1_000 + 8 * 1000));

    // what the server answers to it, 500 later
    assert!(link.noop_resp_at(
        1_500,
        mars_stn::longlink::NOOP_CMDID,
        Task::NOOP_TASK_ID,
        &[]
    ));
    assert!(!link.is_nooping());
    assert_eq!(link.noop_timeout_due(), None, "the alarm was cancelled");
}

#[test]
fn the_heartbeats_of_a_run_are_what_the_profile_reports() {
    let (mut link, _) = a_connected_longlink();
    link.set_smart_heartbeat(SmartHeartbeat::new());

    // the first heartbeat of a run, and its answer 500 later
    assert!(link.send_heartbeat_at(1_000, false, false));
    assert!(link.noop_resp_at(
        1_500,
        mars_stn::longlink::NOOP_CMDID,
        Task::NOOP_TASK_ID,
        &[]
    ));
    // ... and the second one, one interval later
    wrote(&mut link);
    assert!(link.send_heartbeat_at(1_000 + 210_000, false, false));
    assert!(link.noop_resp_at(
        1_000 + 210_000 + 700,
        mars_stn::longlink::NOOP_CMDID,
        Task::NOOP_TASK_ID,
        &[]
    ));

    assert_eq!(
        link.profile().noop_profiles,
        vec![
            NoopProfile {
                success: true,
                noop_internal: 0,
                noop_actual_internal: 0,
                noop_cost: 500,
                noop_starttime: 1_000,
            },
            // `after` is the interval the alarm was set to, and the actual one
            // is what it really was: the same, because the reading came on time
            NoopProfile {
                success: true,
                noop_internal: 210_000,
                noop_actual_internal: 210_000,
                noop_cost: 700,
                noop_starttime: 211_000,
            },
        ]
    );
}

#[test]
fn the_heartbeat_the_app_asks_for_is_not_the_one_the_interval_asked_for() {
    let (mut link, _) = a_connected_longlink();
    let said: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));
    let alarms = Arc::clone(&said);
    link.set_on_noop_alarm_received(move |noop_timeout| {
        alarms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(noop_timeout)
    });

    // `TrigNoop`, which is what the app's `trigNooping` is
    link.trig_noop_at(2_000);
    assert!(link.is_nooping());
    assert_eq!(
        link.queued()[0].buffer,
        mars_stn::longlink::longlink_pack(mars_stn::longlink::NOOP_CMDID, Task::NOOP_TASK_ID, &[])
    );
    // the interval alarm is not what this heartbeat is waiting on
    assert_eq!(link.noop_due(), None);

    // ... and when the timeout goes off, the link is told which one it was
    assert!(!link.on_noop_alarm_at(2_000 + 8 * 1000 - 1, true));
    assert!(link.on_noop_alarm_at(2_000 + 8 * 1000, true));
    assert_eq!(link.noop_timeout_status(), AlarmStatus::OnAlarm);
    assert_eq!(*said.lock().unwrap_or_else(|e| e.into_inner()), vec![true]);
    assert!(link.is_nooping(), "the heartbeat is still out");
}

#[test]
fn the_identify_check_is_what_the_first_heartbeat_carries() {
    let (mut link, _) = a_connected_longlink();
    let said: Arc<Mutex<Vec<(ErrCmdType, i32)>>> = Arc::new(Mutex::new(Vec::new()));
    let reported = Arc::clone(&said);
    link.set_network_report(move |err_type, err_code, _ip, _port| {
        reported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((err_type, err_code))
    });
    link.set_identify_check_buffer(|_channel, _cmdid| IdentifyBuffer::now(vec![7], vec![7], 42));
    link.set_identify_on_response(|_channel, response, hash| response == hash);

    assert!(link.send_heartbeat_at(1_000, false, false));
    assert_eq!(
        link.queued()[0].buffer,
        mars_stn::longlink::longlink_pack(42, Task::LONG_LINK_IDENTIFY_CHECKER_TASK_ID, &[7])
    );

    // the answer the server sent back is the hash the app handed out
    assert!(link.noop_resp_at(1_100, 42, Task::LONG_LINK_IDENTIFY_CHECKER_TASK_ID, &[7]));
    assert!(link.identify().has_checked());
    assert!(!link.is_nooping());
    // a link the server identified is reported as one that came up
    assert_eq!(
        *said.lock().unwrap_or_else(|e| e.into_inner()),
        vec![(ErrCmdType::Ok, 0)]
    );

    // ... and the next heartbeat is a plain noop
    wrote(&mut link);
    assert!(link.send_heartbeat_at(2_000, false, false));
    assert_eq!(link.queued()[0].task.taskid, Task::NOOP_TASK_ID);
}
