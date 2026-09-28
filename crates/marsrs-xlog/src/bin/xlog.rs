//! `xlog` — the CLI of `marsrs-xlog`, and the command line of one file format:
//! the `.xlog` of Tencent's `mars/xlog`.
//!
//! ```text
//! xlog encode|e [--mode|-m=zlib|zstd] [--compress|-c=0|1] [--sync|-s=0|1]
//!               [--pubkey|-p=HEX] [--level|-l=N] [--region|-r=N]
//!               [--out|-o=PATH] [--in|-i=PATH] [INPUT]
//! xlog decode|d [--privkey|-k=HEX] [--out|-o=PATH] [--in|-i=PATH] [INPUT]
//! xlog keygen|k [--out|-o=PATH]
//! xlog help|h
//! ```
//!
//! * `encode` writes records into a `.xlog`: one record per line of `INPUT`,
//!   the line's own newline included. `--pubkey` is what encrypts it — the
//!   128-character hex public key of the pair an appender is configured with,
//!   without which the file is written in the clear.
//! * `decode` reads one back and prints the log text: `--privkey` is the
//!   private key of that pair, without which an encrypted record is an error
//!   and not a silent skip. This is upstream's `decode_mars_log_file.py` over
//!   the same bytes. A record that cannot be read stops it — but the records
//!   before the damage are written out anyway, to `--out` or to standard
//!   output when there is none, with the reason on standard error: a file
//!   that lost its end still has days of log in it.
//! * `keygen` makes that pair: the 128 hex characters a `pubKey` is configured
//!   with, and the 64 that `decode` reads what it wrote back with. It is drawn
//!   from the system's generator and kept nowhere, so a pair that was not
//!   written down is a file nobody can read — give it `--out`, or keep what it
//!   printed. `--out` creates the file for its owner alone, and never over one
//!   that is already there.
//!
//! An async record is compressed whichever `--compress` says: it is framed as
//! zlib or zstd, there is no framing for one that is not, and every decoder
//! inflates it. So `--compress=0` is asked for with `--sync=1` — the mode that
//! stores a record verbatim — and is refused without it.
//!
//! `INPUT` is a path or `-` for standard input (`--in=` and `--records=` say
//! the same thing), and `--out` is a path or `-` for standard output; it is
//! standard output when it is left out, so `xlog decode a.xlog | less` works.
//!
//! Every option has a short spelling, and takes its value either attached —
//! `-oFILE`, `-o=FILE` — or as the next argument, `-o FILE`. A subcommand has
//! one too, and it is a word and not a flag: `xlog e`, the way `cargo b` is
//! `cargo build`. `xlog help` is the whole command line, and `xlog --version`
//! the version.

use std::fs;
use std::io::{Read, Write};
use std::process::ExitCode;

use marsrs_crypt::TAILER_LEN;
use marsrs_xlog::{bytes::AutoBuffer, decode_records, CompressMode, LogBuffer};

/// `kBufferBlockLength` in `mars/xlog/src/appender.cc` (150 KiB), the size of
/// the region a record is written through.
const DEFAULT_REGION: usize = 150 * 1024;
/// `ZSTD_c_compressionLevel` default of `XlogConfig` in the C++ appender.
const DEFAULT_LEVEL: i32 = 6;
/// The bytes one record needs of the region: its own length, the tailer byte,
/// and the most a compressor adds to an input it cannot compress — zstd's
/// `ZSTD_COMPRESSBOUND` is `len + len / 128 + 64`, and that also covers zlib's
/// stored blocks, five bytes per 64 KiB.
///
/// [`LogBuffer::write`] writes into the room there is and drops the rest, so
/// the CLI has to know this before it writes and not after: a record that
/// turned out not to fit would be truncated in silence.
fn room_for(len: usize) -> usize {
    len + len / 128 + TAILER_LEN + 64
}

const USAGE: &str = "\
xlog — write and read the .xlog files of mars/xlog

usage:
  xlog encode|e [OPTIONS] [INPUT]   one record per line of INPUT -> a .xlog
  xlog decode|d [OPTIONS] [INPUT]   a .xlog -> the log text it holds
  xlog keygen|k [OPTIONS]           a public key for a config, and the private
                                    key that reads what it wrote
  xlog help|h                       this text

options:
  -o, --out=PATH         where the output goes; `-`, or left out, is standard
                         output. `keygen` takes this one and no other
  -i, --in=PATH          the input, for when a positional argument reads badly
  -k, --privkey=HEX      decode: the 64 hex characters of the private key of the
                         pair whose public key the file was written with
  -p, --pubkey=HEX       encode: the 128 hex characters of that public key; an
                         async file is encrypted, and is written in the clear
                         without it
  -m, --mode=zlib|zstd   encode: which compressor, zlib by default
  -c, --compress=0|1     encode: compress the payload, 1 by default. An async
                         record is always framed as zlib or zstd, so
                         `--compress=0` goes with `--sync=1`, the mode that
                         stores a record verbatim
  -s, --sync=0|1         encode: one record per block instead of one block per
                         file, 0 by default. A sync record is neither compressed
                         nor encrypted, which is what the C++ writes
  -l, --level=N          encode: the zstd level, 6 by default
  -r, --region=N         encode: the size of the buffer a record is written
                         through, 153600 by default; a record that needs a
                         bigger one is given it

INPUT of `-`, or none at all, is standard input; so is `--out=-`. A short
option takes its value attached — `-oFILE`, `-o=FILE` — or as the next
argument, `-o FILE`.

`xlog keygen` prints the pair as `pubkey=HEX` and `privkey=HEX`: the 128 hex
characters a `pubKey` is configured with, and the 64 that `xlog decode
--privkey` reads the file back with. It is made of nothing but the system's
randomness and is kept nowhere, so a pair nobody wrote down is a log nobody can
read. `--out=PATH` writes those two lines to a file made for you alone — 0600
on Unix — and refuses to write over one that is already there.";

/// The subcommand the command line named — which options belong to the command
/// line at all, and what a `Command` is parsed for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum What {
    Encode,
    Decode,
    Keygen,
}

/// What the command line asked for.
struct Command {
    input: Option<String>,
    out: Option<String>,
    mode: CompressMode,
    compress: bool,
    sync: bool,
    pubkey: Option<String>,
    privkey: Option<[u8; 32]>,
    level: i32,
    region: usize,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Asked anywhere on the line, not only as a subcommand: `xlog encode --help`
    // is the same question as `xlog help`, and both are answered the same way.
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "help" | "h" | "--help" | "-h"))
    {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--version" | "-V" | "-v"))
    {
        println!("xlog {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    // A subcommand is a word and not a flag: `xlog e` is `xlog encode`, the way
    // `cargo b` is `cargo build`. The dash is what makes `-o` an option, so
    // `xlog -e` is refused as one instead of being read as the command.
    let result = match args.first().map(String::as_str) {
        Some("encode" | "e") => Command::parse(What::Encode, &args[1..]).and_then(encode),
        Some("decode" | "d") => Command::parse(What::Decode, &args[1..]).and_then(decode),
        Some("keygen" | "k") => Command::parse(What::Keygen, &args[1..]).and_then(keygen),
        Some(other) => {
            let what = if other.starts_with('-') {
                "option"
            } else {
                "subcommand"
            };
            Err(format!("unknown {what} `{other}`\n\n{USAGE}"))
        }
        None => {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xlog: {err}");
            ExitCode::FAILURE
        }
    }
}

impl Command {
    /// Parses `--key=value` and `-k value` options, and at most one positional
    /// argument.
    fn parse(what: What, args: &[String]) -> Result<Self, String> {
        let mut command = Self {
            input: None,
            out: None,
            mode: CompressMode::Zlib,
            compress: true,
            sync: false,
            pubkey: None,
            privkey: None,
            level: DEFAULT_LEVEL,
            region: DEFAULT_REGION,
        };

        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            index += 1;
            if let Some(rest) = arg.strip_prefix("--") {
                // `--out=PATH` and `--out PATH` are the same option; the second
                // one is what a command line reads like, and the value is the
                // next argument that is not an option of its own.
                let (key, value) = match rest.split_once('=') {
                    Some(pair) => pair,
                    None => (rest, value_of(arg, rest, &mut index, args)?),
                };
                command.set(what, key, value)?;
            } else if let Some(cluster) = arg.strip_prefix('-').filter(|it| !it.is_empty()) {
                // `-o PATH`, `-o=PATH` and `-oPATH` are the three spellings of
                // `--out=PATH`: one short option, whose value is the rest of the
                // argument when there is one and the next argument when not.
                let letter = cluster.chars().next().unwrap_or_default();
                let Some(key) = short(letter) else {
                    return Err(format!("unknown option `-{letter}`\n\n{USAGE}"));
                };
                let attached = &cluster[letter.len_utf8()..];
                let attached = attached.strip_prefix('=').unwrap_or(attached);
                let value = if attached.is_empty() {
                    value_of(arg, key, &mut index, args)?
                } else {
                    attached
                };
                command.set(what, key, value)?;
            } else if command.input.replace(arg.clone()).is_some() {
                return Err(format!("two inputs given, the second one `{arg}`"));
            }
        }

        if command.pubkey.as_deref() == Some("") {
            command.pubkey = None;
        }
        // An option of another subcommand is a mistake and not a default: a
        // `--privkey` that `encode` ignores writes a file nobody asked for.
        if what != What::Encode && command.pubkey.is_some() {
            return Err("--pubkey is an option of `xlog encode`".into());
        }
        if what != What::Decode && command.privkey.is_some() {
            return Err("--privkey is an option of `xlog decode`".into());
        }
        // `keygen` makes a key of its own, so there is nothing for it to read:
        // `xlog keygen a.xlog` is one command line saying two things.
        if what == What::Keygen {
            if let Some(input) = command.input.take() {
                return Err(format!(
                    "`xlog keygen` takes no input, but `{input}` was given"
                ));
            }
        }
        Ok(command)
    }

    /// One `--key=value`, whichever of the two spellings it was written in.
    fn set(&mut self, what: What, key: &str, value: &str) -> Result<(), String> {
        // A key pair is made of nothing but randomness, so `--out` is the one
        // option `keygen` can be given: a `--mode` or a `--region` it would
        // ignore is a command line that does not say what it does. The two key
        // options fall through to the check above, which is where the answer
        // names the subcommand that does take them.
        if what == What::Keygen && !matches!(key, "out" | "pubkey" | "privkey") {
            return Err(format!("--{key} is not an option of `xlog keygen`"));
        }
        match key {
            "in" | "records" => self.input = Some(value.to_owned()),
            "out" => self.out = Some(value.to_owned()),
            "mode" => {
                self.mode = match value {
                    "zlib" => CompressMode::Zlib,
                    "zstd" => CompressMode::Zstd,
                    other => return Err(format!("--mode must be zlib or zstd, got `{other}`")),
                }
            }
            "compress" | "sync" => {
                let flag = match value {
                    "0" | "false" => false,
                    "1" | "true" => true,
                    other => return Err(format!("--{key} must be 0 or 1, got `{other}`")),
                };
                if key == "compress" {
                    self.compress = flag;
                } else {
                    self.sync = flag;
                }
            }
            "pubkey" => self.pubkey = Some(value.to_owned()),
            "privkey" => self.privkey = Some(privkey(value)?),
            "level" => self.level = number(key, value)?,
            "region" => self.region = number(key, value)?,
            other => return Err(format!("unknown option `--{other}`")),
        }
        Ok(())
    }

    /// The input's bytes: standard input when no path was given.
    fn input(&self) -> Result<Vec<u8>, String> {
        match self.input.as_deref() {
            None | Some("-") => {
                let mut bytes = Vec::new();
                std::io::stdin()
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("read standard input: {e}"))?;
                Ok(bytes)
            }
            Some(path) => fs::read(path).map_err(|e| format!("read {path}: {e}")),
        }
    }

    /// Where the output goes; standard output when no path was given.
    fn output(&self, bytes: &[u8]) -> Result<(), String> {
        match self.out.as_deref() {
            None | Some("-") => std::io::stdout()
                .write_all(bytes)
                .map_err(|e| format!("write standard output: {e}")),
            Some(path) => fs::write(path, bytes).map_err(|e| format!("write {path}: {e}")),
        }
    }

    /// One record per line, the newline of the line included — what an
    /// appender writes, and what makes `encode` of a file and `decode` of the
    /// result give the file back.
    fn records(&self) -> Result<Vec<Vec<u8>>, String> {
        let bytes = self.input()?;
        let records: Vec<Vec<u8>> = bytes
            .split_inclusive(|byte| *byte == b'\n')
            .map(<[u8]>::to_vec)
            .collect();
        if records.is_empty() {
            return Err("the input holds no record".into());
        }
        Ok(records)
    }
}

/// The long name of every short option: `-o PATH` is `--out=PATH`, and `-k` is
/// the key of the subcommand that takes one.
fn short(letter: char) -> Option<&'static str> {
    match letter {
        'i' => Some("in"),
        'o' => Some("out"),
        'm' => Some("mode"),
        'c' => Some("compress"),
        's' => Some("sync"),
        'p' => Some("pubkey"),
        'k' => Some("privkey"),
        'l' => Some("level"),
        'r' => Some("region"),
        _ => None,
    }
}

/// The value of an option that was written without one: the next argument,
/// unless that argument is an option of its own — `--out --mode=zlib` is a
/// value that is missing, and not an output of `--mode=zlib`.
fn value_of<'a>(
    arg: &str,
    key: &str,
    index: &mut usize,
    args: &'a [String],
) -> Result<&'a str, String> {
    let Some(value) = args
        .get(*index)
        .map(String::as_str)
        .filter(|next| !next.starts_with('-') || *next == "-")
    else {
        return Err(format!(
            "`{arg}` needs a value: `--{key}=VALUE` or `{arg} VALUE`"
        ));
    };
    *index += 1;
    Ok(value)
}

/// Writes the records of the input into a `.xlog`, encrypted when `--pubkey`
/// was given — the write path of `XloggerAppender`, over a region in memory
/// instead of the mmap'd cache file it keeps.
fn encode(command: Command) -> Result<(), String> {
    // An async record is framed as zlib or zstd whatever `--compress` says,
    // because that is the only async framing there is: the C++ hardcodes
    // `true` for `is_compress` where it builds its `Log*Buffer`, and a decoder
    // — this one included — inflates a body whose magic says async. So
    // `--compress=0` would write a file that decodes to nothing, and is
    // refused: `--sync=1` is the mode that stores a record verbatim.
    if !command.compress && !command.sync {
        return Err(
            "--compress=0 writes a record no reader can read back: an async body is \
             always framed as zlib or zstd and is inflated on the way out. Use \
             --sync=1, the mode whose records are stored verbatim"
                .into(),
        );
    }

    let records = command.records()?;
    let largest = records.iter().map(Vec::len).max().unwrap_or_default();
    let region_len = command.region.max(room_for(largest));

    let mut region = vec![0u8; region_len];
    let mut buffer = LogBuffer::new(
        command.compress,
        command.pubkey.as_deref(),
        command.mode,
        command.level,
    );
    if command.pubkey.is_some() && !buffer.is_crypt() {
        return Err(
            "--pubkey is not the 128 hex characters of a secp256k1 public key, so the \
             file would be written in the clear"
                .into(),
        );
    }

    let mut bytes = Vec::new();
    if command.sync {
        // `__WriteFile` hands `Write(data, len, out_buff)` a fresh `AutoBuffer`
        // per record, and `LogCrypt::CryptSyncLog` overwrites it, so the blocks
        // are copied out one at a time on both sides.
        for (index, record) in records.iter().enumerate() {
            let mut block = AutoBuffer::new();
            if !buffer.write_sync(record, &mut block) {
                return Err(format!("record {index} is empty"));
            }
            bytes.extend_from_slice(block.as_slice());
        }
    } else {
        for (index, record) in records.iter().enumerate() {
            // `LogBuffer::write` writes into the room there is and drops what
            // does not fit, and it says `true` either way — so a record the
            // region cannot hold whole would be truncated in silence. Flush
            // first whenever the room left is not provably enough for the
            // record, its tailer and the most a compressor that could not
            // compress adds to it.
            if region_len.saturating_sub(buffer.len()) < room_for(record.len()) {
                let mut block = AutoBuffer::new();
                buffer.flush(&mut region, &mut block);
                bytes.extend_from_slice(block.as_slice());
                // The block is out of the region and in `bytes`, which is the
                // file it was asked for, so the region can be given up:
                // `drained` is the other half of `flush`, and without it the
                // next record would join the block that was just copied out.
                buffer.drained(&mut region);
            }
            // A region that filled up anyway is flushed and the record written
            // to the next one, the way the appender's own cache file is: one
            // file holds a block per flush and not one per run.
            if !buffer.write(&mut region, record) {
                let mut block = AutoBuffer::new();
                buffer.flush(&mut region, &mut block);
                bytes.extend_from_slice(block.as_slice());
                buffer.drained(&mut region);
                if !buffer.write(&mut region, record) {
                    return Err(format!(
                        "record {index} ({} bytes) does not fit in a {region_len} byte buffer; raise --region",
                        record.len()
                    ));
                }
            }
        }
        let mut block = AutoBuffer::new();
        buffer.flush(&mut region, &mut block);
        bytes.extend_from_slice(block.as_slice());
        buffer.drained(&mut region);
    }

    command.output(&bytes)?;
    // A sync record is neither compressed nor encrypted — see the usage text
    // — so a `--sync` file carries none of the TEA even when `--pubkey` was
    // given, and the summary has to be true of what was written. Its records
    // are headed like any other: the 73-byte header carries the client
    // public key whatever mode the file was written in.
    eprintln!(
        "xlog: {} records -> {} bytes{}",
        records.len(),
        bytes.len(),
        if buffer.is_crypt() && !command.sync {
            ", encrypted"
        } else {
            ""
        }
    );
    Ok(())
}

/// Reads a `.xlog` and writes the log text of every record in it, decrypting
/// with `--privkey` — upstream's `decode_mars_log_file.py` over the same
/// framing.
fn decode(command: Command) -> Result<(), String> {
    let bytes = command.input()?;
    match decode_records(&bytes, command.privkey.as_ref()) {
        Ok(plain) => {
            command.output(&plain)?;
            eprintln!("xlog: {} bytes -> {} bytes", bytes.len(), plain.len());
            Ok(())
        }
        Err(err) => {
            // `parseFile` of `decode_log_file.c` writes the output it has
            // whether or not the walk ended in an error, and so does this: a
            // file that lost its end to a process killed between two writes
            // still holds every record before the damage, and an operator told
            // "truncated" with nothing beside it cannot read a single one of
            // them. The error is still what the command answers with — the
            // file's tail is missing, and that has to be said.
            if !err.recovered.is_empty() {
                command.output(&err.recovered)?;
                eprintln!(
                    "xlog: {} bytes -> {} bytes, then {}",
                    bytes.len(),
                    err.recovered.len(),
                    err.reason
                );
            }
            Err(err.reason)
        }
    }
}

/// Makes a key pair: the 128 hex characters a `pubKey` is configured with, and
/// the 64 that read what it wrote back.
///
/// The curve is the one the appender's own handshake is over: `LogCrypt` draws
/// a client key of it per file and agrees a TEA key with the public key it was
/// given, so what a config is handed is the *public* half of a pair and what
/// reads a file back is the private one — which is why this command prints both
/// and keeps neither.
fn keygen(command: Command) -> Result<(), String> {
    use k256::elliptic_curve::{sec1::ToSec1Point, Generate};

    // `uECC_make_key` over the same generator `LogCrypt` reaches for: two runs
    // draw two pairs, so a pair is written down and not remembered.
    let private = k256::SecretKey::try_generate().map_err(|e| format!("make a key: {e}"))?;
    // uECC stores a point as `X || Y` with no leading tag, and that — 64 bytes,
    // 128 hex characters — is the shape a `pubKey` has in every config.
    let point = private.public_key().to_sec1_point(false);
    let public = &point.as_bytes()[1..];

    let pair = format!(
        "pubkey={}\nprivkey={}\n",
        to_hex(public),
        to_hex(&private.to_bytes())
    );

    // A file the private half is written to is not the same destination a `.xlog`
    // is: it is created for its owner alone, and it is never written over — see
    // [`create_key_file`]. The terminal has neither a mode nor a file to lose.
    match command.out.as_deref() {
        Some(path) if path != "-" => {
            let mut file = create_key_file(path)?;
            file.write_all(pair.as_bytes())
                .map_err(|e| format!("write {path}: {e}"))?;
        }
        _ => std::io::stdout()
            .write_all(pair.as_bytes())
            .map_err(|e| format!("write standard output: {e}"))?,
    }
    eprintln!(
        "xlog: a key pair{} — the public key goes in the config, the private one \
         reads the file back",
        match command.out.as_deref() {
            Some(path) if path != "-" => format!(" -> {path}"),
            _ => String::new(),
        }
    );
    Ok(())
}

/// Creates the file a key pair is written to: for its owner alone, and only when
/// there is nothing at `path` yet.
///
/// The two things `fs::write` would do wrong, both because what lands in the
/// file is the one thing that reads every log an app ever wrote: it creates with
/// the ordinary `0666 & umask`, which under the usual `022` leaves the private
/// key readable by every user of the machine, and it truncates a file that is
/// already there, which is how the only copy of a pair — and with it every log
/// it was the key of — is lost. The mode is given to the `open` that creates the
/// file, so there is no window in which it is permissive.
fn create_key_file(path: &str) -> Result<std::fs::File, String> {
    #[cfg(unix)]
    let opened = {
        use std::os::unix::fs::OpenOptionsExt;

        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    };
    #[cfg(not(unix))]
    let opened = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path);

    opened.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            format!(
                "`{path}` is already there: `xlog keygen` will not overwrite a key, because \
                 a pair nobody wrote down is a log nobody can read"
            )
        } else {
            format!("write {path}: {e}")
        }
    })
}

/// The 32 bytes of a `--privkey`, which is 64 hex characters.
fn privkey(hex: &str) -> Result<[u8; 32], String> {
    let raw = hex_to_bytes(hex)
        .ok_or_else(|| format!("--privkey must be 64 hex characters, got `{hex}`"))?;
    <[u8; 32]>::try_from(raw)
        .map_err(|_| format!("--privkey must be 64 hex characters, got `{hex}`"))
}

fn number<T: std::str::FromStr>(key: &str, value: &str) -> Result<T, String> {
    value
        .parse::<T>()
        .map_err(|_| format!("--{key} must be a number, got `{value}`"))
}

/// The hex of `bytes` — two lowercase characters per byte, the spelling every
/// key on this command line has.
fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: [u8; 16] = *b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(char::from(DIGITS[usize::from(byte >> 4)]));
        hex.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    hex
}

fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    let digits = hex.as_bytes();
    let mut out = Vec::with_capacity(digits.len() / 2);
    let mut index = 0;
    while index + 2 <= digits.len() {
        let pair = std::str::from_utf8(&digits[index..index + 2]).ok()?;
        out.push(u8::from_str_radix(pair, 16).ok()?);
        index += 2;
    }
    // An odd number of digits leaves one character unpaired.
    (index == digits.len()).then_some(out)
}
