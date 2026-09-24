//! Every symbol of the C ABI, called from Rust the way a C/C++/JNI caller
//! would: with valid arguments, and with the bad arguments that have to be
//! reported as `MARS_XLOG_ERR_*` instead of crashing.
//!
//! The appender is a process-wide singleton, so the tests serialise on
//! [`LOCK`].

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_uint};
use std::sync::{Mutex, MutexGuard, OnceLock};

use mars_ffi::abi::{
    mars_xlog_close, mars_xlog_current_log_cache_path, mars_xlog_current_log_path, mars_xlog_flush,
    mars_xlog_flush_instance, mars_xlog_flush_sync, mars_xlog_get_instance, mars_xlog_get_level,
    mars_xlog_is_enabled_for, mars_xlog_new_instance, mars_xlog_open, mars_xlog_release_instance,
    mars_xlog_set_console_log, mars_xlog_set_level, mars_xlog_set_level_instance,
    mars_xlog_set_max_alive_duration, mars_xlog_set_max_file_size, mars_xlog_set_mode,
    mars_xlog_set_mode_instance, mars_xlog_write, mars_xlog_write_instance, MarsXLogConfig,
    MARS_XLOG_ERR_APPENDER, MARS_XLOG_ERR_BAD_COMPRESS, MARS_XLOG_ERR_BAD_MODE,
    MARS_XLOG_ERR_EMPTY_LOG_DIR, MARS_XLOG_ERR_NO_PATH, MARS_XLOG_ERR_NO_SPACE,
    MARS_XLOG_ERR_NULL_CONFIG, MARS_XLOG_ERR_NULL_OUT, MARS_XLOG_OK,
};

fn serial() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mars-ffi-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A config whose `CString`s live as long as the borrow the pointers came from.
///
/// The fields are only held so that the pointers in `raw` stay valid; nothing
/// reads them.
#[allow(dead_code)]
struct ConfigBundle {
    log_dir: CString,
    prefix: CString,
    pub_key: CString,
    raw: MarsXLogConfig,
}

fn make_config(dir: &std::path::Path, mode: c_int, compress: c_int) -> ConfigBundle {
    let log_dir = CString::new(dir.to_str().unwrap()).unwrap();
    let prefix = CString::new("Mars").unwrap();
    let pub_key = CString::new("").unwrap();
    let raw = MarsXLogConfig {
        mode,
        log_dir: log_dir.as_ptr(),
        name_prefix: prefix.as_ptr(),
        pub_key: pub_key.as_ptr(),
        compress_mode: compress,
        compress_level: 0,
        cache_dir: std::ptr::null(),
        cache_days: 0,
    };
    ConfigBundle {
        log_dir,
        prefix,
        pub_key,
        raw,
    }
}

#[test]
fn open_rejects_a_bad_config_before_touching_the_disk() {
    let _guard = serial();
    assert_eq!(mars_xlog_open(std::ptr::null()), MARS_XLOG_ERR_NULL_CONFIG);

    let dir = tempdir("bad");
    for (mode, compress, expected) in [
        (7, 0, MARS_XLOG_ERR_BAD_MODE),
        (0, 9, MARS_XLOG_ERR_BAD_COMPRESS),
    ] {
        let config = make_config(&dir, mode, compress);
        assert_eq!(mars_xlog_open(&config.raw), expected);
    }

    let empty = CString::new("").unwrap();
    let mut config = make_config(&dir, 0, 0);
    config.raw.log_dir = empty.as_ptr();
    assert_eq!(mars_xlog_open(&config.raw), MARS_XLOG_ERR_EMPTY_LOG_DIR);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_whole_abi_runs_over_one_appender() {
    let _guard = serial();
    let dir = tempdir("surface");
    let config = make_config(&dir, 0, 0);
    assert_eq!(mars_xlog_open(&config.raw), MARS_XLOG_OK);
    // a second open is refused, like the C++ singleton
    assert_eq!(mars_xlog_open(&config.raw), MARS_XLOG_ERR_APPENDER);

    let tag = CString::new("Net").unwrap();
    let message = CString::new("hello").unwrap();
    // null pieces are allowed
    mars_xlog_write(
        2,
        tag.as_ptr(),
        std::ptr::null(),
        std::ptr::null(),
        0,
        message.as_ptr(),
    );
    mars_xlog_write(
        2,
        std::ptr::null(),
        std::ptr::null(),
        std::ptr::null(),
        0,
        message.as_ptr(),
    );

    // the exact level is asserted in the (single-threaded) JNI tests: another
    // test file can close the singleton appender under this one
    mars_xlog_set_level(1);
    let level = mars_xlog_get_level(0);
    assert!((-1..=6).contains(&level), "unexpected level {level}");
    let _ = mars_xlog_is_enabled_for(0, 2);
    let _ = mars_xlog_is_enabled_for(0, 0);
    // an unknown instance has no level
    assert_eq!(mars_xlog_get_level(0xdead_beef), -1);
    assert_eq!(mars_xlog_is_enabled_for(0xdead_beef, 5), 0);

    mars_xlog_set_console_log(1);
    mars_xlog_set_console_log(0);
    mars_xlog_set_max_file_size(0);
    mars_xlog_set_max_file_size(1 << 20);
    mars_xlog_set_max_alive_duration(-1);
    mars_xlog_set_max_alive_duration(3600);
    mars_xlog_set_mode(1);
    mars_xlog_set_mode(0);
    // an unknown mode is ignored
    mars_xlog_set_mode(9);

    let mut path = vec![0u8; 512];
    let written =
        mars_xlog_current_log_path(path.as_mut_ptr() as *mut c_char, path.len() as c_uint);
    assert!(written > 0, "no current log path: {written}");
    assert!(std::str::from_utf8(&path[..written as usize]).is_ok());
    // too small a buffer, and a null buffer
    assert_eq!(
        mars_xlog_current_log_path(path.as_mut_ptr() as *mut c_char, 0),
        MARS_XLOG_ERR_NO_SPACE
    );
    assert_eq!(
        mars_xlog_current_log_path(std::ptr::null_mut(), 64),
        MARS_XLOG_ERR_NULL_OUT
    );

    let mut cache = vec![0u8; 512];
    let written = mars_xlog_current_log_cache_path(cache.as_mut_ptr(), cache.len() as c_uint);
    assert!(written != MARS_XLOG_ERR_NULL_OUT);
    assert_eq!(
        mars_xlog_current_log_cache_path(std::ptr::null_mut(), 64),
        MARS_XLOG_ERR_NULL_OUT
    );

    mars_xlog_flush();
    mars_xlog_flush_sync();
    mars_xlog_flush_instance(0, 0);
    mars_xlog_flush_instance(0, 1);
    mars_xlog_close();
    // closed: there is no current file any more
    let mut path = vec![0u8; 512];
    assert_eq!(
        mars_xlog_current_log_path(path.as_mut_ptr() as *mut c_char, path.len() as c_uint),
        MARS_XLOG_ERR_NO_PATH
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn instances_are_created_addressed_and_released() {
    let _guard = serial();
    let dir = tempdir("instances");
    let config = make_config(&dir, 1, 1);
    let handle = mars_xlog_new_instance(&config.raw, 2);
    assert!(handle > 0);
    let prefix = CString::new("Mars").unwrap();
    assert_eq!(mars_xlog_get_instance(prefix.as_ptr()), handle);
    assert_eq!(mars_xlog_get_instance(std::ptr::null()), 0);

    let tag = CString::new("Net").unwrap();
    let file = CString::new("net.cc").unwrap();
    let func = CString::new("Send").unwrap();
    let message = CString::new("through an instance").unwrap();
    mars_xlog_write_instance(
        handle,
        2,
        tag.as_ptr(),
        file.as_ptr(),
        func.as_ptr(),
        7,
        message.as_ptr(),
    );
    mars_xlog_set_level_instance(handle, 3);
    assert_eq!(mars_xlog_get_level(handle), 3);
    mars_xlog_set_mode_instance(handle, 0);
    mars_xlog_flush_instance(handle, 1);
    mars_xlog_release_instance(prefix.as_ptr());
    assert_eq!(mars_xlog_get_instance(prefix.as_ptr()), 0);

    // a null config has no instance
    assert_eq!(mars_xlog_new_instance(std::ptr::null(), 2), 0);
    // an empty log dir is refused
    let empty = CString::new("").unwrap();
    let mut broken = make_config(&dir, 0, 0);
    broken.raw.log_dir = empty.as_ptr();
    assert_eq!(mars_xlog_new_instance(&broken.raw, 2), 0);
    // and so is a bad mode
    let broken_mode = make_config(&dir, 9, 0);
    assert_eq!(mars_xlog_new_instance(&broken_mode.raw, 2), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_void_symbols_survive_a_closed_appender() {
    let _guard = serial();
    // nothing is open: every void symbol has to be a no-op instead of a crash
    mars_xlog_write(
        2,
        std::ptr::null(),
        std::ptr::null(),
        std::ptr::null(),
        0,
        std::ptr::null(),
    );
    mars_xlog_write_instance(
        0,
        2,
        std::ptr::null(),
        std::ptr::null(),
        std::ptr::null(),
        0,
        std::ptr::null(),
    );
    mars_xlog_flush();
    mars_xlog_flush_sync();
    mars_xlog_flush_instance(0, 1);
    mars_xlog_close();
    mars_xlog_set_level(2);
    mars_xlog_set_level_instance(0, 2);
    mars_xlog_set_console_log(0);
    mars_xlog_set_max_file_size(0);
    mars_xlog_set_max_alive_duration(0);
    mars_xlog_set_mode(0);
    mars_xlog_set_mode_instance(0, 0);
    mars_xlog_release_instance(std::ptr::null());
}
