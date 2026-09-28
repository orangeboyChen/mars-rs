//! Port of `mars/xlog/unix/ConsoleLog.cc` (`mars::xlog::ConsoleLog`).
//!
//! The C++ `printf`s to stdout; the port writes to stderr so that it does not
//! mix with a program's normal output (the record format is unchanged).

use std::io::Write;
use std::sync::RwLock;

use crate::config::XLoggerInfo;
use crate::formater::{extract_file_name, extract_function_name, LEVEL_STRINGS};

/// `mars::xlog::TConsoleFun` — where a console record goes instead of the
/// built-in sink, handed to [`set_console_fun`].
///
/// The C++ `TConsoleFun` is an Apple-only enum of three sinks of its own
/// (`kConsolePrintf`, `kConsoleNSLog`, `kConsoleOSLog` — see
/// `mars/xlog/objc/objc_console.mm`), and `os_log_with_type` is a macro with
/// no symbol to link against, so the port cannot offer the three. What it
/// offers instead is the sink itself: a record and its info, handed over
/// unformatted so that the app decides what a console record looks like on
/// the platform it is running on.
pub type ConsoleFun = fn(&XLoggerInfo, &str);

/// `sg_console_fun` of `mars/xlog/objc/objc_console.mm` — the sink every
/// console record is handed to, `None` until an app sets one.
static CONSOLE_FUN: RwLock<Option<ConsoleFun>> = RwLock::new(None);

/// `appender_set_console_fun` — takes the sink every console record is handed
/// to, or takes it away again with `None`.
///
/// The C++ declares it under `#ifdef __APPLE__`, which is where its own
/// default sink is one an app wants to replace; the port's default sink is
/// stderr on every platform, so there is no platform on which an app has more
/// reason to replace it than on another.
pub fn set_console_fun(fun: Option<ConsoleFun>) {
    *CONSOLE_FUN
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = fun;
}

/// The sink [`set_console_fun`] was last given, if it was given one.
pub fn get_console_fun() -> Option<ConsoleFun> {
    *CONSOLE_FUN
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `mars::xlog::ConsoleLog`.
///
/// Does nothing when `_info` is `None` (the C++ returns early on
/// `NULL == _info`). A sink an app set takes the record from here, unformatted:
/// the built-in stderr line is only the default.
pub(crate) fn console_log(info: Option<&XLoggerInfo>, log: &str) {
    let Some(info) = info else {
        return;
    };

    if let Some(fun) = get_console_fun() {
        fun(info, log);
        return;
    }

    let level = LEVEL_STRINGS[info.level as usize];
    let tag = info.tag.as_deref().unwrap_or("");
    let file_name = extract_file_name(info.filename.as_deref());
    // `ConsoleLog.cc` trims the name on every platform (`char
    // strFuncName[128]` filled by `ExtractFunctionName`), unlike `formater.cc`,
    // which only does it on Windows.
    let func_name = extract_function_name(info.func_name.as_deref());

    // The `io::Result` of the write is dropped rather than propagated: a
    // stderr that cannot be written to — closed, or a pipe with no reader —
    // is not a reason to fail the record being logged, and on the async
    // writer thread a panic has nothing to unwind into. `eprintln!`, the
    // obvious way to write this line, panics on that same failure (`failed
    // printing to stderr`), so one closed stderr would end logging for the
    // process.
    let _ = writeln!(
        std::io::stderr(),
        "[{level}][{tag}][{file_name}, {func_name}, {}][{log}",
        info.line
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LogLevel;
    use std::sync::Mutex;

    #[test]
    fn console_log_does_not_panic() {
        let info = XLoggerInfo {
            level: LogLevel::Warn,
            tag: Some("tag".into()),
            filename: Some("/a/b/c.cc".into()),
            func_name: Some("fn".into()),
            line: 7,
            ..Default::default()
        };
        console_log(Some(&info), "message");
        console_log(None, "no info");
    }

    #[test]
    fn console_log_of_the_level_that_disables_logging_does_not_panic() {
        // `kLevelNone` is one past the C++ `levelStrings[]`.
        let info = XLoggerInfo {
            level: LogLevel::None,
            tag: Some("tag".into()),
            filename: Some("/a/b/c.cc".into()),
            func_name: Some("fn".into()),
            line: 7,
            ..Default::default()
        };
        console_log(Some(&info), "message");
    }

    /// A sink an app set is handed the record instead of the built-in stderr
    /// line, and it is handed everything that line would have said.
    #[test]
    fn a_sink_an_app_set_is_handed_the_record() {
        let _guard = crate::test_lock::serial();
        // The sink is one process-wide static, so it has to be taken away
        // again — including when this test panicked, or every other test
        // writing with the console open would write through it.
        struct NoSink;
        impl Drop for NoSink {
            fn drop(&mut self) {
                set_console_fun(None);
            }
        }
        let _no_sink = NoSink;

        static SEEN: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new();
        fn remember(info: &XLoggerInfo, log: &str) {
            *SEEN
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                format!("{}:{log}", info.level as i32);
        }

        set_console_fun(Some(remember));
        let info = XLoggerInfo {
            level: LogLevel::Error,
            ..Default::default()
        };
        console_log(Some(&info), "to-the-app");
        // A record with no info does not reach a sink either, in the C++ as
        // here: `ConsoleLog` returns before it is asked.
        console_log(None, "no-info");

        let seen = SEEN
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(seen, "4:to-the-app");
    }
}
