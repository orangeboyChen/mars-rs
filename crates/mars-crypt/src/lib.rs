//! `mars-crypt` — port of `mars/xlog/crypt/log_crypt.{h,cc}` and
//! `mars/xlog/crypt/log_magic_num.h`.
//!
//! Every xlog record on disk is framed as
//!
//! ```text
//! | magic start (u8) | seq (u16) | begin hour (u8) | end hour (u8)
//!   | length (u32) | client pubkey ([u8; 64]) |  == 73 bytes
//! | <payload> |
//! | magic end (u8) |                                 == 1 byte
//! ```
//!
//! All multi-byte fields are little-endian: the C++ `memcpy`s native
//! `uint16_t` / `uint32_t` and every platform Mars ships on is little-endian.
//!
//! | C++                                   | Rust                                  |
//! |---------------------------------------|---------------------------------------|
//! | `mars::xlog::LogCrypt`                | [`LogCrypt`]                          |
//! | `mars::xlog::LogMagicNum`             | [`magic`]                             |
//! | `LogCrypt::GetHeaderLen()`            | [`HEADER_LEN`] / [`LogCrypt::header_len`] |
//! | `LogCrypt::GetTailerLen()`            | [`TAILER_LEN`] / [`LogCrypt::tailer_len`] |
//! | `LogCrypt::GetLogHour()`              | [`LogCrypt::get_log_hour`]            |
//! | `LogCrypt::UpdateLogHour()`           | [`LogCrypt::update_log_hour`]         |
//! | `LogCrypt::GetLogLen()`               | [`LogCrypt::get_log_len`]             |
//! | `LogCrypt::UpdateLogLen()`            | [`LogCrypt::update_log_len`]          |
//! | `LogCrypt::SetHeaderInfo()`           | [`LogCrypt::set_header_info`]         |
//! | `LogCrypt::SetTailerInfo()`           | [`LogCrypt::set_tailer_info`]         |
//! | `LogCrypt::CryptSyncLog()`            | [`LogCrypt::crypt_sync_log`]          |
//! | `LogCrypt::CryptAsyncLog()`           | [`LogCrypt::crypt_async_log`]         |
//! | `LogCrypt::Fix()`                     | [`LogCrypt::fix`]                     |
//! | `LogCrypt::IsCrypt()`                 | [`LogCrypt::is_crypt`]                |
//! | `__TeaEncrypt()`                      | (private) `tea_encrypt`               |
//! | `__GetSeq()`                          | (private) `get_seq`                   |
//! | `Hex2Buffer()`                        | (private) `hex_to_buffer`             |

#![deny(unsafe_code)]

use std::sync::atomic::{AtomicU16, Ordering};

use mars_core::{le, local_hour, AutoBuffer};

/// `LogCrypt::GetHeaderLen()` — size of the on-disk record header, in bytes.
///
/// `sizeof(char) * 3 + sizeof(uint16_t) + sizeof(uint32_t) + sizeof(char) * 64`.
pub const HEADER_LEN: usize = 3 + 2 + 4 + CLIENT_PUBKEY_LEN;

/// `LogCrypt::GetTailerLen()` — size of the on-disk record tailer, in bytes.
pub const TAILER_LEN: usize = 1;

/// Length of the raw (uncompressed SEC1) client public key stored in the
/// header. Mirrors `char client_pubkey_[64]` / `PUB_KEY_LEN`.
pub const CLIENT_PUBKEY_LEN: usize = 64;

/// `TEA_BLOCK_LEN` — the TEA cipher operates on 8-byte blocks.
pub const TEA_BLOCK_LEN: usize = 8;

/// Byte offsets of each header field. Kept private; the layout is exposed
/// through the `get_*` / `update_*` / `set_*` API (and asserted in tests).
mod off {
    pub const MAGIC: usize = 0;
    pub const SEQ: usize = 1;
    pub const BEGIN_HOUR: usize = 3;
    pub const END_HOUR: usize = 4;
    pub const LENGTH: usize = 5;
    pub const PUBKEY: usize = 9;
}

/// Port of `mars::xlog::LogMagicNum`.
pub mod magic {
    /// `LogMagicNum::kMagicSyncZlibStart`.
    pub const SYNC_ZLIB_START: u8 = 0x06;
    /// `LogMagicNum::kMagicSyncNoCryptZlibStart`.
    pub const SYNC_NOCRYPT_ZLIB_START: u8 = 0x08;
    /// `LogMagicNum::kMagicAsyncZlibStart`.
    pub const ASYNC_ZLIB_START: u8 = 0x07;
    /// `LogMagicNum::kMagicAsyncNoCryptZlibStart`.
    pub const ASYNC_NOCRYPT_ZLIB_START: u8 = 0x09;
    /// `LogMagicNum::kMagicSyncZstdStart`.
    pub const SYNC_ZSTD_START: u8 = 0x0A;
    /// `LogMagicNum::kMagicSyncNoCryptZstdStart`.
    pub const SYNC_NOCRYPT_ZSTD_START: u8 = 0x0B;
    /// `LogMagicNum::kMagicAsyncZstdStart`.
    pub const ASYNC_ZSTD_START: u8 = 0x0C;
    /// `LogMagicNum::kMagicAsyncNoCryptZstdStart`.
    pub const ASYNC_NOCRYPT_ZSTD_START: u8 = 0x0D;
    /// `LogMagicNum::kMagicEnd`.
    pub const END: u8 = 0x00;

    /// `LogMagicNum::MagicStartIsValid()` — true when `_magic` is one of the
    /// eight known record-start bytes.
    pub fn magic_start_is_valid(m: u8) -> bool {
        matches!(
            m,
            SYNC_ZLIB_START
                | SYNC_NOCRYPT_ZLIB_START
                | ASYNC_ZLIB_START
                | ASYNC_NOCRYPT_ZLIB_START
                | SYNC_ZSTD_START
                | SYNC_NOCRYPT_ZSTD_START
                | ASYNC_ZSTD_START
                | ASYNC_NOCRYPT_ZSTD_START
        )
    }
}

/// Number of TEA rounds used by Mars (`for (i = 0; i < 16; i++)`).
const TEA_ROUNDS: u32 = 16;
/// `const static uint32_t delta = 0x9e3779b9`.
const TEA_DELTA: u32 = 0x9e37_79b9;

/// Port of the file-static `__TeaEncrypt(uint32_t* v, uint32_t* k)`.
///
/// Encrypts the two `u32` halves of one 8-byte block in place. Note this is
/// *only* ever called with bytes decoded little-endian; Mars never decrypts on
/// device, so there is no `__TeaDecrypt` counterpart in the C++ source.
fn tea_encrypt(v: &mut [u32; 2], k: [u32; 4]) {
    let (mut v0, mut v1) = (v[0], v[1]);
    let (k0, k1, k2, k3) = (k[0], k[1], k[2], k[3]);
    let mut sum: u32 = 0;
    for _ in 0..TEA_ROUNDS {
        sum = sum.wrapping_add(TEA_DELTA);
        v0 = v0.wrapping_add(
            (v1.wrapping_shl(4).wrapping_add(k0))
                ^ (v1.wrapping_add(sum))
                ^ (v1.wrapping_shr(5).wrapping_add(k1)),
        );
        v1 = v1.wrapping_add(
            (v0.wrapping_shl(4).wrapping_add(k2))
                ^ (v0.wrapping_add(sum))
                ^ (v0.wrapping_shr(5).wrapping_add(k3)),
        );
    }
    v[0] = v0;
    v[1] = v1;
}

/// Port of the file-static `__GetSeq(bool _is_async)`.
///
/// The C++ counter is a process-global `static uint16_t s_seq`: sync records
/// always get `0`, async records get a monotonically increasing value that
/// skips `0` on wrap.
fn get_seq(is_async: bool) -> u16 {
    if !is_async {
        return 0;
    }

    static SEQ: AtomicU16 = AtomicU16::new(0);

    let mut seq = SEQ.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    if seq == 0 {
        seq = SEQ.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    }
    seq
}

/// Port of the file-static `Hex2Buffer(const char*, size_t, unsigned char*)`.
///
/// Accepts only `[0-9a-fA-F]`; returns `None` for empty / odd-length input,
/// mirroring the C++ early return.
fn hex_to_buffer(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(2) {
        return None;
    }

    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.as_chunks::<2>().0 {
        let hi = hex_nibble(pair[0])?;
        let lo = hex_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Result of a successful ECDH handshake: the raw client public key that goes
/// into every header plus the four `u32` TEA key words.
#[derive(Debug)]
struct KeyPair {
    client_pubkey: [u8; CLIENT_PUBKEY_LEN],
    tea_key: [u32; 4],
}

/// Port of the key-exchange half of `LogCrypt::LogCrypt(const char*)`.
///
/// The C++ calls `uECC_make_key` / `uECC_shared_secret` on **secp256k1** with
/// the server public key given as 64 raw bytes `X || Y` (128 hex chars).
/// `k256` reproduces both: the shared secret is the `x` coordinate of
/// `client_priv * server_pub`, 32 bytes big-endian — exactly what uECC's
/// `uECC_shared_secret` produces.
///
/// Returns `None` for any failure; the caller then keeps `is_crypt_ == false`,
/// which mirrors the C++ early returns.
/// `uECC_make_key` + `uECC_shared_secret`: a fresh client key pair per
/// process, so two runs never agree on a key.
fn make_key(svr_pubkey: &[u8]) -> Option<KeyPair> {
    let mut rng = k256::elliptic_curve::rand_core::OsRng;
    let client_pri = k256::SecretKey::random(&mut rng);
    derive_key(svr_pubkey, &client_pri)
}

/// The deterministic half of [`make_key`]: the same handshake with the client
/// private key *given* instead of generated, which is what lets a
/// known-answer vector pin the shared secret and the TEA key it becomes.
fn derive_key(svr_pubkey: &[u8], client_pri: &k256::SecretKey) -> Option<KeyPair> {
    use k256::elliptic_curve::sec1::ToEncodedPoint;

    if svr_pubkey.len() != CLIENT_PUBKEY_LEN {
        return None;
    }

    // uECC stores public keys as X||Y with no prefix; SEC1 uncompressed points
    // need the 0x04 tag in front.
    let mut sec1 = [0u8; 1 + CLIENT_PUBKEY_LEN];
    sec1[0] = 0x04;
    sec1[1..].copy_from_slice(svr_pubkey);

    let server_pub = k256::PublicKey::from_sec1_bytes(&sec1).ok()?;

    let client_pub = client_pri.public_key();

    let encoded = client_pub.to_encoded_point(false);
    let point_bytes = encoded.as_bytes();
    let mut client_pubkey = [0u8; CLIENT_PUBKEY_LEN];
    // `to_encoded_point(false)` yields 0x04 || X || Y.
    client_pubkey.copy_from_slice(&point_bytes[1..]);

    let shared = k256::elliptic_curve::ecdh::diffie_hellman(
        client_pri.to_nonzero_scalar(),
        server_pub.as_affine(),
    );
    let secret = shared.raw_secret_bytes();

    // `memcpy(tea_key_, ecdh_key, sizeof(tea_key_))` on a little-endian host.
    let mut tea_key = [0u32; 4];
    for (i, word) in tea_key.iter_mut().enumerate() {
        *word = le::read_u32(secret, i * 4);
    }

    Some(KeyPair {
        client_pubkey,
        tea_key,
    })
}

/// Port of `mars::xlog::LogCrypt`.
///
/// Owns the per-process ECDH state (client public key + derived TEA key) and
/// provides the static header/tailer helpers used by the buffer layer.
#[derive(Debug)]
pub struct LogCrypt {
    /// `uint16_t seq_` — restored by [`LogCrypt::fix`].
    seq_: u16,
    /// `uint32_t tea_key_[4]` — first 16 bytes of the ECDH secret.
    tea_key_: [u32; 4],
    /// `char client_pubkey_[64]` — copied verbatim into every header.
    client_pubkey_: [u8; CLIENT_PUBKEY_LEN],
    /// `bool is_crypt_` — false when no (valid) server public key was given.
    is_crypt_: bool,
}

impl LogCrypt {
    /// `LogCrypt(const char* _pubkey)`.
    ///
    /// `_pubkey` must be exactly 128 hex characters describing a 64-byte
    /// secp256k1 public key. `None`, empty, wrong length, non-hex or
    /// cryptographically invalid input all leave `is_crypt() == false`, which
    /// mirrors the C++ early returns.
    pub fn new(pubkey: Option<&str>) -> Self {
        let mut crypt = Self {
            seq_: 0,
            tea_key_: [0; 4],
            client_pubkey_: [0; CLIENT_PUBKEY_LEN],
            is_crypt_: false,
        };

        let Some(pubkey) = pubkey else {
            return crypt;
        };

        // `PUB_KEY_LEN * 2 != strnlen(_pubkey, 256)`
        if pubkey.len() != CLIENT_PUBKEY_LEN * 2 {
            return crypt;
        }

        let Some(svr_pubkey) = hex_to_buffer(pubkey) else {
            return crypt;
        };

        let Some(pair) = make_key(&svr_pubkey) else {
            return crypt;
        };

        crypt.client_pubkey_ = pair.client_pubkey;
        crypt.tea_key_ = pair.tea_key;
        crypt.is_crypt_ = true;
        crypt
    }

    /// `LogCrypt::IsCrypt()` — true when a TEA key was successfully derived.
    pub fn is_crypt(&self) -> bool {
        self.is_crypt_
    }

    /// `LogCrypt::GetHeaderLen()`.
    pub fn header_len(&self) -> usize {
        HEADER_LEN
    }

    /// `LogCrypt::GetTailerLen()`.
    pub fn tailer_len(&self) -> usize {
        TAILER_LEN
    }

    /// `LogCrypt::GetLogHour()`.
    ///
    /// Returns `None` — instead of the C++ `false` — when the region is
    /// shorter than [`HEADER_LEN`] or the magic byte is unknown.
    pub fn get_log_hour(data: &[u8]) -> Option<(u8, u8)> {
        if data.len() < HEADER_LEN {
            return None;
        }
        if !magic::magic_start_is_valid(data[off::MAGIC]) {
            return None;
        }
        Some((data[off::BEGIN_HOUR], data[off::END_HOUR]))
    }

    /// `LogCrypt::UpdateLogHour()` — stamps the *end* hour with the current
    /// local hour (`GetHeaderLen() - sizeof(uint32_t) - 64 - 1 == 4`).
    pub fn update_log_hour(data: &mut [u8]) {
        if data.len() < HEADER_LEN {
            return;
        }
        data[off::END_HOUR] = local_hour();
    }

    /// `LogCrypt::GetLogLen()` — the payload length recorded in the header, or
    /// `0` when the header is truncated / the magic is unknown.
    pub fn get_log_len(data: &[u8]) -> u32 {
        if data.len() < HEADER_LEN {
            return 0;
        }
        if !magic::magic_start_is_valid(data[off::MAGIC]) {
            return 0;
        }
        le::read_u32(data, off::LENGTH)
    }

    /// `LogCrypt::UpdateLogLen()` — adds `_add_len` to the recorded payload
    /// length (wrapping, exactly like the C++ `uint32_t` arithmetic).
    pub fn update_log_len(data: &mut [u8], add_len: u32) {
        if data.len() < HEADER_LEN {
            return;
        }
        let current = Self::get_log_len(data).wrapping_add(add_len);
        le::write_u32(data, off::LENGTH, current);
    }

    /// `LogCrypt::SetTailerInfo()` — writes `_magic_end` at `_data[0]`.
    ///
    /// Callers pass the slice starting at the tailer offset.
    pub fn set_tailer_info(data: &mut [u8], magic_end: u8) {
        if data.is_empty() {
            return;
        }
        data[0] = magic_end;
    }

    /// `LogCrypt::SetHeaderInfo()`.
    ///
    /// Writes the magic byte, the sequence number (`0` for sync, otherwise the
    /// next value of the process-global async counter), begin/end hour set to
    /// the current local hour, a zero length and the client public key.
    pub fn set_header_info(&mut self, data: &mut [u8], is_async: bool, magic_start: u8) {
        if data.len() < HEADER_LEN {
            return;
        }

        data[off::MAGIC] = magic_start;
        self.seq_ = get_seq(is_async);
        le::write_u16(data, off::SEQ, self.seq_);

        let hour = local_hour();
        data[off::BEGIN_HOUR] = hour;
        data[off::END_HOUR] = hour;

        le::write_u32(data, off::LENGTH, 0);
        data[off::PUBKEY..off::PUBKEY + CLIENT_PUBKEY_LEN].copy_from_slice(&self.client_pubkey_);
    }

    /// `LogCrypt::CryptSyncLog()`.
    ///
    /// Allocates [`HEADER_LEN`] + [`TAILER_LEN`] + `log_data.len()` bytes in
    /// `out`, fills in header + payload + tailer and returns the total count.
    ///
    /// The body is always stored in the clear: the C++ has the TEA loop
    /// commented out, so sync records are byte-identical to the no-crypt path.
    pub fn crypt_sync_log(
        &mut self,
        log_data: &[u8],
        out: &mut AutoBuffer,
        magic_start: u8,
        magic_end: u8,
    ) -> usize {
        let total = HEADER_LEN + TAILER_LEN + log_data.len();

        out.reset();
        out.alloc_write(total);

        {
            let buf = out.as_mut_slice();
            self.set_header_info(buf, false, magic_start);
            Self::update_log_len(buf, log_data.len() as u32);
            buf[HEADER_LEN..HEADER_LEN + log_data.len()].copy_from_slice(log_data);
            Self::set_tailer_info(&mut buf[HEADER_LEN + log_data.len()..], magic_end);
        }

        total
    }

    /// `LogCrypt::CryptAsyncLog()`.
    ///
    /// Without a TEA key the payload is copied verbatim and
    /// `remain_nocrypt_len` is `0`. Otherwise `log_data.len() / 8` blocks are
    /// TEA-encrypted and the trailing `log_data.len() % 8` bytes are appended
    /// in the clear; that count is reported through `remain_nocrypt_len` so the
    /// caller can re-crypt them together with the next chunk.
    pub fn crypt_async_log(
        &self,
        log_data: &[u8],
        out: &mut Vec<u8>,
        remain_nocrypt_len: &mut usize,
    ) {
        out.clear();

        let blocks = log_data.len() / TEA_BLOCK_LEN;
        *remain_nocrypt_len = log_data.len() % TEA_BLOCK_LEN;

        if !self.is_crypt_ {
            out.extend_from_slice(log_data);
            *remain_nocrypt_len = 0;
            return;
        }

        for i in 0..blocks {
            let block = &log_data[i * TEA_BLOCK_LEN..(i + 1) * TEA_BLOCK_LEN];
            let mut v = [le::read_u32(block, 0), le::read_u32(block, 4)];
            tea_encrypt(&mut v, self.tea_key_);
            out.extend_from_slice(&v[0].to_le_bytes());
            out.extend_from_slice(&v[1].to_le_bytes());
        }

        let tail = log_data.len() - *remain_nocrypt_len;
        out.extend_from_slice(&log_data[tail..]);
    }

    /// `LogCrypt::CryptAsyncLog()` without a second buffer.
    ///
    /// TEA is a block cipher over 8 bytes at a time, so the blocks can be
    /// encrypted where they already are: `data` becomes the ciphertext, and the
    /// trailing `data.len() % 8` bytes are left in the clear for the next
    /// chunk exactly as [`LogCrypt::crypt_async_log`] leaves them. The C++
    /// cannot do this — it encrypts into an `AutoBuffer` and copies back —
    /// and the copy plus the allocation it needs are the whole extra cost of
    /// the async write path.
    ///
    /// Returns `remain_nocrypt_len`, which is `0` when there is no TEA key.
    pub fn crypt_async_log_in_place(&self, data: &mut [u8]) -> usize {
        let remain_nocrypt_len = data.len() % TEA_BLOCK_LEN;
        if !self.is_crypt_ {
            return 0;
        }

        for block in data.as_chunks_mut::<TEA_BLOCK_LEN>().0 {
            let mut v = [le::read_u32(block, 0), le::read_u32(block, 4)];
            tea_encrypt(&mut v, self.tea_key_);
            block[..4].copy_from_slice(&v[0].to_le_bytes());
            block[4..].copy_from_slice(&v[1].to_le_bytes());
        }

        remain_nocrypt_len
    }

    /// `LogCrypt::Fix()`.
    ///
    /// Validates the magic byte, returns the recorded payload length and
    /// restores the instance sequence number from the header. Returns `None`
    /// when the region is truncated or the magic is unknown.
    pub fn fix(&mut self, data: &[u8]) -> Option<u32> {
        if data.len() < HEADER_LEN {
            return None;
        }
        if !magic::magic_start_is_valid(data[off::MAGIC]) {
            return None;
        }

        let raw_log_len = Self::get_log_len(data);
        self.seq_ = le::read_u16(data, off::SEQ);
        Some(raw_log_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a header-only region, the way `LogBaseBuffer::__Reset()` does.
    fn header_region(magic_start: u8) -> Vec<u8> {
        let mut region = vec![0u8; 256];
        let mut crypt = LogCrypt::new(None);
        crypt.set_header_info(&mut region, false, magic_start);
        region
    }

    /// A `LogCrypt` forced into crypt mode with a known TEA key, so the TEA
    /// code paths can be exercised without a live ECDH handshake.
    fn crypt_with_key() -> LogCrypt {
        LogCrypt {
            seq_: 7,
            tea_key_: [0x0011_2233, 0x4455_6677, 0x8899_aabb, 0xccdd_eeff],
            client_pubkey_: [0xAB; CLIENT_PUBKEY_LEN],
            is_crypt_: true,
        }
    }

    #[test]
    fn header_constants_match_mars() {
        assert_eq!(HEADER_LEN, 73);
        assert_eq!(TAILER_LEN, 1);
        assert_eq!(CLIENT_PUBKEY_LEN, 64);
        assert_eq!(TEA_BLOCK_LEN, 8);
        assert_eq!(off::MAGIC, 0);
        assert_eq!(off::SEQ, 1);
        assert_eq!(off::BEGIN_HOUR, 3);
        assert_eq!(off::END_HOUR, 4);
        assert_eq!(off::LENGTH, 5);
        assert_eq!(off::PUBKEY, 9);
        assert_eq!(off::PUBKEY + CLIENT_PUBKEY_LEN, HEADER_LEN);
    }

    #[test]
    fn magic_numbers_match_mars() {
        assert_eq!(magic::SYNC_ZLIB_START, 0x06);
        assert_eq!(magic::SYNC_NOCRYPT_ZLIB_START, 0x08);
        assert_eq!(magic::ASYNC_ZLIB_START, 0x07);
        assert_eq!(magic::ASYNC_NOCRYPT_ZLIB_START, 0x09);
        assert_eq!(magic::SYNC_ZSTD_START, 0x0A);
        assert_eq!(magic::SYNC_NOCRYPT_ZSTD_START, 0x0B);
        assert_eq!(magic::ASYNC_ZSTD_START, 0x0C);
        assert_eq!(magic::ASYNC_NOCRYPT_ZSTD_START, 0x0D);
        assert_eq!(magic::END, 0x00);
    }

    #[test]
    fn magic_start_is_valid_accepts_only_known_bytes() {
        for m in [
            magic::SYNC_ZLIB_START,
            magic::SYNC_NOCRYPT_ZLIB_START,
            magic::ASYNC_ZLIB_START,
            magic::ASYNC_NOCRYPT_ZLIB_START,
            magic::SYNC_ZSTD_START,
            magic::SYNC_NOCRYPT_ZSTD_START,
            magic::ASYNC_ZSTD_START,
            magic::ASYNC_NOCRYPT_ZSTD_START,
        ] {
            assert!(magic::magic_start_is_valid(m), "0x{m:02x} should be valid");
        }
        for m in [0x00u8, 0x01, 0x05, 0x0E, 0x41, 0xFF] {
            assert!(
                !magic::magic_start_is_valid(m),
                "0x{m:02x} should be invalid"
            );
        }
    }

    #[test]
    fn set_header_info_layout() {
        let region = header_region(magic::SYNC_NOCRYPT_ZLIB_START);
        let mut crypt = LogCrypt::new(None);

        assert_eq!(region[off::MAGIC], magic::SYNC_NOCRYPT_ZLIB_START);
        // sync => seq 0
        assert_eq!(le::read_u16(&region, off::SEQ), 0);
        assert_eq!(region[off::BEGIN_HOUR], local_hour());
        assert_eq!(region[off::END_HOUR], local_hour());
        assert_eq!(le::read_u32(&region, off::LENGTH), 0);
        assert_eq!(&region[off::PUBKEY..HEADER_LEN], &[0u8; CLIENT_PUBKEY_LEN]);

        // async: the process-global counter is monotonic and never 0.
        let mut buf = vec![0u8; HEADER_LEN];
        let mut prev = 0u16;
        for _ in 0..4 {
            crypt.set_header_info(&mut buf, true, magic::ASYNC_ZLIB_START);
            let seq = le::read_u16(&buf, off::SEQ);
            assert_ne!(seq, 0);
            assert_eq!(seq, prev.wrapping_add(1));
            prev = seq;
            assert_eq!(buf[off::MAGIC], magic::ASYNC_ZLIB_START);
        }

        // A short region is left untouched instead of panicking.
        let mut short = vec![0u8; HEADER_LEN - 1];
        crypt.set_header_info(&mut short, false, magic::SYNC_ZLIB_START);
        assert_eq!(short, vec![0u8; HEADER_LEN - 1]);
    }

    #[test]
    fn set_header_info_writes_client_pubkey() {
        let crypt = crypt_with_key();
        let mut region = vec![0u8; HEADER_LEN];
        let mut owned = crypt;
        owned.set_header_info(&mut region, false, magic::ASYNC_ZSTD_START);
        assert_eq!(
            &region[off::PUBKEY..HEADER_LEN],
            &[0xABu8; CLIENT_PUBKEY_LEN]
        );
    }

    #[test]
    fn get_log_len_and_update_log_len_round_trip() {
        let mut region = header_region(magic::SYNC_NOCRYPT_ZLIB_START);

        assert_eq!(LogCrypt::get_log_len(&region), 0);
        LogCrypt::update_log_len(&mut region, 42);
        assert_eq!(LogCrypt::get_log_len(&region), 42);
        LogCrypt::update_log_len(&mut region, 8);
        assert_eq!(LogCrypt::get_log_len(&region), 50);

        // Only the length field may change.
        assert_eq!(region[off::MAGIC], magic::SYNC_NOCRYPT_ZLIB_START);
        assert_eq!(le::read_u32(&region, off::LENGTH), 50);
        assert_eq!(
            &region[..off::LENGTH],
            &header_region(magic::SYNC_NOCRYPT_ZLIB_START)[..off::LENGTH]
        );
    }

    #[test]
    fn get_log_len_rejects_bad_magic_and_short_region() {
        let mut region = header_region(magic::SYNC_ZLIB_START);
        LogCrypt::update_log_len(&mut region, 1234);
        assert_eq!(LogCrypt::get_log_len(&region), 1234);

        region[off::MAGIC] = 0x55;
        assert_eq!(LogCrypt::get_log_len(&region), 0);

        let short = vec![0u8; HEADER_LEN - 1];
        assert_eq!(LogCrypt::get_log_len(&short), 0);
    }

    #[test]
    fn update_log_len_wraps_like_cpp() {
        let mut region = header_region(magic::SYNC_ZLIB_START);
        LogCrypt::update_log_len(&mut region, u32::MAX);
        assert_eq!(LogCrypt::get_log_len(&region), u32::MAX);
        LogCrypt::update_log_len(&mut region, 1);
        assert_eq!(LogCrypt::get_log_len(&region), 0);
    }

    #[test]
    fn get_log_hour_reads_header() {
        let mut region = header_region(magic::ASYNC_ZLIB_START);
        region[off::BEGIN_HOUR] = 3;
        region[off::END_HOUR] = 21;
        assert_eq!(LogCrypt::get_log_hour(&region), Some((3, 21)));

        region[off::MAGIC] = 0x77;
        assert_eq!(LogCrypt::get_log_hour(&region), None);

        let short = vec![0u8; HEADER_LEN - 1];
        assert_eq!(LogCrypt::get_log_hour(&short), None);
    }

    #[test]
    fn update_log_hour_stamps_end_hour_only() {
        let mut region = header_region(magic::ASYNC_ZLIB_START);
        region[off::BEGIN_HOUR] = 1;
        region[off::END_HOUR] = 2;

        LogCrypt::update_log_hour(&mut region);

        assert_eq!(region[off::BEGIN_HOUR], 1);
        assert_eq!(region[off::END_HOUR], local_hour());
        assert_eq!(LogCrypt::get_log_hour(&region), Some((1, local_hour())));
    }

    #[test]
    fn set_tailer_info_writes_magic_end() {
        let mut tail = vec![0xFFu8; 4];
        LogCrypt::set_tailer_info(&mut tail, magic::END);
        assert_eq!(tail[0], 0x00);
        // The rest of the region is untouched.
        assert_eq!(&tail[1..], &[0xFFu8; 3]);

        let mut empty = Vec::new();
        LogCrypt::set_tailer_info(&mut empty, magic::END);
        assert!(empty.is_empty());
    }

    #[test]
    fn crypt_sync_log_framing() {
        let mut crypt = LogCrypt::new(None);
        let payload = b"hello mars xlog";
        let mut out = AutoBuffer::new();

        let total = crypt.crypt_sync_log(
            payload,
            &mut out,
            magic::SYNC_NOCRYPT_ZLIB_START,
            magic::END,
        );

        assert_eq!(total, HEADER_LEN + TAILER_LEN + payload.len());
        assert_eq!(out.len(), total);

        let bytes = out.as_slice();
        assert_eq!(bytes.len(), total);
        assert_eq!(bytes[off::MAGIC], magic::SYNC_NOCRYPT_ZLIB_START);
        assert_eq!(le::read_u16(bytes, off::SEQ), 0);
        assert_eq!(bytes[off::BEGIN_HOUR], local_hour());
        assert_eq!(bytes[off::END_HOUR], local_hour());
        assert_eq!(le::read_u32(bytes, off::LENGTH), payload.len() as u32);
        assert_eq!(&bytes[HEADER_LEN..HEADER_LEN + payload.len()], payload);
        let tailer_at = HEADER_LEN + payload.len();
        assert_eq!(tailer_at, bytes.len() - TAILER_LEN);
        assert_eq!(bytes[tailer_at], magic::END);
    }

    #[test]
    fn crypt_sync_log_empty_payload_still_framed() {
        let mut crypt = LogCrypt::new(None);
        let mut out = AutoBuffer::new();
        let total = crypt.crypt_sync_log(&[], &mut out, magic::SYNC_ZSTD_START, magic::END);

        assert_eq!(total, HEADER_LEN + TAILER_LEN);
        assert_eq!(out.len(), total);
        assert_eq!(out.as_slice()[off::MAGIC], magic::SYNC_ZSTD_START);
        assert_eq!(out.as_slice()[HEADER_LEN], magic::END);
    }

    #[test]
    fn crypt_sync_log_decode_round_trip() {
        let mut crypt = LogCrypt::new(None);
        let payload: Vec<u8> = (0..200u32).map(|i| (i % 251) as u8).collect();
        let mut out = AutoBuffer::new();
        crypt.crypt_sync_log(
            &payload,
            &mut out,
            magic::SYNC_NOCRYPT_ZSTD_START,
            magic::END,
        );

        let bytes = out.as_slice();
        // "decode": trust the framing and pull the body back out.
        assert!(magic::magic_start_is_valid(bytes[0]));
        let len = LogCrypt::get_log_len(bytes) as usize;
        assert_eq!(len, payload.len());
        let body = &bytes[HEADER_LEN..HEADER_LEN + len];
        assert_eq!(body, &payload[..]);
        assert_eq!(bytes[HEADER_LEN + len], magic::END);
        assert_eq!(bytes.len(), HEADER_LEN + len + TAILER_LEN);
    }

    #[test]
    fn crypt_async_log_nocrypt_copies_verbatim() {
        let crypt = LogCrypt::new(None);
        assert!(!crypt.is_crypt());

        let payload = b"0123456789abcdef";
        let mut out = Vec::new();
        let mut remain = 0;
        crypt.crypt_async_log(payload, &mut out, &mut remain);

        assert_eq!(out, payload.to_vec());
        assert_eq!(remain, 0);
    }

    #[test]
    fn crypt_async_log_tea_encrypts_full_blocks() {
        let crypt = crypt_with_key();
        let payload: Vec<u8> = (0..24u8).collect();
        let mut out = Vec::new();
        let mut remain = 0;
        crypt.crypt_async_log(&payload, &mut out, &mut remain);

        assert_eq!(out.len(), payload.len());
        assert_eq!(remain, 0);

        // 3 whole blocks were encrypted => ciphertext differs everywhere.
        assert_ne!(out, payload);
        for (i, block) in payload.as_chunks::<TEA_BLOCK_LEN>().0.iter().enumerate() {
            let mut expected = [le::read_u32(block, 0), le::read_u32(block, 4)];
            tea_encrypt(&mut expected, crypt.tea_key_);
            let got = &out[i * TEA_BLOCK_LEN..(i + 1) * TEA_BLOCK_LEN];
            assert_eq!(le::read_u32(got, 0), expected[0]);
            assert_eq!(le::read_u32(got, 4), expected[1]);
        }
    }

    #[test]
    fn crypt_async_log_leaves_remainder_in_the_clear() {
        let crypt = crypt_with_key();

        for extra in 0..TEA_BLOCK_LEN {
            let payload: Vec<u8> = (0..(TEA_BLOCK_LEN * 2 + extra) as u8)
                .map(|i| i.wrapping_add(1))
                .collect();
            let mut out = Vec::new();
            let mut remain = 0;
            crypt.crypt_async_log(&payload, &mut out, &mut remain);

            assert_eq!(remain, extra, "remain for {extra} extra bytes");
            assert_eq!(out.len(), payload.len());

            let clear_at = payload.len() - extra;
            assert_eq!(
                &out[clear_at..],
                &payload[clear_at..],
                "tail must be plaintext"
            );
            if extra != 0 {
                assert_ne!(&out[..clear_at], &payload[..clear_at]);
            }
        }
    }

    #[test]
    fn crypt_async_log_shorter_than_one_block_is_all_cleartext() {
        let crypt = crypt_with_key();
        let payload = b"abc";
        let mut out = Vec::new();
        let mut remain = 0;
        crypt.crypt_async_log(payload, &mut out, &mut remain);

        assert_eq!(remain, 3);
        assert_eq!(out, payload.to_vec());
    }

    #[test]
    fn crypt_async_log_reuses_out_buffer() {
        let crypt = crypt_with_key();
        let mut out = vec![0xFFu8; 32];
        let mut remain = 0;
        crypt.crypt_async_log(&[1, 2, 3], &mut out, &mut remain);
        assert_eq!(out, vec![1u8, 2, 3]);
    }

    #[test]
    fn tea_encrypt_matches_cpp_reference() {
        // Produced by compiling the C++ `__TeaEncrypt` from
        // mars/xlog/crypt/log_crypt.cc with gcc.
        let mut v = [0u32, 0u32];
        tea_encrypt(&mut v, [0, 0, 0, 0]);
        assert_eq!(v, [0xa889_f798, 0x182d_8083]);

        let mut v = [0x0123_4567u32, 0x89ab_cdef];
        tea_encrypt(&mut v, [0x0011_2233, 0x4455_6677, 0x8899_aabb, 0xccdd_eeff]);
        assert_eq!(v, [0x7cf6_c003, 0x2c4a_f316]);
    }

    #[test]
    fn tea_encrypt_round_trips_with_reference_decrypt() {
        fn tea_decrypt(v: &mut [u32; 2], k: [u32; 4]) {
            let (mut v0, mut v1) = (v[0], v[1]);
            let (k0, k1, k2, k3) = (k[0], k[1], k[2], k[3]);
            let mut sum = TEA_DELTA.wrapping_mul(TEA_ROUNDS);
            for _ in 0..TEA_ROUNDS {
                v1 = v1.wrapping_sub(
                    (v0.wrapping_shl(4).wrapping_add(k2))
                        ^ (v0.wrapping_add(sum))
                        ^ (v0.wrapping_shr(5).wrapping_add(k3)),
                );
                v0 = v0.wrapping_sub(
                    (v1.wrapping_shl(4).wrapping_add(k0))
                        ^ (v1.wrapping_add(sum))
                        ^ (v1.wrapping_shr(5).wrapping_add(k1)),
                );
                sum = sum.wrapping_sub(TEA_DELTA);
            }
            v[0] = v0;
            v[1] = v1;
        }

        let key = [0xdead_beef, 0x1234_5678, 0x9abc_def0, 0x0fed_cba9];
        let mut v = [0x1122_3344u32, 0x5566_7788];
        let plain = v;
        tea_encrypt(&mut v, key);
        assert_ne!(v, plain);
        tea_decrypt(&mut v, key);
        assert_eq!(v, plain);
    }

    #[test]
    fn fix_accepts_valid_magic_and_restores_seq() {
        let mut crypt = LogCrypt::new(None);
        let mut region = header_region(magic::ASYNC_NOCRYPT_ZLIB_START);
        le::write_u16(&mut region, off::SEQ, 0x1234);
        LogCrypt::update_log_len(&mut region, 777);

        assert_eq!(crypt.fix(&region), Some(777));
        assert_eq!(crypt.seq_, 0x1234);
    }

    #[test]
    fn fix_rejects_invalid_magic() {
        let mut crypt = LogCrypt::new(None);
        let mut region = header_region(magic::ASYNC_NOCRYPT_ZLIB_START);
        LogCrypt::update_log_len(&mut region, 777);

        region[off::MAGIC] = 0x00;
        assert_eq!(crypt.fix(&region), None);
        assert_eq!(crypt.seq_, 0);

        region[off::MAGIC] = 0x42;
        assert_eq!(crypt.fix(&region), None);
    }

    #[test]
    fn fix_rejects_truncated_region() {
        let mut crypt = LogCrypt::new(None);
        let region = vec![magic::SYNC_ZLIB_START; HEADER_LEN - 1];
        assert_eq!(crypt.fix(&region), None);
        assert_eq!(crypt.fix(&[]), None);
    }

    #[test]
    fn new_without_pubkey_is_not_crypt() {
        for key in [None, Some(""), Some("zz"), Some(&"a".repeat(128))] {
            let crypt = LogCrypt::new(key);
            assert!(!crypt.is_crypt());
            assert_eq!(crypt.header_len(), HEADER_LEN);
            assert_eq!(crypt.tailer_len(), TAILER_LEN);
        }
    }

    #[test]
    fn new_with_wrong_length_pubkey_is_not_crypt() {
        let valid_hex = "04".repeat(CLIENT_PUBKEY_LEN);
        let crypt = LogCrypt::new(Some(&valid_hex[..valid_hex.len() - 2]));
        assert!(!crypt.is_crypt());

        let crypt = LogCrypt::new(Some(&valid_hex.repeat(2)));
        assert!(!crypt.is_crypt());
    }

    #[test]
    fn hex_to_buffer_matches_cpp() {
        assert_eq!(
            hex_to_buffer("00ffAABB").unwrap(),
            vec![0x00, 0xff, 0xaa, 0xbb]
        );
        assert!(hex_to_buffer("").is_none());
        assert!(hex_to_buffer("abc").is_none());
        assert!(hex_to_buffer("0g").is_none());
        assert!(hex_to_buffer("0!").is_none());
    }

    #[test]
    fn no_crypt_round_trip_through_sync_and_async_paths() {
        let mut crypt = LogCrypt::new(None);
        assert!(!crypt.is_crypt());

        let payload = b"the quick brown fox jumps over the lazy dog";
        let mut out = AutoBuffer::new();
        crypt.crypt_sync_log(
            payload,
            &mut out,
            magic::SYNC_NOCRYPT_ZLIB_START,
            magic::END,
        );

        let bytes = out.as_slice();
        let mut async_out = Vec::new();
        let mut remain = 0;
        crypt.crypt_async_log(payload, &mut async_out, &mut remain);

        assert_eq!(remain, 0);
        assert_eq!(async_out, payload.to_vec());
        assert_eq!(
            &bytes[HEADER_LEN..HEADER_LEN + payload.len()],
            &async_out[..]
        );
        assert_eq!(bytes[bytes.len() - 1], magic::END);
    }

    #[test]
    fn ecdh_derives_a_tea_key_from_a_real_pubkey() {
        // secp256k1 generator point, 64 raw bytes X || Y.
        let x = "79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798";
        let y = "483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8";
        let pubkey = format!("{x}{y}").to_lowercase();

        let crypt = LogCrypt::new(Some(&pubkey));
        // TODO(port): if this ever flips to false, the k256 ECDH path regressed
        // (see `make_key`) — the non-crypt path stays byte compatible either
        // way, but async logs would stop being encrypted.
        assert!(crypt.is_crypt(), "k256 ECDH derivation should succeed");
        assert!(crypt.tea_key_.iter().any(|w| *w != 0));
        assert!(crypt.client_pubkey_.iter().any(|b| *b != 0));

        // The derived key must be stable for this instance and used by TEA.
        let payload: Vec<u8> = (0..16u8).collect();
        let mut out = Vec::new();
        let mut remain = 0;
        crypt.crypt_async_log(&payload, &mut out, &mut remain);
        assert_eq!(out.len(), 16);
        assert_eq!(remain, 0);
        assert_ne!(out, payload);
    }

    /// The handshake is the one part of the crypt layer only an outside answer
    /// can check: [`make_key`] generates a random client key, so every
    /// in-process assertion about the shared secret is satisfied by whatever
    /// the code happens to produce. This pins the whole chain — secp256k1
    /// ECDH, `memcpy(tea_key_, ecdh_key, sizeof(tea_key_))` on a little-endian
    /// host, `__TeaEncrypt` — to a vector computed outside this crate (the
    /// curve arithmetic and TEA re-implemented from their definitions, not
    /// read off `k256`).
    #[test]
    fn ecdh_matches_an_externally_computed_vector() {
        // `X || Y` of `0x4a7b…5a4b * G`: the 128 hex characters a host passes
        // to `LogCrypt::new`.
        let svr_pubkey = hex_to_buffer(
            "cc7f35af89a891a4bd45a9f73b6783a66fef522e2bfdb2d669676cd546138b8\
             c5834d0a4f864bcab2ba50f98834646dd0b0f37bb4566922df8f60fcae5d81d90",
        )
        .unwrap();
        // The generated client key is the only random part, so it is supplied
        // here instead: `0x03a2…4a3`.
        let client_pri = k256::SecretKey::from_slice(
            &hex_to_buffer("03a2b1c0d9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a4b4c3d2e1f0a9b8c7d6e5f4a3")
                .unwrap(),
        )
        .unwrap();

        let pair = derive_key(&svr_pubkey, &client_pri).expect("the handshake must succeed");

        // `client_pubkey_` = `0x03a2…4a3 * G`, the 64 bytes every header
        // carries in the clear.
        let expected_pubkey = hex_to_buffer(
            "be7288310928da1c217ecf507df5cc36472ce78f2916276cb90f8d2a0ad0f71\
             f10ef0530019817aaa2726afccfa9724f7a48178d6e0888b513e8c691494d350c",
        )
        .unwrap();
        assert_eq!(&pair.client_pubkey[..], &expected_pubkey[..]);

        // `x(client_priv * server_pub)` = `cff28578…91be`, and the TEA key is
        // the first 16 of those bytes read as four **little-endian** `u32`s —
        // so `tea_key_[0]` is the *high* four bytes reversed, `cff28578` →
        // `0x7885f2cf`. Read it the other way round and everything still
        // encrypts and still decrypts in-process, which is exactly why the
        // answer has to come from outside this crate.
        assert_eq!(
            pair.tea_key,
            [0x7885_f2cf, 0x2726_3841, 0xcf97_7dac, 0x2c9d_80f2]
        );

        // ... and the key it produced encrypts to what the C++ encrypts to:
        // `"mars tea"` under this key.
        let crypt = LogCrypt {
            seq_: 0,
            tea_key_: pair.tea_key,
            client_pubkey_: pair.client_pubkey,
            is_crypt_: true,
        };
        let mut out = Vec::new();
        let mut remain = 0;
        crypt.crypt_async_log(b"mars tea", &mut out, &mut remain);
        assert_eq!(remain, 0);
        assert_eq!(out, hex_to_buffer("0a4cd25aef241592").unwrap());
    }

    #[test]
    fn ecdh_rejects_a_point_that_is_not_on_the_curve() {
        // Correct length, valid hex, but not a secp256k1 point.
        let pubkey = "02".repeat(CLIENT_PUBKEY_LEN);
        assert!(!LogCrypt::new(Some(&pubkey)).is_crypt());
    }
}
