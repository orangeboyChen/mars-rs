//! `mars-buffer` — port of `mars/xlog/src/log_base_buffer.{h,cc}`,
//! `log_zlib_buffer.cc` and `log_zstd_buffer.cc`.
//!
//! The C++ class hierarchy is `LogBaseBuffer` with two concrete subclasses
//! (`LogZlibBuffer`, `LogZstdBuffer`) that only differ in their `Compress()`
//! implementation, their magic bytes and (for zstd) the compression level. In
//! Rust that is modelled by one struct plus a [`CompressMode`] tag, with the
//! compression backend selected at runtime inside `compress::Compressor`.
//!
//! | C++                                   | Rust                                   |
//! |---------------------------------------|----------------------------------------|
//! | `LogBaseBuffer` / `LogZlibBuffer` / `LogZstdBuffer` | [`LogBuffer`]          |
//! | `LogBaseBuffer::Write(data, len)`     | [`LogBuffer::write`]                   |
//! | `LogBaseBuffer::Write(data, len, out)`| [`LogBuffer::write_sync`]              |
//! | `LogBaseBuffer::Flush(out)`           | [`LogBuffer::flush`]                   |
//! | `LogBaseBuffer::__Fix()`              | [`LogBuffer::attach`]                  |
//! | `LogBaseBuffer::GetPeriodLogs`        | [`get_period_logs`]                    |
//! | `LogBaseBuffer::GetPeriodLogsWithTimeoutMs` | [`get_period_logs_with_timeout_ms`] |
//!
//! # Design note: the region is a parameter, not a field
//!
//! In the C++ the buffer owns a `PtrBuffer` that points into the mmap'd cache
//! file. The Rust appender owns that mapping itself, so a self-referential
//! `LogBuffer<'a>` holding `&'a mut [u8]` would be impossible to build. Instead
//! [`LogBuffer`] stores *state only* — the `LogCrypt`, the compressor, the
//! logical length and the magic bytes — and the backing region is passed in on
//! every call. No lifetime parameters, no `unsafe`.

#![deny(unsafe_code)]

mod compress;
mod period;

use mars_core::AutoBuffer;
use mars_crypt::{magic, LogCrypt, HEADER_LEN, TAILER_LEN};

use compress::Compressor;

pub use period::{get_period_logs, get_period_logs_with_timeout_ms};

/// Which compression backend to use; replaces the `LogZlibBuffer` /
/// `LogZstdBuffer` subclass choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressMode {
    /// Raw DEFLATE (`log_zlib_buffer.cc`).
    Zlib,
    /// zstd streaming (`log_zstd_buffer.cc`).
    Zstd,
}

/// Port of `mars::xlog::LogBaseBuffer` + `LogZlibBuffer` + `LogZstdBuffer`.
///
/// Holds everything the C++ keeps in the object *except* the backing memory,
/// which is passed to each method (see the crate docs).
pub struct LogBuffer {
    /// `log_crypt_`.
    crypt: LogCrypt,
    /// `is_compress_`.
    is_compress: bool,
    /// `is_crypt_`.
    is_crypt: bool,
    /// `remain_nocrypt_len_` — trailing bytes of the region that were written
    /// but could not be TEA-encrypted yet because they do not fill an 8-byte
    /// block. They are re-crypted together with the next chunk.
    remain_nocrypt_len: usize,
    /// `buff_.Length()`. The C++ keeps `pos_` and `length_` separately, but
    /// every code path sets them to the same value (`__Reset`, `__Flush`,
    /// `Write`, `__Fix`, `__Clear`), so the write destination is always
    /// `region[length..]`.
    length: usize,
    /// `__GetMagicSyncStart()`.
    magic_start_sync: u8,
    /// `__GetMagicAsyncStart()`.
    magic_start_async: u8,
    /// `__GetMagicEnd()` — always `LogMagicNum::kMagicEnd`.
    magic_end: u8,
    /// Subclass selector (`LogZlibBuffer` vs `LogZstdBuffer`).
    mode: CompressMode,
    /// zstd compression level; ignored by the zlib backend.
    level: i32,
    /// The live compression stream, if any. `None` before the first
    /// `__Reset()` and after every [`LogBuffer::flush`] (the C++ calls
    /// `deflateEnd` / `ZSTD_e_end` there).
    compressor: Option<Compressor>,
}

impl LogBuffer {
    /// `LogZlibBuffer::LogZlibBuffer(...)` / `LogZstdBuffer::LogZstdBuffer(...)`.
    ///
    /// * `is_compress` — `false` stores the payload verbatim, `true` runs it
    ///   through the compressor.
    /// * `pubkey` — see `LogCrypt::new`; anything but a 128-char hex
    ///   secp256k1 public key leaves [`LogBuffer::is_crypt`] `false`.
    /// * `mode` — selects the compression backend.
    /// * `level` — zstd level (`ZSTD_c_compressionLevel`); ignored for zlib,
    ///   which always uses `Z_BEST_COMPRESSION`.
    ///
    /// Unlike the C++ constructor this does *not* run `__Fix()`; the caller
    /// maps the cache file first and then calls [`LogBuffer::attach`].
    pub fn new(is_compress: bool, pubkey: Option<&str>, mode: CompressMode, level: i32) -> Self {
        let crypt = LogCrypt::new(pubkey);
        let is_crypt = crypt.is_crypt();

        let (magic_start_sync, magic_start_async) = match mode {
            CompressMode::Zlib => {
                if is_crypt {
                    (magic::SYNC_ZLIB_START, magic::ASYNC_ZLIB_START)
                } else {
                    (
                        magic::SYNC_NOCRYPT_ZLIB_START,
                        magic::ASYNC_NOCRYPT_ZLIB_START,
                    )
                }
            }
            CompressMode::Zstd => {
                if is_crypt {
                    (magic::SYNC_ZSTD_START, magic::ASYNC_ZSTD_START)
                } else {
                    (
                        magic::SYNC_NOCRYPT_ZSTD_START,
                        magic::ASYNC_NOCRYPT_ZSTD_START,
                    )
                }
            }
        };

        Self {
            crypt,
            is_compress,
            is_crypt,
            remain_nocrypt_len: 0,
            length: 0,
            magic_start_sync,
            magic_start_async,
            magic_end: magic::END,
            mode,
            level,
            compressor: None,
        }
    }

    /// `LogCrypt::IsCrypt()` — whether the payload is TEA-encrypted.
    pub fn is_crypt(&self) -> bool {
        self.is_crypt
    }

    /// Whether the payload is compressed (`is_compress_`).
    pub fn is_compress(&self) -> bool {
        self.is_compress
    }

    /// `__Fix()` — call once after mapping the cache file.
    ///
    /// Recovers the logical length from a half-written region (validating the
    /// magic through `LogCrypt::fix`), clamping it so that
    /// `raw + HEADER_LEN + TAILER_LEN <= region.len()`. Resets the logical
    /// length to `0` when the region holds no valid header.
    pub fn attach(&mut self, region: &mut [u8]) {
        let max_length = region.len();

        match self.crypt.fix(region) {
            Some(raw_log_len) => {
                let mut raw_log_len = raw_log_len as usize;
                if raw_log_len
                    .saturating_add(HEADER_LEN)
                    .saturating_add(TAILER_LEN)
                    > max_length
                {
                    raw_log_len = max_length.saturating_sub(HEADER_LEN + TAILER_LEN);
                }
                self.length = raw_log_len.saturating_add(HEADER_LEN).min(max_length);
            }
            None => self.length = 0,
        }

        // The C++ leaves this member indeterminate here; zero is the only
        // sensible value for a freshly attached region.
        self.remain_nocrypt_len = 0;
    }

    /// `PtrBuffer::Length()` — logical length of the region.
    pub fn len(&self) -> usize {
        self.length
    }

    /// Whether the logical length is zero (the buffer holds no header yet).
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// `LogBaseBuffer::Write(const void* _data, size_t _length)` — the async
    /// path.
    ///
    /// Resets (writes the header) when the buffer is empty, then either
    /// compresses `data` into the region or copies it verbatim, TEA-crypts the
    /// freshly written span *in place* and updates the header's length field.
    ///
    /// Returns `false` when `data` is empty, when the region cannot hold the
    /// header, or when there is no room left for the payload (the C++ relies on
    /// `size_t` underflow here and corrupts memory instead).
    pub fn write(&mut self, region: &mut [u8], data: &[u8]) -> bool {
        if data.is_empty() {
            return false;
        }

        if self.length == 0 && !self.reset(region) {
            return false;
        }

        let before_len = self.length;

        let write_len = if self.is_compress {
            // `avail_out = buff_.MaxLength() - buff_.Length() - GetTailerLen()`
            let Some(avail_out) = region
                .len()
                .checked_sub(self.length)
                .and_then(|room| room.checked_sub(TAILER_LEN))
            else {
                return false;
            };
            if avail_out == 0 {
                return false;
            }

            // `__Reset()` normally creates the stream; also create it lazily
            // for a region whose content was recovered by `attach()` (the C++
            // would `deflate()` a zeroed `z_stream` and fail instead).
            if self.compressor.is_none() {
                self.compressor = Compressor::new(self.mode, self.level);
            }
            let Some(compressor) = self.compressor.as_mut() else {
                return false;
            };

            let dst = &mut region[self.length..self.length + avail_out];
            match compressor.compress(data, dst) {
                Some(n) => n,
                None => return false,
            }
        } else {
            // Reserve the tailer byte like the compress branch does, or a full
            // region produces a record with no kMagicEnd and every reader
            // discards it as corrupt.
            let room = region
                .len()
                .saturating_sub(self.length)
                .saturating_sub(TAILER_LEN);
            if room == 0 {
                return false;
            }
            // `buff_.Write(_data, _length)` — the C++ truncates the copy at
            // `MaxLength()` but then keeps using the untruncated length; we
            // clamp so the follow-up crypt pass cannot read out of bounds.
            let n = data.len().min(room);
            region[self.length..self.length + n].copy_from_slice(&data[..n]);
            n
        };

        // `before_len -= remain_nocrypt_len_` — rewind over the bytes the last
        // write could not encrypt, they are part of this chunk's input.
        let last_remain_len = self.remain_nocrypt_len;
        let crypt_start = before_len - last_remain_len;

        // `out_buffer` in the C++ is a second copy of the span that is then
        // written back over it; TEA is a block cipher, so the span is
        // encrypted where it lies instead. That is one allocation and two
        // copies fewer per record, and byte for byte the same answer.
        self.remain_nocrypt_len = self
            .crypt
            .crypt_async_log_in_place(&mut region[crypt_start..before_len + write_len]);
        self.length = before_len + write_len;

        // `UpdateLogLen(buff_.Ptr(), out_buffer.size() - last_remain_len)`
        LogCrypt::update_log_len(region, write_len as u32);

        true
    }

    /// `LogBaseBuffer::Write(const void* _data, size_t _inputlen, AutoBuffer&)`
    /// — the sync path.
    ///
    /// Emits a complete `header + body + tailer` record into `out` (no
    /// compression, exactly like the C++). Returns `false` for empty input.
    pub fn write_sync(&mut self, data: &[u8], out: &mut AutoBuffer) -> bool {
        if data.is_empty() {
            return false;
        }

        self.crypt
            .crypt_sync_log(data, out, self.magic_start_sync, self.magic_end);

        true
    }

    /// `LogBaseBuffer::Flush(AutoBuffer& _buff)`.
    ///
    /// Stamps the current hour into the header, appends the tailer byte, copies
    /// the whole region into `out` and then clears the region. Returns the
    /// number of bytes appended to `out` (`0` when the buffer was empty).
    pub fn flush(&mut self, region: &mut [u8], out: &mut AutoBuffer) -> usize {
        // `LogZlibBuffer::Flush` / `LogZstdBuffer::Flush`: `deflateEnd` /
        // `ZSTD_e_end` — the stream is re-created by the next `__Reset()`.
        self.compressor = None;

        if LogCrypt::get_log_len(region) == 0 {
            self.clear(region);
            return 0;
        }

        // `__Flush()`
        LogCrypt::update_log_hour(region);
        if self.length + TAILER_LEN <= region.len() {
            LogCrypt::set_tailer_info(
                &mut region[self.length..self.length + TAILER_LEN],
                self.magic_end,
            );
            self.length += TAILER_LEN;
        }

        let flush_len = self.length;
        out.write(&region[..flush_len]);

        // `__Clear()`
        self.clear(region);

        flush_len
    }

    /// `LogBaseBuffer::__Reset()`.
    ///
    /// Clears the region, writes the async header and (re)starts the
    /// compression stream.
    fn reset(&mut self, region: &mut [u8]) -> bool {
        if region.len() < HEADER_LEN {
            return false;
        }

        self.clear(region);

        // NOTE(port): the C++ passes `is_compress_` as `_is_async`, which looks
        // like a bug — the sequence number should only depend on the sync/async
        // path. This is the async path, so we pass `true`.
        self.crypt
            .set_header_info(region, true, self.magic_start_async);
        self.length = HEADER_LEN;

        if self.is_compress {
            self.compressor = Compressor::new(self.mode, self.level);
            if self.compressor.is_none() {
                // Mirrors `deflateInit2` failing in `LogZlibBuffer::__Reset`.
                return false;
            }
        }

        true
    }

    /// `LogBaseBuffer::__Clear()` — zeroes the whole region and resets the
    /// logical length and the crypto remainder.
    fn clear(&mut self, region: &mut [u8]) {
        region.fill(0);
        self.length = 0;
        self.remain_nocrypt_len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mars_core::{le, local_hour};

    /// 4 KiB, like a small mmap'd cache file.
    const REGION_LEN: usize = 4 * 1024;

    /// The secp256k1 base point `G` as `X || Y`, hex encoded — a valid
    /// 128-character server public key, so `LogCrypt` derives a TEA key and
    /// [`LogBuffer::is_crypt`] becomes `true`.
    const TEST_PUBKEY: &str = concat!(
        "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8",
    );

    #[test]
    fn ctor_picks_magic_by_mode_and_crypt_state() {
        let buf = LogBuffer::new(false, None, CompressMode::Zlib, 6);
        assert!(!buf.is_compress());
        assert!(!buf.is_crypt());
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        assert_eq!(buf.magic_start_sync, magic::SYNC_NOCRYPT_ZLIB_START);
        assert_eq!(buf.magic_start_async, magic::ASYNC_NOCRYPT_ZLIB_START);

        let buf = LogBuffer::new(true, None, CompressMode::Zstd, 3);
        assert!(buf.is_compress());
        assert_eq!(buf.magic_start_sync, magic::SYNC_NOCRYPT_ZSTD_START);
        assert_eq!(buf.magic_start_async, magic::ASYNC_NOCRYPT_ZSTD_START);
    }

    #[test]
    fn write_appends_and_updates_header() {
        let mut region = vec![0u8; REGION_LEN];
        let mut buf = LogBuffer::new(false, None, CompressMode::Zlib, 6);
        let hour = local_hour();

        assert!(buf.write(&mut region, b"hello "));
        assert!(buf.write(&mut region, b"world\n"));

        assert_eq!(buf.len(), HEADER_LEN + 12);
        assert_eq!(region[0], magic::ASYNC_NOCRYPT_ZLIB_START);
        assert_eq!(LogCrypt::get_log_len(&region), 12);
        assert_eq!(region[3], hour, "begin hour");
        assert_eq!(region[4], hour, "end hour before flush");
        assert_eq!(le::read_u32(&region, 5), 12);
        assert_eq!(&region[HEADER_LEN..HEADER_LEN + 12], b"hello world\n");
    }

    #[test]
    fn flush_emits_record_and_clears_region() {
        let mut region = vec![0u8; REGION_LEN];
        let mut buf = LogBuffer::new(false, None, CompressMode::Zlib, 6);
        let hour = local_hour();

        assert!(buf.write(&mut region, b"one\n"));
        assert!(buf.write(&mut region, b"two\n"));

        let mut out = AutoBuffer::new();
        let n = buf.flush(&mut region, &mut out);

        assert_eq!(n, HEADER_LEN + 8 + TAILER_LEN);
        assert_eq!(out.len(), n);
        assert_eq!(buf.len(), 0);
        assert!(region.iter().all(|&b| b == 0), "region must be zeroed");

        let data = out.as_slice().to_vec();
        assert_eq!(data[0], magic::ASYNC_NOCRYPT_ZLIB_START);
        assert_eq!(data[3], hour, "begin hour");
        assert_eq!(data[4], hour, "end hour stamped by flush");
        assert_eq!(LogCrypt::get_log_len(&data), 8);
        assert_eq!(&data[HEADER_LEN..HEADER_LEN + 8], b"one\ntwo\n");
        assert_eq!(data[n - 1], magic::END, "tailer");
    }

    #[test]
    fn flush_empty_buffer_is_a_noop() {
        let mut region = vec![0u8; REGION_LEN];
        let mut buf = LogBuffer::new(false, None, CompressMode::Zlib, 6);
        buf.attach(&mut region);

        let mut out = AutoBuffer::new();
        assert_eq!(buf.flush(&mut region, &mut out), 0);
        assert!(out.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn write_rejects_empty_input_and_full_region() {
        let mut region = vec![0u8; REGION_LEN];
        let mut buf = LogBuffer::new(true, None, CompressMode::Zlib, 6);
        assert!(!buf.write(&mut region, b""));

        // Too small to even hold the header: `__Reset` fails.
        let mut tiny = vec![0u8; HEADER_LEN - 1];
        let mut buf = LogBuffer::new(false, None, CompressMode::Zlib, 6);
        assert!(!buf.write(&mut tiny, b"x"));
        assert_eq!(buf.len(), 0);

        // Room for the header but not for the tailer: no payload can be added.
        let mut tight = vec![0u8; HEADER_LEN + TAILER_LEN];
        let mut buf = LogBuffer::new(true, None, CompressMode::Zlib, 6);
        assert!(!buf.write(&mut tight, b"x"));
    }

    #[test]
    fn attach_recovers_length() {
        // A hand-crafted region: valid magic + a payload length.
        let mut region = vec![0u8; REGION_LEN];
        region[0] = magic::ASYNC_ZLIB_START;
        LogCrypt::update_log_len(&mut region, 100);

        let mut buf = LogBuffer::new(false, None, CompressMode::Zlib, 6);
        buf.attach(&mut region);
        assert_eq!(buf.len(), HEADER_LEN + 100);

        // An oversized length is clamped so header + payload + tailer fit.
        le::write_u32(&mut region, 5, u32::MAX);
        buf.attach(&mut region);
        assert_eq!(buf.len(), REGION_LEN - TAILER_LEN);

        // An unknown magic resets the buffer.
        region[0] = 0xAB;
        buf.attach(&mut region);
        assert_eq!(buf.len(), 0);

        // Too short to hold a header at all.
        let mut tiny = vec![0u8; HEADER_LEN - 1];
        buf.attach(&mut tiny);
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn attach_then_flush_recovers_a_leftover_region() {
        let mut region = vec![0u8; REGION_LEN];
        {
            let mut buf = LogBuffer::new(false, None, CompressMode::Zlib, 6);
            assert!(buf.write(&mut region, b"leftover\n"));
            assert_eq!(buf.len(), HEADER_LEN + 9);
        }

        // Simulate a fresh process mapping the same cache file.
        let mut buf = LogBuffer::new(false, None, CompressMode::Zlib, 6);
        buf.attach(&mut region);
        assert_eq!(buf.len(), HEADER_LEN + 9);

        let mut out = AutoBuffer::new();
        let n = buf.flush(&mut region, &mut out);
        assert_eq!(n, HEADER_LEN + 9 + TAILER_LEN);
        assert_eq!(&out.as_slice()[HEADER_LEN..HEADER_LEN + 9], b"leftover\n");
    }

    #[test]
    fn write_sync_emits_a_complete_record() {
        let mut buf = LogBuffer::new(false, None, CompressMode::Zstd, 3);
        let mut out = AutoBuffer::new();

        assert!(buf.write_sync(b"sync body", &mut out));
        assert!(!buf.write_sync(b"", &mut out));

        let data = out.as_slice().to_vec();
        assert_eq!(data.len(), HEADER_LEN + 9 + TAILER_LEN);
        assert_eq!(data[0], magic::SYNC_NOCRYPT_ZSTD_START);
        assert_eq!(LogCrypt::get_log_len(&data), 9);
        assert_eq!(&data[HEADER_LEN..HEADER_LEN + 9], b"sync body");
        assert_eq!(data[data.len() - 1], magic::END);
        assert_eq!(
            LogCrypt::get_log_hour(&data),
            Some((local_hour(), local_hour()))
        );
    }

    /// Inflates a *raw* DEFLATE stream produced with `Z_SYNC_FLUSH` markers and
    /// no final block.
    fn inflate_raw(body: &[u8], capacity: usize) -> Vec<u8> {
        let mut decompressor = flate2::Decompress::new(false);
        let mut out = vec![0u8; capacity];
        let before = decompressor.total_out();
        decompressor
            .decompress(body, &mut out, flate2::FlushDecompress::None)
            .expect("inflate");
        let n = (decompressor.total_out() - before) as usize;
        out.truncate(n);
        out
    }

    /// Decodes a zstd stream that was flushed but never ended.
    fn zstd_decode(body: &[u8], capacity: usize) -> Vec<u8> {
        use zstd::stream::raw::Operation;

        let mut decoder = zstd::stream::raw::Decoder::new().expect("decoder");
        let mut out = vec![0u8; capacity];
        let mut read = 0usize;
        let mut written = 0usize;

        while read < body.len() {
            let status = decoder
                .run_on_buffers(&body[read..], &mut out[written..])
                .expect("zstd decode");
            read += status.bytes_read;
            written += status.bytes_written;
            if status.bytes_read == 0 && status.bytes_written == 0 {
                break;
            }
        }

        out.truncate(written);
        out
    }

    /// Writes every record into `buf`/`region` and returns the concatenation.
    fn write_all(buf: &mut LogBuffer, region: &mut [u8], records: &[Vec<u8>]) -> Vec<u8> {
        let mut expected = Vec::new();
        for record in records {
            assert!(buf.write(region, record), "write must succeed");
            expected.extend_from_slice(record);
        }
        expected
    }

    fn sample_records() -> Vec<Vec<u8>> {
        (0..8)
            .map(|i| format!("[{i}] the quick brown fox jumps over the lazy dog\n").into_bytes())
            .collect()
    }

    #[test]
    fn write_many_records_stays_inside_the_region() {
        let mut region = vec![0u8; REGION_LEN];
        let mut buf = LogBuffer::new(true, None, CompressMode::Zlib, 6);
        let payload = b"0123456789abcdef".repeat(4);

        let mut written = 0usize;
        for _ in 0..64 {
            if !buf.write(&mut region, &payload) {
                break;
            }
            written += 1;
            assert!(buf.len() + TAILER_LEN <= region.len());
        }
        assert!(
            written > 1,
            "the compressor must accept more than one record"
        );
        assert_eq!(
            LogCrypt::get_log_len(&region),
            (buf.len() - HEADER_LEN) as u32
        );

        let mut out = AutoBuffer::new();
        let n = buf.flush(&mut region, &mut out);
        assert_eq!(n, out.len());
        assert!(n <= REGION_LEN);
        assert!(n > HEADER_LEN + TAILER_LEN);
    }

    #[test]
    fn zlib_compressed_records_round_trip() {
        let mut region = vec![0u8; REGION_LEN];
        let mut buf = LogBuffer::new(true, None, CompressMode::Zlib, 6);
        let records = sample_records();
        let expected = write_all(&mut buf, &mut region, &records);

        let mut out = AutoBuffer::new();
        let n = buf.flush(&mut region, &mut out);
        let data = out.as_slice().to_vec();

        assert_eq!(data[0], magic::ASYNC_NOCRYPT_ZLIB_START);
        let body = &data[HEADER_LEN..n - TAILER_LEN];
        assert_eq!(LogCrypt::get_log_len(&data) as usize, body.len());
        assert_eq!(inflate_raw(body, 1 << 16), expected);
    }

    #[test]
    fn zstd_compressed_records_round_trip() {
        let mut region = vec![0u8; REGION_LEN];
        let mut buf = LogBuffer::new(true, None, CompressMode::Zstd, 3);
        let records = sample_records();
        let expected = write_all(&mut buf, &mut region, &records);

        let mut out = AutoBuffer::new();
        let n = buf.flush(&mut region, &mut out);
        let data = out.as_slice().to_vec();

        assert_eq!(data[0], magic::ASYNC_NOCRYPT_ZSTD_START);
        let body = &data[HEADER_LEN..n - TAILER_LEN];
        assert_eq!(&body[..4], &[0x28, 0xB5, 0x2F, 0xFD], "zstd frame magic");
        assert_eq!(LogCrypt::get_log_len(&data) as usize, body.len());
        assert_eq!(zstd_decode(body, 1 << 16), expected);
    }

    #[test]
    fn flush_starts_a_fresh_stream_for_the_next_record() {
        let mut region = vec![0u8; REGION_LEN];
        let mut buf = LogBuffer::new(true, None, CompressMode::Zstd, 3);

        for round in 0..2 {
            let payload = format!("round {round}\n").into_bytes();
            assert!(buf.write(&mut region, &payload));

            let mut out = AutoBuffer::new();
            let n = buf.flush(&mut region, &mut out);
            let data = out.as_slice().to_vec();
            let body = &data[HEADER_LEN..n - TAILER_LEN];
            assert_eq!(zstd_decode(body, 1 << 10), payload);
            assert_eq!(LogCrypt::get_log_len(&data) as usize, body.len());
            assert_eq!(data[n - 1], magic::END);
            assert_eq!(buf.len(), 0);
        }
    }

    #[test]
    fn crypt_path_accounts_for_the_unencrypted_remainder() {
        let mut region = vec![0u8; REGION_LEN];
        let mut buf = LogBuffer::new(false, Some(TEST_PUBKEY), CompressMode::Zlib, 6);
        assert!(buf.is_crypt(), "a valid pubkey must enable TEA");

        // Chunk sizes that are not a multiple of TEA_BLOCK_LEN force the
        // rewind-and-recrypt path in `write()`.
        for chunk in [&b"AAAAA"[..], &b"BBBBB"[..], &b"CCCCC"[..]] {
            assert!(buf.write(&mut region, chunk));
        }

        // Every plaintext byte is accounted for exactly once.
        assert_eq!(buf.len(), HEADER_LEN + 15);
        assert_eq!(LogCrypt::get_log_len(&region), 15);
        assert_ne!(
            &region[HEADER_LEN..HEADER_LEN + 15],
            &b"AAAAABBBBBCCCCC"[..],
            "the payload must be encrypted"
        );
        // Only 15 % 8 == 7 trailing bytes are left in the clear.
        assert_eq!(&region[HEADER_LEN + 8..HEADER_LEN + 15], b"BBCCCCC");

        let mut out = AutoBuffer::new();
        let n = buf.flush(&mut region, &mut out);
        assert_eq!(n, HEADER_LEN + 15 + TAILER_LEN);
        assert_eq!(LogCrypt::get_log_len(out.as_slice()), 15);
    }

    #[test]
    fn crypt_path_encrypts_whole_blocks() {
        let mut region = vec![0u8; REGION_LEN];
        let mut buf = LogBuffer::new(false, Some(TEST_PUBKEY), CompressMode::Zlib, 6);
        assert!(buf.write(&mut region, b"12345678"));

        assert_eq!(buf.len(), HEADER_LEN + 8);
        assert_eq!(LogCrypt::get_log_len(&region), 8);
        assert_ne!(&region[HEADER_LEN..HEADER_LEN + 8], &b"12345678"[..]);
    }

    /// The C++ encrypts into a second buffer and copies the result back over
    /// the span it came from. [`LogCrypt::crypt_async_log_in_place`] encrypts
    /// the span itself, so this is the test that the two are the same bytes —
    /// including the remainder that the next chunk has to rewind over.
    #[test]
    fn encrypting_in_place_is_byte_identical_to_the_copy_path() {
        let crypt = LogCrypt::new(Some(TEST_PUBKEY));
        assert!(crypt.is_crypt());

        for len in [0usize, 1, 7, 8, 9, 16, 23] {
            let data: Vec<u8> = (0..len)
                .map(|i| (i as u8).wrapping_mul(37).wrapping_add(1))
                .collect();

            let mut copied = Vec::new();
            let mut remain_copy = 0;
            crypt.crypt_async_log(&data, &mut copied, &mut remain_copy);

            let mut in_place = data.clone();
            let remain_in_place = crypt.crypt_async_log_in_place(&mut in_place);

            assert_eq!(copied, in_place, "len {len}");
            assert_eq!(remain_copy, remain_in_place, "len {len}");
            // The trailing bytes never survive a round trip as ciphertext:
            // they are the ones the next write re-encrypts.
            assert_eq!(in_place.len() - remain_in_place, len - len % 8);
            if remain_in_place > 0 {
                assert_eq!(
                    &in_place[len - remain_in_place..],
                    &data[len - remain_in_place..]
                );
            }
        }

        // No TEA key: the payload is the payload, and nothing is left over.
        let plain = LogCrypt::new(None);
        let mut in_place = b"unencrypted".to_vec();
        assert_eq!(plain.crypt_async_log_in_place(&mut in_place), 0);
        assert_eq!(in_place, b"unencrypted");
    }

    #[test]
    fn an_unusable_pubkey_leaves_the_payload_in_the_clear() {
        for pubkey in [None, Some(""), Some("not-hex"), Some(&TEST_PUBKEY[..127])] {
            let buf = LogBuffer::new(false, pubkey, CompressMode::Zlib, 6);
            assert!(!buf.is_crypt(), "{pubkey:?} must not enable TEA");
        }
    }

    #[test]
    fn flush_stamps_the_end_hour() {
        use mars_crypt::magic;

        // A record opened "long ago" so that begin hour != end hour: flush()
        // has to overwrite the end hour with the current one, which is what
        // `LogCrypt::update_log_hour` does in the C++ and nothing else does.
        let mut region = vec![0u8; 4096];
        let mut buffer = LogBuffer::new(true, None, CompressMode::Zlib, 6);
        buffer.write(&mut region, b"hour test");
        assert!(buffer.len() > HEADER_LEN);

        let begin_hour = region[3];
        region[4] = begin_hour.wrapping_add(3) & 0x0f;
        assert_ne!(region[3], region[4]);

        let mut out = AutoBuffer::new();
        buffer.flush(&mut region, &mut out);

        // The flushed copy carries the stamped end hour.
        let flushed = out.as_slice();
        assert_eq!(flushed[3], begin_hour, "begin hour must be preserved");
        assert_eq!(
            flushed[4],
            local_hour_now(),
            "flush() must stamp the current hour as the end hour"
        );
        assert_eq!(flushed[0], magic::ASYNC_NOCRYPT_ZLIB_START);
    }

    #[test]
    fn zstd_compress_level_reaches_the_encoder() {
        // Nothing else pins `XLogConfig::compress_level`: the byte comparison
        // accepts a different size (the C++ vendors zstd 1.4.4), so an
        // ignored level would ship unnoticed.
        fn encode(level: i32) -> usize {
            let mut region = vec![0u8; 64 * 1024];
            let mut buffer = LogBuffer::new(true, None, CompressMode::Zstd, level);
            let payload = std::iter::repeat_n(b'a', 8192).collect::<Vec<_>>();
            buffer.write(&mut region, &payload);
            let mut out = AutoBuffer::new();
            buffer.flush(&mut region, &mut out);
            out.len()
        }

        let fast = encode(1);
        let best = encode(19);
        assert!(
            fast != best,
            "compress level is not reaching the encoder: level 1 and 19 both produced {fast} bytes"
        );
    }

    /// The hour `LogCrypt::update_log_hour` writes, i.e. the port's
    /// `localtime()->tm_hour`, probed through the same call the test checks.
    fn local_hour_now() -> u8 {
        let mut probe = [0u8; HEADER_LEN];
        mars_crypt::LogCrypt::update_log_hour(&mut probe);
        probe[4] // off::END_HOUR
    }
}
