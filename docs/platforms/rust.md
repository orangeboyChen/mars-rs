# Rust

```bash
cargo add marsrs          # the whole port: xlog, stn and sdt
cargo add marsrs-xlog     # xlog alone
```

```toml
# the whole port with the net half left out of it
[dependencies]
marsrs = { version = "0.1", default-features = false, features = ["xlog"] }
```

```rust
use marsrs::xlog::{appender_close, appender_flush_sync, appender_open, appender_write, XLogConfig};

let mut config = XLogConfig::default();
config.logdir = std::path::PathBuf::from("/tmp/mars-log");
appender_open(config).unwrap();
appender_write(None, "hello from mars");
appender_flush_sync();
appender_close();
```

`marsrs` is one module per component — `xlog`, `stn`, `sdt`, `comm`, `bytes` —
and each of the first four is behind a feature of its own, all four on by
default, so `--no-default-features --features xlog` is the build that carries
the logger and nothing else. `bytes` is not gated: it is the byte buffer every
signature is written in terms of. `marsrs-xlog` is that build as a crate of its
own, and is what `marsrs` re-exports as `xlog`.

The nine crates are published by the release workflow, in dependency order, at
the version of the tag.
