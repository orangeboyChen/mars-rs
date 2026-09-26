//! `xlogger_memory_dump()` and `xlogger_dump()` — the hex dump helpers from
//! `mars/xlog/src/appender.cc`.
//!
//! The C++ returns a `const char*` into a `thread_local std::string`, so the
//! buffer only survives until the next call on the same thread. The port
//! returns an owned [`String`] instead, which is the same text without the
//! lifetime trap.

use std::fs;
use std::path::Path;

use chrono::{Datelike, Local, Timelike};

/// `kMaxDumpLength` in `appender.cc`.
const MAX_DUMP_LENGTH: usize = 4096;
/// Bytes per dump line (`32` in `xlogger_memory_dump`, `16` in `Dump`).
const LINE_BYTES: usize = 32;
/// Bytes per line of `XloggerAppender::Dump`.
const DUMP_LINE_BYTES: usize = 16;
/// `for (int x = 0; x < 32 && dump_len < (int)_len; ++x)` in `Dump`: at most 32
/// lines, i.e. 512 bytes of the blob.
const DUMP_MAX_LINES: usize = 32;
/// `HEX_STRING` in `appender.cc` — lower-case hex, no `0x` prefix.
const HEX: &[u8; 16] = b"0123456789abcdef";

/// `xlogger_memory_dump(const void* _dumpbuffer, size_t _len)`.
///
/// Renders `bytes` as
///
/// ```text
/// \n<len> bytes:\n
/// <hex bytes separated by spaces>\n
/// <printable bytes separated by spaces>\n
/// \n
/// ```
///
/// repeated until the input is consumed or the 4096 byte budget is reached.
/// Empty input produces an empty string, exactly like the C++.
pub fn xlogger_memory_dump(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push('\n');
    out.push_str(&bytes.len().to_string());
    out.push_str(" bytes:\n");

    let mut offset = 0;
    while offset < bytes.len() && out.len() < MAX_DUMP_LENGTH {
        // `bytes = std::min(len - src_offset, LINE_BYTES)`, then shrunk until
        // the rendered line fits in what is left of the budget.
        let left = MAX_DUMP_LENGTH - out.len();
        let mut line = std::cmp::min(bytes.len() - offset, LINE_BYTES);
        while line > 0 && dump_line_len(line) >= left {
            line -= 1;
        }
        if line == 0 {
            break;
        }

        out.push_str(&dump_line(&bytes[offset..offset + line]));
        offset += line;
        // next line
        out.push('\n');
    }

    out
}

/// `XloggerAppender::Dump` — writes the blob to
/// `<logdir>/<YYYYMMDD>/<YYYYMMDDHHMMSS>_<len>.dump` and reports where it went.
///
/// The report is `"\n dump file to <path> :\n"` followed by up to 32 lines of
/// 16 bytes — a *recordable* dump, because the file outlives the process that
/// wrote it. Empty when there is nothing to dump, no log directory, or the file
/// cannot be written: the C++ `ASSERT2`s and returns `""` on a failed `fopen`.
pub(crate) fn dump_to_logdir(bytes: &[u8], logdir: &Path) -> String {
    if bytes.is_empty() || logdir.as_os_str().is_empty() {
        return String::new();
    }

    // `%d%02d%02d` / `%d%02d%02d%02d%02d%02d_%d.dump` of the local time the
    // dump is taken, which is why two dumps in the same second overwrite each
    // other unless their lengths differ — as in the C++.
    let now = Local::now();
    let day_dir = logdir.join(format!(
        "{:04}{:02}{:02}",
        now.year(),
        now.month(),
        now.day()
    ));
    if let Err(err) = fs::create_dir_all(&day_dir) {
        eprintln!("[mars-appender] xlogger_dump: {}: {err}", day_dir.display());
        return String::new();
    }

    let path = day_dir.join(format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}_{}.dump",
        now.year(),
        now.month(),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        bytes.len()
    ));
    if let Err(err) = fs::write(&path, bytes) {
        eprintln!("[mars-appender] xlogger_dump: {}: {err}", path.display());
        return String::new();
    }

    let mut out = format!("\n dump file to {} :\n", path.display());
    for chunk in bytes.chunks(DUMP_LINE_BYTES).take(DUMP_MAX_LINES) {
        out.push_str(&dump_line(chunk));
        out.push('\n');
    }
    out
}

/// `calc_dump_required_length()` — `srcbytes * 6 + 1` (3 per byte for the hex
/// column, 3 per byte for the text column, plus the newline in between).
fn dump_line_len(line: usize) -> usize {
    line * 6 + 1
}

/// `to_string()` — the hex column, a newline, then the text column: one line
/// of the dump, which the caller pushes onto what it has.
fn dump_line(chunk: &[u8]) -> String {
    let mut line = String::with_capacity(dump_line_len(chunk.len()));
    for byte in chunk {
        line.push(HEX[(byte >> 4) as usize] as char);
        line.push(HEX[(byte & 0x0f) as usize] as char);
        line.push(' ');
    }
    line.push('\n');
    for byte in chunk {
        // `isgraph(c) ? c : ' '` in the C locale: ASCII 0x21..=0x7e.
        let printable = byte.is_ascii_graphic();
        line.push(if printable { *byte as char } else { ' ' });
        line.push(' ');
        line.push(' ');
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_empty_output() {
        assert_eq!(xlogger_memory_dump(&[]), "");
    }

    #[test]
    fn dumps_two_columns_of_32_bytes() {
        // "hello" + a NUL (non printable) + 26 'A' = exactly one 32 byte line.
        let mut bytes = b"hello".to_vec();
        bytes.push(0x00);
        bytes.extend(std::iter::repeat_n(b'A', 26));
        assert_eq!(bytes.len(), 32);

        let dump = xlogger_memory_dump(&bytes);
        let body = dump.strip_prefix("\n32 bytes:\n").expect("header");
        let mut lines = body.split('\n');

        // `to_string()` writes three characters per byte, so every column ends
        // with a trailing space — including the last byte of the line.
        let hex = "68 65 6c 6c 6f 00 ".to_owned() + &"41 ".repeat(26);
        assert_eq!(lines.next().unwrap(), hex);

        let text = "h  e  l  l  o     ".to_owned() + &"A  ".repeat(26);
        assert_eq!(lines.next().unwrap(), text);

        // The C++ appends a newline after every line.
        assert_eq!(lines.next().unwrap(), "");
    }

    #[test]
    fn stops_at_the_length_budget() {
        let bytes = vec![0x5a_u8; 4096];
        let dump = xlogger_memory_dump(&bytes);
        assert!(dump.len() <= MAX_DUMP_LENGTH + 2 * LINE_BYTES * 3);
        // The budget cuts the dump off mid-input rather than truncating a line.
        assert!(dump.len() < bytes.len() * 6);
    }

    /// `calc_dump_required_length()` — what a line costs, in characters.
    #[test]
    fn a_line_costs_six_characters_a_byte_and_the_newline_between_them() {
        assert_eq!(dump_line_len(1), 7);
        assert_eq!(dump_line_len(LINE_BYTES), 193);
        // ... which is why a whole `MAX_DUMP_LENGTH` line can never be rendered
        // inside the budget
        assert_eq!(dump_line_len(MAX_DUMP_LENGTH), 24_577);
    }

    #[test]
    fn the_dump_is_exactly_as_long_as_the_c_plus_plus_makes_it() {
        // `\n<len> bytes:\n`, then `6 * bytes` for the line and one more for
        // the newline that ends it
        assert_eq!(xlogger_memory_dump(&[0x5a]).len(), 18);
        // 121 bytes: three full lines and one of 25
        assert_eq!(xlogger_memory_dump(&[0x5a; 121]).len(), 746);
        // 128 bytes: four full lines
        assert_eq!(xlogger_memory_dump(&[0x5a; 128]).len(), 788);
        // 673 bytes: 21 full ones and one shrunk to a single byte, which is
        // all the budget has room for — so the dump ends there however much
        // input is left, and four kilobytes of it is dumped the same
        assert_eq!(xlogger_memory_dump(&[0x5a; 673]).len(), 4094);
        // ... and four kilobytes of input ends one character further along,
        // which is the extra digit of its own header and nothing else
        assert_eq!(xlogger_memory_dump(&[0x5a; 4096]).len(), 4095);
    }

    #[test]
    fn only_the_graphic_bytes_are_printed_as_themselves() {
        let dump = xlogger_memory_dump(&[0x20, 0x21, 0x7e, 0x7f]);
        let body = dump.strip_prefix("\n4 bytes:\n").expect("header");
        let mut lines = body.split('\n');
        assert_eq!(lines.next().unwrap(), "20 21 7e 7f ");
        // `isgraph(c) ? c : ' '` in the C locale: 0x21..=0x7e are themselves,
        // and the space and DEL either side of them are blanks
        assert_eq!(lines.next().unwrap(), "   !  ~     ");
    }

    #[test]
    fn dump_writes_the_blob_and_reports_where_it_went() {
        let tmp = tempfile::tempdir().unwrap();
        let report = dump_to_logdir(b"hello", tmp.path());
        assert!(report.starts_with("\n dump file to "), "{report}");

        let path = report
            .split(" dump file to ")
            .nth(1)
            .unwrap()
            .split(" :\n")
            .next()
            .unwrap();
        // `<logdir>/<YYYYMMDD>/<YYYYMMDDHHMMSS>_<len>.dump`, and the file holds
        // the blob itself — the point of `Dump` over `xlogger_memory_dump`.
        let written = std::fs::read(path).unwrap();
        assert_eq!(written, b"hello");
        let name = std::path::Path::new(path)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        assert!(name.ends_with("_5.dump"), "{name}");
        let day = std::path::Path::new(path).parent().unwrap();
        assert_eq!(
            day.file_name().unwrap().to_str().unwrap().len(),
            8,
            "the dump lives in a `YYYYMMDD` directory: {path}"
        );

        // 16 bytes a line here, not the 32 of `xlogger_memory_dump`.
        let body = report.split(" :\n").nth(1).unwrap();
        assert_eq!(body.lines().next().unwrap(), "68 65 6c 6c 6f ", "{body}");
        assert_eq!(body.lines().nth(1).unwrap(), "h  e  l  l  o  ", "{body}");
    }

    /// `for (int x = 0; x < 32 ...)`: 32 lines of 16 bytes, however long the
    /// blob is.
    #[test]
    fn dump_reports_at_most_32_lines_of_16_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let report = dump_to_logdir(&[0x5a; 1024], tmp.path());
        let body = report.split(" :\n").nth(1).unwrap();
        // Two lines per chunk: the hex column and the text column.
        assert_eq!(body.lines().count(), 64, "{body}");
        // The whole blob is still written to the file.
        let path = report
            .split(" dump file to ")
            .nth(1)
            .unwrap()
            .split(" :\n")
            .next()
            .unwrap();
        assert_eq!(std::fs::read(path).unwrap().len(), 1024);
    }

    #[test]
    fn a_dump_of_nothing_or_nowhere_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(dump_to_logdir(b"", tmp.path()), "");
        assert_eq!(dump_to_logdir(b"x", std::path::Path::new("")), "");
    }
}
