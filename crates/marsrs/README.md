# marsrs

The Rust port of [Tencent's mars](https://github.com/Tencent/mars): the xlog
appender, the STN task pipeline and the SDT network diagnosis, in safe Rust and
without the C++ toolchain.

```toml
[dependencies]
marsrs = "0.1"
```

```rust
use marsrs::xlog::{LogLevel, XLogConfig, Xlog};

let mut config = XLogConfig::default();
config.logdir = std::path::PathBuf::from("/tmp/mars-log");

let xlog = Xlog::open(config, LogLevel::Info).unwrap();
xlog.i("startup", "hello from mars");
xlog.flush_now();
```

| module   | what it is                                        | C++         |
|----------|---------------------------------------------------|-------------|
| `xlog`   | the logger: open, write, flush, close              | `mars/xlog` |
| `stn`    | the task pipeline: anti-avalanche, dynamic timeout | `mars/stn`  |
| `sdt`    | the diagnosis: ping, DNS, TCP and HTTP             | `mars/sdt`  |
| `comm`   | the utilities the other two are written over       | `mars/comm` |
| `bytes`  | `AutoBuffer` and `PtrBuffer`                       | `comm/autobuffer.h` |

This is the whole port: what the C++ project calls `mars-core`, under the name
this port gives every artifact of its own. The other one is
[`marsrs-xlog`](https://crates.io/crates/marsrs-xlog), the logger on its own:
take that one if all you do is log, and you carry none of STN's or SDT's bytes.

Every module is behind a feature of its own (`xlog`, `stn`, `sdt`, `comm`, all
on by default), so `--no-default-features --features xlog` is the same pair of
crates with the net half left out.

The crates behind this one (`marsrs-core`, `marsrs-comm`, `marsrs-stn`,
`marsrs-sdt`, `marsrs-xlog`) are implementation details. They are published only
because a published crate cannot depend on a crate that is not on crates.io.

MIT licensed.
