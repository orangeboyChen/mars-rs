//! `mars/comm/basepacker.cc` — the wire format the long link spoke before
//! `mars/stn/proto/longlink_packer.cc`: a header of `LongLinkPack`, the URL of
//! the package and its body, with an Adler-32 over the two of them.
//!
//! ```text
//! magic | ver | head_length | url_length | total_length | sequence | hash
//! ```
//!
//! One byte each for `magic`, `ver`, `head_length` and `url_length`, then
//! three `uint32_t` in network order: the length of the whole package, the
//! sequence, and the hash. `magic` is the low byte of the sum of the three
//! lengths, which is the only check there is on the header, and `ver` is
//! always `0x1`.
//!
//! Alongside it are the `Simple*` helpers: a package that is nothing but a
//! length and a body, `uint16_t` (`simple_short_*`) or `uint32_t`
//! (`simple_int_*`).
//!
//! No caller is left for any of this in mars — `longlink_packer.cc` took the
//! long link over and the short link writes its own — so this is the file as
//! it stands, with the C++ semantics kept: the three `LONGLINKPACK_CONTINUE*`
//! answers are the "read more bytes" ones, and a package that is refused
//! answers [`Refused`], where the C++ answers the `__LINE__` of the check that
//! failed.
//!
//! The `AutoBuffer` and `PtrBuffer` overloads of the C++ write into a buffer
//! the caller owns; the port hands back a `Vec<u8>` of its own, which is the
//! same bytes either way, and the `PtrBuffer` overloads — which only attach to
//! bytes someone else owns instead of copying them — are not ported.

use crate::adler32::{adler32, adler32_seeded};

/// `SIMPLE_CONTINUE` — fewer bytes than the length in front of the body.
pub const SIMPLE_CONTINUE: i32 = -2;
/// `SIMPLE_CONTINUE_DATA` — the length is there, the body is not.
pub const SIMPLE_CONTINUE_DATA: i32 = -1;
/// `SIMPLE_OK`.
pub const SIMPLE_OK: i32 = 0;
/// What the port answers for a simple package that was refused: the C++ has no
/// such answer, because it hands the caller a body of `_packlen - sizeof(T)`
/// bytes whatever that comes to.
pub const SIMPLE_FALSE: i32 = 1;

/// `LONGLINKPACK_CONTINUE` — fewer bytes than a header.
pub const LONGLINKPACK_CONTINUE: i32 = -3;
/// `LONGLINKPACK_CONTINUE_HEAD` — the header is there, the URL is not.
pub const LONGLINKPACK_CONTINUE_HEAD: i32 = -2;
/// `LONGLINKPACK_CONTINUE_data` — the URL is there, the body is not.
pub const LONGLINKPACK_CONTINUE_DATA: i32 = -1;
/// `LONGLINKPACK_OK`.
pub const LONGLINKPACK_OK: i32 = 0;
/// What the port answers for a package that was refused: the C++ answers the
/// `__LINE__` of the check that failed, a positive number that means nothing
/// outside `basepacker.cc`.
pub const LONGLINKPACK_FALSE: i32 = 1;

/// `sizeof(LongLinkPack)`, `#pragma pack(1)` — four bytes and three
/// `uint32_t`, no padding.
pub const HEAD_LEN: usize = 16;
/// `ver` of every package, the only one there ever was.
pub const VER: u8 = 0x1;
/// `strnlen(_url, 128)` — a URL longer than this is packed without its tail.
pub const MAX_URL_LEN: usize = 128;
/// A package bigger than this is refused — the C++'s `1024 * 1024`.
pub const MAX_PACKAGE_LEN: usize = 1024 * 1024;

/// `SimplePackLength<uint16_t>` — the body and the two bytes in front of it.
pub fn simple_short_pack_length(data_len: usize) -> usize {
    data_len + 2
}

/// `SimplePackLength<unsigned int>` — the body and the four bytes in front of
/// it.
pub fn simple_int_pack_length(data_len: usize) -> usize {
    data_len + 4
}

/// `SimpleShortPack` — the length of the body in network order, then the body.
pub fn simple_short_pack(data: &[u8]) -> Vec<u8> {
    simple_pack(data, 2)
}

/// `SimpleIntPack` — the length of the body in network order, then the body.
pub fn simple_int_pack(data: &[u8]) -> Vec<u8> {
    simple_pack(data, 4)
}

/// What `simple_short_unpack` / `simple_int_unpack` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimpleUnpacked {
    /// `SIMPLE_CONTINUE` — not enough bytes for the length yet.
    Continue,
    /// `SIMPLE_CONTINUE_DATA` — the length is there and asks for more bytes
    /// than there are.
    ContinueData {
        /// How long the whole package is.
        pack_len: usize,
    },
    /// The length in front of the body is shorter than the length itself, so
    /// there is no body to read — the C++ computes `_packlen - sizeof(T)`,
    /// which has wrapped around, and writes that many bytes.
    /// [`Refused::Length`] is why, and the C++ has no such answer.
    Refused(Refused),
    /// `SIMPLE_OK` — a whole package.
    Ok {
        /// How long the whole package is.
        pack_len: usize,
        /// The body.
        data: Vec<u8>,
    },
}

impl SimpleUnpacked {
    /// The `int` the C++ answers.
    pub fn code(&self) -> i32 {
        match self {
            SimpleUnpacked::Continue => SIMPLE_CONTINUE,
            SimpleUnpacked::ContinueData { .. } => SIMPLE_CONTINUE_DATA,
            SimpleUnpacked::Refused(_) => SIMPLE_FALSE,
            SimpleUnpacked::Ok { .. } => SIMPLE_OK,
        }
    }
}

/// `SimpleShortUnpack` — a body behind a `uint16_t` length.
pub fn simple_short_unpack(raw: &[u8]) -> SimpleUnpacked {
    simple_unpack(raw, 2)
}

/// `SimpleIntUnpack` — a body behind a `uint32_t` length.
pub fn simple_int_unpack(raw: &[u8]) -> SimpleUnpacked {
    simple_unpack(raw, 4)
}

/// Why a package was refused. The C++ answers the `__LINE__` of the check, and
/// [`LONGLINKPACK_FALSE`] for any of them here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// `magic` is not the low byte of the three lengths added up.
    Magic,
    /// The header — and the URL, where there is one — does not fit in the
    /// length the package claims.
    Length,
    /// The package claims to be bigger than [`MAX_PACKAGE_LEN`].
    TooBig,
    /// The hash is not the Adler-32 of the URL and the body.
    Hash,
}

/// What `packer_unpack` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackerUnpacked {
    /// `LONGLINKPACK_CONTINUE` — not enough bytes for a header yet.
    Continue,
    /// `LONGLINKPACK_CONTINUE_HEAD` — the header is there, the URL behind it is
    /// not.
    ContinueHead,
    /// `LONGLINKPACK_CONTINUE_data` — the URL is there, the body behind it is
    /// not.
    ContinueData {
        /// The URL of the package.
        url: String,
        /// The sequence of the package.
        sequence: u32,
        /// How long the whole package is.
        pack_len: usize,
    },
    /// The bytes are a package, but not one that checks out.
    Refused(Refused),
    /// `LONGLINKPACK_OK` — a whole package.
    Ok {
        /// The URL of the package.
        url: String,
        /// The sequence of the package.
        sequence: u32,
        /// How long the whole package is.
        pack_len: usize,
        /// The body.
        data: Vec<u8>,
    },
}

impl PackerUnpacked {
    /// The `int` the C++ answers.
    pub fn code(&self) -> i32 {
        match self {
            PackerUnpacked::Continue => LONGLINKPACK_CONTINUE,
            PackerUnpacked::ContinueHead => LONGLINKPACK_CONTINUE_HEAD,
            PackerUnpacked::ContinueData { .. } => LONGLINKPACK_CONTINUE_DATA,
            PackerUnpacked::Refused(_) => LONGLINKPACK_FALSE,
            PackerUnpacked::Ok { .. } => LONGLINKPACK_OK,
        }
    }
}

/// `Packer_Pack` — the header, then the URL, then the body.
///
/// The URL is cut at [`MAX_URL_LEN`] bytes (`strnlen(_url, 128)`), which the
/// C++ only asserts is short enough for the header to hold it. With `do_hash`
/// the hash is the Adler-32 of the URL read on over the body — the C++ seeds
/// the second call with the first one's answer, which is [`adler32_seeded`] —
/// and without it the hash stays `0`, which `packer_unpack` then does not
/// check.
pub fn packer_pack(url: &str, sequence: u32, data: &[u8], do_hash: bool) -> Vec<u8> {
    let url = &url.as_bytes()[..url.len().min(MAX_URL_LEN)];

    let head_length = HEAD_LEN as u8;
    let url_length = url.len() as u8;
    let total_length = head_length as usize + url.len() + data.len();
    let magic = (head_length as usize + url.len() + total_length) as u8;

    let mut hash = 0u32;
    if do_hash {
        hash = adler32(url);
        if !data.is_empty() {
            hash = adler32_seeded(hash, data);
        }
    }

    let mut out = Vec::with_capacity(total_length);
    out.push(magic);
    out.push(VER);
    out.push(head_length);
    out.push(url_length);
    out.extend_from_slice(&(total_length as u32).to_be_bytes());
    out.extend_from_slice(&sequence.to_be_bytes());
    out.extend_from_slice(&hash.to_be_bytes());
    out.extend_from_slice(url);
    out.extend_from_slice(data);
    out
}

/// `Packer_Unpack` — the header, the URL and the body, read back out of a
/// stream. [`PackerUnpacked::pack_len`](PackerUnpacked::ContinueData) is how
/// long the whole package is, so a caller can wait for the rest of it; the
/// bytes before [`HEAD_LEN`] are not a header yet and the ones after a whole
/// package are the next one's.
pub fn packer_unpack(raw: &[u8]) -> PackerUnpacked {
    if raw.len() < HEAD_LEN {
        return PackerUnpacked::Continue;
    }

    let magic = raw[0];
    let head_length = usize::from(raw[2]);
    let url_length = usize::from(raw[3]);
    let total_length = be_u32(raw, 4) as usize;
    let sequence = be_u32(raw, 8);
    let hash = be_u32(raw, 12);

    if (head_length + url_length + total_length) as u8 != magic {
        return PackerUnpacked::Refused(Refused::Magic);
    }

    if url_length + head_length > total_length {
        return PackerUnpacked::Refused(Refused::Length);
    }

    if MAX_PACKAGE_LEN < total_length {
        return PackerUnpacked::Refused(Refused::TooBig);
    }

    if url_length + head_length > raw.len() {
        return PackerUnpacked::ContinueHead;
    }

    let url = String::from_utf8_lossy(&raw[head_length..head_length + url_length]).into_owned();
    let pack_len = total_length;

    if total_length > raw.len() {
        return PackerUnpacked::ContinueData {
            url,
            sequence,
            pack_len,
        };
    }

    // `url_length + head_length <= total_length` (checked above), so this is
    // the body and nothing under it
    let body = &raw[head_length + url_length..total_length];

    if 0 != hash && hash != adler32(&raw[head_length..total_length]) {
        return PackerUnpacked::Refused(Refused::Hash);
    }

    PackerUnpacked::Ok {
        url,
        sequence,
        pack_len,
        data: body.to_vec(),
    }
}

/// `hton(SimplePackLength<T>)` and the body: `_packlen` counts the length in
/// front, which is why it is `data.len() + head_len` and not just the body.
fn simple_pack(data: &[u8], head_len: usize) -> Vec<u8> {
    let pack_len = data.len() + head_len;
    let mut out = Vec::with_capacity(pack_len);
    match head_len {
        2 => out.extend_from_slice(&(pack_len as u16).to_be_bytes()),
        _ => out.extend_from_slice(&(pack_len as u32).to_be_bytes()),
    }
    out.extend_from_slice(data);
    out
}

/// `ntoh(*(T*)_rawbuf)` and the body behind it.
fn simple_unpack(raw: &[u8], head_len: usize) -> SimpleUnpacked {
    if head_len > raw.len() {
        return SimpleUnpacked::Continue;
    }

    let pack_len = match head_len {
        2 => usize::from(u16::from_be_bytes([raw[0], raw[1]])),
        _ => be_u32(raw, 0) as usize,
    };

    if pack_len < head_len {
        return SimpleUnpacked::Refused(Refused::Length);
    }

    if pack_len > raw.len() {
        return SimpleUnpacked::ContinueData { pack_len };
    }

    SimpleUnpacked::Ok {
        pack_len,
        data: raw[head_len..pack_len].to_vec(),
    }
}

/// Four bytes of the stream, network order — the `ntohl` of the C++.
fn be_u32(raw: &[u8], at: usize) -> u32 {
    let bytes: [u8; 4] = raw[at..at + 4]
        .try_into()
        .expect("a header field of four bytes");
    u32::from_be_bytes(bytes)
}
