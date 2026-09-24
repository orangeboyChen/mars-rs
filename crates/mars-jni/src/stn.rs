//! `io.github.marsrs.stn.StnLogic` — the state behind the `native` methods of
//! `com/tencent/mars/stn/StnLogic.java`.
//!
//! The C++ (`mars/stn/jni/com_tencent_mars_stn_StnLogic_Java2C.cc`) forwards
//! every Java call to a free function of `mars/stn/stn_logic.h`, which reaches
//! `NetCore`. This port keeps the *Java2C surface* — the same calls, the same
//! argument order, the same answers — and stores what they set, because the
//! networking itself (`NetCore`, the long link and the short link) is not part
//! of this repository.
//!
//! Everything the JVM touches lives in [`crate::jni_bridge`]; what is here is
//! plain Rust and is covered by `cargo test`.

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use mars_stn::Task;

/// `Task::kReservedTaskIDStart` of `mars/stn/stn.h`: the ids from here up are
/// taken by the noop, the long-link identify check and the signalling keeper,
/// so `GenTaskID()` wraps before it reaches them.
pub const RESERVED_TASK_ID_START: u32 = 0xFFFF_FFF0;

/// The default signalling period, in milliseconds (`SignallingKeeper`).
pub const DEFAULT_SIGNALLING_PERIOD: i64 = 5000;
/// The default signalling keep time, in milliseconds.
pub const DEFAULT_SIGNALLING_KEEP_TIME: i64 = 20000;

/// What `getLoadLibraries` reports: the C++ lists the modules the process
/// loaded, which in this port is this one library.
pub const LOAD_LIBRARIES: &[&str] = &["marsxlog"];

/// The process-wide state, the counterpart of the C++ singletons (`NetCore`,
/// `StnManager`) the Java2C calls reach.
#[derive(Debug, Clone, Default)]
struct StnState {
    longlink_host: String,
    longlink_ports: Vec<u16>,
    longlink_debug_ip: String,
    shortlink_port: u16,
    shortlink_debug_ip: String,
    /// `setDebugIP` — host to ip, the highest priority of the three setters.
    debug_ips: BTreeMap<String, String>,
    /// `setBackupIPs` — host to the ip list to fall back to.
    backup_ips: BTreeMap<String, Vec<String>>,
    /// The tasks `StartTask` handed over that have not ended yet.
    tasks: BTreeMap<u32, Task>,
    client_version: u32,
    encoder_version: i32,
    encoder_name: String,
    signalling_period: i64,
    signalling_keep_time: i64,
    /// Whether `KeepSignalling()` is in force.
    signalling: bool,
    /// How many noops `TrigNooping()` asked for.
    noop_count: u64,
    /// How often `MakesureLonglinkConnected()` was called.
    makesure_count: u64,
    next_task_id: u32,
    next_sequence_id: u16,
}

impl StnState {
    /// `GenTaskID()` — `atomic_inc32` with the wrap the C++ does before the
    /// reserved ids, so the ids handed out run 1, 2, 3, …
    fn gen_task_id(&mut self) -> u32 {
        if self.next_task_id >= RESERVED_TASK_ID_START {
            self.next_task_id = 1;
        }
        self.next_task_id += 1;
        self.next_task_id
    }

    /// `GenSequenceId()` — the C++ draws a random `unsigned short`; the port
    /// walks the same range instead, so the sequence is reproducible.
    fn gen_sequence_id(&mut self) -> u16 {
        self.next_sequence_id = self.next_sequence_id.wrapping_add(1);
        self.next_sequence_id
    }
}

fn state() -> &'static Mutex<StnState> {
    static STATE: OnceLock<Mutex<StnState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(StnState::default()))
}

/// Runs `f` on the process-wide state. A poisoned lock keeps the state a panic
/// left behind rather than resetting it, which is what the C++ would leave.
fn with_state<R>(f: impl FnOnce(&mut StnState) -> R) -> R {
    let mut state = state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut state)
}

/// `StnLogic.reset` — clears the tasks and re-initialises, which in the C++
/// means rebuilding `NetCore`.
pub fn reset_impl() {
    with_state(|state| {
        state.tasks.clear();
        state.signalling = false;
        state.signalling_period = 0;
        state.signalling_keep_time = 0;
    })
}

/// `StnLogic.resetAndInitEncoderVersion` — `Reset()` plus the encoder the C++
/// hands to the new `NetCore`.
///
/// The signalling session goes with the rest: this is documented as reset plus
/// the encoder, and the C++ rebuilds `NetCore` here, so a host that kept
/// signalling must not find the old session still running afterwards.
pub fn reset_and_init_encoder_version_impl(version: i32, name: &str) {
    with_state(|state| {
        state.tasks.clear();
        state.signalling = false;
        state.signalling_period = 0;
        state.signalling_keep_time = 0;
        state.encoder_version = version;
        state.encoder_name = name.to_owned();
    })
}

/// `StnLogic.setLonglinkSvrAddr` — the ports are `uint16_t` in the C++, so a
/// Java int outside that range is truncated the way the C++ cast truncates it.
pub fn set_longlink_svr_addr_impl(host: &str, ports: &[i32], debug_ip: &str) {
    with_state(|state| {
        state.longlink_host = host.to_owned();
        state.longlink_ports = ports.iter().map(|port| *port as u16).collect();
        state.longlink_debug_ip = debug_ip.to_owned();
    })
}

/// `StnLogic.setShortlinkSvrAddr`.
pub fn set_shortlink_svr_addr_impl(port: i32, debug_ip: &str) {
    with_state(|state| {
        state.shortlink_port = port as u16;
        state.shortlink_debug_ip = debug_ip.to_owned();
    })
}

/// `StnLogic.setDebugIP` — an empty ip drops the entry, so the host goes back
/// to being resolved.
pub fn set_debug_ip_impl(host: &str, ip: &str) {
    with_state(|state| {
        if ip.is_empty() {
            state.debug_ips.remove(host);
        } else {
            state.debug_ips.insert(host.to_owned(), ip.to_owned());
        }
    })
}

/// `StnLogic.setBackupIPs` — an empty list drops the host.
pub fn set_backup_ips_impl(host: &str, ips: &[String]) {
    with_state(|state| {
        if ips.is_empty() {
            state.backup_ips.remove(host);
        } else {
            state.backup_ips.insert(host.to_owned(), ips.to_vec());
        }
    })
}

/// `StnLogic.startTask` — `false` when that task id is already in flight, which
/// is what the C++ refuses as well.
pub fn start_task_impl(task: Task) -> bool {
    with_state(|state| match state.tasks.entry(task.taskid) {
        Entry::Occupied(_) => false,
        Entry::Vacant(slot) => {
            slot.insert(task);
            true
        }
    })
}

/// `StnLogic.stopTask`.
pub fn stop_task_impl(taskid: u32) -> bool {
    with_state(|state| state.tasks.remove(&taskid).is_some())
}

/// `StnLogic.hasTask`.
pub fn has_task_impl(taskid: u32) -> bool {
    with_state(|state| state.tasks.contains_key(&taskid))
}

/// `StnLogic.redoTask` — how many tasks were put back. The C++ re-runs them and
/// reconnects the long link, so the reconnect is counted here too.
pub fn redo_task_impl() -> usize {
    with_state(|state| {
        state.makesure_count += 1;
        state.tasks.len()
    })
}

/// `StnLogic.touchTasks` — the C++ re-sorts the queues; the port reports how
/// many tasks there are to sort.
pub fn touch_tasks_impl() -> usize {
    with_state(|state| state.tasks.len())
}

/// `StnLogic.clearTask` — how many tasks were dropped.
pub fn clear_task_impl() -> usize {
    with_state(|state| {
        let count = state.tasks.len();
        state.tasks.clear();
        count
    })
}

/// `StnLogic.makesureLongLinkConnected` — `true` when there is an address to
/// connect to, which is all this port can tell about it.
pub fn makesure_longlink_connected_impl() -> bool {
    with_state(|state| {
        state.makesure_count += 1;
        !state.longlink_host.is_empty() || !state.longlink_debug_ip.is_empty()
    })
}

/// `StnLogic.setSignallingStrategy` — a non-positive period or keep time falls
/// back to the `SignallingKeeper` defaults.
pub fn set_signalling_strategy_impl(period: i64, keep_time: i64) {
    with_state(|state| {
        state.signalling_period = if period > 0 {
            period
        } else {
            DEFAULT_SIGNALLING_PERIOD
        };
        state.signalling_keep_time = if keep_time > 0 {
            keep_time
        } else {
            DEFAULT_SIGNALLING_KEEP_TIME
        };
    })
}

/// `StnLogic.keepSignalling`.
pub fn keep_signalling_impl() {
    with_state(|state| state.signalling = true)
}

/// `StnLogic.stopSignalling`.
pub fn stop_signalling_impl() {
    with_state(|state| state.signalling = false)
}

/// `StnLogic.setClientVersion`.
pub fn set_client_version_impl(version: u32) {
    with_state(|state| state.client_version = version)
}

/// `StnLogic.genTaskID`.
pub fn gen_task_id_impl() -> u32 {
    with_state(StnState::gen_task_id)
}

/// `StnLogic.genSequenceId` — a `u16`, like the C++ `unsigned short`.
pub fn gen_sequence_id_impl() -> u16 {
    with_state(StnState::gen_sequence_id)
}

/// `StnLogic.trigNooping` — `SmartHeartbeat::SetHeartBeat(0)` and a noop on the
/// default long link.
pub fn trig_nooping_impl() {
    with_state(|state| state.noop_count += 1)
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
    use super::*;

    /// The state is process-wide, so the tests take it in turn, and each one
    /// leaves it as `reset()` would.
    fn isolated<R>(f: impl FnOnce() -> R) -> R {
        let guard = crate::test_lock();
        reset_impl();
        let result = f();
        reset_impl();
        drop(guard);
        result
    }

    fn task(taskid: u32) -> Task {
        Task::new(taskid, 1)
    }

    #[test]
    fn task_ids_run_upwards_and_never_reach_the_reserved_ones() {
        isolated(|| {
            assert_eq!(gen_task_id_impl(), 1);
            assert_eq!(gen_task_id_impl(), 2);

            // the wrap the C++ does just below the reserved ids
            with_state(|state| state.next_task_id = RESERVED_TASK_ID_START);
            assert_eq!(gen_task_id_impl(), 2, "wrapped to 1, then incremented");
            assert!(gen_task_id_impl() < RESERVED_TASK_ID_START);
        })
    }

    #[test]
    fn the_sequence_id_wraps_within_the_unsigned_short() {
        isolated(|| {
            with_state(|state| state.next_sequence_id = u16::MAX);
            assert_eq!(gen_sequence_id_impl(), 0);
            assert_eq!(gen_sequence_id_impl(), 1);
        })
    }

    #[test]
    fn a_task_is_started_found_and_stopped() {
        isolated(|| {
            assert!(start_task_impl(task(7)));
            assert!(has_task_impl(7));
            assert!(!has_task_impl(8));

            // the same id cannot be in flight twice
            assert!(!start_task_impl(task(7)));

            assert!(stop_task_impl(7));
            assert!(!has_task_impl(7));
            assert!(!stop_task_impl(7), "nothing to stop the second time");

            assert_eq!(touch_tasks_impl(), 0);
        })
    }

    #[test]
    fn clear_and_redo_count_the_tasks_they_touch() {
        isolated(|| {
            assert!(start_task_impl(task(1)));
            assert!(start_task_impl(task(2)));
            assert_eq!(touch_tasks_impl(), 2);
            assert_eq!(redo_task_impl(), 2);
            assert_eq!(clear_task_impl(), 2);
            assert_eq!(clear_task_impl(), 0);
        })
    }

    #[test]
    fn reset_drops_the_tasks_and_the_signalling() {
        isolated(|| {
            assert!(start_task_impl(task(1)));
            set_signalling_strategy_impl(1000, 2000);
            keep_signalling_impl();
            reset_impl();

            assert!(!has_task_impl(1));
            with_state(|state| assert!(!state.signalling));
        })
    }

    #[test]
    fn reset_and_init_encoder_version_keeps_the_encoder() {
        isolated(|| {
            assert!(start_task_impl(task(1)));
            reset_and_init_encoder_version_impl(3, "wechat");

            assert!(!has_task_impl(1));
            with_state(|state| {
                assert_eq!(state.encoder_version, 3);
                assert_eq!(state.encoder_name, "wechat");
            });
        })
    }

    #[test]
    fn reset_and_init_encoder_version_drops_the_signalling_too() {
        isolated(|| {
            set_signalling_strategy_impl(1000, 2000);
            keep_signalling_impl();
            reset_and_init_encoder_version_impl(3, "wechat");

            with_state(|state| {
                assert!(!state.signalling, "the old session is still running");
                assert_eq!(state.signalling_period, 0);
                assert_eq!(state.signalling_keep_time, 0);
                // and it is still reset plus the encoder
                assert_eq!(state.encoder_version, 3);
                assert_eq!(state.encoder_name, "wechat");
            });
        })
    }

    #[test]
    fn the_addresses_the_java_side_sets_are_kept() {
        isolated(|| {
            set_longlink_svr_addr_impl("long.weixin.qq.com", &[80, 443, 8080], "1.2.3.4");
            set_shortlink_svr_addr_impl(80, "5.6.7.8");
            with_state(|state| {
                assert_eq!(state.longlink_host, "long.weixin.qq.com");
                assert_eq!(state.longlink_ports, vec![80, 443, 8080]);
                assert_eq!(state.longlink_debug_ip, "1.2.3.4");
                assert_eq!(state.shortlink_port, 80);
                assert_eq!(state.shortlink_debug_ip, "5.6.7.8");
            });

            // the C++ casts a jint to uint16_t, so the range wraps
            set_longlink_svr_addr_impl("host", &[65536 + 81], "");
            with_state(|state| assert_eq!(state.longlink_ports, vec![81]));
        })
    }

    #[test]
    fn makesure_longlink_connected_needs_an_address() {
        isolated(|| {
            assert!(!makesure_longlink_connected_impl(), "no address yet");

            set_longlink_svr_addr_impl("long.weixin.qq.com", &[80], "");
            assert!(makesure_longlink_connected_impl());

            // a debug ip alone is enough for the C++ as well
            reset_impl();
            set_longlink_svr_addr_impl("", &[], "1.2.3.4");
            assert!(makesure_longlink_connected_impl());
        })
    }

    #[test]
    fn the_debug_ips_and_the_backup_ips_are_maps_keyed_by_host() {
        isolated(|| {
            set_debug_ip_impl("short.weixin.qq.com", "9.9.9.9");
            set_debug_ip_impl("long.weixin.qq.com", "8.8.8.8");
            set_backup_ips_impl("short.weixin.qq.com", &["1.1.1.1".to_owned()]);

            with_state(|state| {
                assert_eq!(state.debug_ips.len(), 2);
                assert_eq!(state.debug_ips["long.weixin.qq.com"], "8.8.8.8");
                assert_eq!(state.backup_ips["short.weixin.qq.com"], vec!["1.1.1.1"]);
            });

            // an empty value takes the host back out of both maps
            set_debug_ip_impl("short.weixin.qq.com", "");
            set_backup_ips_impl("short.weixin.qq.com", &[]);
            with_state(|state| {
                assert!(!state.debug_ips.contains_key("short.weixin.qq.com"));
                assert!(!state.backup_ips.contains_key("short.weixin.qq.com"));
            });
        })
    }

    #[test]
    fn a_non_positive_signalling_strategy_falls_back_to_the_defaults() {
        isolated(|| {
            set_signalling_strategy_impl(0, -1);
            with_state(|state| {
                assert_eq!(state.signalling_period, DEFAULT_SIGNALLING_PERIOD);
                assert_eq!(state.signalling_keep_time, DEFAULT_SIGNALLING_KEEP_TIME);
            });

            set_signalling_strategy_impl(1500, 3000);
            with_state(|state| {
                assert_eq!(state.signalling_period, 1500);
                assert_eq!(state.signalling_keep_time, 3000);
            });
        })
    }

    #[test]
    fn keep_and_stop_signalling_move_the_flag() {
        isolated(|| {
            keep_signalling_impl();
            with_state(|state| assert!(state.signalling));
            stop_signalling_impl();
            with_state(|state| assert!(!state.signalling));
        })
    }

    #[test]
    fn the_client_version_the_noop_and_the_libraries() {
        isolated(|| {
            set_client_version_impl(0x0102_0304);
            with_state(|state| assert_eq!(state.client_version, 0x0102_0304));

            trig_nooping_impl();
            trig_nooping_impl();
            with_state(|state| assert_eq!(state.noop_count, 2));

            assert_eq!(get_load_libraries_impl(), vec!["marsxlog".to_owned()]);
        })
    }
}
