//! CLI front-end for the comm cross-read tests: the Rust of
//! `mars/comm/basepacker.cc`, of `mars/comm/adler32.c` and of
//! `mars/comm/strutil.cc`, driven the way `scripts/compat/upstream_comm.cpp`
//! drives the C++.
//!
//! ```text
//! comm-compat adler32 --data=HEX [--seed=N]
//! comm-compat packer pack --url=U --seq=N --data=HEX [--hash=no] --out=PATH
//! comm-compat packer unpack --in=PATH
//! comm-compat simple pack --kind=short|int --data=HEX --out=PATH
//! comm-compat simple unpack --kind=short|int --in=PATH
//! comm-compat socket --ip=TEXT|--v4=HEX|--v6=HEX --port=N [--map=yes]
//! comm-compat strutil FN --data=HEX [--arg=HEX] [--pos=N]
//! ```
//!
//! `socket` is one line of what a caller reads off a [`SocketAddress`]: the
//! family, the bytes of the address, the port, the four `valid_*`, the three
//! `is*` and the length, and then `ip`, `ipv6` and `url`, an empty one printed
//! as `-`. `--map=yes` maps the address to its `::ffff:` form first.
//!
//! `strutil` is the string helpers: `--data` is the bytes of the string, hex
//! so that a byte the shell would eat is still one a case can name, and
//! `--arg` is the second string — the delimiters of `split_token`, the prefix
//! or suffix, the needle. One line comes back, and it is the same line the
//! C++ half prints for the same call.
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
use std::net::{Ipv4Addr, Ipv6Addr};
use std::process::ExitCode;

use marsrs_comm::adler32::adler32_seeded;
use marsrs_comm::basepacker::{
    packer_pack, packer_unpack, simple_int_pack, simple_int_unpack, simple_short_pack,
    simple_short_unpack, PackerUnpacked, SimpleUnpacked,
};
use marsrs_comm::socket_address::SocketAddress;
use marsrs_comm::strutil;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        return ExitCode::FAILURE;
    };
    // `packer`, `simple` and `strutil` are the three subcommands with an
    // action of their own, and it sits where every option sits: before them,
    // and not among them.
    let action = matches!(command.as_str(), "packer" | "simple" | "strutil")
        .then(|| args.next())
        .flatten();

    let opts = parse_opts(args);
    let result = match (command.as_str(), action.as_deref()) {
        ("adler32", _) => checksum(&opts),
        ("packer", Some("pack")) => packer(&opts),
        ("packer", Some("unpack")) => unpack_packer(&opts),
        ("simple", Some("pack")) => simple(&opts),
        ("simple", Some("unpack")) => unpack_simple(&opts),
        ("socket", _) => socket(&opts),
        ("strutil", Some(name)) => strutil(name, &opts),
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
         comm-compat simple unpack --kind=short|int --in=PATH\n       \
         comm-compat socket --ip=TEXT|--v4=HEX|--v6=HEX --port=N [--map=yes]\n       \
         comm-compat strutil FN --data=HEX [--arg=HEX] [--pos=N]"
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

/// One `socket_address`, and every answer a caller reads off it: the family,
/// the bytes of the address, the port, the four `valid_*`, the three `is*`,
/// the length, and `ip`, `ipv6` and `url`.
///
/// The two calls that ask the platform which network it is on are not driven
/// here: they look the NAT64 prefix up over DNS, and a harness has to answer
/// the same line on every run.
fn socket(opts: &Opts) -> Result<(), String> {
    let port = number(opts, "port")? as u16;
    let mut addr = match (opts.value("ip"), opts.value("v4"), opts.value("v6")) {
        (Some(ip), _, _) => SocketAddress::new(ip, port),
        (_, Some(v4), _) => SocketAddress::from_v4(array(unhex(v4)?)?, port),
        (_, _, Some(v6)) => SocketAddress::from_v6(array(unhex(v6)?)?, port),
        _ => return Err("socket needs --ip=TEXT, --v4=HEX or --v6=HEX".to_owned()),
    };
    if opts.value("map") == Some("yes") {
        addr.v4_to_v4mapped_address();
    }

    let family = if !addr.valid() {
        "unspec"
    } else if addr.is_v6() || addr.is_v4mapped_address() {
        "v6"
    } else {
        "v4"
    };
    // The bytes of the address as it was built: for a v4 one that is the text
    // in `ip`, and for a v6 one the text in `ipv6`, which keeps the prefix a
    // mapped or NAT64 address carries.
    let bytes = match family {
        "v4" => hex(&addr
            .ip()
            .parse::<Ipv4Addr>()
            .map_err(|_| "ip is not a v4".to_owned())?
            .octets()),
        "v6" => hex(&addr
            .ipv6()
            .parse::<Ipv6Addr>()
            .map_err(|_| "ipv6 is not a v6".to_owned())?
            .octets()),
        _ => "-".to_owned(),
    };
    println!(
        "{family} {bytes} {} {} {} {} {} {} {} {} {} {} {} {} {}",
        addr.port(),
        bit(addr.valid()),
        bit(addr.is_v4()),
        bit(addr.is_v6()),
        bit(addr.is_v4mapped_address()),
        addr.address_length(),
        bit(addr.valid_server_address(false, false)),
        bit(addr.valid_loopback_ip()),
        bit(addr.valid_broadcast_ip()),
        bit(addr.valid_broadcast_address()),
        dash(addr.ip()),
        dash(addr.ipv6()),
        dash(addr.url()),
    );
    Ok(())
}

/// An empty string, which is what an address that never parsed answers with, is
/// not a field a line can hold: it would move every field behind it.
fn dash(s: &str) -> &str {
    if s.is_empty() {
        "-"
    } else {
        s
    }
}

/// The bytes of `--v4` or `--v6`, which have to be exactly as many as the
/// address is made of.
fn array<const N: usize>(bytes: Vec<u8>) -> Result<[u8; N], String> {
    bytes
        .try_into()
        .map_err(|_| format!("--v{N} is not {N} bytes"))
}
/// One `strutil` call, named by its action. `--data` is the bytes of the
/// string and `--arg` the second string, both hex; `--pos` is where
/// `ci_find_substr` starts looking.
///
/// A `str2hex` that found nothing and a `ci_find_substr` that found nothing
/// print `-` and `-1`: the C++ answers an empty string and
/// `std::string::npos`, and an unsigned `npos` is not a number a case should
/// have to spell.
fn strutil(name: &str, opts: &Opts) -> Result<(), String> {
    let data = unhex(opts.value("data").unwrap_or(""))?;
    // Owned, and not a borrow of the bytes `--arg` was unhexed into: the
    // string outlives the vector it came from.
    let arg = match opts.value("arg") {
        Some(arg) => {
            let bytes = unhex(arg)?;
            Some(String::from_utf8(bytes).map_err(|_| "--arg is not UTF-8".to_owned())?)
        }
        None => None,
    };
    let pos = number(opts, "pos")? as usize;

    // Two of the helpers are helpers of bytes and the rest are helpers of a
    // string, so only the latter ask that `--data` is one.
    let answer = match name {
        "hex2str" => strutil::hex2str(&data),
        "md5" => strutil::buffer_md5(&data),
        _ => strutil_text(name, text(&data)?, arg.as_deref(), pos)?,
    };
    println!("{answer}");
    Ok(())
}

/// The `strutil` helpers that take a string rather than bytes.
fn strutil_text(name: &str, text: &str, arg: Option<&str>, pos: usize) -> Result<String, String> {
    let answer = match name {
        "url_encode" => strutil::url_encode(text),
        "trim" => strutil::trim(text).to_owned(),
        "trim_left" => strutil::trim_left(text).to_owned(),
        "trim_right" => strutil::trim_right(text).to_owned(),
        "lower" => strutil::cast_lower(text),
        "upper" => strutil::cast_upper(text),
        // `1` and `0`, and not `true` and `false`: the C++ prints a `%d` of its
        // `bool`, and the two sides print the same line for the same call.
        "starts_with" => bit(strutil::starts_with(text, second(name, arg)?)),
        "ends_with" => bit(strutil::ends_with(text, second(name, arg)?)),
        "split_token" => strutil::split_token(text, second(name, arg)?).join("|"),
        "str2hex" => match strutil::str2hex(text) {
            Some(bytes) => hex(&bytes),
            None => "-".to_owned(),
        },
        "file_name_from_path" => strutil::file_name_from_path(text).to_owned(),
        "ci_find_substr" => match strutil::ci_find_substr(text, second(name, arg)?, pos) {
            Some(at) => at.to_string(),
            None => "-1".to_owned(),
        },
        _ => return Err(format!("strutil {name} is not a helper this CLI drives")),
    };
    Ok(answer)
}

/// A `bool` as the `1` or `0` of a `%d`, which is what the C++ prints it as.
fn bit(value: bool) -> String {
    if value { "1" } else { "0" }.to_owned()
}

/// `--data` as a string, which is what most of the helpers read.
fn text(data: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(data).map_err(|_| "--data is not UTF-8".to_owned())
}

/// `--arg`, which every helper but the ones of one string needs.
fn second<'a>(name: &str, arg: Option<&'a str>) -> Result<&'a str, String> {
    arg.ok_or_else(|| format!("strutil {name} needs --arg=HEX"))
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
