//! CLI front-end for the comm cross-read tests: the Rust of
//! `mars/comm/basepacker.cc`, of `mars/comm/adler32.c`, of
//! `mars/comm/crypt/ibase64.cc`, of `mars/comm/strutil.cc`, of
//! `mars/comm/socket/socket_address.cc` and of `mars/comm/http.cc`, driven the
//! way `scripts/compat/upstream_comm.cpp` drives the C++.
//!
//! ```text
//! comm-compat adler32 --data=HEX [--seed=N]
//! comm-compat base64 --data=HEX
//! comm-compat http ACTION [--data=HEX] [--set=N:V|N:V] [--in=PATH]
//! comm-compat packer pack --url=U --seq=N --data=HEX [--hash=no] --out=PATH
//! comm-compat packer unpack --in=PATH
//! comm-compat simple pack --kind=short|int --data=HEX --out=PATH
//! comm-compat simple unpack --kind=short|int --in=PATH
//! comm-compat socket --ip=TEXT|--v4=HEX|--v6=HEX --port=N [--map=yes]
//! comm-compat strutil FN --data=HEX [--arg=HEX] [--pos=N]
//! ```
//!
//! `http` is the request or the answer a caller writes and reads:
//! `request-line`/`status-line` print `ToString()` as hex, or, given
//! `--data`, what `FromString()` read; `fields` prints a head and how many
//! fields it holds; `reads` the numbers a caller reads out of one; `looks`
//! the one field `--name` asks for; `build` the bytes of a request or of an
//! answer, written to `--out`; `parse` what reading those bytes gives.
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
use marsrs_comm::http::{Body, Builder, CsMode, HeaderFields, Method, Parser, RecvStatus};
use marsrs_comm::http::{RequestLine, StatusLine, Version};
use marsrs_comm::socket_address::SocketAddress;
use marsrs_comm::strutil;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        return ExitCode::FAILURE;
    };
    // `packer`, `simple`, `strutil` and `http` are the subcommands with an
    // action of their own, and it sits where every option sits: before them,
    // and not among them.
    let action = matches!(command.as_str(), "packer" | "simple" | "strutil" | "http")
        .then(|| args.next())
        .flatten();

    let opts = parse_opts(args);
    let result = match (command.as_str(), action.as_deref()) {
        ("adler32", _) => checksum(&opts),
        ("base64", _) => base64(&opts),
        ("packer", Some("pack")) => packer(&opts),
        ("packer", Some("unpack")) => unpack_packer(&opts),
        ("simple", Some("pack")) => simple(&opts),
        ("simple", Some("unpack")) => unpack_simple(&opts),
        ("socket", _) => socket(&opts),
        ("strutil", Some(name)) => strutil(name, &opts),
        ("http", Some(name)) => http(name, &opts),
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
         comm-compat base64 --data=HEX\n       \
         comm-compat http ACTION [--data=HEX] [--set=N:V|N:V] [--in=PATH]\n       \
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

/// `EncodeBase64` — how many characters a caller got, and the characters,
/// which is the `Basic` of a `Proxy-Authorization`: `username:password`.
fn base64(opts: &Opts) -> Result<(), String> {
    let data = unhex(opts.value("data").unwrap_or(""))?;
    let encoded = marsrs_comm::base64::encode(&data);
    println!("{} {}", encoded.len(), dash(&encoded));
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
        Kind::Short => simple_short_pack(&data)
            .ok_or("a body two bytes cannot say the length of is not packed")?,
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
    // A port is a `u16` and not a `u32` with its top half cut off: `--port
    // 65536` used to come back as `0`. A port of `0` is not an unspec socket
    // either — the family below is the address's, so what `0` costs is
    // `valid_server_address`, and nothing else.
    let port = port(opts)?;
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
/// `mars/comm/http.cc` — the first line, the head and the body of a request
/// or of an answer, the way `scripts/compat/comm.sh` drives them.
///
/// * `request-line` and `status-line` print `ToString()` as hex, or, given
///   `--data=HEX`, what `FromString()` read out of that line;
/// * `fields` prints how many fields a head holds and the head itself;
/// * `reads` prints the numbers a caller reads out of a head;
/// * `looks` prints the one field `--name` asks for;
/// * `build` prints the bytes of a request or of an answer, and writes them
///   to `--out`;
/// * `parse` prints what reading those bytes gives.
fn http(name: &str, opts: &Opts) -> Result<(), String> {
    match name {
        "request-line" => first_line(opts, true),
        "status-line" => first_line(opts, false),
        "fields" => fields(opts),
        "reads" => reads(opts),
        "looks" => looks(opts),
        "build" => build(opts),
        "parse" => parse_http(opts),
        _ => Err(format!("http {name} is not an action this CLI drives")),
    }
}

/// `RequestLine::ToString` / `FromString`, and the two of `StatusLine`.
///
/// `FromString` wants a line that is `CRLF`-terminated, and a line it cannot
/// read is `refused` — the `false` the C++ answers with.
fn first_line(opts: &Opts, is_request: bool) -> Result<(), String> {
    let version = Version::parse(opts.value("version").unwrap_or("HTTP/1.1"));
    if let Some(data) = opts.value("data") {
        let line = String::from_utf8(unhex(data)?).map_err(|_| "--data is not UTF-8".to_owned())?;
        let answer = if is_request {
            RequestLine::parse(&line).map(|line| {
                format!(
                    "{} {} {}",
                    line.method.as_str(),
                    line.url,
                    line.version.as_str()
                )
            })
        } else {
            StatusLine::parse(&line).map(|line| {
                format!(
                    "{} {} {}",
                    line.version.as_str(),
                    line.status_code,
                    dash(&line.reason_phrase)
                )
            })
        };
        println!("{}", answer.unwrap_or_else(|| "refused".to_owned()));
        return Ok(());
    }

    let answer = if is_request {
        let method = Method::parse(opts.value("method").unwrap_or("GET"));
        RequestLine::new(method, opts.value("url").unwrap_or(""), version).to_string()
    } else {
        let code = number(opts, "code")? as i32;
        StatusLine::new(version, code, opts.value("reason").unwrap_or("")).to_string()
    };
    println!("{}", hex(answer.as_bytes()));
    Ok(())
}

/// `HeaderFields::ToString` — how many fields a head holds, and the head.
fn fields(opts: &Opts) -> Result<(), String> {
    let fields = head(opts)?;
    println!(
        "{} {}",
        fields.len(),
        dash(&hex(fields.to_string().as_bytes()))
    );
    Ok(())
}

/// What a caller reads out of a head: `ContentLength()`, `KeepAliveTimeout()`,
/// the three `Is*`, and the two ranges.
fn reads(opts: &Opts) -> Result<(), String> {
    let fields = head(opts)?;
    let range = fields.range().map(|(from, to)| format!("{from},{to}"));
    let content_range = fields
        .content_range()
        .map(|it| format!("{},{},{}", it.start, it.end, it.total));
    println!(
        "{} {} {} {} {} {} {}",
        fields.content_length(),
        fields.keep_alive_timeout(),
        bit(fields.is_chunked()),
        bit(fields.is_connection_close()),
        bit(fields.is_connection_keep_alive()),
        range.as_deref().unwrap_or("-"),
        content_range.as_deref().unwrap_or("-"),
    );
    Ok(())
}

/// `HeaderField(key)` — one field, whichever way its name was written.
fn looks(opts: &Opts) -> Result<(), String> {
    let name = opts.value("name").ok_or("http looks needs --name=NAME")?;
    println!("{}", dash(head(opts)?.get(name).unwrap_or("-")));
    Ok(())
}

/// `Builder::HttpToBuffer` — the bytes of a request or of an answer, which is
/// the artifact `parse` reads back.
fn build(opts: &Opts) -> Result<(), String> {
    let version = Version::parse(opts.value("version").unwrap_or("HTTP/1.1"));
    let mode = match opts.value("mode") {
        Some("respond") => CsMode::Respond,
        _ => CsMode::Request,
    };
    let mut builder = Builder::new(mode);
    match mode {
        CsMode::Request => {
            let method = Method::parse(opts.value("method").unwrap_or("GET"));
            *builder.request_mut() =
                RequestLine::new(method, opts.value("url").unwrap_or(""), version);
        }
        CsMode::Respond => {
            *builder.status_mut() = StatusLine::new(
                version,
                number(opts, "code")? as i32,
                opts.value("reason").unwrap_or(""),
            );
        }
    }
    *builder.fields_mut() = head(opts)?;
    if let Some(body) = opts.value("body") {
        builder.set_body(Body::Block(unhex(body)?));
    }
    if let Some(chunks) = opts.value("chunks") {
        builder.set_body(Body::Chunks(unhex(chunks)?));
    }

    let buffer = builder.to_buffer();
    if let Some(path) = opts.value("out") {
        std::fs::write(path, buffer.as_deref().unwrap_or(&[]))
            .map_err(|err| format!("write {path}: {err}"))?;
    }
    // A head that is not there at all is `none`, which is the C++'s `false`,
    // and a block body of no bytes is a buffer of no bytes, which is not the
    // same answer.
    match buffer {
        Some(bytes) => println!("ok {}", dash(&hex(&bytes))),
        None => println!("none -"),
    }
    Ok(())
}

/// `Parser::Recv` — how far the parser got, the two lengths, the body, the
/// mode the first line decided, and the first line and the head it read.
fn parse_http(opts: &Opts) -> Result<(), String> {
    let bytes = match opts.value("in") {
        Some(path) => std::fs::read(path).map_err(|err| format!("read {path}: {err}"))?,
        None => unhex(opts.value("data").unwrap_or(""))?,
    };
    let mut parser = Parser::new();
    let status = if opts.value("header-only") == Some("yes") {
        parser.recv_header_only(&bytes)
    } else {
        parser.recv(&bytes)
    };
    // The first line whether it was read or not: what a caller asking for it
    // before the answer is whole gets is the one the parser started with.
    let first = match parser.mode() {
        CsMode::Request => parser.request().to_string(),
        CsMode::Respond => parser.status().to_string(),
    };
    println!(
        "{} {} {} {} {} {} {} {}",
        status_name(status),
        parser.first_line_len(),
        parser.header_len(),
        parser.body_len(),
        mode_name(parser.mode()),
        dash(&hex(first.as_bytes())),
        dash(&hex(parser.fields().to_string().as_bytes())),
        dash(&hex(parser.body())),
    );
    Ok(())
}

/// `Parser::TRecvStatus`, named the way the C++ names it.
fn status_name(status: RecvStatus) -> &'static str {
    match status {
        RecvStatus::Start => "start",
        RecvStatus::FirstLine => "first-line",
        RecvStatus::FirstLineError => "first-line-error",
        RecvStatus::HeaderFields => "header-fields",
        RecvStatus::HeaderFieldsError => "header-fields-error",
        RecvStatus::Body => "body",
        RecvStatus::BodyError => "body-error",
        RecvStatus::End => "end",
    }
}

/// `TCsMode` — whether what came in is a request or an answer.
fn mode_name(mode: CsMode) -> &'static str {
    match mode {
        CsMode::Request => "request",
        CsMode::Respond => "respond",
    }
}

/// The head of a case: `--set`, then `--manipulate`, then `--update`, each a
/// `|`-separated list of `name:value`.
fn head(opts: &Opts) -> Result<HeaderFields, String> {
    let mut fields = HeaderFields::new();
    for pair in list(opts.value("set")) {
        let (name, value) = pair_of(pair)?;
        fields.set(name, value);
    }
    for pair in list(opts.value("manipulate")) {
        let (name, value) = pair_of(pair)?;
        fields.manipulate(name, value);
    }
    for pair in list(opts.value("update")) {
        let (name, value) = pair_of(pair)?;
        fields.set(name, value);
    }
    Ok(fields)
}

/// One `|`-separated list, which is how a case names a head: a value with a
/// space in it would not survive the shell's splitting of a row.
fn list(value: Option<&str>) -> impl Iterator<Item = &str> {
    value
        .unwrap_or("")
        .split('|')
        .filter(|pair| !pair.is_empty())
}

/// One `name:value` — the first colon is the one that parts them, and a value
/// may hold another.
fn pair_of(pair: &str) -> Result<(&str, &str), String> {
    pair.split_once(':')
        .ok_or_else(|| format!("{pair} is not a name:value"))
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

fn port(opts: &Opts) -> Result<u16, String> {
    let value = opts.value("port").unwrap_or("0");
    value
        .parse()
        .map_err(|_| format!("--port={value} is not a u16"))
}

fn unhex(text: &str) -> Result<Vec<u8>, String> {
    // ASCII first: the digits below are taken two *bytes* at a time, and an
    // even byte count is no promise that a byte index is a char boundary —
    // one two-byte character is even, and slicing it panics.
    if !text.is_ascii() {
        return Err("--data is not ASCII".to_owned());
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: &[(&str, &str)]) -> Opts {
        Opts {
            values: pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    /// A two-byte character is an *even* byte count, and slicing it at a byte
    /// index that is not a char boundary panics — which is what `--data=é`
    /// used to do on the way to a perfectly good error message.
    #[test]
    fn unhex_refuses_a_digit_that_is_not_a_char_boundary() {
        assert!(unhex("é").is_err(), "one two-byte character is even");
        assert!(unhex("éé").is_err());
        assert!(unhex("ffé").is_err());
        // and an odd count is still reported as one, not as a panic
        assert!(unhex("abc").is_err());
    }

    #[test]
    fn unhex_reads_hex_in_pairs() {
        assert_eq!(unhex("").unwrap(), Vec::<u8>::new());
        assert_eq!(unhex("00ff10").unwrap(), vec![0x00, 0xff, 0x10]);
        assert!(unhex("zz").is_err(), "not a digit");
    }

    /// A port is 16 bits, and a bigger number is an argument that is wrong —
    /// not one whose top half can be thrown away, which is what turned
    /// `--port 65536` into a socket at port 0.
    #[test]
    fn a_port_outside_u16_is_reported_and_not_truncated() {
        assert_eq!(port(&opts(&[("port", "80")])).unwrap(), 80);
        assert_eq!(port(&opts(&[("port", "65535")])).unwrap(), 65535);
        assert!(port(&opts(&[("port", "65536")])).is_err());
        assert!(port(&opts(&[("port", "-1")])).is_err());
        // no `--port` at all is port 0, which is what the CLI has always done
        assert_eq!(port(&opts(&[])).unwrap(), 0);
    }
}
