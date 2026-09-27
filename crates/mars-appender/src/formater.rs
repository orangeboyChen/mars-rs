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

use std::fmt::{self, Write};
use std::sync::atomic::{AtomicIsize, Ordering};

use chrono::{DateTime, Datelike, Timelike};
use mars_core::{local_time, PtrBuffer};

use crate::config::{LogLevel, XLoggerInfo};

/// `levelStrings[]` in `formater.cc` / `ConsoleLog.cc`, plus the one level the
/// C++ array has no string for: `kLevelNone` is 6 and `kLevelFatal` is the last
/// entry, so `levelStrings[_info->level]` reads one past the end for it. A
/// record of that level is not impossible — `Xlog.logWrite` hands the port
/// whatever `Xlog.LEVEL_*` the caller passed — and the port writes `N` where
/// the C++ reads out of bounds.
pub(crate) const LEVEL_STRINGS: [&str; 7] = ["V", "D", "I", "W", "E", "F", "N"];

/// `char strFuncName[128]` in `ConsoleLog.cc` / `formater.cc`, minus the
/// terminating `NUL`.
const MAX_FUNCTION_NAME: usize = 127;

/// `mars::comm::ExtractFunctionName` (`loginfo_extract.c`).
///
/// Trims a compiler signature down to the bare name: the part after the last
/// `' '` (the return type, `void Foo::bar(int)` → `bar`) or after a `"::"`, up
/// to the `'('` of the parameter list — or up to a `':'` / `']'`, which is how
/// the Objective-C `-[Class method]` form ends. The result is capped at 127
/// bytes, like the 128-byte buffer the C++ copies into.
///
/// When nothing can be trimmed (or the trimmed name would be a single byte) the
/// whole signature is kept, again capped: the C++ `strncpy`s the original into
/// the same buffer.
///
/// The answer is a slice of `_func`, because that is what the C++ hands out
/// too: a `memcpy` out of the caller's `__FUNCTION__` into a stack array. The
/// port used to return an owned `String`, which is a heap allocation per
/// record on the console path — where the C++ spends none.
pub(crate) fn extract_function_name(func: Option<&str>) -> &str {
    let Some(func) = func else {
        // `NULL == _func` returns without touching the buffer: an empty name.
        return "";
    };

    let bytes = func.as_bytes();
    let mut start = 0usize;
    let mut end: Option<usize> = None;
    let mut pos = 0usize;
    while pos < bytes.len() {
        if end.is_none() && bytes[pos] == b' ' {
            pos += 1;
            start = pos;
            continue;
        }
        if bytes[pos] == b':' && bytes.get(pos + 1) == Some(&b':') {
            pos += 2;
            start = pos;
            continue;
        }
        if bytes[pos] == b'(' {
            end = Some(pos);
        } else if bytes[pos] == b':' || bytes[pos] == b']' {
            end = Some(pos);
            break;
        }
        pos += 1;
    }

    match end {
        // `start + 1 >= end`: a one-byte name is not a name, so the C++ keeps
        // the whole signature.
        Some(end) if start + 1 < end => {
            // Both ends are ASCII delimiters, so the range is a whole slice;
            // only the 127-byte cut can land inside a character, and it is
            // pulled back to the previous boundary like [`cap`] does.
            let mut len = (end - start).min(MAX_FUNCTION_NAME);
            while len > 0 && !func.is_char_boundary(start + len) {
                len -= 1;
            }
            &func[start..start + len]
        }
        _ => cap(func),
    }
}

/// The C++'s `strncpy(_func_ret, _func, _len)`: at most 127 bytes, never in the
/// middle of a UTF-8 sequence.
fn cap(func: &str) -> &str {
    let mut len = func.len().min(MAX_FUNCTION_NAME);
    while len > 0 && !func.is_char_boundary(len) {
        len -= 1;
    }
    &func[..len]
}

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

/// `char temp_time[64]` in `formater.cc`, and the `char[1024]` its header
/// `snprintf` writes into: both on the stack, which is why formatting a record
/// does not touch the heap.
const TEMP_TIME_SIZE: usize = 64;
const HEADER_SIZE: usize = 1024;

/// `snprintf` into a buffer the caller owns.
///
/// The C++ formats into stack arrays and truncates when they fill up; this does
/// the same, so a record costs no allocation — the port used to build two
/// `String`s per record, on the one path a logger has to be able to run even
/// when the allocator is the thing that is struggling.
///
/// The integer overloads below exist because `write!` is not free: a
/// `{}` of an integer costs ~30 ns of `Formatter` machinery, and one header has
/// five of them. Writing the digits here — into the same buffer, with the same
/// truncation — is what keeps the header at the cost of the bytes it writes.
struct Snprintf<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> Snprintf<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, len: 0 }
    }

    fn len(&self) -> usize {
        self.len
    }

    /// `snprintf`'s truncation: write what fits, drop the rest.
    fn push(&mut self, bytes: &[u8]) {
        let room = self.buf.len().saturating_sub(self.len);
        let n = bytes.len().min(room);
        self.buf[self.len..self.len + n].copy_from_slice(&bytes[..n]);
        self.len += n;
    }

    /// `%0*u` — `value` in decimal, padded on the left with `0` to `width`
    /// digits. A value wider than `width` is written in full, like `printf`.
    ///
    /// No `%`: [`clippy::modulo_arithmetic`] is `deny` in this workspace, and
    /// `value - (value / 10) * 10` is the same digit anyway.
    fn push_decimal(&mut self, value: u64, width: usize) {
        // The digits come out backwards; 20 is what `u64::MAX` needs.
        let mut digits = [0u8; 20];
        let mut count = 0usize;
        let mut rest = value;
        while rest > 0 && count < digits.len() {
            let digit = usize::try_from(rest - (rest / 10) * 10).unwrap_or(0);
            digits[count] = DIGITS[digit.min(DIGITS.len() - 1)];
            rest /= 10;
            count += 1;
        }
        // `0` has no digits of its own, and `printf` still writes one.
        if count == 0 {
            digits[0] = DIGITS[0];
            count = 1;
        }

        for _ in count..width {
            self.push(&[DIGITS[0]]);
        }
        for index in (0..count).rev() {
            self.push(&digits[index..index + 1]);
        }
    }

    /// `%d` / `%0*d` — [`Self::push_decimal`] with a sign in front of it.
    fn push_signed(&mut self, value: i64, width: usize) {
        if value < 0 {
            self.push(b"-");
        }
        self.push_decimal(value.unsigned_abs(), width);
    }
}

/// `0`..`9`, so [`Snprintf::push_decimal`] never has to do arithmetic on a
/// digit.
const DIGITS: &[u8; 10] = b"0123456789";

impl Write for Snprintf<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push(s.as_bytes());
        Ok(())
    }
}

/// `"yyyy-mm-dd +12.75 hh:mm:ss."` — the widest `"%Y-%m-%d %+.1f %H:%M:%S."`
/// there is, and the buffer is never indexed past what was written.
const STAMP_TEXT_SIZE: usize = 32;

/// The header's timestamp for one second, up to and including the `.` before
/// the milliseconds.
///
/// Everything but the milliseconds is decided by `tv_sec` alone, so a second
/// of logging renders it once: the C++ `snprintf`s all of it per record
/// (`formater.cc`), and so did the port — six integer conversions and a float,
/// ~60 ns of a ~150 ns header, for an answer that changes once a second.
#[derive(Clone, Copy)]
struct LocalStamp {
    /// The `tv_sec` this was rendered for.
    secs: i64,
    /// `"yyyy-mm-dd +z hh:mm:ss."`, not NUL-terminated.
    text: [u8; STAMP_TEXT_SIZE],
    /// How much of [`Self::text`] is the timestamp.
    len: usize,
}

thread_local! {
    /// The last second's [`LocalStamp`] on this thread.
    static LOCAL_STAMP: std::cell::Cell<Option<LocalStamp>> =
        const { std::cell::Cell::new(None) };
}

/// The `"yyyy-mm-dd +z hh:mm:ss."` of `secs`, cached for that second.
///
/// The offset comes from [`local_time`], which the day-rollover check and the
/// crypt hour share — one `localtime` per second for the whole record, which is
/// what replaces the per-record one the C++ does (`formater.cc:90`).
fn local_stamp(secs: i64) -> Option<LocalStamp> {
    LOCAL_STAMP.with(|cell| {
        if let Some(cached) = cell.get() {
            if cached.secs == secs {
                return Some(cached);
            }
        }

        let offset = local_time(secs).offset;
        // `localtime` hands back local fields; `secs + gmtoff` read as UTC *are*
        // those fields, and `DateTime::from_timestamp` is integer arithmetic
        // where `Local` is a time-zone lookup — 2 ns against ~90 ns.
        let local = DateTime::from_timestamp(secs + i64::from(offset), 0)?;

        let mut text = [0u8; STAMP_TEXT_SIZE];
        let mut out = Snprintf::new(&mut text);
        out.push_decimal(u64::try_from(local.year()).unwrap_or(0), 0);
        out.push(b"-");
        out.push_decimal(u64::from(local.month()), 2);
        out.push(b"-");
        out.push_decimal(u64::from(local.day()), 2);
        out.push(b" ");
        // The one float in a record, formatted exactly the way the C++ formats
        // it — an offset changes twice a year, so this line runs twice a year.
        let _ = write!(out, "{:+.1}", f64::from(offset) / 3600.0);
        out.push(b" ");
        out.push_decimal(u64::from(local.hour()), 2);
        out.push(b":");
        out.push_decimal(u64::from(local.minute()), 2);
        out.push(b":");
        out.push_decimal(u64::from(local.second()), 2);
        out.push(b".");

        let len = out.len();
        let fresh = LocalStamp { secs, text, len };
        cell.set(Some(fresh));
        Some(fresh)
    })
}

/// Formats `timeval` into `out` the way the C++ `snprintf` does:
/// `"%d-%02d-%02d %+.1f %02d:%02d:%02d.%.3d"` with `tm_gmtoff / 3600.0`.
///
/// Writes nothing when the timestamp cannot be represented, which mirrors the
/// C++ `char temp_time[64] = {0}` fallback when `tv_sec == 0`.
fn write_timeval(timeval: (i64, i64), out: &mut Snprintf<'_>) {
    let Some(stamp) = local_stamp(timeval.0) else {
        return;
    };

    out.push(&stamp.text[..stamp.len]);
    // `%.3d` of `tv_usec / 1000`: a negative microsecond count prints its sign
    // inside the three digits, like the C++ does.
    out.push_signed(timeval.1 / 1000, 3);
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
            let mut msg = [0u8; HEADER_SIZE];
            let mut msg = Snprintf::new(&mut msg);
            let _ = writeln!(msg, "[F]log_size <= 5*1024, err({count}, {size})");
            let ret = msg.len().min(1023);
            out.write(&msg.buf[..ret]);
            // C++: `_log.Write("")` — a zero-length write.
            out.write(b"");

            ERROR_COUNT.store(0, Ordering::SeqCst);
            ERROR_SIZE.store(0, Ordering::SeqCst);
        }

        return;
    }

    if let Some(info) = info {
        let filename = extract_file_name(info.filename.as_deref());
        // `#if _WIN32` in `formater.cc`: only there is the name trimmed into a
        // `char[128]` first. Everywhere else the C++ hands `strFuncName` the
        // caller's `__FUNCTION__` unchanged.
        let func_name = if cfg!(windows) {
            extract_function_name(info.func_name.as_deref())
        } else {
            info.func_name.as_deref().unwrap_or("")
        };
        let tag = info.tag.as_deref().unwrap_or("");
        // C++: `_logbody ? levelStrings[_info->level] : levelStrings[kLevelFatal]`
        let level = if logbody.is_some() {
            LEVEL_STRINGS[info.level as usize]
        } else {
            LEVEL_STRINGS[LogLevel::Fatal as usize]
        };
        let main_thread = if info.tid == info.maintid { "*" } else { "" };

        // `char temp_time[64]` — `snprintf` truncates at its own 63 bytes
        // before the header embeds it, which is why it gets a buffer of its
        // own.
        let mut temp_time = [0u8; TEMP_TIME_SIZE];
        let mut temp_time = Snprintf::new(&mut temp_time);
        if info.timeval.0 != 0 {
            write_timeval(info.timeval, &mut temp_time);
        }
        let temp_time = &temp_time.buf[..temp_time.len()];

        let mut header = [0u8; HEADER_SIZE];
        let mut header = Snprintf::new(&mut header);
        // C++: `"[%s][%s][%PRIdMAX, %PRIdMAX%s][%s][%s:%d, %s]["`. Five of
        // those conversions are integers, and `write!` spent ~150 ns of a
        // record on them; [`Snprintf`] writes the same bytes by hand.
        header.push(b"[");
        header.push(level.as_bytes());
        header.push(b"][");
        header.push(temp_time);
        header.push(b"][");
        header.push_signed(info.pid, 0);
        header.push(b", ");
        header.push_signed(info.tid, 0);
        header.push(main_thread.as_bytes());
        header.push(b"][");
        header.push(tag.as_bytes());
        header.push(b"][");
        header.push(filename.as_bytes());
        header.push(b":");
        header.push_signed(i64::from(info.line), 0);
        header.push(b", ");
        header.push(func_name.as_bytes());
        header.push(b"][");

        // C++ writes through `snprintf(..., 1024, ...)`, so at most 1023 bytes.
        let ret = header.len().min(1023);
        out.write(&header.buf[..ret]);
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

    fn info(level: LogLevel) -> XLoggerInfo<'static> {
        XLoggerInfo {
            level,
            tag: Some("tag".into()),
            filename: Some("/tmp/src/hello.cc".into()),
            func_name: Some("main".into()),
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

        // The second field is "yyyy-mm-dd +z hh:mm:ss.mmm", with the offset in
        // hours (`tm_gmtoff / 3600.0`, "%+.1f"): four characters for an offset
        // inside ten hours ("+5.5") and five for one outside it ("+10.0"),
        // because the C++ does not pad it. What is pinned is the two spaces
        // around the offset and not where they are — a fixed width here is a
        // test that only passes west of Australia.
        let timestamp = s.split("][").nth(1).unwrap();
        let (date, rest) = timestamp.split_once(' ').unwrap();
        assert_eq!(date.len(), 10, "{timestamp}");
        let (offset, clock) = rest.split_once(' ').unwrap();
        assert!(
            offset.starts_with('+') || offset.starts_with('-'),
            "{timestamp}"
        );
        assert!(
            offset[1..].contains('.'),
            "the offset is formatted with one decimal: {timestamp}"
        );
        assert_eq!(clock.len(), 12, "hh:mm:ss.mmm: {timestamp}");
        assert_eq!(&clock[2..3], ":");
        assert_eq!(&clock[5..6], ":");
        assert!(
            clock.ends_with(".123"),
            "millis of 123456 usec: {timestamp}"
        );
    }

    #[test]
    fn a_record_of_the_level_that_disables_logging_has_a_string() {
        // `kLevelNone` (6) is one past the C++ `levelStrings[]`, which stops at
        // `kLevelFatal`: the C++ reads out of bounds, the port used to panic.
        let mut backing = [0u8; 16 * 1024];
        let mut buf = PtrBuffer::new(&mut backing);
        log_formater(Some(&info(LogLevel::None)), Some("body"), &mut buf);

        let s = std::str::from_utf8(&buf.as_slice()[..buf.len()]).unwrap();
        assert!(s.starts_with("[N]["), "{s}");
        assert!(s.contains("][100, 200*][tag][hello.cc:42, main]["), "{s}");
        assert!(s.ends_with("body\n"), "{s}");
    }

    #[test]
    fn every_level_has_a_string() {
        // The array is indexed with the level, so a level with no string is a
        // panic on the write path.
        for level in [
            LogLevel::Verbose,
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
            LogLevel::Fatal,
            LogLevel::None,
        ] {
            assert!(!LEVEL_STRINGS[level as usize].is_empty(), "{level:?}");
        }
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
        let mut buf = [0u8; TEMP_TIME_SIZE];
        let mut out = Snprintf::new(&mut buf);
        write_timeval(tv, &mut out);
        let s = String::from_utf8_lossy(&out.buf[..out.len()]).into_owned();
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

    /// The shapes `ExtractFunctionName` was written for: a C++ signature, a
    /// namespaced one, and the Objective-C `-[Class method]` form.
    #[test]
    fn extract_function_name_trims_a_signature() {
        assert_eq!(extract_function_name(None), "");
        assert_eq!(extract_function_name(Some("main")), "main");
        assert_eq!(
            extract_function_name(Some("void Foo::bar(int)")),
            "bar",
            "the return type and the namespace are dropped"
        );
        assert_eq!(
            extract_function_name(Some("-[Foo bar]")),
            "bar",
            "an ObjC selector loses its class"
        );
        // Not trimmed: a name of one byte is not a name, so the C++ keeps the
        // whole signature.
        assert_eq!(extract_function_name(Some("f(")), "f(");
    }

    #[test]
    fn extract_function_name_is_capped_at_the_cpp_buffer() {
        let long = format!("void {}::long_name(int)", "N".repeat(200));
        let name = extract_function_name(Some(&long));
        assert_eq!(name, "long_name");

        // Nothing to trim, so the whole signature is copied — and cut off at
        // the 127 bytes of `char strFuncName[128]`, never mid-character.
        let untrimmed = "ü".repeat(100);
        let capped = extract_function_name(Some(&untrimmed));
        assert_eq!(capped.len(), 126, "127 bytes leaves a `ü` whole");
        assert!(untrimmed.starts_with(capped));
    }
}
