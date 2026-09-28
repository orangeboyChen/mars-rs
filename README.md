# mars, in Rust

Rust implementation of [Tencent/mars](https://github.com/Tencent/mars): the
**xlog** logging pipeline, the **STN** task model and the **SDT** network
diagnosis. What its logger writes is the `.xlog` the C++ implementation writes —
same framing, same compression, same encryption — so the tooling that already
reads mars logs reads these.

**Documentation** — [English](https://orangeboychen.github.io/mars-rs/) ·
[简体中文](https://orangeboychen.github.io/mars-rs/zh/)

## Install

```bash
cargo add marsrs          # the whole port: xlog, stn and sdt
cargo add marsrs-xlog     # xlog alone — the logger and nothing else
```

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")
.product(name: "MarsRSXlog", package: "mars-rs")
```

```kotlin
// build.gradle.kts — settings.gradle.kts: maven { url = uri("https://jitpack.io") }
implementation("io.github.orangeboychen.marsrs:xlog:0.1.0")   // xlog alone
```

```kotlin
// build.gradle.kts of a Kotlin Multiplatform shared module
implementation("io.github.orangeboychen.marsrs:xlog-kmp:0.1.0")
```

Every release ships a package per platform — a C ABI archive and a HarmonyOS
build as well; see [getting started](https://orangeboychen.github.io/mars-rs/getting-started).

## Use it

Open an appender once when the app starts, write through it, and flush before you
read or upload its files:

```rust
use marsrs::xlog::{appender_close, appender_flush_sync, appender_open, appender_write, XLogConfig};

let mut config = XLogConfig::default();
config.logdir = std::path::PathBuf::from("/tmp/mars-log");
config.nameprefix = "marsrs".to_owned();
appender_open(config)?;

appender_write(None, "hello from mars");

appender_flush_sync();
appender_close();
```

```kotlin
val xlog = Xlog.open(
    XlogConfig(
        logDir = File(context.filesDir, "xlog/log").path,
        namePrefix = "marsrs",
        level = LogLevel.INFO,
    )
)
xlog.i("startup", "cold start in $elapsedMillis ms")

xlog.flush(sync = true)
```

The file is `<logDir>/<namePrefix>_YYYYMMDD.xlog` — `marsrs_20260927.xlog` above.
The default mode is async, so a record can sit in the cache for a moment: flush
before the file is read or uploaded, and again before the process goes away.
Every option, and its name on each platform, is on
[the configuration page](https://orangeboychen.github.io/mars-rs/configuration).

## License

MIT, like the upstream project — see [LICENSE](LICENSE).
