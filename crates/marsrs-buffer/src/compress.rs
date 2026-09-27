//! Port of the `Compress()` virtual method in `log_zlib_buffer.cc` and
//! `log_zstd_buffer.cc`.
//!
//! Both backends are *streaming*: the same context is reused for every
//! [`LogBuffer::write`](crate::LogBuffer::write) until the buffer is flushed,
//! and every write ends with a flush of the stream (never an "end"), so the
//! payload of a half-written region can still be recovered.

use crate::CompressMode;

/// `ZSTD_CCtx_setParameter(cctx_, ZSTD_c_windowLog, 16)`.
const ZSTD_WINDOW_LOG: u32 = 16;
/// `deflateInit2(..., -MAX_WBITS, ...)`: raw DEFLATE, 15-bit window.
const ZLIB_WINDOW_BITS: u8 = 15;

/// Upper bound on the zstd flush loop; a flush that cannot make progress stops
/// the loop anyway.
const ZSTD_FLUSH_ROUNDS: usize = 8;

/// The live compression stream for one [`LogBuffer`](crate::LogBuffer).
pub enum Compressor {
    /// Raw DEFLATE, `deflateInit2(..., Z_BEST_COMPRESSION, Z_DEFLATED,
    /// -MAX_WBITS, MAX_MEM_LEVEL, Z_DEFAULT_STRATEGY)`.
    Zlib(flate2::Compress),
    /// `ZSTD_createCCtx()` + `ZSTD_c_compressionLevel` + `ZSTD_c_windowLog`.
    Zstd(zstd::stream::raw::Encoder<'static>),
}

impl Compressor {
    /// Creates a fresh stream for `mode`. Returns `None` when the backend
    /// cannot be initialised (mirrors `deflateInit2` / `ZSTD_createCCtx`
    /// failing in the C++ `__Reset()`).
    pub fn new(mode: CompressMode, level: i32) -> Option<Self> {
        match mode {
            CompressMode::Zlib => {
                // `deflateInit2(..., Z_BEST_COMPRESSION, Z_DEFLATED, -MAX_WBITS,
                // MAX_MEM_LEVEL, Z_DEFAULT_STRATEGY)`: raw DEFLATE, no zlib
                // header, 15-bit window. flate2 only exposes the window bits
                // with an `any_zlib` backend, which is why the workspace
                // builds it on zlib-rs rather than miniz_oxide: miniz_oxide
                // emits different bytes for the same input, and the port has
                // to stay byte-compatible with the C++ on disk.
                Some(Self::Zlib(flate2::Compress::new_with_window_bits(
                    flate2::Compression::best(),
                    false,
                    ZLIB_WINDOW_BITS,
                )))
            }
            CompressMode::Zstd => {
                let mut encoder = zstd::stream::raw::Encoder::new(level).ok()?;
                encoder
                    .set_parameter(zstd::stream::raw::CParameter::WindowLog(ZSTD_WINDOW_LOG))
                    .ok()?;
                Some(Self::Zstd(encoder))
            }
        }
    }

    /// `Compress(src, inLen, dst, outLen)` — compresses `src` into `dst`,
    /// flushing the stream afterwards (`Z_SYNC_FLUSH` / `ZSTD_e_flush`), and
    /// returns the number of bytes written to `dst`.
    ///
    /// Returns `None` when the backend fails; the C++ signals that with
    /// `(size_t)-1`.
    ///
    /// Note that, exactly like the C++, input that does not fit in `dst` is
    /// dropped rather than buffered.
    pub fn compress(&mut self, src: &[u8], dst: &mut [u8]) -> Option<usize> {
        match self {
            Self::Zlib(stream) => {
                let before = stream.total_out();
                let status = stream
                    .compress(src, dst, flate2::FlushCompress::Sync)
                    .ok()?;
                match status {
                    flate2::Status::Ok | flate2::Status::StreamEnd => {
                        Some((stream.total_out() - before) as usize)
                    }
                    flate2::Status::BufError => None,
                }
            }
            Self::Zstd(stream) => zstd_compress(stream, src, dst),
        }
    }
}

/// `ZSTD_compressStream2(cctx_, &output, &input, ZSTD_e_flush)`.
///
/// The `zstd` crate splits that single call into a `run` (`ZSTD_e_continue`)
/// followed by a `flush` (`ZSTD_e_flush` with an empty input), which produces
/// the same bitstream.
fn zstd_compress(
    stream: &mut zstd::stream::raw::Encoder<'static>,
    src: &[u8],
    dst: &mut [u8],
) -> Option<usize> {
    use zstd::stream::raw::{InBuffer, Operation, OutBuffer};

    let mut input = InBuffer::around(src);
    let mut written = 0usize;

    while input.pos() < src.len() {
        let n = {
            let mut output = OutBuffer::around(&mut dst[written..]);
            stream.run(&mut input, &mut output).ok()?;
            output.pos()
        };
        written += n;
        if n == 0 {
            // No room left (or nothing more to emit): stop, like the C++ does
            // when `output.pos` stops growing.
            break;
        }
    }

    for _ in 0..ZSTD_FLUSH_ROUNDS {
        let (n, hint) = {
            let mut output = OutBuffer::around(&mut dst[written..]);
            let hint = stream.flush(&mut output).ok()?;
            (output.pos(), hint)
        };
        written += n;
        if hint == 0 || n == 0 {
            break;
        }
    }

    Some(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inflates a *raw* DEFLATE stream (no zlib header) that was produced with
    /// `Z_SYNC_FLUSH` markers and no final block.
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

    #[test]
    fn zlib_stream_is_raw_deflate_and_round_trips() {
        let mut compressor = Compressor::new(CompressMode::Zlib, 6).expect("zlib stream");
        let mut out = vec![0u8; 256];

        let n = compressor
            .compress(b"hello world", &mut out)
            .expect("compress");
        assert!(n > 0);
        // Raw deflate: no zlib header (0x78 0x9c / 0x78 0x01 / 0x78 0xda).
        assert_ne!(&out[..2], &[0x78, 0x9c]);
        assert_eq!(inflate_raw(&out[..n], 256), b"hello world");

        // The stream is reused: sync-flushed blocks concatenate.
        let before = n;
        let n = compressor
            .compress(b" and goodbye", &mut out[before..])
            .expect("compress");
        assert!(n > 0);
        let body = &out[..before + n];
        assert_eq!(inflate_raw(body, 256), b"hello world and goodbye");
    }

    #[test]
    fn zstd_stream_round_trips() {
        let mut compressor = Compressor::new(CompressMode::Zstd, 3).expect("zstd stream");
        let mut out = vec![0u8; 512];

        let n = compressor
            .compress(b"hello world", &mut out)
            .expect("compress");
        assert!(n >= 4);
        assert_eq!(&out[..4], &[0x28, 0xB5, 0x2F, 0xFD], "zstd frame magic");

        let before = n;
        let n = compressor
            .compress(b" and goodbye", &mut out[before..])
            .expect("compress");
        let body = &out[..before + n];
        assert_eq!(zstd_decode(body, 512), b"hello world and goodbye");
    }

    #[test]
    fn compress_into_a_zero_length_output_fails() {
        let mut compressor = Compressor::new(CompressMode::Zlib, 6).expect("zlib stream");
        let mut out = [0u8; 0];
        assert!(compressor.compress(b"payload", &mut out).is_none());

        let mut compressor = Compressor::new(CompressMode::Zstd, 3).expect("zstd stream");
        let mut out = [0u8; 0];
        // A zero-length destination simply produces no output for zstd.
        assert_eq!(compressor.compress(b"payload", &mut out), Some(0));
    }
}
