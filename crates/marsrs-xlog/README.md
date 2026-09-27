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

This is the `mars-xlog` of the pair the C++ project publishes. The other one is
[`marsrs`](https://crates.io/crates/marsrs), which is everything — this crate
plus STN and SDT — and re-exports this one as its `xlog` module.

An app that only logs takes **this** crate: `marsrs` carries the net half with
it unless it is built `--no-default-features --features xlog`.

The crates behind this one (`marsrs-core`, `marsrs-buffer`, `marsrs-appender`)
are implementation details. They are published only because a published crate
cannot depend on a crate that is not on crates.io.

MIT licensed.
