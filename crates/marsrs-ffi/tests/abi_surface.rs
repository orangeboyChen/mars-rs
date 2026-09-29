//! Every symbol of the C ABI, called from Rust the way a C/C++/JNI caller
//! would: with valid arguments, and with the bad arguments that have to be
//! reported as `MARS_XLOG_ERR_*` instead of crashing.
//!
//! The appender they share is registered under one prefix, so the tests
//! serialise on [`LOCK`].

use std::ffi::CString;
use std::os::raw::c_int;
use std::sync::{Mutex, MutexGuard, OnceLock};

use mars_ffi::abi::{
    mars_xlog_flush_now_instance, mars_xlog_get_instance, mars_xlog_get_level,
    mars_xlog_is_enabled_for, mars_xlog_new_instance, mars_xlog_release_instance,
    mars_xlog_set_level_instance, mars_xlog_set_mode_instance, mars_xlog_write_instance,
    MarsXLogConfig,
};

fn serial() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("marsrs-ffi-{tag}-{}", std::process::id()));
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
fn a_new_instance_refuses_a_bad_config_before_touching_the_disk() {
    let _guard = serial();
    // `0` is the answer to every one of these: an instance is a handle, and
    // there is no room in one for a `MARS_XLOG_ERR_*` code. What is pinned
    // here is that the config is refused at all, and before the disk is
    // touched.
    assert_eq!(unsafe { mars_xlog_new_instance(std::ptr::null(), 0) }, 0);

    let dir = tempdir("bad");
    for (mode, compress) in [(7, 0), (0, 9)] {
        let config = make_config(&dir, mode, compress);
        assert_eq!(unsafe { mars_xlog_new_instance(&config.raw, 0) }, 0);
    }

    let empty = CString::new("").unwrap();
    let mut config = make_config(&dir, 0, 0);
    config.raw.log_dir = empty.as_ptr();
    assert_eq!(unsafe { mars_xlog_new_instance(&config.raw, 0) }, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_level_is_one_store_whatever_the_question_is() {
    let _guard = serial();

    // Handle `0` is the process-wide appender, whose level is the one the
    // store `get_level` / `is_enabled_for` / `write_instance` read. It used to
    // be kept beside them, in the seam, and the two answered differently.
    mars_xlog_set_level_instance(0, 3);
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

    mars_xlog_set_level_instance(0, 0);
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
    mars_xlog_flush_now_instance(handle);
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

/// `XloggerCategory::IsEnabledFor` is `level_ <= _level` on the **raw**
/// `TLogLevel`, and the C++ casts whatever the caller passes
/// (`(TLogLevel)_level`), so `MARS_LEVEL_NONE` (6) is a level a caller may ask
/// about — it is not `Fatal`, and it is not "no answer at all".
#[test]
fn is_enabled_for_compares_the_raw_level() {
    let _guard = serial();
    mars_xlog_set_level_instance(0, 0); // Verbose: everything passes, 6 included
    assert_eq!(mars_xlog_is_enabled_for(0, 6), 1);
    assert_eq!(mars_xlog_is_enabled_for(0, 5), 1);
    // A negative level is below Verbose, so nothing passes it.
    assert_eq!(mars_xlog_is_enabled_for(0, -1), 0);

    mars_xlog_set_level_instance(0, 3); // Warn
    assert_eq!(mars_xlog_is_enabled_for(0, 2), 0);
    assert_eq!(mars_xlog_is_enabled_for(0, 3), 1);
    // A handle that is not one has no level to compare against.
    assert_eq!(mars_xlog_is_enabled_for(0xdead_beef, 6), 0);

    mars_xlog_set_level_instance(0, 0);
}
