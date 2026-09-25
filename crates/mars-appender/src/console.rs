//! Port of `mars/xlog/unix/ConsoleLog.cc` (`mars::xlog::ConsoleLog`).
//!
//! The C++ `printf`s to stdout; the port writes to stderr so that it does not
//! mix with a program's normal output (the record format is unchanged).

use std::io::Write;

use crate::config::XLoggerInfo;
use crate::formater::{extract_file_name, LEVEL_STRINGS};

/// `mars::xlog::ConsoleLog`.
///
/// Does nothing when `_info` is `None` (the C++ returns early on
/// `NULL == _info`).
pub(crate) fn console_log(info: Option<&XLoggerInfo>, log: &str) {
    let Some(info) = info else {
        return;
    };

    let level = LEVEL_STRINGS[info.level as usize];
    let tag = info.tag.as_deref().unwrap_or("");
    let file_name = extract_file_name(info.filename.as_deref());
    let func_name = info.func_name.as_deref().unwrap_or("");

    // `eprintln!` panics when stderr cannot be written (EPIPE, full device).
    // On the async writer thread there is no panic barrier, so that one panic
    // would end logging for the whole process: ignore the error instead.
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
            tag: Some("tag".to_owned()),
            filename: Some("/a/b/c.cc".to_owned()),
            func_name: Some("fn".to_owned()),
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
            tag: Some("tag".to_owned()),
            filename: Some("/a/b/c.cc".to_owned()),
            func_name: Some("fn".to_owned()),
            line: 7,
            ..Default::default()
        };
        console_log(Some(&info), "message");
    }
}
