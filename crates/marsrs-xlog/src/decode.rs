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

/// The round count both halves of `LogCrypt::CryptSyncLog` loop over.
const TEA_ROUNDS: u32 = 16;
const TEA_DELTA: u32 = 0x9e37_79b9;

/// `decodeBuffer`'s marker for a span of the file no record could be read out
/// of: `[F]decode_log_file.py decode error len=%d`. The C writes it into the
/// decoded text itself — and so does the Python decoder it was ported from —
/// so it is the string the tools built on either of them look for.
const DAMAGE_MARKER: &str = "[F]decode_log_file.py decode error len=";
/// `"Get ECDH key error"` — `decodeBuffer`'s stand-in for the text of a record
/// whose client public key is not a point a secret can be derived from.
const ECDH_MARKER: &str = "Get ECDH key error";
/// `"zstd decompress error"` — what `zstdDecompress` puts in the output in
/// place of a record the decompressor makes no progress on.
const ZSTD_MARKER: &str = "zstd decompress error";
/// `decodeBuffer`'s marker for a hole in the sequence numbers —
/// `[F]decode_log_file.py log seq:%d-%d is missing`, the two numbers being the
/// first and the last sequence the file lost. Written into the decoded text
/// like the rest of them, so that a tool that greps the text for the markers
/// finds the hole that no byte of the file mentions.
const MISSING_SEQ_MARKER: &str = "[F]decode_log_file.py log seq:";

/// Where `uint16_t seq` sits in a record's header: one byte of magic in front
/// of it. `decodeBuffer` reads it at `headerLen - cryptKeyLen - 4 - 2 - 2`,
/// which is this offset for both of the header lengths.
const SEQ_OFFSET: usize = 1;

/// Reads `path` and returns the log text of every record in it.
///
/// `privkey` is the private key of the pair whose public key the writer was
/// configured with — `None` for a file that was written with none.
///
/// A span of the file that holds no readable record — one byte gone wrong, a
/// block a process killed between two writes never finished — is skipped and
/// marked, and what stands behind it is decoded all the same. [`DecodeError`]
/// is what comes back when there is nothing behind the damage, and when the
/// file is encrypted and no private key was given; the text read before either
/// comes back with it.
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
/// Damage does not end the walk. A record that is not there is skipped — the
/// next byte a whole record starts at is where it goes on, and the span it
/// skipped is named in the text — and a record that is there but whose text
/// cannot be recovered leaves its marker in the text where that text would
/// have been, a key no secret can be derived with included. What ends it is a
/// file with no record in it at all, and an encrypted record met with no
/// private key at all: [`DecodeError::recovered`] is what was read before
/// either.
pub fn decode_records(data: &[u8], privkey: Option<&[u8; 32]>) -> Result<Vec<u8>, DecodeError> {
    let mut plain = Vec::new();
    let mut offset = 0;
    let mut blocks = 0;
    // `decode_log_file.c`'s file-static `int lastseq`, reset per file: a hole
    // is a hole in one file's own numbering, and not in the one read before it.
    let mut lastseq: u16 = 0;

    let stopped = loop {
        if data.len() - offset < HEADER_LEN + TAILER_LEN {
            break None;
        }
        match record_text(data, offset, privkey) {
            Ok((text, next)) => {
                mark_missing_seq(&mut plain, data, offset, &mut lastseq);
                plain.extend_from_slice(&text);
                offset = next;
                blocks += 1;
            }
            // `getLogStartPos(buffer + offset, …, 1)`, and the marker
            // `decodeBuffer` leaves for the span it skipped.
            Err(Failure::Damaged(reason)) => match next_record_start(data, offset) {
                Some(next) => {
                    let skipped = next - offset;
                    plain.extend_from_slice(format!("{DAMAGE_MARKER}{skipped}\n").as_bytes());
                    offset = next;
                }
                // Nothing past the damage is a record either, which is the one
                // case the C's `parseFile` cannot go on from: the reason is
                // what the caller is told.
                None => break Some(reason),
            },
            // The record is whole, so the walk goes on at the one behind it —
            // its marker is the text it would have carried. It counts as a
            // record found: a file of nothing but records like this one is a
            // file the decoder read, and not a file with no record in it.
            Err(Failure::Unreadable(marker, next)) => {
                // The C reads the sequence and writes its marker before it
                // tries the body, so a record whose text is not recoverable
                // still names the hole standing in front of it.
                mark_missing_seq(&mut plain, data, offset, &mut lastseq);
                plain.extend_from_slice(marker.as_bytes());
                plain.push(b'\n');
                offset = next;
                blocks += 1;
            }
            // The C reads the sequence and writes its marker before it tries
            // the body, so a record the walk ends on still names the hole
            // standing in front of it: what was read before the record that
            // ended it includes that marker.
            Err(Failure::Fatal(reason)) => {
                mark_missing_seq(&mut plain, data, offset, &mut lastseq);
                break Some(reason);
            }
        }
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

/// Why the record at an offset produced no text, and what the walk does about
/// it.
enum Failure {
    /// The bytes there are not a record — a magic that is not one, a length
    /// that runs past the file, a tailer that is not `MAGIC_END`.
    /// [`next_record_start`] decides where the log starts again, and the span
    /// it skipped is named in the text.
    Damaged(String),
    /// The record is whole but its text is not in it: a client public key that
    /// is not a point, a private key no secret comes out of, a stream that will
    /// not inflate. The marker stands where the text would have been, and the
    /// walk goes on at the record behind it.
    Unreadable(String, usize),
    /// Nothing behind this record can be read either — an encrypted record met
    /// with no private key at all, which is the one key the C cannot go on
    /// from — so the walk ends and this is its reason.
    Fatal(String),
}

/// The `seq` of the record at `offset`, against the one read before it: writes
/// the marker for the hole between the two, if there is one, in front of that
/// record's text, and moves `lastseq` on.
///
/// The record that went missing took its own bytes with it, so the sequence
/// numbers of the two records either side of the hole are the only trace it
/// leaves — and without this the file decodes to a text that reads as whole
/// while a block of it is gone. That is what the C writes the marker for, and
/// why it goes into the text rather than to stderr.
///
/// Two sequence numbers say nothing: `0`, which is what the writer leaves in
/// every sync record, and `1`, which is the first record of most files — the
/// walk starts at `lastseq = 0`, and `seq != 1` is what keeps a file opening
/// on 1 from being read as having lost everything before it.
fn mark_missing_seq(out: &mut Vec<u8>, data: &[u8], offset: usize, lastseq: &mut u16) {
    let seq = u16::from_le_bytes(
        data[offset + SEQ_OFFSET..offset + SEQ_OFFSET + 2]
            .try_into()
            .expect("slice of 2"),
    );
    let previous = *lastseq;
    if seq != 0 {
        *lastseq = seq;
    }
    // Widened, so that the record behind a sequence of `u16::MAX` is compared
    // against 65536 and not against a wrapped 0.
    if seq == 0 || seq == 1 || previous == 0 || u32::from(seq) == u32::from(previous) + 1 {
        return;
    }
    out.extend_from_slice(
        format!(
            "{MISSING_SEQ_MARKER}{}-{} is missing\n",
            u32::from(previous) + 1,
            u32::from(seq) - 1
        )
        .as_bytes(),
    );
}

/// The text of the record at `offset`, and the offset of the record behind it.
fn record_text(
    data: &[u8],
    offset: usize,
    privkey: Option<&[u8; 32]>,
) -> Result<(Vec<u8>, usize), Failure> {
    let magic_start = data[offset];
    if !magic::magic_start_is_valid(magic_start) {
        return Err(Failure::Damaged(format!(
            "bad magic 0x{magic_start:02x} at {offset}"
        )));
    }
    let len =
        u32::from_le_bytes(data[offset + 5..offset + 9].try_into().expect("slice of 4")) as usize;
    let body_start = offset + HEADER_LEN;
    // The length is read out of the file, so it is compared against what is
    // left of the input and never added to an offset: a record declaring
    // `u32::MAX` wraps that sum on a 32-bit target, which would panic on the
    // slices below instead of being reported as a truncated record.
    if len > data.len() - body_start - TAILER_LEN {
        return Err(Failure::Damaged(format!("record at {offset} is truncated")));
    }
    let body_end = body_start + len;
    if data[body_end] != magic::END {
        return Err(Failure::Damaged(format!(
            "bad tailer 0x{:02x} at {body_end}",
            data[body_end]
        )));
    }
    // Where the walk goes on, whatever the body turns out to hold.
    let next = body_end + TAILER_LEN;

    let body = &data[body_start..body_end];
    match magic_start {
        // `LogCrypt::CryptSyncLog` stores sync records verbatim: no TEA,
        // no compression (the C++ has the TEA loop commented out).
        magic::SYNC_ZLIB_START
        | magic::SYNC_NOCRYPT_ZLIB_START
        | magic::SYNC_ZSTD_START
        | magic::SYNC_NOCRYPT_ZSTD_START => Ok((body.to_vec(), next)),
        magic::ASYNC_ZLIB_START | magic::ASYNC_ZSTD_START => {
            let Some(privkey) = privkey else {
                return Err(Failure::Fatal(format!(
                    "record at {offset} is encrypted, and no private key was given"
                )));
            };
            let mut client_pubkey = [0u8; CLIENT_PUBKEY_LEN];
            client_pubkey.copy_from_slice(&data[body_start - CLIENT_PUBKEY_LEN..body_start]);
            let tea_key = match tea_key(privkey, &client_pubkey) {
                // `uECC_shared_secret` answering 0, which is what both a
                // damaged header and a private key no secret comes out of look
                // like from in here: `decodeBuffer` puts its marker where the
                // record's text would have been and goes on at the record
                // behind it. A key that is wrong for one record of a file is
                // wrong for every one of them, so what the walk answers is one
                // marker per record — which is what the C writes, and is more
                // than a walk that gives up at the first.
                Err(KeyError::Client(reason)) | Err(KeyError::Private(reason)) => {
                    return Err(Failure::Unreadable(
                        format!("{ECDH_MARKER} ({reason})"),
                        next,
                    ))
                }
                Ok(key) => key,
            };
            inflate(magic_start, &tea_decrypt_all(body, &tea_key))
                .map_err(|reason| Failure::Unreadable(reason, next))
                .map(|text| (text, next))
        }
        magic::ASYNC_NOCRYPT_ZLIB_START | magic::ASYNC_NOCRYPT_ZSTD_START => {
            inflate(magic_start, body)
                .map_err(|reason| Failure::Unreadable(reason, next))
                .map(|text| (text, next))
        }
        other => Err(Failure::Damaged(format!("unhandled magic 0x{other:02x}"))),
    }
}

/// `getLogStartPos(_buffer + _offset, …, 1)` — the first offset past `from` at
/// which a whole record starts, which is where the walk goes on after damage.
///
/// One byte at a time, the way the C does it: the framing carries no length of
/// its own to skip by, so a byte that looks like the start of a record is the
/// only hint there is.
fn next_record_start(data: &[u8], from: usize) -> Option<usize> {
    (from + 1..data.len()).find(|offset| record_is_whole(data, *offset))
}

/// `isGoodLogBuffer(…, 1)` — whether a whole record starts at `offset`.
///
/// All the scan has to go on: the bytes in front of a record are the payload of
/// the one before it and can be anything, so a span of damage is only over
/// where a record that checks out begins.
fn record_is_whole(data: &[u8], offset: usize) -> bool {
    if data.len() - offset < HEADER_LEN + TAILER_LEN {
        return false;
    }
    if !magic::magic_start_is_valid(data[offset]) {
        return false;
    }
    let len =
        u32::from_le_bytes(data[offset + 5..offset + 9].try_into().expect("slice of 4")) as usize;
    let body_start = offset + HEADER_LEN;
    len <= data.len() - body_start - TAILER_LEN && data[body_start + len] == magic::END
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
    inflate_zstd(body)
}

/// `zstdDecompress` — `ZSTD_decompressStream` in a loop, tolerating a frame
/// that was never terminated.
///
/// `LogZstdBuffer::Flush` ends the stream with `ZSTD_compressStream2(...,
/// ZSTD_e_end)` against a *zero-sized* output buffer, so the frame epilogue is
/// never written and the decompressor keeps the tail of the last block back.
/// `decode_log_file.c` accepts that and returns what it got; so does this, and
/// the same failure with nothing recovered yet is the one it answers with its
/// marker.
fn inflate_zstd(body: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;

    let mut out = Vec::new();
    let Ok(mut decoder) = zstd::stream::read::Decoder::new(body) else {
        return Err(ZSTD_MARKER.to_owned());
    };
    let mut chunk = vec![0u8; 8192];
    loop {
        match decoder.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            // The frame a flush never ended is expected to fail here, and what
            // it produced before failing is the record.
            Err(_) if !out.is_empty() => break,
            // `ZSTD_decompressStream` that consumed no input at all: the C
            // puts its marker in the output where the record's text went.
            Err(_) => return Err(ZSTD_MARKER.to_owned()),
        }
    }
    Ok(out)
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

/// Why the TEA key of a record could not be derived.
enum KeyError {
    /// The private key the decoder was handed is not a secp256k1 one — `k256`
    /// refuses a scalar of zero, or one past the curve's order, before a
    /// record is read. Upstream's `uECC_shared_secret` refuses it the same
    /// way it refuses a public key that is not a point, and per record: the
    /// answer is that record's marker, and not the end of the walk.
    Private(String),
    /// The client public key the record's header carries is not a point, which
    /// is what a damaged header looks like: `uECC_shared_secret` answers 0 for
    /// it, and the C puts its marker in the output and goes on.
    Client(String),
}

/// `uECC_shared_secret(client_pub, svr_pri, ecdh_key)` — the decoder side of
/// the ECDH: the *server* private key against the client public key carried in
/// the record header. The first 16 bytes of the secret are the TEA key.
fn tea_key(
    privkey: &[u8; 32],
    client_pubkey: &[u8; CLIENT_PUBKEY_LEN],
) -> Result<[u32; 4], KeyError> {
    use k256::elliptic_curve::ecdh::diffie_hellman;

    let secret = k256::SecretKey::from_slice(privkey)
        .map_err(|e| KeyError::Private(format!("private key: {e}")))?;

    // uECC stores points as X||Y; SEC1 wants the 0x04 tag in front.
    let mut sec1 = [0u8; 1 + CLIENT_PUBKEY_LEN];
    sec1[0] = 0x04;
    sec1[1..].copy_from_slice(client_pubkey);
    let public = k256::PublicKey::from_sec1_bytes(&sec1)
        .map_err(|e| KeyError::Client(format!("client key: {e}")))?;

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
