//! Every symbol of the C ABI, called from Rust the way a C/C++/JNI caller
//! would: with valid arguments, and with the bad arguments that have to be
//! reported as `MARS_XLOG_ERR_*` instead of crashing.
//!
//! The appender they share is registered under one prefix, so the tests
//! serialise on [`LOCK`].

use std::ffi::{c_char, CString};
use std::os::raw::c_int;
use std::sync::{Mutex, MutexGuard, OnceLock};

use mars_ffi::abi::{
    mars_xlog_current_log_path_instance, mars_xlog_flush_now_instance, mars_xlog_get_instance,
    mars_xlog_get_level, mars_xlog_getfilepath_from_timespan_instance, mars_xlog_is_enabled_for,
    mars_xlog_make_logfile_name_instance, mars_xlog_new_instance, mars_xlog_release_instance,
    mars_xlog_release_instance_of, mars_xlog_set_console_log_instance,
    mars_xlog_set_level_instance, mars_xlog_set_max_alive_duration_instance,
    mars_xlog_set_max_file_size_instance, mars_xlog_set_mode_instance, mars_xlog_write_instance,
    MarsXLogConfig, MARS_XLOG_ERR_NO_PATH, MARS_XLOG_ERR_NO_SPACE, MARS_XLOG_ERR_NULL_OUT,
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

/// A prefix is a name and not text, so one that is not UTF-8 is kept — with
/// U+FFFD for the byte that is not — and the appender is registered under it.
/// The lookup and the release have to read it the same way: read as a `&str`,
/// the whole of such a prefix is `""`, so a caller found no appender where it
/// had just opened one and never closed it.
#[test]
fn a_prefix_that_is_not_utf8_is_the_one_the_appender_was_opened_with() {
    let _guard = serial();
    let dir = tempdir("lossy-prefix");
    let log_dir = CString::new(dir.to_str().unwrap()).unwrap();
    let pub_key = CString::new("").unwrap();
    let prefix = CString::new([b'a', b'p', b'p', 0xff, b'n', b'a', b'm', b'e']).unwrap();
    let raw = MarsXLogConfig {
        mode: 0,
        log_dir: log_dir.as_ptr(),
        name_prefix: prefix.as_ptr(),
        pub_key: pub_key.as_ptr(),
        compress_mode: 0,
        compress_level: 0,
        cache_dir: std::ptr::null(),
        cache_days: 0,
    };

    let handle = unsafe { mars_xlog_new_instance(&raw, 0) };
    assert!(handle > 0, "an instance is a handle");
    assert_eq!(
        unsafe { mars_xlog_get_instance(prefix.as_ptr()) },
        handle,
        "the prefix it was opened with names it"
    );

    unsafe {
        mars_xlog_release_instance(prefix.as_ptr());
    }
    assert_eq!(
        unsafe { mars_xlog_get_instance(prefix.as_ptr()) },
        0,
        "and it is the one that closed it"
    );
    let _ = std::fs::remove_dir_all(&dir);
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
    let dir = tempdir("level");
    let config = make_config(&dir, 0, 0);
    let handle = unsafe { mars_xlog_new_instance(&config.raw, 0) };
    assert!(handle > 0, "an instance is a handle");

    // One level, three readers: `get_level`, `is_enabled_for` and the write
    // all read the store the setter wrote, and not a copy beside it — which
    // is what the seam used to keep, and what made the two answer differently.
    mars_xlog_set_level_instance(handle, 3);
    assert_eq!(mars_xlog_get_level(handle), 3, "one level, two answers");
    assert_eq!(mars_xlog_is_enabled_for(handle, 2), 0);
    assert_eq!(mars_xlog_is_enabled_for(handle, 3), 1);

    // `MARS_LEVEL_NONE` through the instance path, which used to be dropped
    // for want of a `LogLevel` to turn it into.
    mars_xlog_set_level_instance(handle, 6);
    assert_eq!(mars_xlog_get_level(handle), 6);
    assert_eq!(mars_xlog_is_enabled_for(handle, 5), 0);

    // `(TLogLevel)-1` is "everything", not "nothing at all".
    mars_xlog_set_level_instance(handle, -1);
    assert_eq!(mars_xlog_get_level(handle), 0);
    assert_eq!(mars_xlog_is_enabled_for(handle, 0), 1);

    // Handle `0` is no logger at all, so neither question has a level to
    // answer from: it answers what a released instance answers.
    mars_xlog_set_level_instance(0, 3);
    assert_eq!(mars_xlog_get_level(0), -1);
    assert_eq!(mars_xlog_is_enabled_for(0, 6), 0);

    let prefix = CString::new("Mars").unwrap();
    unsafe {
        mars_xlog_release_instance(prefix.as_ptr());
    }
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

/// Releasing takes the prefix and not the handle, so a caller that asks the
/// registry and then releases is answered twice and not once: an open of the
/// same prefix that lands between the two is handed a handle of its own, and
/// the release closes that appender instead of the one the caller asked about.
/// `mars_xlog_release_instance_of` is the two under one lock.
#[test]
fn a_release_names_the_instance_it_closes() {
    let _guard = serial();
    let dir = tempdir("release-of");
    let config = make_config(&dir, 1, 0);
    let prefix = CString::new("Mars").unwrap();
    let first = unsafe { mars_xlog_new_instance(&config.raw, 2) };
    assert!(first > 0, "an instance is a handle");

    // A handle that is not the one this prefix is registered under closes
    // nothing at all, `0` included: releasing took the prefix only, and both
    // of these would have closed this appender.
    unsafe {
        mars_xlog_release_instance_of(prefix.as_ptr(), 0);
    }
    assert_eq!(unsafe { mars_xlog_get_instance(prefix.as_ptr()) }, first);
    unsafe {
        mars_xlog_release_instance_of(prefix.as_ptr(), first + 1);
    }
    assert_eq!(unsafe { mars_xlog_get_instance(prefix.as_ptr()) }, first);

    // The handle it *is* registered under does.
    unsafe {
        mars_xlog_release_instance_of(prefix.as_ptr(), first);
    }
    assert_eq!(unsafe { mars_xlog_get_instance(prefix.as_ptr()) }, 0);

    // And the same call again — a second `close` of a prefix another part of
    // the app has opened since — is what the re-open race looks like from the
    // caller that lost it: the appender that is there now is not the one this
    // handle named, so it stays.
    let second = unsafe { mars_xlog_new_instance(&config.raw, 2) };
    assert!(second > 0, "the prefix opens again");
    unsafe {
        mars_xlog_release_instance_of(prefix.as_ptr(), first);
    }
    assert_eq!(
        unsafe { mars_xlog_get_instance(prefix.as_ptr()) },
        second,
        "a stale handle closes none but its own"
    );
    unsafe {
        mars_xlog_release_instance_of(prefix.as_ptr(), second);
    }
    assert_eq!(unsafe { mars_xlog_get_instance(prefix.as_ptr()) }, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `IsEnabledFor` answers about the raw level a record carries, so
/// (`(TLogLevel)_level`) — so `-1` is asked about as `-1`, and not as the
/// `Verbose` the filter would make of it.
///
/// What the raw comparison does not answer is a level no record can carry:
/// `MARS_LEVEL_NONE` (6) is a filter, and the write refuses it as a level, so
/// it is answered `0` and not `1`. Answering `1` there told a caller a write
/// was coming that the write itself drops.
#[test]
fn is_enabled_for_answers_about_the_levels_a_record_can_have() {
    let _guard = serial();
    let dir = tempdir("enabled");
    let config = make_config(&dir, 0, 0);
    let handle = unsafe { mars_xlog_new_instance(&config.raw, 0) };
    assert!(handle > 0, "an instance is a handle");

    mars_xlog_set_level_instance(handle, 0); // Verbose: every record passes
                                             // `MARS_LEVEL_NONE` is a filter and not a record's level — the write
                                             // refuses it — so the answer is `0` even when everything else passes.
    assert_eq!(mars_xlog_is_enabled_for(handle, 6), 0);
    // … and so is anything past it, which the C++ would have answered `1` to.
    assert_eq!(mars_xlog_is_enabled_for(handle, 7), 0);
    assert_eq!(mars_xlog_is_enabled_for(handle, 100), 0);
    assert_eq!(mars_xlog_is_enabled_for(handle, 5), 1);
    // A negative level is below Verbose, so nothing passes it.
    assert_eq!(mars_xlog_is_enabled_for(handle, -1), 0);

    mars_xlog_set_level_instance(handle, 3); // Warn
    assert_eq!(mars_xlog_is_enabled_for(handle, 2), 0);
    assert_eq!(mars_xlog_is_enabled_for(handle, 3), 1);
    // A handle that is not one has no level to compare against.
    assert_eq!(mars_xlog_is_enabled_for(0xdead_beef, 6), 0);
    // … and neither has handle `0`, which is no instance at all.
    assert_eq!(mars_xlog_is_enabled_for(0, 6), 0);

    let prefix = CString::new("Mars").unwrap();
    unsafe {
        mars_xlog_release_instance(prefix.as_ptr());
    }
    let _ = std::fs::remove_dir_all(&dir);

    mars_xlog_set_level_instance(0, 0);
}

/// The three questions about a file, asked of the instance that owns it: the
/// directory it is writing into, and the day's files and names.
///
/// The walk is the C ABI's own protocol — one index at a time until it answers
/// `MARS_XLOG_ERR_NO_PATH` — so it is pinned here too, along with the two
/// buffer contracts: a null `out`, and one a path does not fit in.
#[test]
fn the_file_questions_are_answered_by_the_instance() {
    let _guard = serial();
    let dir = tempdir("files");
    let config = make_config(&dir, 1, 1); // sync, zstd: a record lands at once
    let handle = unsafe { mars_xlog_new_instance(&config.raw, 2) };
    assert!(handle > 0);

    let tag = CString::new("Net").unwrap();
    let message = CString::new("into the day's file").unwrap();
    unsafe {
        mars_xlog_write_instance(
            handle,
            2,
            tag.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            message.as_ptr(),
        );
    }
    mars_xlog_flush_now_instance(handle);

    // The directory, and not the file: the C++'s `GetCurrentLogPath` hands back
    // `sg_logdir`.
    let mut buffer = [0u8; 1024];
    let size = dir.to_str().unwrap().len();
    let written = unsafe {
        mars_xlog_current_log_path_instance(handle, buffer.as_mut_ptr().cast(), buffer.len() as u32)
    };
    assert_eq!(written as usize, size);
    assert_eq!(&buffer[..size], dir.to_str().unwrap().as_bytes());

    // Today's file, and nothing beside it.
    let files = day_paths(handle, |instance, timespan, index, out, len| unsafe {
        mars_xlog_getfilepath_from_timespan_instance(instance, timespan, index, out, len)
    });
    assert_eq!(
        files.len(),
        1,
        "one record in one day is one file: {files:?}"
    );
    assert!(files[0].starts_with(&dir), "{files:?} is not in {dir:?}");

    let names = day_paths(handle, |instance, timespan, index, out, len| unsafe {
        mars_xlog_make_logfile_name_instance(instance, timespan, index, out, len)
    });
    assert_eq!(names, files, "a file that is there is its own name");

    // A null `out`, and one a path does not fit in.
    assert_eq!(
        unsafe { mars_xlog_current_log_path_instance(handle, std::ptr::null_mut(), 1024) },
        MARS_XLOG_ERR_NULL_OUT
    );
    assert_eq!(
        unsafe { mars_xlog_current_log_path_instance(handle, buffer.as_mut_ptr().cast(), 3) },
        MARS_XLOG_ERR_NO_SPACE
    );

    // A released appender answers nothing at all.
    let mut after = [0u8; 1024];
    unsafe {
        mars_xlog_release_instance(config.raw.name_prefix);
    }
    let closed = unsafe {
        mars_xlog_current_log_path_instance(handle, after.as_mut_ptr().cast(), after.len() as u32)
    };
    assert_eq!(
        closed, MARS_XLOG_ERR_NO_PATH,
        "a released appender answered {closed} and not 'no path'"
    );
}

/// The four setters the C ABI carries and no getter answers for — the mode, the
/// console and the two sizes — are the four no test called. They are asked for
/// here, and what is asserted is the one of the four whose effect can be seen
/// from outside: a file that is closed once it reaches its size.
#[test]
fn the_setters_the_abi_has_no_getter_for_take_effect() {
    let _guard = serial();
    let dir = tempdir("setters");
    let config = make_config(&dir, 1, 1); // sync, so every record is filed at once
    let handle = unsafe { mars_xlog_new_instance(&config.raw, 2) };
    assert!(handle > 0, "an instance is a handle");

    mars_xlog_set_mode_instance(handle, 1);
    mars_xlog_set_console_log_instance(handle, 0);
    mars_xlog_set_max_alive_duration_instance(handle, 0);
    // Small enough that one record closes the file: what shows the setter
    // reached the appender is the second file that opens after it.
    mars_xlog_set_max_file_size_instance(handle, 64);

    let tag = CString::new("Net").unwrap();
    let message =
        CString::new("a record long enough to pass sixty-four bytes, twice over").unwrap();
    for _ in 0..4 {
        unsafe {
            mars_xlog_write_instance(
                handle,
                2,
                tag.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                message.as_ptr(),
            );
        }
        mars_xlog_flush_now_instance(handle);
    }

    let split: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "xlog"))
        .collect();
    assert!(
        split.len() > 1,
        "a max file size of 64 bytes did not split the day: {} file(s)",
        split.len()
    );

    // And the appender is still an appender afterwards.
    assert_eq!(mars_xlog_get_level(handle), 2);
    let mut out = [0u8; 1024];
    let written = unsafe {
        mars_xlog_current_log_path_instance(handle, out.as_mut_ptr().cast(), out.len() as u32)
    };
    assert!(written > 0, "the directory was answered as {written}");

    let prefix = CString::new("Mars").unwrap();
    unsafe {
        mars_xlog_release_instance(prefix.as_ptr());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The index walk the two day-of-files symbols share: up to the first index
/// they answer nothing for.
fn day_paths(
    handle: i64,
    symbol: unsafe fn(i64, c_int, u32, *mut c_char, u32) -> c_int,
) -> Vec<std::path::PathBuf> {
    let mut walked = Vec::new();
    let mut buffer = [0u8; 1024];
    for index in 0..64u32 {
        let written = unsafe {
            symbol(
                handle,
                0,
                index,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
            )
        };
        if written == MARS_XLOG_ERR_NO_PATH {
            break;
        }
        assert!(written > 0, "index {index} answered {written}");
        walked.push(std::path::PathBuf::from(
            std::str::from_utf8(&buffer[..written as usize]).unwrap(),
        ));
    }
    walked
}
