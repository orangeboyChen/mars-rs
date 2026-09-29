//! CLI front-end for the SDT cross-read tests: the Rust of
//! `mars/sdt/src/checkimpl/http_url_parser.h`, driven the way
//! `scripts/compat/upstream_sdt.cpp` drives the C++.
//!
//! ```text
//! sdt-compat url --url=U
//! ```
//!
//! `url` prints what the parser read out of one URL — one line per thing, the
//! same lines the C++ harness prints — because that is all a caller can see of a
//! parse: `Port()` is `80` for a URL that named none, `Path()` is `/` for one
//! that named no path, and a `Host()` of nothing is a URL that did not parse,
//! which is why its `Path()` is still empty.

use std::collections::BTreeMap;
use std::process::ExitCode;

use marsrs_sdt::HttpUrlParser;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        return ExitCode::FAILURE;
    };

    let opts = parse_opts(args);
    let result = match command.as_str() {
        "url" => url(&opts),
        other => Err(format!("unknown subcommand `{other}`")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("sdt-compat: {err}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!("usage: sdt-compat url --url=U");
}

fn url(opts: &Opts) -> Result<(), String> {
    let url = opts.value("url").ok_or("url needs --url=U")?;
    let parsed = HttpUrlParser::new(url);
    println!("host {}", parsed.host());
    println!("port {}", parsed.port());
    println!("path {}", parsed.path());
    Ok(())
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
            eprintln!("sdt-compat: ignored argument `{arg}`");
            continue;
        };
        opts.values.insert(key.to_owned(), value.to_owned());
    }
    opts
}
