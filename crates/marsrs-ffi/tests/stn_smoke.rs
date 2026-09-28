//! Drives the STN C ABI the way an app's `StnLogic` does: hand over the app as
//! one function pointer, start a task over it, and read what STN asked back.
//!
//! The task pipeline is one process-wide value — the counterpart of the C++'s
//! singleton — so every test takes [`lock()`]: `cargo test` runs the cases of
//! this file on parallel threads and they would otherwise race over the same
//! net core.

#![cfg(feature = "stn")]

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::sync::{Mutex, MutexGuard, OnceLock};

use mars_ffi::stn::{
    mars_stn_clear_tasks, mars_stn_create_longlink, mars_stn_destroy_longlink, mars_stn_due_time,
    mars_stn_gen_sequence_id, mars_stn_gen_task_id, mars_stn_has_task, mars_stn_keep_signalling,
    mars_stn_longlink_is_connected_ext, mars_stn_makesure_longlink_connected,
    mars_stn_mark_main_longlink, mars_stn_redo_tasks, mars_stn_reset,
    mars_stn_reset_and_init_encoder_version, mars_stn_run_pending, mars_stn_set_app,
    mars_stn_set_backup_ips, mars_stn_set_client_version, mars_stn_set_debug_ip,
    mars_stn_set_longlink_svr_addr, mars_stn_set_shortlink_svr_addr,
    mars_stn_set_signalling_strategy, mars_stn_start_task, mars_stn_stop_signalling,
    mars_stn_stop_task, mars_stn_touch_tasks, mars_stn_trig_nooping, MarsStnAnswer,
    MarsStnAnswerKind, MarsStnLonglinkConfig, MarsStnQuestion, MarsStnQuestionKind, MarsStnStrings,
    MarsStnTask, MARS_STN_ERR_NO_DUE, MARS_STN_ERR_NULL_CONFIG, MARS_STN_ERR_NULL_TASK,
    MARS_STN_ERR_PANIC, MARS_STN_ERR_REFUSED, MARS_STN_OK,
};
use marsrs_stn::task_profile::{LOCAL_CHANNEL_SELECT, LOCAL_RESET};
use marsrs_stn::ErrCmdType;

/// The host every task in these tests goes out on.
const HOST: &str = "long.weixin.qq.com";

/// `Task::CHANNEL_ALL` — a task that may go out on either link.
const CHANNEL_ALL: c_int = 0x7;

/// `ErrCmdType::Local` — the `err_type` of a task that ended here, not on the
/// wire: every end these tests see is one the net core decided on.
const ERR_TYPE_LOCAL: c_int = ErrCmdType::Local as c_int;

/// Serialises the tests that share the process-wide pipeline.
fn lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    // A poisoned mutex only means an earlier assertion failed; the pipeline is
    // still usable, so recover instead of cascading the panic.
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// What the app was asked, in the terms a test reads. A question holds pointers
/// that are only good while it is being answered, so what is written down is
/// what they said and not the pointers.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Said {
    kind: MarsStnQuestionKind,
    taskid: u32,
    err_type: c_int,
    err_code: c_int,
}

/// What the app was asked, in order.
fn asked() -> &'static Mutex<Vec<Said>> {
    static ASKED: OnceLock<Mutex<Vec<Said>>> = OnceLock::new();
    ASKED.get_or_init(Default::default)
}

fn said() -> Vec<Said> {
    asked().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The app: it writes every question down, answers `Req2Buf` with the body of a
/// task, `OnNewDns` with one address, and remembers the code a task ended with.
extern "C" fn ask(ctx: *mut c_void, question: *const MarsStnQuestion, answer: *mut MarsStnAnswer) {
    // SAFETY: `mars_stn_set_app` hands a question and an answer it owns, and
    // reads the answer only after this call returns.
    let (question, answer) = unsafe { (&*question, &mut *answer) };
    // SAFETY: `ctx` is the `&mut usize` the test handed to `mars_stn_set_app`,
    // which outlives the pipeline.
    unsafe {
        *(ctx as *mut usize) += 1;
    }
    let kind = question.kind;
    asked()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Said {
            kind,
            taskid: question.taskid,
            err_type: question.err_type,
            err_code: question.err_code,
        });

    *answer = match kind {
        MarsStnQuestionKind::Req2Buf => MarsStnAnswer {
            kind: MarsStnAnswerKind::Encoded,
            bytes: b"/cgi-bin".as_ptr(),
            byte_count: b"/cgi-bin".len() as c_uint,
            ..MarsStnAnswer::default()
        },
        MarsStnQuestionKind::OnTaskEnd => MarsStnAnswer {
            kind: MarsStnAnswerKind::Ended,
            error_code: question.err_code,
            ..MarsStnAnswer::default()
        },
        MarsStnQuestionKind::OnNewDns => {
            // The addresses are read after this callback has returned, so they
            // cannot live on this frame: they are held in a static, as
            // integers because a static that holds raw pointers is not `Sync`,
            // which a static has to be.
            static IPS: OnceLock<[usize; 1]> = OnceLock::new();
            let ips = IPS.get_or_init(|| [leaked("1.2.3.4").as_ptr() as usize]);
            MarsStnAnswer {
                kind: MarsStnAnswerKind::Ips,
                ips: ips.as_ptr() as *const *const c_char,
                ip_count: ips.len() as c_uint,
                ..MarsStnAnswer::default()
            }
        }
        MarsStnQuestionKind::MakesureAuthed | MarsStnQuestionKind::IdentifyResponse => {
            MarsStnAnswer {
                kind: MarsStnAnswerKind::Yes,
                yes: 1,
                ..MarsStnAnswer::default()
            }
        }
        // `1` — ask again on the next connect.
        MarsStnQuestionKind::IdentifyCheckBuffer => MarsStnAnswer {
            kind: MarsStnAnswerKind::Identified,
            mode: 1,
            ..MarsStnAnswer::default()
        },
        _ => MarsStnAnswer::default(),
    };
}

/// A string that outlives every test, which is what the strings of a task have
/// to do for as long as the task is being read.
fn leaked(value: &str) -> &'static CStr {
    Box::leak(CString::new(value).unwrap().into_boxed_c_str())
}

/// Installs the app, and hands back how many times it has been asked.
fn set_app() -> &'static mut usize {
    let count: &'static mut usize = Box::leak(Box::new(0));
    // SAFETY: `count` is leaked, so it outlives the pipeline, and `ask` is a
    // valid function pointer.
    unsafe {
        mars_stn_set_app(count as *mut usize as *mut c_void, Some(ask));
    }
    asked().lock().unwrap_or_else(|e| e.into_inner()).clear();
    count
}

/// One task the queues would take: a cgi for the short link, a cmdid for the
/// long one, and a host to go out on.
fn a_task(taskid: u32, channel_select: c_int) -> MarsStnTask {
    // The addresses of the host list are held as integers, because a static
    // that holds raw pointers is not `Sync`, which a static has to be.
    static HOSTS: OnceLock<[usize; 1]> = OnceLock::new();
    let hosts = HOSTS.get_or_init(|| [leaked(HOST).as_ptr() as usize]);
    MarsStnTask {
        taskid,
        cmdid: 12,
        channel_select,
        cgi: leaked(&format!("/cgi-bin/{taskid}")).as_ptr(),
        longlink_host_list: MarsStnStrings {
            items: hosts.as_ptr() as *const *const c_char,
            count: 1,
        },
        ..MarsStnTask::default()
    }
}

/// A task the queues would take, started; its id is the one it was handed.
fn start(taskid: u32, channel_select: c_int) -> c_int {
    let task = a_task(taskid, channel_select);
    // SAFETY: `task` is alive for the call, and the strings it points at are
    // leaked, i.e. alive for as long as the task is read.
    unsafe { mars_stn_start_task(&task) }
}

#[test]
fn a_task_the_caller_starts_is_one_the_queues_keep() {
    let _guard = lock();
    mars_stn_reset();
    let count = set_app();

    assert_eq!(start(7, CHANNEL_ALL), MARS_STN_OK);
    assert_eq!(mars_stn_has_task(7), 1);
    // Nothing runs a task here: no link is connected, and nothing asks the app
    // for the body of one until there is a link to send it on.
    assert_eq!(said(), vec![]);

    // Ours to stop once, and nobody's the second time.
    assert_eq!(mars_stn_stop_task(7), 1);
    assert_eq!(mars_stn_has_task(7), 0);
    assert_eq!(mars_stn_stop_task(7), 0);

    assert_eq!(*count, 0);
}

/// A task with no channel to go out on is not queued at all: the net core ends
/// it where it stands, and the app hears about it like any other end.
#[test]
fn a_task_with_no_channel_to_go_out_on_ends_at_once() {
    let _guard = lock();
    mars_stn_reset();
    let count = set_app();

    assert_eq!(start(8, 0), MARS_STN_ERR_REFUSED);
    assert_eq!(mars_stn_has_task(8), 0);
    assert_eq!(
        said(),
        vec![Said {
            kind: MarsStnQuestionKind::OnTaskEnd,
            taskid: 8,
            err_type: ERR_TYPE_LOCAL,
            err_code: LOCAL_CHANNEL_SELECT,
        }]
    );
    assert_eq!(*count, 1);
}

/// A net core made again from nothing ends the tasks the one before it held:
/// the app is told, and the queues are empty.
#[test]
fn a_reset_net_core_ends_the_tasks_it_holds() {
    let _guard = lock();
    mars_stn_reset();
    set_app();
    assert_eq!(start(7, CHANNEL_ALL), MARS_STN_OK);

    mars_stn_reset();

    assert_eq!(mars_stn_has_task(7), 0);
    assert_eq!(
        said(),
        vec![Said {
            kind: MarsStnQuestionKind::OnTaskEnd,
            taskid: 7,
            err_type: ERR_TYPE_LOCAL,
            err_code: LOCAL_RESET,
        }]
    );
}

/// The queues hold on to a task through `RedoTask` and `TouchTasks` — which is
/// what they are for — and `ClearTask` empties them.
#[test]
fn the_queues_keep_a_task_until_it_is_cleared() {
    let _guard = lock();
    mars_stn_reset();
    set_app();
    assert_eq!(start(7, CHANNEL_ALL), MARS_STN_OK);
    assert_eq!(start(8, CHANNEL_ALL), MARS_STN_OK);

    mars_stn_redo_tasks();
    mars_stn_touch_tasks();
    assert_eq!((mars_stn_has_task(7), mars_stn_has_task(8)), (1, 1));

    mars_stn_clear_tasks();
    assert_eq!((mars_stn_has_task(7), mars_stn_has_task(8)), (0, 0));
}

/// No task is no task to start, and it is reported as such.
#[test]
fn no_task_is_reported() {
    let _guard = lock();
    mars_stn_reset();
    set_app();

    // SAFETY: null is explicitly allowed by the contract.
    assert_eq!(
        unsafe { mars_stn_start_task(std::ptr::null()) },
        MARS_STN_ERR_NULL_TASK
    );
    assert_eq!(said(), vec![]);
}

/// The rest of the surface a caller reaches: the addresses the links go out on,
/// the signalling session, the version a package goes out with, the noop.
#[test]
fn the_surface_a_caller_reaches_is_one_it_can_call() {
    let _guard = lock();
    // SAFETY: every pointer handed in is either null or a valid NUL-terminated
    // string, or an array of them, alive for the call.
    unsafe {
        mars_stn_reset_and_init_encoder_version(200, leaked("encoder").as_ptr());
        let ports = [80u16, 443];
        mars_stn_set_longlink_svr_addr(
            leaked(HOST).as_ptr(),
            ports.as_ptr(),
            ports.len() as c_uint,
            std::ptr::null(),
        );
        mars_stn_set_shortlink_svr_addr(443, std::ptr::null());
        mars_stn_set_debug_ip(leaked(HOST).as_ptr(), leaked("1.2.3.4").as_ptr());
        let ips = [leaked("1.2.3.4").as_ptr()];
        mars_stn_set_backup_ips(leaked(HOST).as_ptr(), ips.as_ptr(), ips.len() as c_uint);
    }
    // Zeroes are the keeper's own defaults, which is what the C++ leaves alone.
    mars_stn_set_signalling_strategy(0, 0);
    mars_stn_keep_signalling();
    mars_stn_stop_signalling();
    mars_stn_set_client_version(200);
    mars_stn_trig_nooping();
    // There is a long link to connect now that it has a host to go out on.
    assert_eq!(mars_stn_makesure_longlink_connected(), 1);
}

/// The ids a caller asks for are new every time: a counter for the tasks of the
/// whole process, and a sequence short enough for a long-link package.
#[test]
fn the_ids_the_caller_asks_for_are_new_every_time() {
    let _guard = lock();
    mars_stn_reset();

    let first = mars_stn_gen_task_id();
    let second = mars_stn_gen_task_id();
    assert_eq!(second, first + 1, "the counter hands one out at a time");
    assert!(first > 0, "a task id is never 0");

    // A sequence is a `u16` the pipeline draws, not a counter: eight draws are
    // not eight of the same one.
    let draws: Vec<u16> = (0..8).map(|_| mars_stn_gen_sequence_id()).collect();
    assert!(
        draws.iter().any(|id| *id != draws[0]),
        "eight draws were the same one: {draws:?}"
    );
}

/// A long link the caller names is one the calls that take a name find, which
/// is what an app with a second long link asks for: `mars/stn/stn_logic.h` has
/// the trio under "Support multi longlinks for mars", and no binding but the
/// Rust one reached it until now.
#[test]
fn a_link_the_caller_names_is_one_the_named_calls_find() {
    let _guard = lock();
    mars_stn_reset();

    let name = leaked("minor");
    let host = leaked("minor.weixin.qq.com");
    let hosts = [host.as_ptr()];
    let config = MarsStnLonglinkConfig {
        name: name.as_ptr(),
        host_list: MarsStnStrings {
            items: hosts.as_ptr(),
            count: 1,
        },
        is_keep_alive: 1,
        group: std::ptr::null(),
        is_main: 0,
        link_type: 0,
        need_tls: 1,
    };

    // Every string these calls take is `name`, which is leaked: a valid
    // NUL-terminated string for as long as the process runs.
    // SAFETY: `name` is leaked, i.e. a valid NUL-terminated string.
    assert_eq!(
        unsafe { mars_stn_longlink_is_connected_ext(name.as_ptr()) },
        0
    );
    // SAFETY: `name` is leaked, i.e. a valid NUL-terminated string.
    assert_eq!(unsafe { mars_stn_mark_main_longlink(name.as_ptr()) }, 0);

    // SAFETY: `config` is alive for the call, and every string it points at is
    // leaked.
    assert_eq!(unsafe { mars_stn_create_longlink(&config) }, MARS_STN_OK);

    // The link is there, which is what the two by-name calls are for: it is not
    // connected, because nothing has connected it, but it is a link.
    // SAFETY: `name` is leaked, i.e. a valid NUL-terminated string.
    assert_eq!(
        unsafe { mars_stn_longlink_is_connected_ext(name.as_ptr()) },
        0
    );
    // SAFETY: `name` is leaked, i.e. a valid NUL-terminated string.
    assert_eq!(unsafe { mars_stn_mark_main_longlink(name.as_ptr()) }, 1);
    // The second time it is already the main one, which is not a change.
    // SAFETY: `name` is leaked, i.e. a valid NUL-terminated string.
    assert_eq!(unsafe { mars_stn_mark_main_longlink(name.as_ptr()) }, 0);

    // SAFETY: `name` is leaked, i.e. a valid NUL-terminated string.
    assert_eq!(unsafe { mars_stn_destroy_longlink(name.as_ptr()) }, 1);
    // SAFETY: `name` is leaked, i.e. a valid NUL-terminated string.
    assert_eq!(unsafe { mars_stn_destroy_longlink(name.as_ptr()) }, 0);
    // SAFETY: `name` is leaked, i.e. a valid NUL-terminated string.
    assert_eq!(
        unsafe { mars_stn_longlink_is_connected_ext(name.as_ptr()) },
        0
    );
    // SAFETY: `name` is leaked, i.e. a valid NUL-terminated string.
    assert_eq!(unsafe { mars_stn_mark_main_longlink(name.as_ptr()) }, 0);
}

/// A config the caller did not hand over is an error and not a link made out of
/// nothing: the C++ cannot tell the two apart, its `CreateLonglink_ext` being
/// `void`, and a caller that forgot the struct is better served by the answer.
#[test]
fn a_config_the_caller_did_not_hand_over_is_an_error() {
    let _guard = lock();
    mars_stn_reset();
    // SAFETY: null is the one pointer this symbol promises to answer for.
    assert_eq!(
        unsafe { mars_stn_create_longlink(std::ptr::null()) },
        MARS_STN_ERR_NULL_CONFIG
    );
}

/// The two symbols a host's loop is made of, which are the reason a task a C
/// caller starts is not one that sits in its queue forever: `mars_stn_due_time`
/// says when, and `mars_stn_run_pending` does the pass.
#[test]
fn the_hosts_loop_is_a_pair_the_header_declares() {
    let _guard = lock();
    mars_stn_reset();
    set_app();

    // A pass over a core with nothing in it is one that says nothing.
    mars_stn_run_pending();
    assert_eq!(said(), vec![]);

    assert_eq!(start(7, CHANNEL_ALL), MARS_STN_OK);

    // There is a wait to be had, and it is not one of the two errors: the
    // timing sync arms an alarm of its own the moment the core is made, and a
    // task that is out is waiting on its first package besides.
    let due = mars_stn_due_time();
    assert_ne!(due, MARS_STN_ERR_NO_DUE);
    assert_ne!(due, MARS_STN_ERR_PANIC.into());

    // What it answers is the wait that is left and not the tick the alarm is
    // armed at: a tick is measured from an origin a C caller cannot read, so it
    // would grow with the age of the process instead of running down.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let later = mars_stn_due_time();
    assert!(
        later < due,
        "the wait ran down instead of growing: {due} then {later}"
    );

    // A pass made before that tick has come is one that leaves the task alone:
    // what would end it is the tick, not the call.
    mars_stn_run_pending();
    assert_eq!(mars_stn_has_task(7), 1);

    mars_stn_clear_tasks();
    assert_eq!(mars_stn_has_task(7), 0);
}
