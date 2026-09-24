//! `mars/comm/strutil.h` + `strutil.cc`, for UTF-8 strings.
//!
//! The C++ trims with `isspace`, lower-cases with `::tolower` and upper-cases
//! with `::toupper`, i.e. all of them are byte-wise and ASCII-only. The port
//! keeps that: `trim` removes `b" \t\n\v\f\r"`, and case conversion only
//! touches ASCII letters, so a non-ASCII byte is never modified.

use std::ffi::CStr;
use std::fmt;
use std::os::raw::c_char;

/// The bytes `isspace()` accepts in the C locale.
const ASCII_WHITESPACE: &[u8] = b" \t\n\x0b\x0c\r";

fn is_space(byte: u8) -> bool {
    ASCII_WHITESPACE.contains(&byte)
}

/// `strutil::URLEncode` — unreserved characters pass through, a space becomes
/// `+`, everything else becomes `%XX` with **upper-case** hex digits.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    url_encode_into(s, &mut out);
    out
}

/// `strutil::URLEncode(url, out)` — appends to `out` instead of allocating.
pub fn url_encode_into(s: &str, out: &mut String) {
    for byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'*' => {
                out.push(*byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
}

/// `strutil::TrimLeft`.
pub fn trim_left(s: &str) -> &str {
    let bytes = s.as_bytes();
    let start = bytes
        .iter()
        .position(|b| !is_space(*b))
        .unwrap_or(bytes.len());
    &s[start..]
}

/// `strutil::TrimRight`.
pub fn trim_right(s: &str) -> &str {
    let bytes = s.as_bytes();
    let end = bytes
        .iter()
        .rposition(|b| !is_space(*b))
        .map_or(0, |i| i + 1);
    &s[..end]
}

/// `strutil::Trim`.
pub fn trim(s: &str) -> &str {
    trim_right(trim_left(s))
}

/// `strutil::ToLower`, ASCII only.
pub fn to_lower_in_place(s: &mut String) {
    for byte in unsafe { s.as_mut_vec() } {
        if byte.is_ascii_uppercase() {
            *byte += b'a' - b'A';
        }
    }
}

/// `strutil::ToUpper`, ASCII only.
pub fn to_upper_in_place(s: &mut String) {
    for byte in unsafe { s.as_mut_vec() } {
        if byte.is_ascii_lowercase() {
            *byte -= b'a' - b'A';
        }
    }
}

/// `strutil::cast_lower`.
pub fn cast_lower(s: &str) -> String {
    let mut out = s.to_owned();
    to_lower_in_place(&mut out);
    out
}

/// `strutil::cast_upper`.
pub fn cast_upper(s: &str) -> String {
    let mut out = s.to_owned();
    to_upper_in_place(&mut out);
    out
}

/// `strutil::StartsWith`.
pub fn starts_with(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
}

/// `strutil::EndsWith`.
pub fn ends_with(s: &str, suffix: &str) -> bool {
    s.ends_with(suffix)
}

/// The default delimiters of `strutil::Tokenizer<std::string>`: `" \t\n\r;:,.?"`.
pub const DEFAULT_DELIMITERS: &str = " \t\n\r;:,.?";

/// `strutil::SplitToken` — every run of delimiters separates two tokens, empty
/// tokens are skipped, and the delimiters themselves are dropped.
pub fn split_token<'a>(s: &'a str, delimiters: &'a str) -> Vec<&'a str> {
    Tokenizer::new(s, delimiters).collect()
}

/// `strutil::Tokenizer<std::string>`, as an iterator over the tokens.
pub struct Tokenizer<'a> {
    rest: &'a str,
    delimiters: &'a str,
}

impl<'a> Tokenizer<'a> {
    /// `Tokenizer(str, delimiters)`.
    pub fn new(s: &'a str, delimiters: &'a str) -> Self {
        Self {
            rest: s,
            delimiters,
        }
    }

    /// `Tokenizer(str)` with [`DEFAULT_DELIMITERS`].
    pub fn with_default_delimiters(s: &'a str) -> Self {
        Self::new(s, DEFAULT_DELIMITERS)
    }

    fn next_token(&mut self) -> Option<&'a str> {
        let bytes = self.rest.as_bytes();
        let start = bytes
            .iter()
            .position(|b| !self.delimiters.as_bytes().contains(b))?;
        let end = bytes[start..]
            .iter()
            .position(|b| self.delimiters.as_bytes().contains(b))
            .map_or(bytes.len(), |i| start + i);
        let token = &self.rest[start..end];
        self.rest = &self.rest[end..];
        Some(token)
    }
}

impl<'a> Iterator for Tokenizer<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        self.next_token()
    }
}

/// `strutil::MergeToken` — joins `[begin, end)` with `delimiter` between the
/// items. `false` when the input is empty or the delimiter is empty.
pub fn merge_token(items: &[&str], delimiter: &str) -> Option<String> {
    if items.is_empty() || delimiter.is_empty() {
        return None;
    }
    Some(items.join(delimiter))
}

/// `strutil::Hex2Str` — bytes to lower-case hex. (The C++ name reads the wrong
/// way round; it is kept so the two sides stay comparable.)
pub fn hex2str(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// `strutil::Str2Hex` — hex text back to bytes.
///
/// The C++ bails out above 1024 input characters (512 bytes of output), which
/// is kept as a documented limit: `None` is returned instead of an assert.
pub fn str2hex(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if bytes.len() > 1024 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        if pair.len() != 2 {
            break;
        }
        out.push(u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?);
    }
    Some(out)
}

/// `strutil::ReplaceChar`.
pub fn replace_char(s: &str, be_replaced: char, replace_with: char) -> String {
    s.replace(be_replaced, &replace_with.to_string())
}

/// `strutil::GetFileNameFromPath` — everything after the last `/` or `\`.
/// A trailing separator (or no separator at all) yields the whole input.
pub fn file_name_from_path(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(pos) if pos + 1 < path.len() => &path[pos + 1..],
        _ => path,
    }
}

/// `strutil::ci_find_substr` — case-insensitive search starting at `pos`.
/// Returns the byte offset, or `None` like `std::string::npos`.
pub fn ci_find_substr(haystack: &str, needle: &str, pos: usize) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() {
        return Some(pos.min(bytes.len()));
    }
    (pos..=bytes.len().saturating_sub(needle.len())).find(|&start| {
        bytes[start..start + needle.len()]
            .iter()
            .zip(needle)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

/// `strutil::DigestToBase16` — lower-case hex of a digest of any length.
pub fn digest_to_base16(digest: &[u8]) -> String {
    hex2str(digest)
}

/// `strutil::MD5DigestToBase16`.
pub fn md5_digest_to_base16(digest: &[u8; 16]) -> String {
    digest_to_base16(digest)
}

/// `strutil::BufferMD5` — MD5 of `bytes`, hex encoded.
pub fn buffer_md5(bytes: &[u8]) -> String {
    md5_digest_to_base16(&md5::compute(bytes).0)
}

/// `strutil::CStr2StringSafe` — `None` for a null pointer.
///
/// # Safety
///
/// `ptr` has to be null or a valid NUL-terminated C string.
pub unsafe fn cstr_to_string_safe(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
}

/// `strutil::CStrNullOrEmpty`.
///
/// # Safety
///
/// See [`cstr_to_string_safe`].
pub unsafe fn cstr_null_or_empty(ptr: *const c_char) -> bool {
    cstr_to_string_safe(ptr).is_none_or(|s| s.is_empty())
}

/// `strutil::CStrCmpSafe` — `false` when either pointer is null.
///
/// # Safety
///
/// See [`cstr_to_string_safe`].
pub unsafe fn cstr_cmp_safe(a: *const c_char, b: *const c_char) -> bool {
    match (cstr_to_string_safe(a), cstr_to_string_safe(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// `strutil::CStr2Int32Safe` — `atoi` with a fallback for a null pointer.
///
/// # Safety
///
/// See [`cstr_to_string_safe`].
pub unsafe fn cstr_to_i32_safe(ptr: *const c_char, default_num: i32) -> i32 {
    cstr_to_string_safe(ptr)
        .and_then(|s| s.trim().parse::<i32>().ok())
        .unwrap_or(default_num)
}

/// `strutil::to_hex_string`.
pub fn to_hex_string<T: fmt::LowerHex>(value: T) -> String {
    format!("{value:x}")
}

/// `strutil::to_oct_string`.
pub fn to_oct_string<T: fmt::Octal>(value: T) -> String {
    format!("{value:o}")
}

/// `strutil::join_to_string` — `{item,item,}` shaped output with a prefix and a
/// postfix; the separator is appended after **every** item, exactly like the
/// C++ (which is why the postfix normally closes the trailing separator).
pub fn join_to_string<I, T>(items: I, separator: &str, prefix: &str, postfix: &str) -> String
where
    I: IntoIterator<Item = T>,
    T: fmt::Display,
{
    let mut out = String::new();
    let mut empty = true;
    for item in items {
        if empty {
            out.push_str(prefix);
            empty = false;
        }
        out.push_str(&item.to_string());
        out.push_str(separator);
    }
    if empty {
        return String::new();
    }
    out.push_str(postfix);
    out
}

/// `strutil::join_to_string_for_log` — `{a,b,c}`.
pub fn join_to_string_for_log<I, T>(items: I) -> String
where
    I: IntoIterator<Item = T>,
    T: fmt::Display,
{
    join_to_string(items, ",", "{", "}")
}
