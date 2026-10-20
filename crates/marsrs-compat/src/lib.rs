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
//! xlog-compat decode [--privkey=<hex>] --in=a.xlog --out=a.plain
//! ```
//!
//! * `encode` runs the real [`LogBuffer`] (`log_base_buffer.cc` +
//!   `log_zlib_buffer.cc` / `log_zstd_buffer.cc`) over a region and flushes it,
//!   exactly like `XloggerAppender` does with its mmap'd cache file.
//! * `decode` is the reader side: it walks the `header + body + tailer`
//!   records, undoes TEA (ECDH between the *server* private key and the client
//!   public key stored in the header) and inflates/decompresses the payload.
//!   The reader itself is `marsrs-xlog`'s — the port of
//!   `mars/xlog/crypt/decode_log_file_c_impl/decode_log_file.c` lives there
//!   now, where the CLI reads through it too — so the golden files below are
//!   what proves the reader this workspace ships.
//!
//! Nothing here is production code: it exists so the two implementations can
//! be diffed byte for byte in CI, and so the golden fixtures under
//! `fixtures/` can be replayed after the C++ side is gone.
//!
//! The `xlog-compat` binary is a thin CLI over [`encode`] and [`decode`].

use std::collections::HashMap;
use std::fs;

use marsrs_buffer::{CompressMode, LogBuffer};
use marsrs_core::AutoBuffer;
use marsrs_crypt::{magic, HEADER_LEN, TAILER_LEN};

/// `kBufferBlockLength` in `mars/xlog/src/appender.cc` (150 KiB).
const DEFAULT_REGION: usize = 150 * 1024;
/// `ZSTD_c_compressionLevel` default of `XlogConfig` in the C++ appender.
const DEFAULT_LEVEL: i32 = 6;

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
///   `marsrs_buffer`'s `flush_stamps_the_end_hour` test instead.
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
            // A counter since the last flush, and not
            // `index.is_multiple_of(flush_every)`: what a block is made of
            // is what the region still holds, which a record that did not
            // fit in it does not add to.
            if flush_every > 0 && buffered >= flush_every {
                let mut block = AutoBuffer::new();
                buffer.flush(&mut region, &mut block);
                bytes.extend_from_slice(block.as_slice());
                // The block is out of the region now, so the next record opens
                // one of its own: `drained` is what the appender calls after
                // these bytes have reached the file.
                buffer.drained(&mut region);
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
        buffer.drained(&mut region);
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

/// Reads a file produced by either implementation and writes the plain log
/// text, mirroring `decode_log_file.c`.
///
/// `parseFile` there writes the output it has whether or not the walk ended in
/// an error, so this does too: a file whose last record is missing its end is
/// still written out up to the damage, and the error is what comes back.
pub fn decode(opts: &Opts) -> Result<(), String> {
    let input = required(opts, "in")?;
    let out_path = required(opts, "out")?;

    // `--privkey=` (empty) means "no key", the way `--pubkey=` does: upstream
    // reads its `PRIV_KEY` only where a record is encrypted
    // (`decode_log_file.c:437`), so a file written with no key is read with
    // none. A key that is given and is not 64 hex digits is still an error,
    // because a wrong one costs every crypt record in the file.
    let given = opts
        .get("privkey")
        .map(String::as_str)
        .filter(|hex| !hex.is_empty());
    let privkey = match given {
        Some(hex) => Some(
            hex_to_bytes(hex)
                .and_then(|raw| <[u8; 32]>::try_from(raw).ok())
                .ok_or_else(|| format!("--privkey must be 64 hex chars, got `{hex}`"))?,
        ),
        None => None,
    };

    let bytes = fs::read(&input).map_err(|e| format!("read {input}: {e}"))?;
    match decode_records(&bytes, privkey.as_ref()) {
        Ok(plain) => {
            fs::write(&out_path, &plain).map_err(|e| format!("write {out_path}: {e}"))?;
            println!("decode: {} bytes -> {} bytes", bytes.len(), plain.len());
            Ok(())
        }
        Err(err) => {
            // What was recovered is the file's text as far as it goes, and it
            // is the only copy of the records before the damage.
            fs::write(&out_path, &err.recovered).map_err(|e| format!("write {out_path}: {e}"))?;
            Err(err.reason)
        }
    }
}

/// The reader every decoder of this workspace reads through:
/// `marsrs_xlog::decode_records`, the port of `decode_log_file.c`.
///
/// It takes the key the way `marsrs_xlog::decode_records` does — an `Option`,
/// because upstream's is one too: `PRIV_KEY` is read only where a record is
/// encrypted, so a file written with no key is read with `None`, and a file
/// that is encrypted is what `None` cannot read past — where upstream's
/// `exit(7)` costs the whole file, this costs the records behind that point
/// and keeps the ones in front of it.
///
/// A record that cannot be read is skipped and marked rather than ending the
/// walk, so a damaged file still yields the records behind the damage. What
/// does end it is a file with no record left in it at all, and
/// [`marsrs_xlog::DecodeError::recovered`] is then the text of the records
/// before that point.
pub fn decode_records(
    data: &[u8],
    privkey: Option<&[u8; 32]>,
) -> Result<Vec<u8>, marsrs_xlog::DecodeError> {
    marsrs_xlog::decode_records(data, privkey)
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
