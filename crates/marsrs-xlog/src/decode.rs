//! Reading a `.xlog` back — the port of
//! `mars/xlog/crypt/decode_log_file_c_impl/decode_log_file.c`.
//!
//! The appender writes, this reads: the two halves of one on-disk format, and
//! the reason a caller of this crate can do more than produce a file nobody
//! has a reader for. Upstream ships the same pair — `decode_log_file.c` under
//! `mars/xlog/crypt/`, next to the appender — and its
//! `decode_mars_log_file.py` is the tool that reaches an operator's shell.
//!
//! A record on disk is `header + body + tailer` (see `marsrs-crypt`):
//!
//! * a **sync** record's body is the log text itself, verbatim. `marsrs-crypt`
//!   never encrypts it and never compresses it — the C++ has the TEA loop of
//!   `LogCrypt::CryptSyncLog` commented out — so it is copied out as it is.
//! * an **async** record's body is the compressed log text, and TEA-encrypted
//!   on top of that when the file was written with a public key. Undoing it
//!   takes the private key of the pair: the header of every record carries the
//!   64-byte client public key the writer generated for that file, and the TEA
//!   key is the ECDH secret of it against the private key — the first 16 bytes
//!   of it, the way `decode_log_file.c` takes them.
//!
//! Encryption is a property of the file and not of a command, so the private
//! key is optional here: an unencrypted file decodes with `None`, and an
//! encrypted record met without one is an error that names the record rather
//! than a silent skip.
//!
//! ```no_run
//! use std::path::Path;
//!
//! // a file written with no public key: no private key is asked for
//! let plain = marsrs_xlog::decode_log_file(Path::new("marsrs_20260927.xlog"), None)?;
//! # Ok::<(), marsrs_xlog::DecodeError>(())
//! ```

use std::fs;
use std::path::Path;

use marsrs_crypt::{magic, CLIENT_PUBKEY_LEN, HEADER_LEN, TAILER_LEN, TEA_BLOCK_LEN};

/// `LogCrypt::CryptSyncLog` delta, and the round count both halves loop over.
const TEA_ROUNDS: u32 = 16;
const TEA_DELTA: u32 = 0x9e37_79b9;

/// Reads `path` and returns the log text of every record in it.
///
/// `privkey` is the private key of the pair whose public key the writer was
/// configured with — `None` for a file that was written with none.
///
/// A record that cannot be read is [`DecodeError`], and the text of the records
/// before it comes back with the error.
pub fn decode_log_file(path: &Path, privkey: Option<&[u8; 32]>) -> Result<Vec<u8>, DecodeError> {
    let bytes = fs::read(path).map_err(|e| DecodeError {
        recovered: Vec::new(),
        reason: format!("read {}: {e}", path.display()),
    })?;
    decode_records(&bytes, privkey)
}

/// Why [`decode_records`] stopped, and what it had read by then.
///
/// The C decoder answers the same question with both halves: `decodeBuffer`
/// appends its `[F]decode_log_file.py decode error len=N` marker to the output
/// it has already produced and carries on, and `parseFile` writes that output to
/// the `.log` file whether or not the walk ended in an error. A file whose end
/// is missing — a process killed between two writes, a block copied out by
/// halves — therefore still yields every record before the damage.
///
/// Returning the reason alone is what this port used to do, and it cost the
/// whole file for the sake of one record: the operator was told
/// `record at 1096 is truncated` and got none of the days of log that were
/// sitting intact in front of it.
#[derive(Debug)]
pub struct DecodeError {
    /// The log text of the records that decoded before the damage. Empty when
    /// the very first record is the one that is broken.
    pub recovered: Vec<u8>,
    /// What stopped the walk, named by the offset of the record it stopped at.
    pub reason: String,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for DecodeError {}

/// Walks every record of `data` and concatenates the recovered log text, the
/// way `decode_log_file.c` does over a buffer of its own.
///
/// A record whose magic says it is encrypted is an error when `privkey` is
/// `None`, and so is a record that is malformed; the walk stops at the first of
/// them, and [`DecodeError::recovered`] is what it had decoded by then.
pub fn decode_records(data: &[u8], privkey: Option<&[u8; 32]>) -> Result<Vec<u8>, DecodeError> {
    let mut plain = Vec::new();
    let mut offset = 0;
    let mut blocks = 0;

    // `Some` is the record that could not be read, `None` the end of the input.
    // The text decoded before either is the point: it is kept, and handed back
    // with the error, rather than dropped with it.
    let stopped = loop {
        if data.len() - offset < HEADER_LEN + TAILER_LEN {
            break None;
        }
        let magic_start = data[offset];
        if !magic::magic_start_is_valid(magic_start) {
            break Some(format!("bad magic 0x{magic_start:02x} at {offset}"));
        }
        let len = u32::from_le_bytes(data[offset + 5..offset + 9].try_into().expect("slice of 4"))
            as usize;
        let body_start = offset + HEADER_LEN;
        // The length is read out of the file, so it is compared against what is
        // left of the input and never added to an offset: a record declaring
        // `u32::MAX` wraps that sum on a 32-bit target, which would panic on the
        // slices below instead of being reported as a truncated record.
        if len > data.len() - body_start - TAILER_LEN {
            break Some(format!("record at {offset} is truncated"));
        }
        let body_end = body_start + len;
        if data[body_end] != magic::END {
            break Some(format!("bad tailer 0x{:02x} at {body_end}", data[body_end]));
        }

        let body = &data[body_start..body_end];
        let text = match magic_start {
            // `LogCrypt::CryptSyncLog` stores sync records verbatim: no TEA,
            // no compression (the C++ has the TEA loop commented out).
            magic::SYNC_ZLIB_START
            | magic::SYNC_NOCRYPT_ZLIB_START
            | magic::SYNC_ZSTD_START
            | magic::SYNC_NOCRYPT_ZSTD_START => Ok(body.to_vec()),
            magic::ASYNC_ZLIB_START | magic::ASYNC_ZSTD_START => {
                let Some(privkey) = privkey else {
                    break Some(format!(
                        "record at {offset} is encrypted, and no private key was given"
                    ));
                };
                let mut client_pubkey = [0u8; CLIENT_PUBKEY_LEN];
                client_pubkey.copy_from_slice(&data[body_start - CLIENT_PUBKEY_LEN..body_start]);
                let tea_key = match tea_key(privkey, &client_pubkey) {
                    Ok(key) => key,
                    Err(reason) => break Some(reason),
                };
                inflate(magic_start, &tea_decrypt_all(body, &tea_key))
            }
            magic::ASYNC_NOCRYPT_ZLIB_START | magic::ASYNC_NOCRYPT_ZSTD_START => {
                inflate(magic_start, body)
            }
            other => break Some(format!("unhandled magic 0x{other:02x}")),
        };
        match text {
            Ok(text) => plain.extend_from_slice(&text),
            Err(reason) => break Some(reason),
        }

        offset = body_end + TAILER_LEN;
        blocks += 1;
    };

    match stopped {
        // Every record decoded — or a tail too short to hold one, which is not
        // damage: a block the writer never finished is not in the file.
        None if blocks > 0 => Ok(plain),
        None => Err(DecodeError {
            recovered: plain,
            reason: "no record found".into(),
        }),
        Some(reason) => Err(DecodeError {
            recovered: plain,
            reason,
        }),
    }
}

/// Raw DEFLATE (`inflateInit2(-MAX_WBITS)` + `Z_SYNC_FLUSH`) or zstd, matching
/// `zlibDecompress` / `zstdDecompress` in `decode_log_file.c`.
fn inflate(magic_start: u8, body: &[u8]) -> Result<Vec<u8>, String> {
    if matches!(
        magic_start,
        magic::ASYNC_ZLIB_START | magic::ASYNC_NOCRYPT_ZLIB_START
    ) {
        return inflate_raw(body);
    }
    Ok(inflate_zstd(body))
}

/// `zstdDecompress` — `ZSTD_decompressStream` in a loop, tolerating a frame
/// that was never terminated.
///
/// `LogZstdBuffer::Flush` ends the stream with `ZSTD_compressStream2(...,
/// ZSTD_e_end)` against a *zero-sized* output buffer, so the frame epilogue is
/// never written and the decompressor keeps the tail of the last block back.
/// `decode_log_file.c` accepts that and returns what it got; so does this.
fn inflate_zstd(body: &[u8]) -> Vec<u8> {
    use std::io::Read;

    let mut out = Vec::new();
    let Ok(mut decoder) = zstd::stream::read::Decoder::new(body) else {
        return out;
    };
    let mut chunk = vec![0u8; 8192];
    loop {
        match decoder.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            Err(_) => break, // truncated frame: keep what was recovered
        }
    }
    out
}

/// `zlibDecompress` — raw inflate of a stream that was never terminated
/// (`deflate(..., Z_SYNC_FLUSH)` + `deflateEnd`), so `Z_STREAM_END` is never
/// reached and the loop has to stop on "input consumed".
fn inflate_raw(body: &[u8]) -> Result<Vec<u8>, String> {
    use flate2::{Decompress, FlushDecompress, Status};

    let mut decoder = Decompress::new(false);
    let mut output = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut consumed = 0;

    loop {
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = decoder
            .decompress(&body[consumed..], &mut chunk, FlushDecompress::Sync)
            .map_err(|e| format!("inflate: {e}"))?;
        let produced = (decoder.total_out() - before_out) as usize;
        let advanced = (decoder.total_in() - before_in) as usize;
        output.extend_from_slice(&chunk[..produced]);
        consumed += advanced;

        // Keep calling after the input is exhausted: miniz_oxide holds the
        // rest of the output back when the chunk filled up.
        if (advanced == 0 && produced == 0) || matches!(status, Status::StreamEnd) {
            break;
        }
    }
    Ok(output)
}

/// TEA-decrypts the leading whole blocks; the trailing `len % 8` bytes were
/// never encrypted (`LogCrypt::CryptAsyncLog`).
fn tea_decrypt_all(body: &[u8], key: &[u32; 4]) -> Vec<u8> {
    let mut out = body.to_vec();
    for start in (0..out.len())
        .step_by(TEA_BLOCK_LEN)
        .take(out.len() / TEA_BLOCK_LEN)
    {
        let mut block = [
            u32::from_le_bytes(out[start..start + 4].try_into().expect("slice of 4")),
            u32::from_le_bytes(out[start + 4..start + 8].try_into().expect("slice of 4")),
        ];
        for (index, word) in tea_decrypt(&mut block, key).iter().enumerate() {
            out[start + index * 4..start + index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
    }
    out
}

/// Inverse of `__TeaEncrypt` / `teaDecrypt`.
fn tea_decrypt(block: &mut [u32; 2], key: &[u32; 4]) -> [u32; 2] {
    let (mut low, mut high) = (block[0], block[1]);
    let (k0, k1, k2, k3) = (key[0], key[1], key[2], key[3]);
    let mut sum = TEA_DELTA.wrapping_mul(TEA_ROUNDS);
    for _ in 0..TEA_ROUNDS {
        high = high.wrapping_sub(
            (low.wrapping_shl(4).wrapping_add(k2))
                ^ (low.wrapping_add(sum))
                ^ (low.wrapping_shr(5).wrapping_add(k3)),
        );
        low = low.wrapping_sub(
            (high.wrapping_shl(4).wrapping_add(k0))
                ^ (high.wrapping_add(sum))
                ^ (high.wrapping_shr(5).wrapping_add(k1)),
        );
        sum = sum.wrapping_sub(TEA_DELTA);
    }
    [low, high]
}

/// `uECC_shared_secret(client_pub, svr_pri, ecdh_key)` — the decoder side of
/// the ECDH: the *server* private key against the client public key carried in
/// the record header. The first 16 bytes of the secret are the TEA key.
fn tea_key(
    privkey: &[u8; 32],
    client_pubkey: &[u8; CLIENT_PUBKEY_LEN],
) -> Result<[u32; 4], String> {
    use k256::elliptic_curve::ecdh::diffie_hellman;

    let secret = k256::SecretKey::from_slice(privkey).map_err(|e| format!("private key: {e}"))?;

    // uECC stores points as X||Y; SEC1 wants the 0x04 tag in front.
    let mut sec1 = [0u8; 1 + CLIENT_PUBKEY_LEN];
    sec1[0] = 0x04;
    sec1[1..].copy_from_slice(client_pubkey);
    let public = k256::PublicKey::from_sec1_bytes(&sec1).map_err(|e| format!("client key: {e}"))?;

    let shared = diffie_hellman(secret.to_nonzero_scalar(), public.as_affine());
    let raw = shared.raw_secret_bytes();

    let mut key = [0u32; 4];
    for (index, word) in key.iter_mut().enumerate() {
        let mut le = [0u8; 4];
        le.copy_from_slice(&raw[index * 4..index * 4 + 4]);
        *word = u32::from_le_bytes(le);
    }
    Ok(key)
}
