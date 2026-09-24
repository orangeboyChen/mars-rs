//! Port of `mars/xlog/src/formater.cc` (`mars::xlog::log_formater`) and of the
//! small helpers it uses from `mars/comm/xlogger/loginfo_extract.cc`.
//!
//! The formatter turns one `(XLoggerInfo, body)` pair into the single-line
//! record that later gets compressed/encrypted and appended to the `.xlog`
//! file:
//!
//! ```text
//! [L][yyyy-mm-dd +z hh:mm:ss.mmm][pid, tid*][tag][file:line, func][body\n]
//! ```

use std::sync::atomic::{AtomicIsize, Ordering};

use chrono::{Datelike, Local, TimeZone, Timelike};
use mars_xlog_core::PtrBuffer;

use crate::config::{LogLevel, XLoggerInfo};

/// `levelStrings[]` in `formater.cc` / `ConsoleLog.cc`.
pub(crate) const LEVEL_STRINGS: [&str; 6] = ["V", "D", "I", "W", "E", "F"];

/// `mars::comm::ExtractFileName` — the part of `_path` after the last
/// `/` or `\`.
pub(crate) fn extract_file_name(path: Option<&str>) -> &str {
    match path {
        None => "",
        Some(p) => match p.rsplit(['/', '\\']).next() {
            Some(name) => name,
            None => p,
        },
    }
}

/// Formats `timeval` like the C++ `snprintf`:
/// `"%d-%02d-%02d %+.1f %02d:%02d:%02d.%.3d"` with `tm_gmtoff / 3600.0`.
///
/// Returns an empty string when the timestamp cannot be represented (mirrors
/// the C++ `temp_time[64] = {0}` fallback when `tv_sec == 0`).
fn format_timeval(timeval: (i64, i64)) -> String {
    let usec = timeval.1;
    let nsec = (usec.rem_euclid(1_000_000) * 1_000) as u32;
    let Some(dt) = Local.timestamp_opt(timeval.0, nsec).single() else {
        return String::new();
    };

    let offset_hours = f64::from(dt.offset().local_minus_utc()) / 3600.0;
    let millis = usec / 1000;

    format!(
        "{}-{:02}-{:02} {:+.1} {:02}:{:02}:{:02}.{:03}",
        dt.year(),
        dt.month(),
        dt.day(),
        offset_hours,
        dt.hour(),
        dt.minute(),
        dt.second(),
        millis
    )
}

/// `mars::xlog::log_formater`.
///
/// Writes the formatted record into `out`, keeping every guard of the C++
/// original:
///
/// * when fewer than 5 KiB of room are left, only
///   `[F]log_size <= 5*1024, err(count, size)\n` is emitted (and only if 128
///   bytes still fit);
/// * the header is capped at 1023 bytes (the C++ `snprintf` limit);
/// * the body is capped at `0xFFFF` bytes and at `max_length - length - 130`;
/// * a trailing `\n` is appended when the last byte written is not one.
pub fn log_formater(info: Option<&XLoggerInfo>, logbody: Option<&str>, out: &mut PtrBuffer<'_>) {
    // C++: `static int error_count = 0; static int error_size = 0;`
    static ERROR_COUNT: AtomicIsize = AtomicIsize::new(0);
    static ERROR_SIZE: AtomicIsize = AtomicIsize::new(0);

    if out.max_length() <= out.len() + 5 * 1024 {
        let count = ERROR_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
        // C++: `strnlen(_logbody, 1024 * 1024)`; `NULL` body is not dereferenced here.
        let size = logbody.map_or(0, |body| body.len().min(1024 * 1024)) as isize;
        ERROR_SIZE.store(size, Ordering::SeqCst);

        if out.max_length() >= out.len() + 128 {
            let msg = format!("[F]log_size <= 5*1024, err({count}, {size})\n");
            let ret = msg.len().min(1023);
            out.write(&msg.as_bytes()[..ret]);
            // C++: `_log.Write("")` — a zero-length write.
            out.write(b"");

            ERROR_COUNT.store(0, Ordering::SeqCst);
            ERROR_SIZE.store(0, Ordering::SeqCst);
        }

        return;
    }

    if let Some(info) = info {
        let temp_time = if info.timeval.0 != 0 {
            format_timeval(info.timeval)
        } else {
            String::new()
        };
        let filename = extract_file_name(info.filename.as_deref());
        let func_name = info.func_name.as_deref().unwrap_or("");
        let tag = info.tag.as_deref().unwrap_or("");
        // C++: `_logbody ? levelStrings[_info->level] : levelStrings[kLevelFatal]`
        let level = if logbody.is_some() {
            LEVEL_STRINGS[info.level as usize]
        } else {
            LEVEL_STRINGS[LogLevel::Fatal as usize]
        };
        let main_thread = if info.tid == info.maintid { "*" } else { "" };

        let header = format!(
            "[{}][{}][{}, {}{}][{}][{}:{}, {}][",
            level, temp_time, info.pid, info.tid, main_thread, tag, filename, info.line, func_name
        );

        // C++ writes through `snprintf(..., 1024, ...)`, so at most 1023 bytes.
        let ret = header.len().min(1023);
        out.write(&header.as_bytes()[..ret]);
    }

    if let Some(body) = logbody {
        // C++: `bodylen = MaxLength() - Length() > 130 ? ... - 130 : 0`,
        // then clamped to `0xFFFF` and finally to `strnlen(_logbody, bodylen)`.
        let room = out.max_length() - out.len();
        let mut bodylen = room.saturating_sub(130);
        bodylen = bodylen.min(0xFFFF);
        bodylen = body.len().min(bodylen);
        out.write(&body.as_bytes()[..bodylen]);
    } else {
        out.write(b"error!! NULL==_logbody");
    }

    // C++: `if (*((char*)_log.PosPtr() - 1) != nextline) _log.Write(&nextline, 1);`
    // The C++ reads one byte before the cursor; when nothing was written at all
    // that is out of bounds, so the port treats "nothing written" as "already
    // terminated".
    let last = if out.pos() > 0 {
        out.as_slice()[out.pos() - 1]
    } else {
        b'\n'
    };
    if last != b'\n' {
        out.write(b"\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn info(level: LogLevel) -> XLoggerInfo {
        XLoggerInfo {
            level,
            tag: Some("tag".to_owned()),
            filename: Some("/tmp/src/hello.cc".to_owned()),
            func_name: Some("main".to_owned()),
            line: 42,
            pid: 100,
            tid: 200,
            maintid: 200,
            timeval: (1_700_000_000, 123_456),
        }
    }

    #[test]
    fn record_shape() {
        let mut backing = [0u8; 16 * 1024];
        let mut buf = PtrBuffer::new(&mut backing);
        log_formater(Some(&info(LogLevel::Info)), Some("hello body"), &mut buf);

        let s = std::str::from_utf8(&buf.as_slice()[..buf.len()]).unwrap();
        assert!(s.starts_with("[I]["), "{s}");
        assert!(s.contains("][100, 200*][tag][hello.cc:42, main]["), "{s}");
        assert!(s.ends_with("hello body\n"), "{s}");

        // The second field is "yyyy-mm-dd +z hh:mm:ss.mmm", i.e. 28 chars with
        // the offset in hours (`tm_gmtoff / 3600.0`, "%+.1f").
        let timestamp = s.split("][").nth(1).unwrap();
        assert_eq!(timestamp.len(), 28, "{timestamp}");
        assert_eq!(&timestamp[10..11], " ");
        assert!(
            timestamp[11..].starts_with('+') || timestamp[11..].starts_with('-'),
            "{timestamp}"
        );
        assert_eq!(&timestamp[15..16], " ");
        assert_eq!(&timestamp[18..19], ":");
        assert_eq!(&timestamp[21..22], ":");
        assert!(
            timestamp.ends_with(".123"),
            "millis of 123456 usec: {timestamp}"
        );
    }

    #[test]
    fn non_main_thread_has_no_star() {
        let mut i = info(LogLevel::Debug);
        i.maintid = 999;
        let mut backing = [0u8; 16 * 1024];
        let mut buf = PtrBuffer::new(&mut backing);
        log_formater(Some(&i), Some("x"), &mut buf);
        let s = std::str::from_utf8(&buf.as_slice()[..buf.len()]).unwrap();
        assert!(s.contains("][100, 200]["), "{s}");
    }

    #[test]
    fn null_info_writes_body_only() {
        // 16 KiB, like the `char temp[16 * 1024]` of `__WriteSync`.
        let mut backing = [0u8; 16 * 1024];
        let mut buf = PtrBuffer::new(&mut backing);
        log_formater(None, Some("raw"), &mut buf);
        assert_eq!(&buf.as_slice()[..buf.len()], b"raw\n");
    }

    #[test]
    fn null_body_writes_error_marker() {
        let mut backing = [0u8; 16 * 1024];
        let mut buf = PtrBuffer::new(&mut backing);
        log_formater(None, None, &mut buf);
        assert_eq!(&buf.as_slice()[..buf.len()], b"error!! NULL==_logbody\n");
    }

    #[test]
    fn newline_is_not_duplicated() {
        let mut backing = [0u8; 16 * 1024];
        let mut buf = PtrBuffer::new(&mut backing);
        log_formater(None, Some("already terminated\n"), &mut buf);
        assert_eq!(&buf.as_slice()[..buf.len()], b"already terminated\n");
    }

    #[test]
    fn small_buffer_emits_the_5k_guard() {
        // 6 KiB buffer: `max_length <= length + 5 KiB` as soon as 1 KiB is used.
        let mut backing = [0u8; 6 * 1024];
        let mut buf = PtrBuffer::new(&mut backing);
        buf.write(&[b'x'; 1024]);
        log_formater(None, Some("body"), &mut buf);

        let s = std::str::from_utf8(&buf.as_slice()[1024..buf.len()]).unwrap();
        assert!(s.starts_with("[F]log_size <= 5*1024, err("), "{s}");
        assert!(s.ends_with(")\n"), "{s}");
    }

    #[test]
    fn guard_is_skipped_when_less_than_128_bytes_are_left() {
        let mut backing = [0u8; 5 * 1024 + 64];
        let mut buf = PtrBuffer::new(&mut backing);
        log_formater(None, Some("body"), &mut buf);
        // max_length (5K+64) <= 0 + 5K? No — 5184 > 5120, so it formats normally.
        let s = std::str::from_utf8(&buf.as_slice()[..buf.len()]).unwrap();
        assert_eq!(s, "body\n");

        // Now the same but with the guard active and < 128 bytes of room.
        let mut small = [0u8; 5 * 1024 + 64];
        let mut buf = PtrBuffer::new(&mut small);
        buf.write(&[b'x'; 5 * 1024]);
        log_formater(None, Some("body"), &mut buf);
        assert_eq!(
            buf.len(),
            5 * 1024,
            "nothing is appended when < 128 bytes are left"
        );
    }

    #[test]
    fn body_is_capped_by_the_130_byte_reserve() {
        let mut backing = [0u8; 16 * 1024];
        let mut buf = PtrBuffer::new(&mut backing);
        let body = "y".repeat(32 * 1024);
        log_formater(None, Some(&body), &mut buf);
        assert_eq!(
            buf.len(),
            16 * 1024 - 130 + 1,
            "body + the appended newline"
        );
        assert!(buf.as_slice()[..buf.len() - 1].iter().all(|b| *b == b'y'));
    }

    #[test]
    fn timestamp_uses_local_offset() {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let tv = (now.as_secs() as i64, 1_000_000i64 + 5_000);
        let s = format_timeval(tv);
        // "%Y-%m-%d %+.1f %H:%M:%S.mmm"
        assert!(s.contains(':'), "{s}");
        let (date, rest) = s.split_once(' ').unwrap();
        assert_eq!(date.len(), 10, "{s}");
        let (offset, _clock) = rest.split_once(' ').unwrap();
        assert!(offset.starts_with('+') || offset.starts_with('-'), "{s}");
        assert!(
            offset.contains('.'),
            "the offset is formatted with one decimal: {s}"
        );
    }

    #[test]
    fn extract_file_name_matches_cpp() {
        assert_eq!(extract_file_name(None), "");
        assert_eq!(extract_file_name(Some("a/b/c.cc")), "c.cc");
        assert_eq!(extract_file_name(Some("c:\\tmp\\d.cc")), "d.cc");
        assert_eq!(extract_file_name(Some("plain.cc")), "plain.cc");
    }
}
