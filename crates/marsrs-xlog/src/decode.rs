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
//! * a record written before the public key moved into the header — the five
//!   magics `0x01` to `0x05`, which no appender writes any more — carries no
//!   key at all, so its text is XORed with a byte made of the magic and not
//!   TEA-encrypted, and its header is 5 or 9 bytes instead of 73. These are
//!   read too: `LogMagicNum` in `mars/xlog/crypt/log_magic_num.h` knows the
//!   eight from `0x06` to `0x0D` and nothing before them, so what an appender
//!   writes and what `decode_log_file.c` reads stopped being one set when the
//!   public key arrived.
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

/// The round count `LogCrypt::CryptAsyncLog` loops the blocks of a body
/// through — the half of the cipher a sync record never gets: its body is
/// copied verbatim, so this is the count the async one is decrypted with
/// here and nothing else.
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

/// The most one record's text may inflate to.
///
/// A record's body is at most the block it was written into — the appender's
/// region, 150 KiB — so this is hundreds of times the largest record an
/// appender can put in a file, and nothing one wrote is cut short. What it
/// does cut short is the other thing a small body can ask for: a DEFLATE or
/// zstd stream built to expand without bound, which is a bomb and not a
/// record, and which `xlog decode` is handed by whoever pulled the file off a
/// device. `decode_log_file.c` grows its output by doubling and stops when the
/// decompressor stops, which is to say it never stops.
///
/// Per record, and not per file — which is why [`MAX_PLAIN_LEN`] is there: a
/// cap on one record is no cap on the sum of them, and a file of records that
/// each inflate to just under this is a bomb that walks straight through it.
const MAX_INFLATED_LEN: usize = 64 * 1024 * 1024;

/// The most one file's text may come to, however many records it holds.
///
/// Four times [`MAX_INFLATED_LEN`], which is not a number an appender's log
/// comes near: a block is 150 KiB, so this is the text of some 1 700 of them,
/// and a file that decodes to more is not a log one wrote. What it stops is a
/// file built to make the decoder work — a bomb per record is a bomb per file
/// too, and a file of damage is a marker per span, which is longer than the
/// span it names — and what the walk does about it is stop and say so, keeping
/// the text it read before the line was crossed: see
/// [`DecodeError::recovered`]. Every arm that writes asks, and not only the one
/// that writes a record's own text.
const MAX_PLAIN_LEN: usize = 4 * MAX_INFLATED_LEN;

/// `MAGIC_CRYPT_START` — the oldest record start `decode_log_file.c` reads, and
/// one no appender writes: its body is the log text, XORed and nothing more.
const MAGIC_CRYPT_START: u8 = 0x01;
/// `MAGIC_COMPRESS_CRYPT_START` — the same, deflated after the XOR.
const MAGIC_COMPRESS_CRYPT_START: u8 = 0x02;
/// `NEW_MAGIC_CRYPT_START` — the first header with a sequence and the two hours
/// in it, and still no key: XORed, and not deflated.
const NEW_MAGIC_CRYPT_START: u8 = 0x03;
/// `NEW_MAGIC_COMPRESS_CRYPT_START` — that header, deflated after the XOR.
const NEW_MAGIC_COMPRESS_CRYPT_START: u8 = 0x04;
/// `NEW_MAGIC_COMPRESS_CRYPT_START1` — the same, over a body of chunks: one
/// `uint16_t` length and its bytes per log line, put back to back before the
/// XOR and the deflate.
const NEW_MAGIC_COMPRESS_CRYPT_START1: u8 = 0x05;
/// `BASE_KEY` — what the key of a record with no client public key is made of:
/// `BASE_KEY ^ (0xff & length) ^ magic` for the two that carry no sequence, and
/// `BASE_KEY ^ (0xff & seq) ^ magic` for the three that do.
const BASE_KEY: u8 = 0xcc;
/// The shortest header `decode_log_file.c` reads — `1 + 4`, of the two oldest
/// magics — and so what a byte that is no magic at all costs the walk: such a
/// byte is not the end of the file but the start of a span to look behind.
const SHORTEST_HEADER_LEN: usize = 1 + 4;

/// Where `uint16_t seq` sits in a record's header: one byte of magic in front
/// of it. `decodeBuffer` reads it at `headerLen - cryptKeyLen - 4 - 2 - 2`,
/// which is this offset for every header that has a sequence in it — the two
/// oldest are `1 + 4` bytes long and hold none, which is why their key is made
/// of the length instead ([`xor_key`]).
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

/// [`decode_records`], and how many of the file's records it read and did not.
///
/// The text alone is what a caller that reads a log wants, and it is all
/// [`decode_records`] answers with. What it cannot say is whether the file
/// read at all: a `--privkey` of the wrong pair leaves every record
/// unreadable, and a text of nothing but markers is not one an operator can
/// tell from a log that said so — the command answered well and read no log
/// at all. [`decode_records_counted`] is the walk that says both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// The log text of the records that came out, in the order they stand in
    /// the file, with a marker where a record's text did not.
    pub text: Vec<u8>,
    /// How many records the walk found, the ones whose text did not come out
    /// included.
    pub records: usize,
    /// How many of [`Decoded::records`] whose text did not come out: a client
    /// public key that is not a point, a private key no secret comes out of, a
    /// stream that will not inflate. A span of damage is not a record, so it
    /// is not counted here.
    pub unreadable: usize,
}

impl Decoded {
    /// Whether the walk read a record and no text came out of any of them: a
    /// file that answers this is a file of markers and nothing else, which is
    /// what a key of another pair reads.
    ///
    /// A file with no record in it at all is not this — it is a
    /// [`DecodeError`], and the walk says so — so a file that answers here is
    /// one that was read and gave nothing back.
    pub fn nothing_came_out(&self) -> bool {
        self.records > 0 && self.unreadable == self.records
    }
}

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
///
/// [`decode_records_counted`] is this walk and the two counts beside the text:
/// take it when the caller has to say whether the file read at all.
pub fn decode_records(data: &[u8], privkey: Option<&[u8; 32]>) -> Result<Vec<u8>, DecodeError> {
    Ok(decode_records_counted(data, privkey)?.text)
}

/// [`decode_records`], and how many of the file's records the walk read and
/// did not read: see [`Decoded`].
pub fn decode_records_counted(
    data: &[u8],
    privkey: Option<&[u8; 32]>,
) -> Result<Decoded, DecodeError> {
    decode_records_within(data, privkey, MAX_PLAIN_LEN)
}

/// [`decode_records_counted`] with the ceiling the caller names instead of
/// [`MAX_PLAIN_LEN`]: the ceiling is 256 MiB, and a file that reaches it is not
/// one a test can build, so what a test asks about the ceiling it asks here.
fn decode_records_within(
    data: &[u8],
    privkey: Option<&[u8; 32]>,
    max_plain: usize,
) -> Result<Decoded, DecodeError> {
    let mut plain = Vec::new();
    let mut offset = 0;
    let mut blocks = 0;
    // How many of `blocks` the walk found and did not read: what a caller
    // asking whether the file read at all is asking about.
    let mut unreadable = 0;
    // `decode_log_file.c`'s file-static `int lastseq`, reset per file: a hole
    // is a hole in one file's own numbering, and not in the one read before it.
    let mut lastseq: u16 = 0;

    let stopped = loop {
        if !whole_record_fits(data, offset) {
            break None;
        }
        match record_text(data, offset, privkey) {
            Ok((text, next)) => {
                // [`MAX_PLAIN_LEN`]: a record's own text is bounded, and the
                // file's is too, or a file of records that each stop just
                // short of that bound would be one the decoder grows into
                // without end. The record that crossed the line is not
                // written, and the reason is what ends the walk.
                if !mark_missing_seq(&mut plain, data, offset, &mut lastseq, max_plain)
                    || !push_plain(&mut plain, &text, max_plain)
                {
                    break Some(too_long(max_plain));
                }
                offset = next;
                blocks += 1;
            }
            // `getLogStartPos(buffer + offset, …, 1)`, and the marker
            // `decodeBuffer` leaves for the span it skipped.
            //
            // The scan goes on from the damage and not from the top of the
            // file, so a byte of it is examined once however much damage it
            // holds: each scan starts where the record before it ended and
            // stops at the start it found, and the one that finds none is the
            // one that ends the walk — one pass over the file, and not one
            // pass per span of damage.
            Err(Failure::Damaged(reason)) => match next_record_start(data, offset) {
                Some(next) => {
                    let skipped = next - offset;
                    // A marker per span, and a span can be one byte: the cap
                    // is asked here too, or a file of junk is a file that
                    // grows the text several times over.
                    if !push_plain(
                        &mut plain,
                        format!("{DAMAGE_MARKER}{skipped}\n").as_bytes(),
                        max_plain,
                    ) {
                        break Some(too_long(max_plain));
                    }
                    offset = next;
                }
                // Nothing behind the damage is a record either, so the walk
                // ends here — which is the one case the C's `parseFile` cannot
                // go on from either, and the reason is what the caller is told.
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
                if !mark_missing_seq(&mut plain, data, offset, &mut lastseq, max_plain)
                    || !push_plain(&mut plain, marker.as_bytes(), max_plain)
                    || !push_plain(&mut plain, b"\n", max_plain)
                {
                    break Some(too_long(max_plain));
                }
                offset = next;
                blocks += 1;
                unreadable += 1;
            }
            // the body, so a record the walk ends on still names the hole
            // standing in front of it: what was read before the record that
            // ended it includes that marker.
            Err(Failure::Fatal(reason)) => {
                mark_missing_seq(&mut plain, data, offset, &mut lastseq, max_plain);
                break Some(reason);
            }
        }
    };

    match stopped {
        // Every record decoded — or a tail too short to hold one, which is not
        // damage: a block the writer never finished is not in the file.
        None if blocks > 0 => Ok(Decoded {
            text: plain,
            records: blocks,
            unreadable,
        }),
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

/// What ends the walk when [`MAX_PLAIN_LEN`] is crossed: the text read before
/// the line is kept, and this is the reason.
fn too_long(max_plain: usize) -> String {
    format!("more than {max_plain} bytes of text decoded")
}

/// Appends `bytes` to the text the walk has decoded, and answers `false`
/// without appending when [`MAX_PLAIN_LEN`] is what it would cross.
///
/// Every arm that writes asks, and not only the one that writes a record's own
/// text: a file of damage is mostly markers — one per span, and a span can be
/// a single byte — so the markers a hostile file asks for are more text than
/// the file itself holds, and a cap on records alone is no cap at all.
fn push_plain(plain: &mut Vec<u8>, bytes: &[u8], max_plain: usize) -> bool {
    if plain.len().saturating_add(bytes.len()) > max_plain {
        return false;
    }
    plain.extend_from_slice(bytes);
    true
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
///
/// `false` when [`MAX_PLAIN_LEN`] is what the marker would cross, which is the
/// one way this does not write: the caller ends the walk.
fn mark_missing_seq(
    out: &mut Vec<u8>,
    data: &[u8],
    offset: usize,
    lastseq: &mut u16,
    max_plain: usize,
) -> bool {
    // A record with no sequence in its header takes no part in the numbering:
    // the two oldest magics are the length and nothing else, so there is no
    // hole for them to name.
    let Some(seq) = seq_at(data, offset) else {
        return true;
    };
    let previous = *lastseq;
    if seq != 0 {
        *lastseq = seq;
    }
    // Widened, so that the record behind a sequence of `u16::MAX` is compared
    // against 65536 and not against a wrapped 0. `<=` and not `==`: a
    // sequence that went *backwards* — a file whose records are not in the
    // order they were written — is not a hole either, and `6-1 is missing`
    // names nothing at all.
    if seq == 0 || seq == 1 || previous == 0 || u32::from(seq) <= u32::from(previous) + 1 {
        return true;
    }
    push_plain(
        out,
        format!(
            "{MISSING_SEQ_MARKER}{}-{} is missing\n",
            u32::from(previous) + 1,
            u32::from(seq) - 1
        )
        .as_bytes(),
        max_plain,
    )
}

/// The text of the record at `offset`, and the offset of the record behind it.
fn record_text(
    data: &[u8],
    offset: usize,
    privkey: Option<&[u8; 32]>,
) -> Result<(Vec<u8>, usize), Failure> {
    let magic_start = data[offset];
    let Some(header) = header_of(magic_start) else {
        return Err(Failure::Damaged(format!(
            "bad magic 0x{magic_start:02x} at {offset}"
        )));
    };
    if data.len() - offset < header.len + TAILER_LEN {
        return Err(Failure::Damaged(format!("record at {offset} is truncated")));
    }
    // `headerLen - cryptKeyLen - 4`: the C reads the length backwards from the
    // end of the key, because the key is the last field of a header and not the
    // first — which is what makes one expression answer for all three shapes.
    let length = u32::from_le_bytes(
        data[offset + header.len - header.crypt_key_len - 4..][..4]
            .try_into()
            .expect("slice of 4"),
    );
    let len = length as usize;
    let seq = seq_at(data, offset);
    let body_start = offset + header.len;
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
        // The five no appender writes any more: no client public key in the
        // header, so there is no secret to agree and the text is XORed with a
        // byte of the magic ([`xor_key`]) instead of TEA-encrypted.
        MAGIC_CRYPT_START
        | MAGIC_COMPRESS_CRYPT_START
        | NEW_MAGIC_CRYPT_START
        | NEW_MAGIC_COMPRESS_CRYPT_START
        | NEW_MAGIC_COMPRESS_CRYPT_START1 => {
            // `NEW_MAGIC_COMPRESS_CRYPT_START1` files one chunk per log line,
            // and the chunks are put back to back before the key goes over
            // them.
            let chunks = if magic_start == NEW_MAGIC_COMPRESS_CRYPT_START1 {
                unchunk(body)
            } else {
                body.to_vec()
            };
            let text = xor(&chunks, xor_key(magic_start, length, seq));
            match magic_start {
                // The two that are not compressed end in `decodeBuffer`'s
                // `else`, which is the XOR and nothing behind it.
                MAGIC_CRYPT_START | NEW_MAGIC_CRYPT_START => Ok((text, next)),
                // The three that are: `zlibDecompress` after the XOR.
                _ => inflate_raw(&text)
                    .map_err(|reason| Failure::Unreadable(reason, next))
                    .map(|text| (text, next)),
            }
        }
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
///
/// `from` is where the *last* scan stopped and not where the damage is — see
/// [`decode_records`]. Everything before it has been looked at and rejected
/// already, so scanning it again is work the walk would repeat once per span of
/// damage, and a byte is worth one look and not one look per span.
fn next_record_start(data: &[u8], from: usize) -> Option<usize> {
    (from + 1..data.len()).find(|offset| record_is_whole(data, *offset))
}

/// `isGoodLogBuffer`'s three header shapes, the way `decode_log_file.c` writes
/// them out: `1 + 4` for the two oldest magics, `1 + 2 + 1 + 1 + 4` for the
/// three that carry a sequence and the two hours, and
/// `1 + 2 + 1 + 1 + 4 + 64` — [`HEADER_LEN`] — for the eight an appender
/// writes now. `None` for a byte no appender ever started a record with.
fn header_of(magic_start: u8) -> Option<Header> {
    match magic_start {
        MAGIC_CRYPT_START | MAGIC_COMPRESS_CRYPT_START => Some(Header {
            len: 1 + 4,
            crypt_key_len: 0,
        }),
        NEW_MAGIC_CRYPT_START
        | NEW_MAGIC_COMPRESS_CRYPT_START
        | NEW_MAGIC_COMPRESS_CRYPT_START1 => Some(Header {
            len: 1 + 2 + 1 + 1 + 4,
            crypt_key_len: 0,
        }),
        other if magic::magic_start_is_valid(other) => Some(Header {
            len: HEADER_LEN,
            crypt_key_len: CLIENT_PUBKEY_LEN,
        }),
        _ => None,
    }
}

/// One of [`header_of`]'s three: how long the header is, and how much of it the
/// client public key takes up.
struct Header {
    len: usize,
    crypt_key_len: usize,
}

impl Header {
    /// Whether the header has a sequence in it: nine bytes in front of the
    /// client public key, or in front of the length when there is no key. The
    /// two oldest magics are five bytes long and hold neither, which is why
    /// their key is made of the length ([`xor_key`]) and why no hole in the
    /// numbering can be read out of them.
    fn has_seq(&self) -> bool {
        self.len - self.crypt_key_len == 1 + 2 + 1 + 1 + 4
    }
}

/// Whether there is room at `offset` for the smallest whole record that could
/// start there — the tail of a file that holds none is not damage, but which
/// shape to measure against is the magic byte's to say: five bytes of header
/// for the oldest, 73 for the eight an appender writes now, and five again for
/// a byte that is no magic at all, because what follows such a byte may still
/// be a record of the shortest kind.
fn whole_record_fits(data: &[u8], offset: usize) -> bool {
    // A byte no appender started a record with gets the shortest header there
    // is and not [`HEADER_LEN`]: what the walk does with such a byte is look
    // for a record behind it, and a record written before the public key is
    // whole in six bytes, where one written with it needs seventy-four.
    let shortest = data
        .get(offset)
        .and_then(|magic_start| header_of(*magic_start))
        .map_or(SHORTEST_HEADER_LEN, |header| header.len);
    data.len() - offset >= shortest + TAILER_LEN
}

/// `uint16_t seq` of the record at `offset`, when its header carries one.
fn seq_at(data: &[u8], offset: usize) -> Option<u16> {
    let header = header_of(data[offset])?;
    header.has_seq().then(|| {
        u16::from_le_bytes(
            data[offset + SEQ_OFFSET..offset + SEQ_OFFSET + 2]
                .try_into()
                .expect("slice of 2"),
        )
    })
}

/// `key = BASE_KEY ^ (0xff & length) ^ buffer[offset]`, or `^ (0xff & seq)` for
/// a header that has a sequence in it: the cipher of every record whose header
/// carries no client public key, and the reason a file written before the
/// public key arrived needs no key of the reader's at all.
fn xor_key(magic_start: u8, length: u32, seq: Option<u16>) -> u8 {
    let byte = match seq {
        Some(seq) => seq as u8,
        None => length as u8,
    };
    BASE_KEY ^ byte ^ magic_start
}

/// `tmpBuffer[i] = key ^ buffer[offset + headerLen + i]` — the whole of that
/// cipher, over the bytes of one record.
fn xor(bytes: &[u8], key: u8) -> Vec<u8> {
    bytes.iter().map(|byte| byte ^ key).collect()
}

/// The body of a `NEW_MAGIC_COMPRESS_CRYPT_START1` record: `while (readPos <
/// length)` over one `uint16_t singleLogLen` and its bytes per log line.
///
/// A chunk whose declared length runs past the body ends the *chunking* and
/// not the walk: the record yields the lines read up to it, and the records
/// behind it are still read — which is what the C lands in as well, because
/// its `readPos += singleLogLen + 2` leaves the loop either way. The C
/// `memcpy`s those bytes out of the buffer all the same — an overread of
/// whatever stands behind the record.
fn unchunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut read = 0;
    while read + 2 <= body.len() {
        let single = usize::from(u16::from_le_bytes(
            body[read..read + 2].try_into().expect("slice of 2"),
        ));
        let start = read + 2;
        let Some(end) = start.checked_add(single).filter(|end| *end <= body.len()) else {
            break;
        };
        out.extend_from_slice(&body[start..end]);
        read = end;
    }
    out
}

/// `isGoodLogBuffer(…, 1)` — whether a whole record starts at `offset`.
///
/// All the scan has to go on: the bytes in front of a record are the payload of
/// the one before it and can be anything, so a span of damage is only over
/// where a record that checks out begins.
fn record_is_whole(data: &[u8], offset: usize) -> bool {
    let Some(header) = data
        .get(offset)
        .and_then(|magic_start| header_of(*magic_start))
    else {
        return false;
    };
    if data.len() - offset < header.len + TAILER_LEN {
        return false;
    }
    let length = u32::from_le_bytes(
        data[offset + header.len - header.crypt_key_len - 4..][..4]
            .try_into()
            .expect("slice of 4"),
    );
    let len = length as usize;
    let body_start = offset + header.len;
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
/// Flushing a zstd log ends the writes of one block with
/// `ZSTD_compressStream2(..., ZSTD_e_flush)` and never with `ZSTD_e_end`, so
/// the frame epilogue is never written; the last flush of a block is against
/// a *zero-sized* output buffer, which is why the tail of it never leaves the
/// stream either. `decode_log_file.c` accepts that and returns what it got; so
/// does this, and the same failure with nothing recovered yet is the one it
/// answers with its marker.
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
            Ok(n) => {
                out.extend_from_slice(&chunk[..n]);
                // A frame that expands without bound is not a record: see
                // [`MAX_INFLATED_LEN`]. Stopped here rather than grown into,
                // so that the record's marker is what lands in the output.
                if out.len() > MAX_INFLATED_LEN {
                    return Err(format!(
                        "{ZSTD_MARKER}: over {MAX_INFLATED_LEN} bytes of output"
                    ));
                }
            }
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

        // A stream that expands without bound is not a record: see
        // [`MAX_INFLATED_LEN`].
        if output.len() > MAX_INFLATED_LEN {
            return Err(format!("inflate: over {MAX_INFLATED_LEN} bytes of output"));
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// One record of the shortest kind there is — `MAGIC_CRYPT_START`, a
    /// four-byte length of zero, and the tailer: six bytes that hold no text,
    /// and the shortest record `decode_log_file.c` reads.
    const EMPTY_RECORD: [u8; 6] = [MAGIC_CRYPT_START, 0, 0, 0, 0, marsrs_crypt::magic::END];

    /// A file of damage is mostly markers: one per span the walk skipped, and a
    /// span can be a single byte, so a marker per byte is several times more
    /// text than the file that asked for it — which is why [`MAX_PLAIN_LEN`] is
    /// asked on the way past the damage and not only on a record's own text.
    #[test]
    fn a_file_of_damage_does_not_grow_the_text_without_end() {
        let mut file = Vec::new();
        for _ in 0..32 {
            file.extend_from_slice(&EMPTY_RECORD);
            file.push(0x00);
        }

        let err = decode_records_within(&file, None, 64).expect_err("the ceiling was crossed");
        assert_eq!(err.reason, "more than 64 bytes of text decoded");
        assert!(
            err.recovered.len() <= 64,
            "nothing past the ceiling is written: {}",
            err.recovered.len()
        );
        assert!(
            !err.recovered.is_empty(),
            "and the text read before the ceiling comes back"
        );
        // the marker is longer than the span it names, so what crossed the
        // ceiling is the text of the damage and not of a record
        assert!(String::from_utf8_lossy(&err.recovered).contains(DAMAGE_MARKER));
    }

    /// The ceiling is not a refusal to read: a file that stays under it is a
    /// file that decodes, damage and all.
    #[test]
    fn a_file_under_the_ceiling_is_decoded_whole() {
        // the junk byte is between two records: one at the end of the file is a
        // tail too short to hold a record, and not a span to mark
        let mut file = Vec::new();
        file.extend_from_slice(&EMPTY_RECORD);
        file.push(0x00);
        file.extend_from_slice(&EMPTY_RECORD);

        let plain = decode_records_within(&file, None, 1_024).expect("the walk went on");
        assert!(String::from_utf8_lossy(&plain.text).contains(DAMAGE_MARKER));
    }
}
