//! CLI front-end for the comm cross-read tests: the Rust of
//! `mars/comm/basepacker.cc` and of `mars/comm/adler32.c`, driven the way
//! `scripts/compat/upstream_comm.cpp` drives the C++.
//!
//! ```text
//! comm-compat adler32 --data=HEX [--seed=N]
//! comm-compat packer pack --url=U --seq=N --data=HEX [--hash=no] --out=PATH
//! comm-compat packer unpack --in=PATH
//! comm-compat simple pack --kind=short|int --data=HEX --out=PATH
//! comm-compat simple unpack --kind=short|int --in=PATH
//! ```
//!
//! `packer unpack` and `simple unpack` print one line: the `int` the C++
//! answers, then what it read out of the package. A code that is not `0` is a
//! code and nothing more on either side: the three `LONGLINKPACK_CONTINUE*`
//! answers say "read on", and the one thing the port does not spell out is
//! *which* line of `basepacker.cc` refused a package — the C++ answers that
//! line's `__LINE__`, a positive number, and the port answers `1` for all of
//! them, which is why `scripts/compat/comm.sh` compares a refused package on
//! the sign of its code and not on the code itself.

use std::collections::BTreeMap;
use std::process::ExitCode;

use marsrs_comm::adler32::adler32_seeded;
use marsrs_comm::basepacker::{
    packer_pack, packer_unpack, simple_int_pack, simple_int_unpack, simple_short_pack,
    simple_short_unpack, PackerUnpacked, SimpleUnpacked,
};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        return ExitCode::FAILURE;
    };
    // `packer` and `simple` are the two subcommands with an action of their
    // own, and it sits where every option sits: before them, and not among
    // them.
    let action = matches!(command.as_str(), "packer" | "simple")
        .then(|| args.next())
        .flatten();

    let opts = parse_opts(args);
    let result = match (command.as_str(), action.as_deref()) {
        ("adler32", _) => checksum(&opts),
        ("packer", Some("pack")) => packer(&opts),
        ("packer", Some("unpack")) => unpack_packer(&opts),
        ("simple", Some("pack")) => simple(&opts),
        ("simple", Some("unpack")) => unpack_simple(&opts),
        _ => {
            usage();
            return ExitCode::FAILURE;
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("comm-compat: {err}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!(
        "usage: comm-compat adler32 --data=HEX [--seed=N]\n       \
         comm-compat packer pack --url=U --seq=N --data=HEX [--hash=no] \
         --out=PATH\n       \
         comm-compat packer unpack --in=PATH\n       \
         comm-compat simple pack --kind=short|int --data=HEX --out=PATH\n       \
         comm-compat simple unpack --kind=short|int --in=PATH"
    );
}

/// The Adler-32 of `--data`, read on from `--seed`.
fn checksum(opts: &Opts) -> Result<(), String> {
    let data = unhex(opts.value("data").unwrap_or(""))?;
    let seed = number(opts, "seed")?;
    println!("{}", adler32_seeded(seed, &data));
    Ok(())
}

/// `Packer_Pack` — the header, the URL and the body, into `--out`.
fn packer(opts: &Opts) -> Result<(), String> {
    let url = opts.value("url").unwrap_or("");
    let sequence = number(opts, "seq")?;
    let data = unhex(opts.value("data").unwrap_or(""))?;
    let do_hash = opts.value("hash") != Some("no");
    let out = opts.value("out").ok_or("packer pack needs --out=PATH")?;

    let packed = packer_pack(url, sequence, &data, do_hash);
    std::fs::write(out, packed).map_err(|err| format!("write {out}: {err}"))
}

/// `Packer_Unpack` — one line: the code, the URL, the sequence, how long the
/// package is, and the body.
fn unpack_packer(opts: &Opts) -> Result<(), String> {
    let path = opts.value("in").ok_or("packer unpack needs --in=PATH")?;
    let raw = std::fs::read(path).map_err(|err| format!("read {path}: {err}"))?;

    let unpacked = packer_unpack(&raw);
    let code = unpacked.code();
    // Only the two answers that read something out of the stream have a URL, a
    // sequence, a length or a body to print; the rest print the code and the
    // zeros the C++ would have left in the caller's own variables.
    let (url, sequence, pack_len, data) = match unpacked {
        PackerUnpacked::ContinueData {
            url,
            sequence,
            pack_len,
        } => (url, sequence, pack_len, Vec::new()),
        PackerUnpacked::Ok {
            url,
            sequence,
            pack_len,
            data,
        } => (url, sequence, pack_len, data),
        _ => (String::new(), 0, 0, Vec::new()),
    };
    println!("{code} {url} {sequence} {pack_len} {}", hex(&data));
    Ok(())
}

/// `SimpleShortPack` / `SimpleIntPack` — a length and the body behind it.
fn simple(opts: &Opts) -> Result<(), String> {
    let data = unhex(opts.value("data").unwrap_or(""))?;
    let out = opts.value("out").ok_or("simple pack needs --out=PATH")?;
    let packed = match kind(opts)? {
        Kind::Short => simple_short_pack(&data),
        Kind::Int => simple_int_pack(&data),
    };
    std::fs::write(out, packed).map_err(|err| format!("write {out}: {err}"))
}

/// `SimpleShortUnpack` / `SimpleIntUnpack` — one line: the code, how long the
/// package is, and the body.
fn unpack_simple(opts: &Opts) -> Result<(), String> {
    let path = opts.value("in").ok_or("simple unpack needs --in=PATH")?;
    let raw = std::fs::read(path).map_err(|err| format!("read {path}: {err}"))?;

    let unpacked = match kind(opts)? {
        Kind::Short => simple_short_unpack(&raw),
        Kind::Int => simple_int_unpack(&raw),
    };
    let code = unpacked.code();
    let (pack_len, data) = match unpacked {
        SimpleUnpacked::ContinueData { pack_len } => (pack_len, Vec::new()),
        SimpleUnpacked::Ok { pack_len, data } => (pack_len, data),
        _ => (0, Vec::new()),
    };
    println!("{code} {pack_len} {}", hex(&data));
    Ok(())
}

/// Which of the two `Simple*` pairs a case is about: the `uint16_t` one or the
/// `uint32_t` one.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Short,
    Int,
}

fn kind(opts: &Opts) -> Result<Kind, String> {
    match opts.value("kind") {
        Some("short") => Ok(Kind::Short),
        Some("int") => Ok(Kind::Int),
        Some(other) => Err(format!("--kind={other} is neither `short` nor `int`")),
        None => Err("simple needs --kind=short|int".to_owned()),
    }
}

/// What the command line gave: one value per `--key=`.
struct Opts {
    values: BTreeMap<String, String>,
}

impl Opts {
    fn value(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
}

/// Parses `--key=value` into a map; anything else is a usage error.
fn parse_opts(args: impl Iterator<Item = String>) -> Opts {
    let mut opts = Opts {
        values: BTreeMap::new(),
    };
    for arg in args {
        let Some((key, value)) = arg.strip_prefix("--").and_then(|a| a.split_once('=')) else {
            eprintln!("comm-compat: ignored argument `{arg}`");
            continue;
        };
        opts.values.insert(key.to_owned(), value.to_owned());
    }
    opts
}

fn number(opts: &Opts, key: &str) -> Result<u32, String> {
    let value = opts.value(key).unwrap_or("0");
    value
        .parse()
        .map_err(|_| format!("--{key}={value} is not a u32"))
}

fn unhex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("--data has an odd number of digits".to_owned());
    }
    (0..text.len())
        .step_by(2)
        .map(|at| {
            u8::from_str_radix(&text[at..at + 2], 16).map_err(|_| "--data is not hex".to_owned())
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
