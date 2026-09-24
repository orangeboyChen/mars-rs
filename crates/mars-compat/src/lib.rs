//! `xlog-compat` — the Rust half of the differential format test.
//!
//! The port can only be trusted if the bytes it produces are the bytes the
//! original C++ implementation read, and the other way round. That used to be
//! checked live by driving both implementations over the same inputs; with the
//! C++ gone the very same claim is checked against the golden `.xlog` files of
//! `fixtures/` (see `tests/golden.rs`), which the C++ encoders produced. This
//! binary is the CLI that generates and decodes them:
//!
//! ```text
//! xlog-compat encode --mode=zlib --compress=1 --sync=0 --pubkey=<hex> \
//!     --records=records.bin --out=a.xlog
//! xlog-compat decode --privkey=<hex> --in=a.xlog --out=a.plain
//! ```
//!
//! * `encode` runs the real [`LogBuffer`] (`log_base_buffer.cc` +
//!   `log_zlib_buffer.cc` / `log_zstd_buffer.cc`) over a region and flushes it,
//!   exactly like `XloggerAppender` does with its mmap'd cache file.
//! * `decode` is the reader side: it walks the `header + body + tailer`
//!   records, undoes TEA (ECDH between the *server* private key and the client
//!   public key stored in the header) and inflates/decompresses the payload.
//!   It mirrors `mars/xlog/crypt/decode_log_file_c_impl/decode_log_file.c`,
//!   which is the C++ side's own decoder.
//!
//! Nothing here is production code: it exists so the two implementations can
//! be diffed byte for byte in CI, and so the golden fixtures under
//! `fixtures/` can be replayed after the C++ side is gone.
//!
//! The `xlog-compat` binary is a thin CLI over [`encode`] and [`decode`].

use std::collections::HashMap;
use std::fs;

use mars_buffer::{CompressMode, LogBuffer};
use mars_core::AutoBuffer;
use mars_crypt::{magic, CLIENT_PUBKEY_LEN, HEADER_LEN, TAILER_LEN, TEA_BLOCK_LEN};

/// `kBufferBlockLength` in `mars/xlog/src/appender.cc` (150 KiB).
const DEFAULT_REGION: usize = 150 * 1024;
/// `ZSTD_c_compressionLevel` default of `XlogConfig` in the C++ appender.
const DEFAULT_LEVEL: i32 = 6;
/// `LogCrypt::CryptSyncLog` delta; only used to size the TEA loop.
const TEA_ROUNDS: u32 = 16;
const TEA_DELTA: u32 = 0x9e37_79b9;

/// Reads `--key=value` style options; see the CLI in `main.rs`.
pub type Opts = HashMap<String, String>;

fn required(opts: &Opts, key: &str) -> Result<String, String> {
    opts.get(key)
        .cloned()
        .ok_or_else(|| format!("missing --{key}"))
}

fn flag(opts: &Opts, key: &str, default: bool) -> Result<bool, String> {
    match opts.get(key).map(String::as_str) {
        None => Ok(default),
        Some("0") | Some("false") => Ok(false),
        Some("1") | Some("true") => Ok(true),
        Some(other) => Err(format!("--{key} must be 0 or 1, got `{other}`")),
    }
}

fn number<T: std::str::FromStr>(opts: &Opts, key: &str, default: T) -> Result<T, String> {
    match opts.get(key) {
        None => Ok(default),
        Some(value) => value
            .parse::<T>()
            .map_err(|_| format!("--{key} must be a number, got `{value}`")),
    }
}

/// Rewrites `data` into a canonical form so two encodings can be compared byte
/// for byte.
///
/// * both hours of every header: the begin hour is stamped when the record is
///   opened and the end hour when it is flushed, so a run (or a re-encode)
///   that crosses an hour boundary would report a spurious diff. The end hour
///   being written at all is covered by
///   `mars_buffer`'s `flush_stamps_the_end_hour` test instead.
/// * seq: `__GetSeq()` is a process-global counter in the C++ and a `static` in
///   the port, so the starting value depends on what the process did before.
///   Records are renumbered from 1, which still checks that they increase by
///   one in order.
/// * the 64-byte client public key slot, when `mask_pubkey`: `LogCrypt` leaves
///   it uninitialised when no server key is configured and copies it into the
///   header anyway, so the C++ writes whatever was on the heap.
pub fn normalize_for_compare(data: &[u8], mask_pubkey: bool) -> Vec<u8> {
    let mut out = data.to_vec();
    let mut offset = 0;
    let mut base: Option<u16> = None;
    while offset + HEADER_LEN + TAILER_LEN <= out.len() {
        let seq = u16::from_le_bytes(out[offset + 1..offset + 3].try_into().expect("slice of 2"));
        // Rebase the process-global starting value, but keep the deltas: a
        // duplicated, skipped or out-of-order sequence still shows up.
        let rebase = *base.get_or_insert(seq);
        out[offset + 1..offset + 3]
            .copy_from_slice(&seq.wrapping_sub(rebase).wrapping_add(1).to_le_bytes());
        out[offset + 3] = 0;
        out[offset + 4] = 0;
        if mask_pubkey {
            out[offset + 9..offset + HEADER_LEN].fill(0);
        }
        let length = u32::from_le_bytes(out[offset + 5..offset + 9].try_into().expect("slice of 4"))
            as usize;
        offset += HEADER_LEN + length + TAILER_LEN;
    }
    out
}

/// One record per line; a single trailing newline is not a record.
pub fn read_records(path: &str) -> Result<Vec<Vec<u8>>, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    let mut records: Vec<Vec<u8>> = bytes
        .split(|b| *b == b'\n')
        .map(|line| line.to_vec())
        .collect();
    if records.last().is_some_and(|last| last.is_empty()) {
        records.pop();
    }
    if records.is_empty() {
        return Err(format!("{path} holds no records"));
    }
    Ok(records)
}

/// `--pubkey=` (empty) means "no server key", i.e. the no-crypt magics.
fn pubkey(opts: &Opts) -> Option<&str> {
    opts.get("pubkey")
        .map(String::as_str)
        .filter(|k| !k.is_empty())
}

/// Writes `records` through the ported `LogBuffer` the way the C++ appender
/// does through `LogZlibBuffer`/`LogZstdBuffer` over its mmap'd cache file.
pub fn encode(opts: &Opts) -> Result<(), String> {
    let mode = match required(opts, "mode")?.as_str() {
        "zlib" => CompressMode::Zlib,
        "zstd" => CompressMode::Zstd,
        other => return Err(format!("--mode must be zlib or zstd, got `{other}`")),
    };
    let is_compress = flag(opts, "compress", true)?;
    let sync = flag(opts, "sync", false)?;
    let region_len = number(opts, "region", DEFAULT_REGION)?;
    let flush_every = number(opts, "flush-every", 0usize)?;
    let level = number(opts, "level", DEFAULT_LEVEL)?;
    let records = read_records(&required(opts, "records")?)?;
    let out_path = required(opts, "out")?;

    let mut region = vec![0u8; region_len];
    let mut buffer = LogBuffer::new(is_compress, pubkey(opts), mode, level);
    let mut bytes = Vec::new();

    if sync {
        // `__WriteFile` hands `Write(data, len, out_buff)` a fresh `AutoBuffer`
        // per record, and `LogCrypt::CryptSyncLog` overwrites it, so the blocks
        // are copied out one at a time on both sides.
        let mut block = AutoBuffer::new();
        for record in &records {
            if !buffer.write_sync(record, &mut block) {
                return Err("write_sync rejected a record".into());
            }
            bytes.extend_from_slice(block.as_slice());
        }
    } else {
        let mut buffered = 0;
        for (index, record) in records.iter().enumerate() {
            // Flush every `flush_every` records instead of testing
            // `index % flush_every`, which clippy now wants spelled as
            // `is_multiple_of()` (too new for the toolchains this builds on).
            if flush_every > 0 && buffered >= flush_every {
                let mut block = AutoBuffer::new();
                buffer.flush(&mut region, &mut block);
                bytes.extend_from_slice(block.as_slice());
                buffered = 0;
            }
            if !buffer.write(&mut region, record) {
                return Err(format!(
                    "record {index} ({} bytes) did not fit in the {} byte region",
                    record.len(),
                    region_len
                ));
            }
            buffered += 1;
        }
        let mut block = AutoBuffer::new();
        buffer.flush(&mut region, &mut block);
        bytes.extend_from_slice(block.as_slice());
    }

    fs::write(&out_path, &bytes).map_err(|e| format!("write {out_path}: {e}"))?;
    println!(
        "encode: {} records -> {} bytes ({} blocks)",
        records.len(),
        bytes.len(),
        count_blocks(&bytes)?
    );
    Ok(())
}

/// Counts `header + body + tailer` records; fails on a malformed file.
pub fn count_blocks(bytes: &[u8]) -> Result<usize, String> {
    let mut count = 0;
    let mut offset = 0;
    while offset + HEADER_LEN + TAILER_LEN <= bytes.len() {
        if !magic::magic_start_is_valid(bytes[offset]) {
            return Err(format!("bad magic 0x{:02x} at {offset}", bytes[offset]));
        }
        let len = u32::from_le_bytes(
            bytes[offset + 5..offset + 9]
                .try_into()
                .expect("slice of 4"),
        ) as usize;
        let end = offset + HEADER_LEN + len + TAILER_LEN;
        if end > bytes.len() {
            return Err(format!("record at {offset} is truncated"));
        }
        offset = end;
        count += 1;
    }
    if offset != bytes.len() {
        return Err(format!("{offset} != {} (trailing garbage)", bytes.len()));
    }
    Ok(count)
}

/// Reads a file produced by either implementation and returns the plain log
/// text, mirroring `decode_log_file.c`.
pub fn decode(opts: &Opts) -> Result<(), String> {
    let privkey_hex = required(opts, "privkey")?;
    let input = required(opts, "in")?;
    let out_path = required(opts, "out")?;

    let privkey = hex_to_bytes(&privkey_hex)
        .and_then(|raw| <[u8; 32]>::try_from(raw).ok())
        .ok_or_else(|| format!("--privkey must be 64 hex chars, got `{privkey_hex}`"))?;

    let bytes = fs::read(&input).map_err(|e| format!("read {input}: {e}"))?;
    let plain = decode_records(&bytes, &privkey)?;
    fs::write(&out_path, &plain).map_err(|e| format!("write {out_path}: {e}"))?;
    println!("decode: {} bytes -> {} bytes", bytes.len(), plain.len());
    Ok(())
}

/// Walks every record in `data` and concatenates the recovered log text.
pub fn decode_records(data: &[u8], privkey: &[u8; 32]) -> Result<Vec<u8>, String> {
    let mut plain = Vec::new();
    let mut offset = 0;
    let mut blocks = 0;

    while offset + HEADER_LEN + TAILER_LEN <= data.len() {
        let magic_start = data[offset];
        if !magic::magic_start_is_valid(magic_start) {
            return Err(format!("bad magic 0x{magic_start:02x} at {offset}"));
        }
        let len = u32::from_le_bytes(data[offset + 5..offset + 9].try_into().expect("slice of 4"))
            as usize;
        let body_start = offset + HEADER_LEN;
        let body_end = body_start + len;
        if body_end + TAILER_LEN > data.len() {
            return Err(format!("record at {offset} is truncated"));
        }
        if data[body_end] != magic::END {
            return Err(format!("bad tailer 0x{:02x} at {body_end}", data[body_end]));
        }

        let body = &data[body_start..body_end];
        match magic_start {
            // `LogCrypt::CryptSyncLog` stores sync records verbatim: no TEA,
            // no compression (the C++ has the TEA loop commented out).
            magic::SYNC_ZLIB_START
            | magic::SYNC_NOCRYPT_ZLIB_START
            | magic::SYNC_ZSTD_START
            | magic::SYNC_NOCRYPT_ZSTD_START => plain.extend_from_slice(body),
            magic::ASYNC_ZLIB_START | magic::ASYNC_ZSTD_START => {
                let mut client_pubkey = [0u8; CLIENT_PUBKEY_LEN];
                client_pubkey.copy_from_slice(&data[body_start - CLIENT_PUBKEY_LEN..body_start]);
                let tea_key = tea_key(privkey, &client_pubkey)?;
                let decrypted = tea_decrypt_all(body, &tea_key);
                plain.extend_from_slice(&inflate(magic_start, &decrypted)?);
            }
            magic::ASYNC_NOCRYPT_ZLIB_START | magic::ASYNC_NOCRYPT_ZSTD_START => {
                plain.extend_from_slice(&inflate(magic_start, body)?);
            }
            other => return Err(format!("unhandled magic 0x{other:02x}")),
        }

        offset = body_end + TAILER_LEN;
        blocks += 1;
    }

    if blocks == 0 {
        return Err("no record found".into());
    }
    Ok(plain)
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
        let mut v = [
            u32::from_le_bytes(out[start..start + 4].try_into().expect("slice of 4")),
            u32::from_le_bytes(out[start + 4..start + 8].try_into().expect("slice of 4")),
        ];
        for (i, word) in tea_decrypt(&mut v, key).iter().enumerate() {
            out[start + i * 4..start + i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
    }
    out
}

/// Inverse of `__TeaEncrypt` / `teaDecrypt`.
fn tea_decrypt(v: &mut [u32; 2], k: &[u32; 4]) -> [u32; 2] {
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
    [v0, v1]
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
    for (i, word) in key.iter_mut().enumerate() {
        let mut le = [0u8; 4];
        le.copy_from_slice(&raw[i * 4..i * 4 + 4]);
        *word = u32::from_le_bytes(le);
    }
    Ok(key)
}

fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    let digits = hex.as_bytes();
    let mut out = Vec::with_capacity(digits.len() / 2);
    let mut i = 0;
    while i + 2 <= digits.len() {
        let pair = std::str::from_utf8(&digits[i..i + 2]).ok()?;
        out.push(u8::from_str_radix(pair, 16).ok()?);
        i += 2;
    }
    // An odd number of digits leaves one character unpaired.
    (i == digits.len()).then_some(out)
}
