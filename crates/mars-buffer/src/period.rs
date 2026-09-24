//! Port of `LogBaseBuffer::GetPeriodLogs` and
//! `LogBaseBuffer::GetPeriodLogsWithTimeoutMs` (`log_base_buffer.cc`).
//!
//! The scan walks a finished `.xlog` file record by record
//! (`header | payload | tailer`), looking for the byte range that covers
//! `[begin_hour, end_hour)`. A record whose magic, length or tailer does not
//! add up triggers a *resync*: the scan steps forward one byte at a time until
//! a valid record is found again, exactly like the C++.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use mars_crypt::{magic, LogCrypt, HEADER_LEN, TAILER_LEN};

/// `GetPeriodLogs` — scans `_log_path` for the byte range covering
/// `[begin_hour, end_hour)`.
///
/// Returns `(begin_pos, end_pos)`, or `Err` with the C++-style diagnostic
/// (including the `begintpos/endpos/filesize` report) when no range was found.
pub fn get_period_logs(path: &Path, begin_hour: i32, end_hour: i32) -> Result<(u64, u64), String> {
    let mut err_msg = String::new();
    let magic_end = magic::END;

    // C++: `NULL == _log_path || _end_hour <= _begin_hour`. A Rust `&Path` is
    // never null, so only the hour comparison survives.
    if end_hour <= begin_hour {
        err_msg.push_str(&format!(
            "NULL == _logPath || _endHour <= _beginHour, {begin_hour}, {end_hour}"
        ));
        return Err(err_msg);
    }

    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) => {
            err_msg.push_str(&format!("open file fail:{e}"));
            return Err(err_msg);
        }
    };

    // `fseek(file, 0, SEEK_END)` + `ftell(file)`.
    let file_size = match file.seek(SeekFrom::End(0)) {
        Ok(size) => size,
        Err(e) => {
            err_msg.push_str(&format!("fseek(file, 0, SEEK_END):{e}"));
            return Err(err_msg);
        }
    };

    // `fseek(file, 0, SEEK_SET)`.
    if let Err(e) = file.seek(SeekFrom::Start(0)) {
        err_msg.push_str(&format!("fseek(file, 0, SEEK_SET) error:{e}"));
        return Err(err_msg);
    }

    let mut begin_pos = 0u64;
    let mut end_pos = 0u64;
    let mut find_begin_pos = false;
    let mut last_end_hour = -1i32;
    let mut last_end_pos = 0u64;

    let mut header = [0u8; HEADER_LEN];
    let mut pos = 0u64;
    // `char msg[1024]` — only surfaced when the scan fails.
    let mut msg = String::new();

    while pos < file_size {
        if pos + HEADER_LEN as u64 + TAILER_LEN as u64 > file_size {
            msg.push_str("ftell(file) + __GetHeaderLen() + sizeof(kMagicEnd)) > file_size error");
            break;
        }

        let before_len = pos;
        if let Err(e) = read_exact_at(&mut file, before_len, &mut header) {
            msg.push_str(&format!(
                "fread(buff.Ptr(), 1, __GetHeaderLen(), file) error:{e}, before_len:{before_len}."
            ));
            break;
        }
        pos += HEADER_LEN as u64;

        let mut fix = false;

        if !magic::magic_start_is_valid(header[0]) {
            fix = true;
        } else {
            let len = LogCrypt::get_log_len(&header) as u64;
            if pos + len + TAILER_LEN as u64 > file_size {
                fix = true;
            } else {
                pos += len;
                let mut end = [0u8; TAILER_LEN];
                if let Err(e) = read_exact_at(&mut file, pos, &mut end) {
                    msg.push_str(&format!(
                        "fread(&end, 1, 1, file) err:{e}, before_len:{before_len}, len:{len}."
                    ));
                    break;
                }
                pos += TAILER_LEN as u64;
                if end[0] != magic_end {
                    fix = true;
                }
            }
        }

        if fix {
            // Resync one byte at a time.
            pos = before_len + 1;
            continue;
        }

        let Some((begin, end)) = LogCrypt::get_log_hour(&header) else {
            msg.push_str(&format!(
                "__GetLogHour(buff.Ptr(), buff.Length(), beginHour, endHour) err, before_len:{before_len}."
            ));
            break;
        };

        let mut record_begin = begin as i32;
        let record_end = end as i32;
        if record_begin > record_end {
            record_begin = record_end;
        }

        if err_msg.len() < 1024 * 1024 {
            err_msg.push_str(&format!("{record_begin}-{record_end} "));
        }

        if !find_begin_pos {
            if begin_hour > record_begin && begin_hour <= record_end {
                begin_pos = before_len;
                find_begin_pos = true;
            }

            if begin_hour > last_end_hour && begin_hour <= record_begin {
                begin_pos = before_len;
                find_begin_pos = true;
            }
        }

        if find_begin_pos {
            if end_hour > record_begin && end_hour <= record_end {
                end_pos = pos;
            }

            if end_hour > last_end_hour && end_hour <= record_begin {
                end_pos = last_end_pos;
            }
        }

        last_end_hour = record_end;
        last_end_pos = pos;
    }

    if find_begin_pos && end_hour > last_end_hour {
        end_pos = file_size;
    }

    if end_pos > begin_pos {
        return Ok((begin_pos, end_pos));
    }

    err_msg.push_str(&msg);
    err_msg.push_str(&format!(
        "begintpos:{begin_pos}, endpos:{end_pos}, filesize:{file_size}."
    ));
    Err(err_msg)
}

/// `GetPeriodLogsWithTimeoutMs`.
///
/// The C++ runs the scan on a detached thread because `GetPeriodLogs` can
/// deadlock on iOS; the caller waits on a channel with a timeout instead.
///
/// NOTE(port): the C++ builds its message with `_err_msg + "..."` — the result
/// is discarded, so nothing is reported to the caller. We return it instead,
/// using the same `recv_succ` / `result.succ` wording.
pub fn get_period_logs_with_timeout_ms(
    path: &Path,
    begin_hour: i32,
    end_hour: i32,
    timeout_ms: u32,
) -> Result<(u64, u64), String> {
    let path: PathBuf = path.to_path_buf();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let result = get_period_logs(&path, begin_hour, end_hour);
        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_millis(u64::from(timeout_ms))) {
        Ok(Ok(range)) => Ok(range),
        Ok(Err(err)) => Err(format!("recv_succ:true, result.succ:false, err_msg:{err}")),
        Err(_) => Err("recv_succ:false, result.succ:false".to_string()),
    }
}

/// `fseek` + `fread` helper: reads `dst.len()` bytes at `pos`.
fn read_exact_at(file: &mut File, pos: u64, dst: &mut [u8]) -> std::io::Result<()> {
    file.seek(SeekFrom::Start(pos))?;
    file.read_exact(dst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompressMode, LogBuffer};
    use mars_core::{local_hour, AutoBuffer};

    /// Writes `n` sync records into a fresh file and returns its bytes.
    fn build_xlog(records: &[&str]) -> Vec<u8> {
        let mut buf = LogBuffer::new(true, None, CompressMode::Zlib, 6);
        let mut bytes = Vec::new();
        for record in records {
            let mut out = AutoBuffer::new();
            assert!(buf.write_sync(record.as_bytes(), &mut out));
            bytes.extend_from_slice(out.as_slice());
        }
        bytes
    }

    #[test]
    fn scans_a_file_written_by_write_sync() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.xlog");
        let bytes = build_xlog(&["first\n", "second\n", "third\n"]);
        std::fs::write(&path, &bytes).expect("write");

        let (begin, end) = get_period_logs(&path, 0, 24).expect("scan");
        assert_eq!(begin, 0);
        assert_eq!(end, bytes.len() as u64);

        // The same answer through the timeout wrapper.
        let (begin, end) = get_period_logs_with_timeout_ms(&path, 0, 24, 5_000).expect("scan");
        assert_eq!(begin, 0);
        assert_eq!(end, bytes.len() as u64);
    }

    #[test]
    fn resyncs_after_a_corrupt_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.xlog");

        let mut bytes = vec![0xABu8; 7];
        bytes.extend_from_slice(&build_xlog(&["first\n", "second\n"]));
        std::fs::write(&path, &bytes).expect("write");

        let (begin, end) = get_period_logs(&path, 0, 24).expect("scan");
        assert_eq!(begin, 7, "scan must skip the garbage prefix");
        assert_eq!(end, bytes.len() as u64);
    }

    #[test]
    fn resyncs_after_a_corrupt_record() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.xlog");

        let mut bytes = build_xlog(&["first\n"]);
        // Corrupt the tailer of the first record, then append a good one.
        let last = bytes.len() - 1;
        bytes[last] = 0x7F;
        bytes.extend_from_slice(&build_xlog(&["second\n"]));
        std::fs::write(&path, &bytes).expect("write");

        let (begin, end) = get_period_logs(&path, 0, 24).expect("scan");
        assert_eq!(
            begin,
            bytes.len() as u64 - (HEADER_LEN + 7 + TAILER_LEN) as u64
        );
        assert_eq!(end, bytes.len() as u64);
    }

    #[test]
    fn rejects_bad_hour_ranges_and_missing_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.xlog");
        std::fs::write(&path, build_xlog(&["first\n"])).expect("write");

        let err = get_period_logs(&path, 10, 10).unwrap_err();
        assert!(err.contains("_endHour <= _beginHour"), "{err}");

        let missing = dir.path().join("missing.xlog");
        let err = get_period_logs(&missing, 0, 24).unwrap_err();
        assert!(err.contains("open file fail"), "{err}");
    }

    #[test]
    fn reports_positions_when_nothing_is_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("empty.xlog");
        let bytes = vec![0xABu8; 512];
        std::fs::write(&path, &bytes).expect("write");

        let err = get_period_logs(&path, 0, 24).unwrap_err();
        assert!(
            err.contains("begintpos:0, endpos:0, filesize:512."),
            "{err}"
        );
    }

    #[test]
    fn timeout_wrapper_surfaces_scan_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.xlog");
        std::fs::write(&path, build_xlog(&["first\n"])).expect("write");

        let err = get_period_logs_with_timeout_ms(&path, 10, 5, 5_000).unwrap_err();
        assert!(
            err.starts_with("recv_succ:true, result.succ:false"),
            "{err}"
        );
    }

    #[test]
    fn hour_is_taken_from_the_record_header() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.xlog");
        let bytes = build_xlog(&["first\n"]);
        std::fs::write(&path, &bytes).expect("write");

        let hour = local_hour() as i32;
        let (begin, end) = get_period_logs(&path, hour, hour + 1).expect("scan");
        assert_eq!(begin, 0);
        assert_eq!(end, bytes.len() as u64);

        // A window that ends before the record's hour finds nothing.
        assert!(get_period_logs(&path, 0, 1).is_err() || hour == 0);
    }
}
