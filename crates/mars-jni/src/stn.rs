//! `io.github.marsrs.stn.StnLogic` — what the `native` methods of
//! `com/tencent/mars/stn/StnLogic.java` reach.
//!
//! The C++ (`mars/stn/jni/com_tencent_mars_stn_StnLogic_Java2C.cc`) forwards
//! every Java call to a free function of `mars/stn/stn_logic.h`, and those
//! reach the `NetCore` the process keeps. So does this module: the calls are the
//! same, with the same arguments in the same order, and what they reach is
//! [`mars_stn::StnLogic`] — one value for the whole process, made the first time
//! Java asks for it, which is the C++'s `NetCore` singleton.
//!
//! The app STN talks to is the Java side, so [`crate::stn::set_callback_impl`]
//! is where it is handed over; the one that asks it through the JVM is a later
//! slice, and until then the app is whatever the host installs.
//!
//! What the C++'s own platform does — the threads that run the two queues and
//! the long links — is the host's here too, so a task Java starts is one that
//! waits in its queue until a host drains it
//! ([`mars_stn::StnLogic::run_pending`]).
//!
//! Everything the JVM touches lives in [`crate::jni_bridge`]; what is here is
//! plain Rust and is covered by `cargo test`.

use std::sync::{Mutex, OnceLock};

use mars_stn::{App, StnLogic, Task};

/// `getLoadLibraries` — what the C++ lists: the modules the process loaded,
/// which in this port is this one library.
pub const LOAD_LIBRARIES: &[&str] = &["marsxlog"];

/// `Task::kReservedTaskIDStart` of `mars/stn/stn.h`: the ids from here up are
/// taken by the noop, the long-link identify check and the signalling keeper, so
/// `GenTaskID()` wraps before it reaches them.
pub use mars_stn::RESERVED_TASK_ID_START;

/// The process-wide STN, the counterpart of the `NetCore` the C++'s Java2C
/// calls reach. It is made with a net core in it — the C++'s `NetCore`
/// singleton is made the first time anything asks for it, and `OnInitConfig`
/// runs before any of the calls here.
fn logic() -> &'static Mutex<StnLogic> {
    static LOGIC: OnceLock<Mutex<StnLogic>> = OnceLock::new();
    LOGIC.get_or_init(|| {
        let mut logic = StnLogic::new();
        logic.create();
        Mutex::new(logic)
    })
}

/// Runs `f` on the process-wide STN. A poisoned lock keeps the state a panic
/// left behind rather than resetting it, which is what the C++ would leave.
fn with_logic<R>(f: impl FnOnce(&mut StnLogic) -> R) -> R {
    let mut logic = logic()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut logic)
}

/// `SetCallback` — the app STN asks, which is the Java side's `StnLogic`
/// callback object: everything the core wants to know goes to it, and
/// [`mars_stn::StnCallbackBridge`] is the only thing between the two.
pub fn set_callback_impl(app: impl App + 'static) {
    with_logic(|logic| logic.set_callback(app))
}

/// `StnLogic.reset` — a net core made again from nothing, which is what the
/// C++ does when the app has moved to another account: the tasks, the
/// signalling session and the addresses the setters handed to the net source
/// are gone with the one before it.
pub fn reset_impl() {
    with_logic(StnLogic::reset)
}

/// `StnLogic.resetAndInitEncoderVersion` — reset plus the encoder the C++ hands
/// to the new net core.
pub fn reset_and_init_encoder_version_impl(version: i32, name: &str) {
    with_logic(|logic| logic.reset_with_encoder(version, name))
}

/// `StnLogic.setLonglinkSvrAddr` — the ports are `uint16_t` in the C++, so a
/// Java int outside that range is truncated the way the C++ cast truncates it.
pub fn set_longlink_svr_addr_impl(host: &str, ports: &[i32], debug_ip: &str) {
    with_logic(|logic| {
        logic.set_longlink_svr_addr(
            host,
            ports.iter().map(|port| *port as u16).collect(),
            debug_ip,
        )
    })
}

/// `StnLogic.setShortlinkSvrAddr`.
pub fn set_shortlink_svr_addr_impl(port: i32, debug_ip: &str) {
    with_logic(|logic| logic.set_shortlink_svr_addr(port as u16, debug_ip))
}

/// `StnLogic.setDebugIP` — a host that is reached without asking dns. An empty
/// ip drops the entry, so the host goes back to being resolved.
pub fn set_debug_ip_impl(host: &str, ip: &str) {
    with_logic(|logic| logic.set_debug_ip(host, ip))
}

/// `StnLogic.setBackupIPs` — the pairs a host falls back to. An empty list drops
/// the host.
pub fn set_backup_ips_impl(host: &str, ips: &[String]) {
    with_logic(|logic| logic.set_backup_ips(host, ips.to_vec()))
}

/// `StnLogic.startTask` — `false` when the task is not one the queues would
/// take: the C++ answers the same way for a task it cannot run.
pub fn start_task_impl(task: Task) -> bool {
    with_logic(|logic| logic.start_task(task))
}

/// `StnLogic.stopTask` — `true` when the task was one of ours.
pub fn stop_task_impl(taskid: u32) -> bool {
    with_logic(|logic| logic.stop_task(taskid))
}

/// `StnLogic.hasTask`.
pub fn has_task_impl(taskid: u32) -> bool {
    with_logic(|logic| logic.has_task(taskid))
}

/// `StnLogic.redoTask` — every task that is out is run again.
pub fn redo_task_impl() {
    with_logic(StnLogic::redo_tasks)
}

/// `StnLogic.touchTasks` — the queues are sorted again, which is what a task
/// the app was not logged in for is waiting for.
pub fn touch_tasks_impl() {
    with_logic(StnLogic::touch_tasks)
}

/// `StnLogic.clearTask` — every task that is out is thrown away.
pub fn clear_task_impl() {
    with_logic(StnLogic::clear_tasks)
}

/// `StnLogic.makesureLongLinkConnected` — the default long link is connected,
/// and `true` when there was one to connect: the C++ reaches for it and does
/// nothing at all when there is none.
pub fn makesure_longlink_connected_impl() -> bool {
    with_logic(|logic| {
        let connected = logic.default_link().is_some();
        logic.make_sure_default_long_link_connected();
        connected
    })
}

/// `StnLogic.setSignallingStrategy` — a period or a keep time of `0` leaves the
/// `SignallingKeeper` defaults alone, which is what the C++'s `SetStrategy`
/// does with them.
pub fn set_signalling_strategy_impl(period: i64, keep_time: i64) {
    with_logic(|logic| logic.set_signalling_strategy(period.max(0) as u64, keep_time.max(0) as u64))
}

/// `StnLogic.keepSignalling`.
pub fn keep_signalling_impl() {
    with_logic(StnLogic::keep_signalling)
}

/// `StnLogic.stopSignalling`.
pub fn stop_signalling_impl() {
    with_logic(StnLogic::stop_signalling)
}

/// `StnLogic.setClientVersion` — the version every long-link package goes out
/// with, and the only one `longlink_unpack` accepts back, which is why it is
/// handed to [`mars_stn::longlink::set_client_version`] rather than kept here:
/// that is `mars::stn::SetClientVersion`, the one `stn_logic.cc` reaches.
pub fn set_client_version_impl(version: u32) {
    mars_stn::longlink::set_client_version(version);
}

/// `StnLogic.genTaskID` — one counter for the whole process, like the C++'s
/// `static uint32_t`.
pub fn gen_task_id_impl() -> u32 {
    mars_stn::gen_task_id()
}

/// `StnLogic.genSequenceId` — a `u16`, like the C++ `unsigned short`.
pub fn gen_sequence_id_impl() -> u16 {
    mars_stn::gen_sequence_id()
}

/// `StnLogic.trigNooping` — `SmartHeartbeat::SetHeartBeat(0)` and a noop on the
/// default long link.
pub fn trig_nooping_impl() {
    mars_stn::smart_heartbeat::set_heartbeat(0);
    with_logic(|logic| {
        if let Some(link) = logic.default_long_link() {
            link.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .trig_noop();
        }
    })
}

/// `StnLogic.getLoadLibraries`.
pub fn get_load_libraries_impl() -> Vec<String> {
    LOAD_LIBRARIES
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use mars_stn::longlink_task_manager::Response;
    use mars_stn::task_profile::TaskFailHandleType;
    use mars_stn::{App, ConnectProfile, ErrCmdType, LongLinkStatus, NetStatus, RespHandle};

    use super::*;

    /// The host every task in these samples goes out on.
    const HOST: &str = "long.weixin.qq.com";

    /// Everything the app was asked, as one value a sample reads.
    #[derive(Debug, Default, Clone, PartialEq, Eq)]
    struct Said {
        /// The tasks the app was asked to write a body for.
        encoded: Vec<u32>,
        /// The tasks that ended, and what they ended with.
        ended: Vec<(u32, ErrCmdType, i32)>,
        /// The status the app was told about.
        status: Vec<(NetStatus, NetStatus)>,
    }

    /// An app that writes down what it was asked, and answers the way a sample
    /// wants.
    #[derive(Clone)]
    struct Rec {
        said: Arc<Mutex<Said>>,
    }

    impl Rec {
        fn new() -> Self {
            Self {
                said: Arc::new(Mutex::new(Said::default())),
            }
        }

        fn said(&self) -> Said {
            self.said.lock().unwrap().clone()
        }
    }

    impl App for Rec {
        fn req2buf(
            &mut self,
            taskid: u32,
            _user_id: &str,
            _channel_select: i32,
            _host: &str,
            _sequence: u16,
        ) -> Result<Vec<u8>, i32> {
            self.said.lock().unwrap().encoded.push(taskid);
            Ok(b"/cgi-bin".to_vec())
        }

        fn buf2resp(
            &mut self,
            _taskid: u32,
            _user_id: &str,
            _body: &[u8],
            _channel_select: i32,
        ) -> (i32, TaskFailHandleType) {
            (0, TaskFailHandleType::Normal)
        }

        fn on_task_end(
            &mut self,
            taskid: u32,
            _user_id: &str,
            err_type: ErrCmdType,
            err_code: i32,
            _profile: &mars_stn::CgiProfile,
        ) -> i32 {
            self.said
                .lock()
                .unwrap()
                .ended
                .push((taskid, err_type, err_code));
            err_code
        }

        fn report_connect_status(&mut self, all: NetStatus, longlink: NetStatus) {
            self.said.lock().unwrap().status.push((all, longlink));
        }
    }

    /// STN with a recording app in it, which is what every sample starts from.
    ///
    /// The state is process-wide, so the samples take it in turn: each one
    /// throws away what the one before it left, and hands STN back the way
    /// `reset()` would.
    fn sample<R>(f: impl FnOnce(&Rec) -> R) -> R {
        let _guard = crate::test_lock();
        reset_impl();
        let app = Rec::new();
        set_callback_impl(app.clone());
        let result = f(&app);
        reset_impl();
        result
    }

    /// What the C++'s own platform would have done to the default long link by
    /// now: a task only goes out on a link that is up.
    fn up(status: LongLinkStatus) {
        with_logic(|logic| {
            let link = logic
                .default_long_link()
                .expect("no default long link")
                .clone();
            link.lock()
                .unwrap_or_else(|e| e.into_inner())
                .set_status(status);
        })
    }

    /// A task the queues would take: a cgi for the short link, a cmdid for the
    /// long one, and a host to go out on.
    fn task(taskid: u32) -> Task {
        let mut task = Task::new(taskid, 12);
        task.cgi = format!("/cgi-bin/{taskid}");
        task.channel_select = Task::CHANNEL_ALL;
        task.longlink_host_list = vec![HOST.to_string()];
        task
    }

    /// The net core, which is where a sample looks for what the setters set.
    fn core<R>(f: impl FnOnce(&mut mars_stn::NetCore) -> R) -> R {
        with_logic(|logic| f(logic.net_core().expect("no net core")))
    }

    /// The signalling keeper of the default long link.
    fn keeper<R>(f: impl FnOnce(&mut mars_stn::SignallingKeeper) -> R) -> R {
        core(|core| {
            f(core
                .long_link_meta(mars_stn::DEFAULT_LONGLINK_NAME)
                .expect("the default link")
                .keeper())
        })
    }

    /// Whether the keeper of the default link is holding a mapping up.
    fn keeping() -> bool {
        keeper(|keeper| keeper.is_keeping())
    }

    #[test]
    fn a_task_java_starts_is_one_the_queues_keep() {
        sample(|app| {
            up(LongLinkStatus::Connected);

            assert!(start_task_impl(task(7)), "a task the queues would take");
            assert!(has_task_impl(7));
            assert!(!has_task_impl(8), "no such task");

            // the app was asked for the body before anything went out
            assert_eq!(app.said().encoded, vec![7]);

            assert!(stop_task_impl(7));
            assert!(!has_task_impl(7));
            assert!(!stop_task_impl(7), "nothing to stop the second time");
        })
    }

    #[test]
    fn a_task_with_no_channel_to_go_out_on_ends_at_once() {
        sample(|app| {
            let mut task = task(7);
            task.channel_select = 0;

            assert!(!start_task_impl(task), "the C++ refuses it too");
            assert!(!has_task_impl(7));
            assert_eq!(
                app.said().ended,
                vec![(
                    7,
                    ErrCmdType::Local,
                    mars_stn::task_profile::LOCAL_CHANNEL_SELECT
                )]
            );
        })
    }

    /// `Reset()` ends the tasks that are out before it throws them away, which
    /// is the C++'s `__BatchErrorRespHandle` over both queues.
    #[test]
    fn a_reset_ends_the_tasks_that_are_out() {
        sample(|app| {
            up(LongLinkStatus::Connected);
            assert!(start_task_impl(task(7)));

            reset_impl();

            assert_eq!(
                app.said().ended,
                vec![(7, ErrCmdType::Local, mars_stn::task_profile::LOCAL_RESET)]
            );
        })
    }

    #[test]
    fn clear_and_redo_are_asked_of_every_task_that_is_out() {
        sample(|_app| {
            assert!(start_task_impl(task(1)));
            assert!(start_task_impl(task(2)));

            redo_task_impl();
            touch_tasks_impl();
            assert!(has_task_impl(1), "a redo is not a clear");

            clear_task_impl();
            assert!(!has_task_impl(1));
            assert!(!has_task_impl(2));
        })
    }

    /// The C++'s `Reset()` rebuilds the net core, so the tasks and the
    /// addresses the old one kept go with it — but the app does not: it is the
    /// host's, and the new core is wired to the same bridge.
    #[test]
    fn reset_throws_the_tasks_and_the_addresses_away_and_keeps_the_app() {
        sample(|app| {
            up(LongLinkStatus::Connected);
            set_longlink_svr_addr_impl(HOST, &[80, 443], "1.2.3.4");
            assert!(start_task_impl(task(7)));

            reset_impl();

            assert!(!has_task_impl(7));
            assert!(with_logic(StnLogic::long_link_hosts).is_empty());
            assert_eq!(
                core(|core| core.net_source().longlink_ports()),
                Vec::<u16>::new()
            );

            // the app is still the one STN talks to
            up(LongLinkStatus::Connected);
            assert!(start_task_impl(task(8)));
            assert_eq!(app.said().encoded, vec![7, 8]);
        })
    }

    #[test]
    fn reset_and_init_encoder_version_gives_the_new_core_the_encoder() {
        sample(|_app| {
            assert!(start_task_impl(task(7)));

            reset_and_init_encoder_version_impl(3, "wechat");

            assert!(!has_task_impl(7));
            assert_eq!(core(|core| core.packer_encoder_version()), 3);
            assert_eq!(
                core(|core| core.packer_encoder_name().to_string()),
                "wechat"
            );
        })
    }

    #[test]
    fn the_addresses_java_sets_are_the_ones_the_net_source_keeps() {
        sample(|_app| {
            set_longlink_svr_addr_impl(HOST, &[80, 443], "1.2.3.4");
            set_shortlink_svr_addr_impl(8080, "5.6.7.8");
            set_debug_ip_impl(HOST, "9.9.9.9");
            set_backup_ips_impl(HOST, &["8.8.8.8".to_owned()]);

            assert_eq!(
                with_logic(StnLogic::long_link_hosts),
                vec![HOST.to_string()]
            );
            core(|core| {
                assert_eq!(core.net_source().longlink_ports(), vec![80, 443]);
                assert_eq!(core.net_source().shortlink_port(), 8080);
                assert_eq!(core.net_source().shortlink_debug_ip(), "5.6.7.8");
                assert_eq!(core.net_source().backup_ips(HOST), vec!["8.8.8.8"]);
            });

            // a host with a debug ip is reached without asking dns: one pair per
            // long-link port, all of them on the ip java set
            let items = |core: &mut mars_stn::NetCore| {
                core.net_source()
                    .get_longlink_items(&mars_stn::LonglinkConfig::new(
                        mars_stn::DEFAULT_LONGLINK_NAME,
                    ))
                    .into_iter()
                    .map(|item| (item.ip, item.port))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                core(items),
                vec![("9.9.9.9".to_owned(), 80), ("9.9.9.9".to_owned(), 443)]
            );

            // … and an empty ip takes it back out, which leaves the link on the
            // debug ip of the addresses java set
            set_debug_ip_impl(HOST, "");
            assert_eq!(
                core(items),
                vec![("1.2.3.4".to_owned(), 80), ("1.2.3.4".to_owned(), 443)]
            );

            // an empty list takes the host's backup ips out as well
            set_backup_ips_impl(HOST, &[]);
            core(|core| assert_eq!(core.net_source().backup_ips(HOST), Vec::<String>::new()));

            // the C++ casts a jint to uint16_t, so the range wraps
            set_longlink_svr_addr_impl(HOST, &[65536 + 81], "");
            core(|core| assert_eq!(core.net_source().longlink_ports(), vec![81]));
        })
    }

    #[test]
    fn makesure_longlink_connected_is_asked_of_the_default_link() {
        sample(|_app| {
            assert!(
                makesure_longlink_connected_impl(),
                "there is a default link to connect"
            );

            // and it is the one the core makes sure of, which a host sees as a
            // link that is being connected
            let name = with_logic(|logic| logic.default_link().map(str::to_string));
            assert_eq!(name, Some(mars_stn::DEFAULT_LONGLINK_NAME.to_string()));
        })
    }

    #[test]
    fn the_signalling_strategy_java_sets_is_the_one_the_keeper_uses() {
        sample(|_app| {
            set_signalling_strategy_impl(1500, 3000);
            assert_eq!(mars_stn::signalling_keeper::period(), 1500);
            assert_eq!(mars_stn::signalling_keeper::keep_time(), 3000);

            // `0` is "leave the defaults alone", like the C++'s `SetStrategy`
            set_signalling_strategy_impl(0, 0);
            assert_eq!(mars_stn::signalling_keeper::period(), 1500);

            keep_signalling_impl();
            assert!(keeping());

            // a keeper that never posted is still keeping afterwards, which is
            // what the C++'s `if (keeping_ && postid_ != KNullPost)` does
            stop_signalling_impl();
            assert!(keeping(), "nothing has been posted yet");

            // … and one that has is not: the post is what `Stop()` takes away
            keeper(|keeper| keeper.on_network_data_changed());
            stop_signalling_impl();
            assert!(!keeping());

            mars_stn::signalling_keeper::set_strategy(0, 0);
        })
    }

    #[test]
    fn the_client_version_java_sets_is_the_one_the_packages_go_out_with() {
        sample(|_app| {
            set_client_version_impl(300);
            let packed = mars_stn::longlink::longlink_pack(
                mars_stn::longlink::NOOP_CMDID,
                Task::NOOP_TASK_ID,
                b"",
            );
            let unpacked = mars_stn::longlink::longlink_unpack(&packed);
            let mars_stn::Unpacked::Package { cmdid, seq, .. } = unpacked else {
                panic!("{unpacked:?}");
            };
            assert_eq!(cmdid, mars_stn::longlink::NOOP_CMDID);
            assert_eq!(seq, Task::NOOP_TASK_ID);

            // a package of another version is not one of ours
            set_client_version_impl(301);
            assert_eq!(
                mars_stn::longlink::longlink_unpack(&packed),
                mars_stn::Unpacked::False
            );

            set_client_version_impl(0);
        })
    }

    #[test]
    fn the_ids_stn_hands_out_are_the_ones_of_the_process() {
        sample(|_app| {
            let first = gen_task_id_impl();
            let second = gen_task_id_impl();
            assert!(second > first, "one counter for the whole process");
            assert!(second < RESERVED_TASK_ID_START);

            // the sequence is a random `unsigned short`, not a counter
            let ids: Vec<u16> = std::iter::repeat_with(gen_sequence_id_impl)
                .take(8)
                .collect();
            assert!(ids.iter().any(|id| *id != ids[0]), "{ids:?}");
        })
    }

    /// `TrigNooping` is a heartbeat the app asked for: the noop goes out on the
    /// default long link, which the C++'s own send does — here it is the host's,
    /// so the sample wires it the way a host wires it.
    #[test]
    fn trig_nooping_sends_the_noop_the_app_asked_for() {
        sample(|_app| {
            up(LongLinkStatus::Connected);

            trig_nooping_impl();

            assert_eq!(mars_stn::smart_heartbeat::outer_setted_heart(), 0);

            // the noop is the link's to send, and it is one that is waiting for
            // an answer — the C++'s `isnooping_`, set before the request goes
            // out so that a fast answer is still the answer to it
            with_logic(|logic| {
                let link = logic.default_long_link().expect("no default link").clone();
                let link = link.lock().unwrap_or_else(|e| e.into_inner());
                assert!(link.is_nooping(), "the noop is out");
                assert!(link.noop_timeout_due().is_some(), "and it has to answer");
            });

            mars_stn::smart_heartbeat::set_heartbeat(-1);
        })
    }

    /// What the C++'s own platform does: a task that went out comes back, and
    /// the host hands the answer to the queue it went out on.
    #[test]
    fn an_answer_the_host_hands_back_ends_the_task() {
        sample(|app| {
            up(LongLinkStatus::Connected);
            let sent = Arc::new(Mutex::new(Vec::new()));
            let recorder = Arc::clone(&sent);
            core(|core| {
                core.longlink().set_send(move |_name, task, _body| {
                    recorder.lock().unwrap().push(task.taskid);
                    Some(mars_stn::RunId(u64::from(task.taskid)))
                })
            });

            assert!(start_task_impl(task(7)));
            assert_eq!(*sent.lock().unwrap(), vec![7]);

            let answer = Response {
                name: mars_stn::DEFAULT_LONGLINK_NAME.to_string(),
                err_type: ErrCmdType::Ok,
                err_code: 0,
                cmdid: 12,
                taskid: 7,
                body: b"hello".to_vec(),
                profile: ConnectProfile::new(),
            };
            assert_eq!(
                core(|core| core.longlink().on_response(answer)),
                Some(RespHandle::Ended)
            );
            with_logic(|logic| logic.run_pending());

            assert!(!has_task_impl(7));
            assert_eq!(app.said().ended, vec![(7, ErrCmdType::Ok, 0)]);
        })
    }

    /// The C++'s own threads are the host's here, so what a sample can say
    /// about the connection is what the app was told about it.
    #[test]
    fn what_the_app_is_told_about_the_connection() {
        sample(|app| {
            core(|core| core.on_longlink_status_changed(LongLinkStatus::Connecting));
            up(LongLinkStatus::Connected);
            core(|core| core.on_longlink_status_changed(LongLinkStatus::Connected));

            assert_eq!(
                app.said().status,
                vec![
                    (NetStatus::Connecting, NetStatus::Connecting),
                    (NetStatus::Connected, NetStatus::Connected),
                ]
            );
        })
    }

    #[test]
    fn get_load_libraries_lists_this_library() {
        assert_eq!(get_load_libraries_impl(), vec!["marsxlog".to_owned()]);
    }
}
