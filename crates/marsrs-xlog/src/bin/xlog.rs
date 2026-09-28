//! `xlog` — the CLI of `marsrs-xlog`, and the command line of one file format:
//! the `.xlog` of Tencent's `mars/xlog`.
//!
//! ```text
//! xlog encode [--mode=zlib|zstd] [--compress=0|1] [--sync=0|1] [--pubkey=HEX]
//!             [--level=N] [--region=N] [--out=PATH] [INPUT]
//! xlog decode [--privkey=HEX] [--out=PATH] [INPUT]
//! xlog help
//! ```
//!
//! * `encode` writes records into a `.xlog`: one record per line of `INPUT`,
//!   the line's own newline included. `--pubkey` is what encrypts it — the
//!   128-character hex public key of the pair an appender is configured with,
//!   without which the file is written in the clear.
//! * `decode` reads one back and prints the log text: `--privkey` is the
//!   private key of that pair, without which an encrypted record is an error
//!   and not a silent skip. This is upstream's `decode_mars_log_file.py` over
//!   the same bytes.
//!
//! `INPUT` is a path or `-` for standard input (`--in=` and `--records=` say
//! the same thing), and `--out` is a path or `-` for standard output; it is
//! standard output when it is left out, so `xlog decode a.xlog | less` works.

use std::fs;
use std::io::{Read, Write};
use std::process::ExitCode;

use marsrs_xlog::{bytes::AutoBuffer, decode_records, CompressMode, LogBuffer};

/// `kBufferBlockLength` in `mars/xlog/src/appender.cc` (150 KiB), the size of
/// the region a record is written through.
const DEFAULT_REGION: usize = 150 * 1024;
/// `ZSTD_c_compressionLevel` default of `XlogConfig` in the C++ appender.
const DEFAULT_LEVEL: i32 = 6;
/// Room for a header, a tailer and a compressor that expanded, on top of the
/// largest record of the input, when a region of the default size is too small
/// for it.
const REGION_SLACK: usize = 4096;

const USAGE: &str = "\
xlog — write and read the .xlog files of mars/xlog

usage:
  xlog encode [OPTIONS] [INPUT]   one record per line of INPUT -> a .xlog
  xlog decode [OPTIONS] [INPUT]   a .xlog -> the log text it holds
  xlog help                       this text

options:
  --out=PATH         where the output goes; `-`, or left out, is standard output
  --in=PATH          the input, for when a positional argument reads badly
  --privkey=HEX      decode: the 64 hex characters of the private key of the
                     pair whose public key the file was written with
  --pubkey=HEX       encode: the 128 hex characters of that public key; the
                     file is encrypted, and is written in the clear without it
  --mode=zlib|zstd   encode: which compressor, zlib by default
  --compress=0|1     encode: compress the payload, 1 by default
  --sync=0|1         encode: one record per block instead of one block per
                     file, 0 by default. A sync record is neither compressed
                     nor encrypted, which is what the C++ writes
  --level=N          encode: the zstd level, 6 by default
  --region=N         encode: the size of the buffer a record is written
                     through, 153600 by default

INPUT of `-`, or none at all, is standard input; so is `--out=-`.";

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
        .any(|arg| matches!(arg.as_str(), "help" | "--help" | "-h"))
    {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--version" | "-V"))
    {
        println!("xlog {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let result = match args.first().map(String::as_str) {
        Some("encode") => Command::parse(true, &args[1..]).and_then(encode),
        Some("decode") => Command::parse(false, &args[1..]).and_then(decode),
        Some(other) => Err(format!("unknown subcommand `{other}`\n\n{USAGE}")),
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
    /// Parses `--key=value` options and at most one positional argument.
    fn parse(is_encode: bool, args: &[String]) -> Result<Self, String> {
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
            let Some(rest) = arg.strip_prefix("--") else {
                if command.input.replace(arg.clone()).is_some() {
                    return Err(format!("two inputs given, the second one `{arg}`"));
                }
                continue;
            };
            // `--out=PATH` and `--out PATH` are the same option; the second one
            // is what a command line reads like, and the value is the next
            // argument that is not an option of its own.
            let (key, value) = match rest.split_once('=') {
                Some(pair) => pair,
                None => {
                    let Some(value) = args
                        .get(index)
                        .filter(|next| !next.starts_with("--"))
                        .map(String::as_str)
                    else {
                        return Err(format!(
                            "`{arg}` needs a value: `--{rest}=VALUE` or `--{rest} VALUE`"
                        ));
                    };
                    index += 1;
                    (rest, value)
                }
            };
            match key {
                "in" | "records" => command.input = Some(value.to_owned()),
                "out" => command.out = Some(value.to_owned()),
                "mode" => {
                    command.mode = match value {
                        "zlib" => CompressMode::Zlib,
                        "zstd" => CompressMode::Zstd,
                        other => return Err(format!("--mode must be zlib or zstd, got `{other}`")),
                    }
                }
                "compress" | "sync" => {
                    let flag = match value {
                        "0" | "false" => false,
                        "1" | "true" => true,
                        other => {
                            return Err(format!("--{key} must be 0 or 1, got `{other}`"));
                        }
                    };
                    if key == "compress" {
                        command.compress = flag;
                    } else {
                        command.sync = flag;
                    }
                }
                "pubkey" => command.pubkey = Some(value.to_owned()),
                "privkey" => command.privkey = Some(privkey(value)?),
                "level" => command.level = number(key, value)?,
                "region" => command.region = number(key, value)?,
                other => return Err(format!("unknown option `--{other}`")),
            }
        }

        if command.pubkey.as_deref() == Some("") {
            command.pubkey = None;
        }
        // An option of the other subcommand is a mistake and not a default: a
        // `--privkey` that `encode` ignores writes a file nobody asked for.
        if !is_encode && command.pubkey.is_some() {
            return Err("--pubkey is an option of `xlog encode`".into());
        }
        if is_encode && command.privkey.is_some() {
            return Err("--privkey is an option of `xlog decode`".into());
        }
        Ok(command)
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

/// Writes the records of the input into a `.xlog`, encrypted when `--pubkey`
/// was given — the write path of `XloggerAppender`, over a region in memory
/// instead of the mmap'd cache file it keeps.
fn encode(command: Command) -> Result<(), String> {
    let records = command.records()?;
    let largest = records.iter().map(Vec::len).max().unwrap_or_default();
    let region_len = command.region.max(largest + REGION_SLACK);

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
            // A region that filled up is flushed and the record written to the
            // next one, the way the appender's own cache file is: one file holds
            // a block per flush and not one per run.
            if !buffer.write(&mut region, record) {
                let mut block = AutoBuffer::new();
                buffer.flush(&mut region, &mut block);
                bytes.extend_from_slice(block.as_slice());
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
    }

    command.output(&bytes)?;
    // A sync record is neither compressed nor encrypted — see the usage text —
    // so a `--sync` file carries the key in its header-less records and none of
    // the TEA: what is said here has to be true of what was written.
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
    let plain = decode_records(&bytes, command.privkey.as_ref())?;
    command.output(&plain)?;
    eprintln!("xlog: {} bytes -> {} bytes", bytes.len(), plain.len());
    Ok(())
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
