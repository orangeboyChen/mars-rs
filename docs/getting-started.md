# Getting started

Every release ships a package per platform; take the one the app is built with.
The two crates a Rust caller depends on are `marsrs`, the whole port, and
`marsrs-xlog`, xlog alone; on every other platform the same pair is spelled the
way that platform spells a dependency.

| platform | what to take | where the details are |
|---|---|---|
| Rust | `marsrs` / `marsrs-xlog` | [Rust](/platforms/rust) |
| Apple (SwiftPM) | `marsrs` / `marsrs-xlog` / `marsrs-net` | [SwiftPM](/platforms/swift) |
| Android (JitPack) | `io.github.orangeboychen:mars-rs` / `:mars-rs-xlog` | [Android](/platforms/android) |
| Kotlin Multiplatform | `io.github.orangeboychen:mars-rs-kmp` / `:mars-rs-xlog-kmp` | [Kotlin Multiplatform](/platforms/kotlin-multiplatform) |
| anything with a C FFI | `mars-rs-<version>-<host>.tar.gz` / `.zip` | [The C ABI](/platforms/c-abi) |
| HarmonyOS | `libmars_ffi.so` built from source today | [HarmonyOS](/platforms/harmonyos) |

## Rust

```bash
cargo add marsrs          # the whole port: xlog, stn and sdt
cargo add marsrs-xlog     # xlog alone
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

## Swift

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")
// and, in the target that takes it:
.product(name: "marsrs-xlog", package: "mars-rs")
```

```swift
import MarsRSXlog

let log = try Xlog(XlogConfig(logDirectory: dir))
log.info(message: "hello", tag: "Net")
```

## Android

```kotlin
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen:mars-rs-xlog:0.1.0")  // xlog alone
implementation("io.github.orangeboychen:mars-rs:0.1.0")       // the whole port
```

```kotlin
val xlog = Xlog(
    XlogConfig(
        logDir = File(context.filesDir, "xlog/log").path,
        cacheDir = File(context.filesDir, "xlog/cache").path,
        namePrefix = "Ham",
        level = LogLevel.INFO,
        mode = AppenderMode.ASYNC,
    )
)
xlog.i("startup", "cold start in $elapsedMillis ms")
```

## Kotlin Multiplatform

```kotlin
// in commonMain of the shared module
implementation("io.github.orangeboychen:mars-rs-xlog-kmp:0.1.0")
```

The `androidMain` of the module talks to the JNI bridge of `marsrs-jni` and the
`nativeMain` of it talks to the C ABI of `marsrs-ffi` through cinterop, so one
dependency in `commonMain` compiles its own half on every platform.

## C

```c
#include <mars_xlog.h>
```

`mars-rs-<version>-<host>.tar.gz` (Linux, macOS) and `.zip` (Windows) hold
`include/mars_xlog.h`, `include/mars_sdt.h`, `include/mars_stn.h` and the
static and shared libraries of `marsrs-ffi`.

## What the app pays for a record

A record is written on the thread that logs it, so what one record costs is what
the app pays on its hot path: 830 ns against upstream's 2038 for
`append sync/zlib/1t`. The whole table, and what is behind it, is on
[the performance page](/performance).
