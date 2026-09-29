//! Port of `mars/xlog/unix/ConsoleLog.cc` (`mars::xlog::ConsoleLog`).
//!
//! The C++ `printf`s to stdout; the port writes to stderr so that it does not
//! mix with a program's normal output (the record format is unchanged).

use std::io::Write;

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
///
/// A sink is the app's own code, and the app's own code may log — a logging
/// adapter that routes every record back through xlog is the ordinary shape
/// of one. Such a record is not handed back to the sink: it takes the
/// built-in stderr line, and the recursion guard of `Appender::write` makes
/// it the one recursive-call diagnostic, so the two of them do not call one
/// another until the stack goes.
pub type ConsoleFun = fn(&XLoggerInfo, &str);

// Whether this thread is inside the sink already: a record logged from inside
// it is not the sink's to have, so [`enter_sink`] answers [`None`] for one.
thread_local! {
    static IN_SINK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks the thread as inside the sink, and takes the mark away again on the
/// way out — a sink that panicked included, or one panic would leave the
/// thread's sink switched off for good.
struct InSink;

impl Drop for InSink {
    fn drop(&mut self) {
        let _ = IN_SINK.try_with(|cell| cell.set(false));
    }
}

/// Takes the thread into the sink: [`None`] when it is in there already, which
/// is a record the sink itself is writing.
fn enter_sink() -> Option<InSink> {
    IN_SINK.with(|cell| {
        if cell.replace(true) {
            None
        } else {
            Some(InSink)
        }
    })
}

/// `mars::xlog::ConsoleLog`.
///
/// Does nothing when `_info` is `None` (the C++ returns early on
/// `NULL == _info`). What it writes is the built-in stderr line, and only that
/// one: the sink an app could set was process-wide, and the port has no
/// process-wide surface left.
pub(crate) fn console_log(info: Option<&XLoggerInfo>, log: &str) {
    let Some(info) = info else {
        return;
    };

    // A record written *from inside* a sink still gets the built-in line,
    // which is what kept a sink that logs from recursing.
    let _in_sink = enter_sink();

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
}
