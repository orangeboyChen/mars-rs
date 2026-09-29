//! CLI front-end for the long-link cross-read test: the Rust of
//! `mars/stn/proto/longlink_packer.cc`, driven the way
//! `scripts/compat/upstream_stn.cpp` drives the C++.
//!
//! ```text
//! stn-compat pack --client-version=0 --cmdid=1 --seq=2 --body=HEX --out=PATH
//! stn-compat unpack --client-version=0 --in=PATH
//! ```
//!
//! `unpack` prints one line: the `int` the C++ answers, then the cmdid, the
//! seq, how long the whole package is and the body as hex. Upstream fills the
//! three through references and leaves them alone when it answers anything but
//! `LONGLINK_UNPACK_OK`, so a line whose first field is not `0` is a code and
//! nothing more on either side — which is why `scripts/compat/stn.sh` compares
//! the whole line only for a package that unpacked.

use std::collections::BTreeMap;
use std::process::ExitCode;

use marsrs_stn::longlink::{longlink_pack, longlink_unpack, set_client_version, Unpacked};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        eprintln!(
            "usage: stn-compat pack --client-version=0 --cmdid=1 --seq=2 \
             --body=HEX --out=PATH\n       \
             stn-compat unpack --client-version=0 --in=PATH"
        );
        return ExitCode::FAILURE;
    };

    let opts = parse_opts(args);
    let result = match command.as_str() {
        "pack" => pack(&opts),
        "unpack" => unpack(&opts),
        other => Err(format!("unknown subcommand `{other}`")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("stn-compat: {err}");
            ExitCode::FAILURE
        }
    }
}

/// `--client-version` first, on either subcommand: it is what `longlink_pack`
/// stamps the header with and the only version `longlink_unpack` accepts back,
/// so a pair that disagrees about it answers `LONGLINK_UNPACK_FALSE` on both
/// sides.
fn pack(opts: &BTreeMap<String, String>) -> Result<(), String> {
    set_client_version(number(opts, "client-version")?);
    let cmdid = number(opts, "cmdid")?;
    let seq = number(opts, "seq")?;
    let body = unhex(opts.get("body").map(String::as_str).unwrap_or(""))?;
    let out = opts.get("out").ok_or("pack needs --out=PATH")?;

    let packed = longlink_pack(cmdid, seq, &body);
    std::fs::write(out, packed).map_err(|err| format!("write {out}: {err}"))
}

fn unpack(opts: &BTreeMap<String, String>) -> Result<(), String> {
    set_client_version(number(opts, "client-version")?);
    let path = opts.get("in").ok_or("unpack needs --in=PATH")?;
    let packed = std::fs::read(path).map_err(|err| format!("read {path}: {err}"))?;

    let (code, cmdid, seq, package_len, body) = match longlink_unpack(&packed) {
        Unpacked::Package {
            cmdid,
            seq,
            package_len,
            body,
        } => (0, cmdid, seq, package_len, body),
        other => (other.code(), 0, 0, 0, Vec::new()),
    };
    println!("{code} {cmdid} {seq} {package_len} {}", hex(&body));
    Ok(())
}

/// Parses `--key=value` into a map; anything else is a usage error.
fn parse_opts(args: impl Iterator<Item = String>) -> BTreeMap<String, String> {
    let mut opts = BTreeMap::new();
    for arg in args {
        if let Some((key, value)) = arg.strip_prefix("--").and_then(|a| a.split_once('=')) {
            opts.insert(key.to_owned(), value.to_owned());
        } else {
            eprintln!("stn-compat: ignored argument `{arg}`");
        }
    }
    opts
}

fn number(opts: &BTreeMap<String, String>, key: &str) -> Result<u32, String> {
    let value = opts.get(key).map(String::as_str).unwrap_or("0");
    value
        .parse()
        .map_err(|_| format!("--{key}={value} is not a u32"))
}

fn unhex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("--body has an odd number of digits".to_owned());
    }
    (0..text.len())
        .step_by(2)
        .map(|at| {
            u8::from_str_radix(&text[at..at + 2], 16).map_err(|_| "--body is not hex".to_owned())
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
