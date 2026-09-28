# marsrs-xlog

The xlog half of the Rust port of [Tencent's mars](https://github.com/Tencent/mars):
an appender that writes the same `.xlog` files as `mars/xlog` — same magic
number, same TEA / `zlib` / `zstd` framing, so the files are read back by
upstream's own tools and by the port's `LogBuffer`.

```toml
[dependencies]
marsrs-xlog = "0.1"
```

```rust
use marsrs_xlog::{appender_close, appender_flush_sync, appender_open, appender_write, XLogConfig};

let mut config = XLogConfig::default();
config.logdir = std::path::PathBuf::from("/tmp/mars-log");
appender_open(config).unwrap();
appender_write(None, "hello from mars");
appender_flush_sync();
appender_close();
```

This is the logging half: what the C++ project calls `mars-xlog`, under the name
this port gives every artifact of its own. The other one is
[`marsrs`](https://crates.io/crates/marsrs), which is everything — this crate
plus STN and SDT — and re-exports this one as its `xlog` module.

An app that only logs takes **this** crate: `marsrs` carries the net half with
it unless it is built `--no-default-features --features xlog`.

The crates behind this one (`marsrs-core`, `marsrs-buffer`, `marsrs-appender`)
are implementation details. They are published only because a published crate
cannot depend on a crate that is not on crates.io.

## The `xlog` command

`cargo install marsrs-xlog` puts a CLI on the `$PATH`: the writer and the reader
of the format, for a shell that has a `.xlog` and no Rust in it.

```bash
xlog keygen --out=xlog.key     # the pair: a pubkey for the config, a privkey for decode
xlog decode --privkey=<hex> marsrs_20260927.xlog --out=marsrs.plain
xlog encode --pubkey=<hex> records.txt --out=marsrs_20260927.xlog
xlog help
```

`keygen` makes the pair those two halves share: the 128 hex characters a `pubKey`
is configured with, and the 64 that read what it wrote back — nothing in the port
holds a pair of its own, so this is the one to make and keep. `decode` is what
upstream's `decode_mars_log_file.py` does over the same bytes, and the same
`decode_records` the crate exports; `encode` writes one record per line of its
input, and encrypts it when it is given the public key of the pair whose private
key `decode` takes.

Every subcommand and option has a one-letter spelling — `xlog d -k <hex>
marsrs_20260927.xlog`, `xlog e -p <hex> -o marsrs_20260927.xlog` — and `xlog
help` lists the whole command line, short spellings included.

MIT licensed.
