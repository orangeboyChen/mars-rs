//! `xlogger_memory_dump()` — the hex dump helper from
//! `mars/xlog/src/appender.cc`.
//!
//! The C++ returns a `const char*` into a `thread_local std::string`, so the
//! buffer only survives until the next call on the same thread. The port
//! returns an owned [`String`] instead, which is the same text without the
//! lifetime trap.

/// `kMaxDumpLength` in `appender.cc`.
const MAX_DUMP_LENGTH: usize = 4096;
/// Bytes per dump line (`32` in `xlogger_memory_dump`, `16` in `Dump`).
const LINE_BYTES: usize = 32;
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

        push_dump_line(&mut out, &bytes[offset..offset + line]);
        offset += line;
        // next line
        out.push('\n');
    }

    out
}

/// `calc_dump_required_length()` — `srcbytes * 6 + 1` (3 per byte for the hex
/// column, 3 per byte for the text column, plus the newline in between).
fn dump_line_len(line: usize) -> usize {
    line * 6 + 1
}

/// `to_string()` — the hex column, a newline, then the text column.
fn push_dump_line(out: &mut String, chunk: &[u8]) {
    for byte in chunk {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
        out.push(' ');
    }
    out.push('\n');
    for byte in chunk {
        // `isgraph(c) ? c : ' '` in the C locale: ASCII 0x21..=0x7e.
        let printable = byte.is_ascii_graphic();
        out.push(if printable { *byte as char } else { ' ' });
        out.push(' ');
        out.push(' ');
    }
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
}
