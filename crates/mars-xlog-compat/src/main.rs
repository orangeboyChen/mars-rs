//! CLI front-end for the [`mars_xlog_compat`] library; see the crate docs.
//!
//! ```text
//! xlog-compat encode --mode=zlib --compress=1 --sync=0 --pubkey=<hex> \
//!     --records=records.bin --out=a.xlog
//! xlog-compat decode --privkey=<hex> --in=a.xlog --out=a.plain
//! ```

use std::process::ExitCode;

use mars_xlog_compat::{decode, encode, Opts};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        eprintln!(
            "usage: xlog-compat encode --mode=zlib|zstd [--compress=1] [--sync=0] \
             [--pubkey=HEX] --records=PATH --out=PATH\n       \
             xlog-compat decode --privkey=HEX --in=PATH --out=PATH"
        );
        return ExitCode::FAILURE;
    };

    let opts = parse_opts(args);
    let result = match command.as_str() {
        "encode" => encode(&opts),
        "decode" => decode(&opts),
        other => Err(format!("unknown subcommand `{other}`")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xlog-compat: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Parses `--key=value` into a map; anything else is a usage error.
fn parse_opts(args: impl Iterator<Item = String>) -> Opts {
    let mut opts = Opts::new();
    for arg in args {
        if let Some((key, value)) = arg.strip_prefix("--").and_then(|a| a.split_once('=')) {
            opts.insert(key.to_owned(), value.to_owned());
        } else {
            eprintln!("xlog-compat: ignored argument `{arg}`");
        }
    }
    opts
}
