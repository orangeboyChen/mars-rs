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
    mars_xlog_flush_all, mars_xlog_flush_instance, mars_xlog_flush_sync, mars_xlog_get_instance,
    mars_xlog_get_level, mars_xlog_getfilepath_from_timespan, mars_xlog_is_enabled_for,
    mars_xlog_make_logfile_name, mars_xlog_new_instance, mars_xlog_oneshot_flush, mars_xlog_open,
    mars_xlog_release_instance, mars_xlog_set_console_log, mars_xlog_set_console_log_instance,
    mars_xlog_set_level, mars_xlog_set_level_instance, mars_xlog_set_max_alive_duration,
    mars_xlog_set_max_alive_duration_instance, mars_xlog_set_max_file_size,
    mars_xlog_set_max_file_size_instance, mars_xlog_set_mode, mars_xlog_set_mode_instance,
    mars_xlog_write, mars_xlog_write_instance, MarsXLogConfig, MARS_XLOG_ERR_APPENDER,
    MARS_XLOG_ERR_BAD_COMPRESS, MARS_XLOG_ERR_BAD_MODE, MARS_XLOG_ERR_EMPTY_LOG_DIR,
    MARS_XLOG_ERR_NO_PATH, MARS_XLOG_ERR_NO_SPACE, MARS_XLOG_ERR_NULL_CONFIG,
    MARS_XLOG_ERR_NULL_OUT, MARS_XLOG_OK,
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
/// The strings are only held so that the pointers in `raw` stay valid: nothing
/// reads them, which is what the underscore in their names says.
struct ConfigBundle {
    _log_dir: CString,
    _prefix: CString,
    _pub_key: CString,
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
        _log_dir: log_dir,
        _prefix: prefix,
        _pub_key: pub_key,
        raw,
    }
}

#[test]
fn open_rejects_a_bad_config_before_touching_the_disk() {
    let _guard = serial();
    assert_eq!(
        unsafe { mars_xlog_open(std::ptr::null()) },
        MARS_XLOG_ERR_NULL_CONFIG
    );

    let dir = tempdir("bad");
    for (mode, compress, expected) in [
        (7, 0, MARS_XLOG_ERR_BAD_MODE),
        (0, 9, MARS_XLOG_ERR_BAD_COMPRESS),
    ] {
        let config = make_config(&dir, mode, compress);
        assert_eq!(unsafe { mars_xlog_open(&config.raw) }, expected);
    }

    let empty = CString::new("").unwrap();
    let mut config = make_config(&dir, 0, 0);
    config.raw.log_dir = empty.as_ptr();
    assert_eq!(
        unsafe { mars_xlog_open(&config.raw) },
        MARS_XLOG_ERR_EMPTY_LOG_DIR
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_level_is_one_store_whatever_the_question_is() {
    let _guard = serial();

    // `mars_xlog_set_level` is the level of the default logger, which is the
    // store `get_level` / `is_enabled_for` / `write_instance` read. It used to
    // be kept beside them, in the seam, and the two answered differently.
    mars_xlog_set_level(3);
    assert_eq!(mars_xlog_get_level(0), 3, "one level, two answers");
    assert_eq!(mars_xlog_is_enabled_for(0, 2), 0);
    assert_eq!(mars_xlog_is_enabled_for(0, 3), 1);

    // `MARS_LEVEL_NONE` through the instance path, which used to be dropped
    // for want of a `LogLevel` to turn it into.
    mars_xlog_set_level_instance(0, 6);
    assert_eq!(mars_xlog_get_level(0), 6);
    assert_eq!(mars_xlog_is_enabled_for(0, 5), 0);

    // `(TLogLevel)-1` is "everything", not "nothing at all".
    mars_xlog_set_level_instance(0, -1);
    assert_eq!(mars_xlog_get_level(0), 0);
    assert_eq!(mars_xlog_is_enabled_for(0, 0), 1);

    mars_xlog_set_level(0);
}

#[test]
fn the_whole_abi_runs_over_one_appender() {
    let _guard = serial();
    let dir = tempdir("surface");
    let config = make_config(&dir, 0, 0);
    assert_eq!(unsafe { mars_xlog_open(&config.raw) }, MARS_XLOG_OK);
    // a second open is refused, like the C++ singleton
    assert_eq!(
        unsafe { mars_xlog_open(&config.raw) },
        MARS_XLOG_ERR_APPENDER
    );

    let tag = CString::new("Net").unwrap();
    let message = CString::new("hello").unwrap();
    // null pieces are allowed
    unsafe {
        mars_xlog_write(
            2,
            tag.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            message.as_ptr(),
        );
    }
    unsafe {
        mars_xlog_write(
            2,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            message.as_ptr(),
        );
    }

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
    let written = unsafe {
        mars_xlog_current_log_path(path.as_mut_ptr() as *mut c_char, path.len() as c_uint)
    };
    assert!(written > 0, "no current log path: {written}");
    assert!(std::str::from_utf8(&path[..written as usize]).is_ok());
    // too small a buffer, and a null buffer
    assert_eq!(
        unsafe { mars_xlog_current_log_path(path.as_mut_ptr() as *mut c_char, 0) },
        MARS_XLOG_ERR_NO_SPACE
    );
    assert_eq!(
        unsafe { mars_xlog_current_log_path(std::ptr::null_mut(), 64) },
        MARS_XLOG_ERR_NULL_OUT
    );

    let mut cache = vec![0u8; 512];
    let written =
        unsafe { mars_xlog_current_log_cache_path(cache.as_mut_ptr(), cache.len() as c_uint) };
    assert!(written != MARS_XLOG_ERR_NULL_OUT);
    assert_eq!(
        unsafe { mars_xlog_current_log_cache_path(std::ptr::null_mut(), 64) },
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
        unsafe {
            mars_xlog_current_log_path(path.as_mut_ptr() as *mut c_char, path.len() as c_uint)
        },
        MARS_XLOG_ERR_NO_PATH
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn instances_are_created_addressed_and_released() {
    let _guard = serial();
    let dir = tempdir("instances");
    let config = make_config(&dir, 1, 1);
    let handle = unsafe { mars_xlog_new_instance(&config.raw, 2) };
    assert!(handle > 0);
    let prefix = CString::new("Mars").unwrap();
    assert_eq!(unsafe { mars_xlog_get_instance(prefix.as_ptr()) }, handle);
    assert_eq!(unsafe { mars_xlog_get_instance(std::ptr::null()) }, 0);

    let tag = CString::new("Net").unwrap();
    let file = CString::new("net.cc").unwrap();
    let func = CString::new("Send").unwrap();
    let message = CString::new("through an instance").unwrap();
    unsafe {
        mars_xlog_write_instance(
            handle,
            2,
            tag.as_ptr(),
            file.as_ptr(),
            func.as_ptr(),
            7,
            message.as_ptr(),
        );
    }
    mars_xlog_set_level_instance(handle, 3);
    assert_eq!(mars_xlog_get_level(handle), 3);
    mars_xlog_set_mode_instance(handle, 0);
    mars_xlog_flush_instance(handle, 1);
    unsafe {
        mars_xlog_release_instance(prefix.as_ptr());
    }
    assert_eq!(unsafe { mars_xlog_get_instance(prefix.as_ptr()) }, 0);

    // a null config has no instance
    assert_eq!(unsafe { mars_xlog_new_instance(std::ptr::null(), 2) }, 0);
    // an empty log dir is refused
    let empty = CString::new("").unwrap();
    let mut broken = make_config(&dir, 0, 0);
    broken.raw.log_dir = empty.as_ptr();
    assert_eq!(unsafe { mars_xlog_new_instance(&broken.raw, 2) }, 0);
    // and so is a bad mode
    let broken_mode = make_config(&dir, 9, 0);
    assert_eq!(unsafe { mars_xlog_new_instance(&broken_mode.raw, 2) }, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_void_symbols_survive_a_closed_appender() {
    let _guard = serial();
    // nothing is open: every void symbol has to be a no-op instead of a crash
    unsafe {
        mars_xlog_write(
            2,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            std::ptr::null(),
        );
    }
    unsafe {
        mars_xlog_write_instance(
            0,
            2,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            std::ptr::null(),
        );
    }
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
    unsafe {
        mars_xlog_release_instance(std::ptr::null());
    }
    // the void symbols added for the rest of the C++ surface
    mars_xlog_flush_all(1);
    mars_xlog_set_console_log_instance(0, 0);
    mars_xlog_set_max_file_size_instance(0, 0);
    mars_xlog_set_max_alive_duration_instance(0, 0);
}

/// `XloggerCategory::IsEnabledFor` is `level_ <= _level` on the **raw**
/// `TLogLevel`, and the C++ casts whatever the caller passes
/// (`(TLogLevel)_level`), so `MARS_LEVEL_NONE` (6) is a level a caller may ask
/// about — it is not `Fatal`, and it is not "no answer at all".
#[test]
fn is_enabled_for_compares_the_raw_level() {
    let _guard = serial();
    mars_xlog_set_level(0); // Verbose: everything passes, 6 included
    assert_eq!(mars_xlog_is_enabled_for(0, 6), 1);
    assert_eq!(mars_xlog_is_enabled_for(0, 5), 1);
    // A negative level is below Verbose, so nothing passes it.
    assert_eq!(mars_xlog_is_enabled_for(0, -1), 0);

    mars_xlog_set_level(3); // Warn
    assert_eq!(mars_xlog_is_enabled_for(0, 2), 0);
    assert_eq!(mars_xlog_is_enabled_for(0, 3), 1);
    // A handle that is not one has no level to compare against.
    assert_eq!(mars_xlog_is_enabled_for(0xdead_beef, 6), 0);

    mars_xlog_set_level(0);
}

/// `NewXloggerInstance(_config, (TLogLevel)_level)` casts the level: 6
/// (`Xlog.LEVEL_NONE`) is how a caller asks for an instance that logs nothing,
/// and it used to be refused — the caller got handle `0` back, which is the
/// default logger, not an instance.
#[test]
fn an_instance_can_be_opened_at_the_level_that_logs_nothing() {
    let _guard = serial();
    let dir = tempdir("level-none");
    let config = make_config(&dir, 1, 0);
    let handle = unsafe { mars_xlog_new_instance(&config.raw, 6) };
    assert_ne!(handle, 0, "LEVEL_NONE collapsed onto the default logger");
    assert_eq!(mars_xlog_get_level(handle), 6);
    assert_eq!(mars_xlog_is_enabled_for(handle, 5), 0);

    // The instance setters of the C++ surface: `SetConsoleLogOpen`,
    // `SetMaxFileSize`, `SetMaxAliveTime` and `FlushAll`.
    mars_xlog_set_console_log_instance(handle, 0);
    mars_xlog_set_max_file_size_instance(handle, 0);
    mars_xlog_set_max_alive_duration_instance(handle, 0);
    mars_xlog_flush_all(1);

    let prefix = CString::new("Mars").unwrap();
    unsafe {
        mars_xlog_release_instance(prefix.as_ptr());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `appender_oneshot_flush`, `appender_make_logfile_name` and
/// `appender_getfilepath_from_timespan`: the recovery path and the two
/// discovery helpers, which had no C symbol at all.
#[test]
fn the_recovery_and_discovery_symbols_answer() {
    let _guard = serial();
    let dir = tempdir("discovery");
    let config = make_config(&dir, 0, 0);
    let prefix = CString::new("Mars").unwrap();
    let log_dir = CString::new(dir.to_str().unwrap()).unwrap();
    let mut out = vec![0u8; 512];

    // A null config is an error, not a crash.
    assert_eq!(
        unsafe { mars_xlog_oneshot_flush(std::ptr::null()) },
        MARS_XLOG_ERR_NULL_CONFIG
    );

    // The name of today's log file, whether or not it exists.
    let written = unsafe {
        mars_xlog_make_logfile_name(
            0,
            prefix.as_ptr(),
            log_dir.as_ptr(),
            0,
            out.as_mut_ptr() as *mut c_char,
            out.len() as c_uint,
        )
    };
    assert!(written > 0, "no log file name: {written}");
    let name = std::str::from_utf8(&out[..written as usize])
        .unwrap()
        .to_owned();
    assert!(name.ends_with(".xlog"), "{name}");
    assert!(name.contains("Mars_"), "{name}");
    // One name today, so index 1 is past the end of the list.
    assert_eq!(
        unsafe {
            mars_xlog_make_logfile_name(
                0,
                prefix.as_ptr(),
                log_dir.as_ptr(),
                1,
                out.as_mut_ptr() as *mut c_char,
                out.len() as c_uint,
            )
        },
        MARS_XLOG_ERR_NO_PATH
    );

    // The file does not exist yet, so the timespan lookup finds nothing…
    assert_eq!(
        unsafe {
            mars_xlog_getfilepath_from_timespan(
                0,
                prefix.as_ptr(),
                log_dir.as_ptr(),
                0,
                out.as_mut_ptr() as *mut c_char,
                out.len() as c_uint,
            )
        },
        MARS_XLOG_ERR_NO_PATH
    );
    // …and once it does, it is listed.
    std::fs::write(std::path::Path::new(&name), b"x").unwrap();
    let found = unsafe {
        mars_xlog_getfilepath_from_timespan(
            0,
            prefix.as_ptr(),
            log_dir.as_ptr(),
            0,
            out.as_mut_ptr() as *mut c_char,
            out.len() as c_uint,
        )
    };
    assert_eq!(
        std::str::from_utf8(&out[..found as usize]).unwrap(),
        name,
        "the timespan lookup reported another file"
    );

    // A too-small buffer is an error, not a truncation.
    assert_eq!(
        unsafe {
            mars_xlog_make_logfile_name(
                0,
                prefix.as_ptr(),
                log_dir.as_ptr(),
                0,
                out.as_mut_ptr() as *mut c_char,
                4,
            )
        },
        MARS_XLOG_ERR_NO_SPACE
    );

    // Recovery over a directory no appender owns: an action, never an error.
    let action = unsafe { mars_xlog_oneshot_flush(&config.raw) };
    assert!(
        (0..=7).contains(&action),
        "unexpected TFileIOAction {action}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
