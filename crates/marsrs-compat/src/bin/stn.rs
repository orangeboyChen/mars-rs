//! CLI front-end for the STN cross-read tests: the Rust of
//! `mars/stn/proto/longlink_packer.cc` and of `shortlink_packer.cc`, driven the
//! way `scripts/compat/upstream_stn.cpp` and `upstream_shortlink.cpp` drive the
//! C++.
//!
//! ```text
//! stn-compat pack --client-version=0 --cmdid=1 --seq=2 --body=HEX --out=PATH
//! stn-compat unpack --client-version=0 --in=PATH
//! stn-compat shortlink pack --url=U [--header=N:V ...] --body=HEX --out=PATH
//! stn-compat shortlink parse --in=PATH
//! ```
//!
//! `unpack` prints one line: the `int` the C++ answers, then the cmdid, the
//! seq, how long the whole package is and the body as hex. Upstream fills the
//! three through references and leaves them alone when it answers anything but
//! `LONGLINK_UNPACK_OK`, so a line whose first field is not `0` is a code and
//! nothing more on either side — which is why `scripts/compat/stn.sh` compares
//! the whole line only for a package that unpacked.
//!
//! `shortlink parse` prints a canonical form of what it read — one line per
//! thing, the same lines the C++ harness prints — because a request is more
//! than one line's worth of answer: its status, its request line, every field
//! of its head in the order the head holds them, and its body.

use std::collections::BTreeMap;
use std::process::ExitCode;

use marsrs_comm::http::{Parser, RecvStatus};
use marsrs_stn::longlink::{longlink_pack, longlink_unpack, set_client_version, Unpacked};
use marsrs_stn::shortlink::{pack as shortlink_pack, Headers};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        return ExitCode::FAILURE;
    };
    // `shortlink` is the one subcommand with an action of its own, and it sits
    // where every other option sits: before them, and not among them.
    let action = (command == "shortlink").then(|| args.next()).flatten();

    let opts = parse_opts(args);
    let result = match command.as_str() {
        "pack" => pack(&opts),
        "unpack" => unpack(&opts),
        "shortlink" => match action.as_deref() {
            Some("pack") => pack_shortlink(&opts),
            Some("parse") => parse_shortlink(&opts),
            _ => {
                usage();
                return ExitCode::FAILURE;
            }
        },
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

fn usage() {
    eprintln!(
        "usage: stn-compat pack --client-version=0 --cmdid=1 --seq=2 \
         --body=HEX --out=PATH\n       \
         stn-compat unpack --client-version=0 --in=PATH\n       \
         stn-compat shortlink pack --url=U [--header=N:V ...] --body=HEX \
         --out=PATH\n       \
         stn-compat shortlink parse --in=PATH"
    );
}

/// `--client-version` first, on either subcommand: it is what `longlink_pack`
/// stamps the header with and the only version `longlink_unpack` accepts back,
/// so a pair that disagrees about it answers `LONGLINK_UNPACK_FALSE` on both
/// sides.
fn pack(opts: &Opts) -> Result<(), String> {
    set_client_version(number(opts, "client-version")?);
    let cmdid = number(opts, "cmdid")?;
    let seq = number(opts, "seq")?;
    let body = unhex(opts.value("body").unwrap_or(""))?;
    let out = opts.value("out").ok_or("pack needs --out=PATH")?;

    let packed = longlink_pack(cmdid, seq, &body);
    std::fs::write(out, packed).map_err(|err| format!("write {out}: {err}"))
}

fn unpack(opts: &Opts) -> Result<(), String> {
    set_client_version(number(opts, "client-version")?);
    let path = opts.value("in").ok_or("unpack needs --in=PATH")?;
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

fn pack_shortlink(opts: &Opts) -> Result<(), String> {
    let url = opts.value("url").ok_or("shortlink pack needs --url=U")?;
    let body = unhex(opts.value("body").unwrap_or(""))?;
    let out = opts.value("out").ok_or("shortlink pack needs --out=PATH")?;
    let headers: Headers = opts.headers.iter().cloned().collect();

    let packed = shortlink_pack(url, &headers, &body);
    std::fs::write(out, packed).map_err(|err| format!("write {out}: {err}"))
}

fn parse_shortlink(opts: &Opts) -> Result<(), String> {
    let path = opts.value("in").ok_or("shortlink parse needs --in=PATH")?;
    let bytes = std::fs::read(path).map_err(|err| format!("read {path}: {err}"))?;

    // A short link is one request on its own socket, so the whole thing is one
    // read: what a socket would have given in two is here in one call.
    let mut parser = Parser::new();
    let status = parser.recv(&bytes);
    let request = parser.request();

    println!("status {}", status_code(status));
    println!(
        "request {} {} {}",
        request.method.as_str(),
        request.url,
        request.version.as_str()
    );
    for header in parser.fields().headers() {
        println!("field {} {}", header.name, header.value);
    }
    println!("body {}", hex(parser.body()));
    Ok(())
}

/// `Parser::TRecvStatus` as the number the C++ would print: the two
/// enumerations hold the same eight states in the same order, so the number
/// itself travels between the two harnesses.
fn status_code(status: RecvStatus) -> i32 {
    match status {
        RecvStatus::Start => 0,
        RecvStatus::FirstLine => 1,
        RecvStatus::FirstLineError => 2,
        RecvStatus::HeaderFields => 3,
        RecvStatus::HeaderFieldsError => 4,
        RecvStatus::Body => 5,
        RecvStatus::BodyError => 6,
        RecvStatus::End => 7,
    }
}

/// What the command line gave: one value per `--key=`, and one field per
/// `--header=N:V`, which is the only option a case may repeat.
struct Opts {
    values: BTreeMap<String, String>,
    headers: Vec<(String, String)>,
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
        headers: Vec::new(),
    };
    for arg in args {
        let Some((key, value)) = arg.strip_prefix("--").and_then(|a| a.split_once('=')) else {
            eprintln!("stn-compat: ignored argument `{arg}`");
            continue;
        };
        if key == "header" {
            match value.split_once(':') {
                Some((name, field)) => opts.headers.push((name.to_owned(), field.to_owned())),
                None => eprintln!("stn-compat: `--header={value}` has no colon"),
            }
        } else {
            opts.values.insert(key.to_owned(), value.to_owned());
        }
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
