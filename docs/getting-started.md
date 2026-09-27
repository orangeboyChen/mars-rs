# Getting started

Three steps on every platform: **open** an appender once when the process or the
app starts, **write** records through it, and **flush** before you read or upload
its files.

## 1. Add the dependency

::: code-group

```bash [Rust]
cargo add marsrs          # the whole port: xlog, stn and sdt
cargo add marsrs-xlog     # xlog alone — the logger and nothing else
```

```swift [SwiftPM]
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")

// and, in the target that takes it:
.product(name: "MarsRSXlog", package: "mars-rs")
```

```kotlin [Android]
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen:mars-rs-xlog:0.1.0")  // xlog alone
```

```kotlin [Kotlin Multiplatform]
// build.gradle.kts of the shared module
implementation("io.github.orangeboychen:mars-rs-xlog-kmp:0.1.0")
```

```text [C]
mars-rs-<version>-<host>.tar.gz   (Linux, macOS)
mars-rs-<version>-<host>.zip      (Windows)
    include/mars_xlog.h
    include/mars_sdt.h
    include/mars_stn.h
    the static and shared libraries of marsrs-ffi
```

:::

## 2. Open, write, flush

::: code-group

```rust [Rust]
use marsrs::xlog::{appender_close, appender_flush_sync, appender_open, appender_write, XLogConfig};

let mut config = XLogConfig::default();
config.logdir = std::path::PathBuf::from("/tmp/mars-log");
config.nameprefix = "Ham".to_owned();
appender_open(config)?;

appender_write(None, "hello from mars");

appender_flush_sync();   // the records are on disk when this returns
appender_close();
```

```swift [Swift]
import MarsRSXlog

let log = try Xlog(
    XlogConfig(
        logDirectory: logDirectory.path,
        namePrefix: "Ham",
        level: .info
    )
)
log.isConsoleLogEnabled = true

log.info(message: "hello from mars", tag: "startup")

log.flush(sync: true)    // the records are on disk when this returns
```

```kotlin [Android]
val xlog = Xlog(
    XlogConfig(
        logDir = File(context.filesDir, "xlog/log").path,
        namePrefix = "Ham",
        level = LogLevel.INFO,
    )
)
xlog.consoleLogEnabled = BuildConfig.DEBUG

xlog.i("startup", "hello from mars")

xlog.flush(sync = true)  // the records are on disk when this returns
```

```kotlin [Kotlin Multiplatform]
Xlog.open(
    XlogConfig(
        logDir = logDirectory,
        namePrefix = "Ham",
        level = LogLevel.Info,
    )
)

Xlog.write(LogLevel.Info, "startup", "hello from mars")

Xlog.flush(sync = true)  // the records are on disk when this returns
Xlog.close()
```

```c [C]
#include <mars_xlog.h>

MarsXLogConfig config = {
    .mode = MarsAppenderAsync,
    .log_dir = "/tmp/mars-log",
    .name_prefix = "Ham",
    .compress_mode = MarsCompressZlib,
};
if (mars_xlog_open(&config) != MARS_XLOG_OK) { /* see the return code */ }

mars_xlog_write(MarsLevelInfo, "startup", __FILE__, __func__, __LINE__, "hello from mars");

mars_xlog_flush_sync();  /* the records are on disk when this returns */
mars_xlog_close();
```

:::

## 3. Find the files

An appender writes `<namePrefix>_YYYYMMDD.xlog` into the log directory you gave
it — `/tmp/mars-log/Ham_20260927.xlog` above. The default mode is async, so a
record can sit in the cache for a moment: **flush before you read or upload**, and
again before the process goes away. See [log files](/log-files).

## Where to go next

- [Configuration](/configuration) — every option and its default.
- [Log files](/log-files) — where they are, what they are called, how to read
  them back.
- The page of [your platform](/platforms/rust) — the full API, the levels and
  the modes, and what else the package carries.
