# mars, in Rust

[![Rust](https://github.com/orangeboyChen/mars-rs/actions/workflows/rust.yml/badge.svg)](https://github.com/orangeboyChen/mars-rs/actions/workflows/rust.yml) [![codecov](https://codecov.io/gh/orangeboyChen/mars-rs/graph/badge.svg?branch=main)](https://codecov.io/gh/orangeboyChen/mars-rs) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Rust implementation of [Tencent/mars](https://github.com/Tencent/mars), the
mobile library WeChat runs on. It is the same three pieces the C++ project is:

| piece | what it is |
|---|---|
| **xlog** | the logging pipeline — a logger that writes off the thread that called it |
| **STN** | the task model: a request with a channel, a timeout and a retry behind it |
| **SDT** | the network diagnosis: the probes that say which end a connection broke at |

One Rust core writes all three, and a package per platform hands that core to an
app that is not Rust. What the logger writes is the `.xlog` the C++
implementation writes — same framing, same compression, same encryption — so the
decoders that came with upstream, and any tooling built on them, read these files
without a conversion step.

**Documentation** — [English](https://orangeboychen.github.io/mars-rs/) ·
[简体中文](https://orangeboychen.github.io/mars-rs/zh/) — install, configure,
write, and read the files back. The site is one tab per piece: the logger,
[the task pipeline](https://orangeboychen.github.io/mars-rs/stn/getting-started)
and [the network
diagnosis](https://orangeboychen.github.io/mars-rs/sdt/getting-started). Not every
package carries the last two — [use it](#use-it) says which do.

## Where it runs

| your app is | what you take | where it comes from |
|---|---|---|
| Rust | `marsrs` / `marsrs-xlog` | [crates.io](https://crates.io) |
| iOS 12+, watchOS 10+, Swift or Objective-C | the `MarsRSXlog` product or pod | SwiftPM or CocoaPods, from this repository |
| Android, Kotlin or Java | `xlog` / `marsrs` | [JitPack](https://jitpack.io) |
| Kotlin Multiplatform | `xlog-kmp` / `marsrs-kmp` | GitHub Packages, or the release's `marsrs-kmp-maven.zip` |
| Flutter | `marsrs_xlog` / `marsrs` | [pub.dev](https://pub.dev), or the release's `marsrs-flutter-xlog-<version>.tar.gz` |
| React Native 0.74+ | `marsrs-react-native-xlog` / `marsrs-react-native` | [npm](https://www.npmjs.com), or the release's `marsrs-react-native-xlog-<version>.tgz` |
| anything with a C FFI | the `marsrs-<version>-<host>` archive | the release: Linux, macOS and Windows hosts |
| HarmonyOS | `marsrs-harmonyos-xlog`, or the three `.so` of `marsrs-harmony-<version>.tar.gz` | the release — ohpm is not switched on yet |

The Kotlin Multiplatform package is the widest of them: the same calls in shared
code write the same file on Android, iOS, watchOS, tvOS, macOS, Linux and
Windows, each platform compiling the half that reaches this core — JNI on
Android, cinterop over the C ABI everywhere else. HarmonyOS has both ways in:
`marsrs-harmonyos-xlog` is the package, a HAR an app installs and writes ArkTS
through, and `marsrs-harmony-<version>.tar.gz` is the three `.so` and the header
for an app that would rather write NAPI of its own.

Every release also ships the `xlog` CLI, which writes and reads those files from
a shell and makes the key pair that decides who can — `cargo install marsrs-xlog`,
`xlog keygen`.

## Use it

Two ways in, and the difference is how much of the port you take: **xlog alone**,
or **the whole port** — xlog, STN and SDT. An app that only logs takes the first;
the pair is the same one the crates on crates.io are. Two of the packages carry
the logger under both names without the other two pieces — Flutter's `marsrs` and
`marsrs-react-native` have no STN or SDT surface yet, so on those two the choice
is one of name and not of contents. Everywhere else the wider name is the wider
port: `marsrs` on Android, `marsrs-kmp` in shared Kotlin and the `MarsRS` product
on Apple are where an app reaches the task pipeline and the diagnosis, and the C
archive ships both headers beside the libraries.

```bash
cargo add marsrs-xlog    # xlog alone — the logger and nothing else
cargo add marsrs         # the whole port: + STN, SDT
```

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")
.product(name: "MarsRSXlog", package: "mars-rs")   // or "MarsRS", for both halves
```

```ruby
# Podfile — Swift or Objective-C
pod 'MarsRSXlog', :podspec => 'https://raw.githubusercontent.com/orangeboyChen/mars-rs/v0.1.0/MarsRSXlog.podspec'
```

```kotlin
// settings.gradle.kts: maven { url = uri("https://jitpack.io") }
implementation("io.github.orangeboychen.marsrs:xlog:0.1.0")    // xlog alone
implementation("io.github.orangeboychen.marsrs:marsrs:0.1.0")  // + STN, SDT

// build.gradle.kts of a Kotlin Multiplatform shared module
implementation("io.github.orangeboychen.marsrs:xlog-kmp:0.1.0")
implementation("io.github.orangeboychen.marsrs:marsrs-kmp:0.1.0")
```

```bash
ohpm install marsrs-harmonyos-xlog    # HarmonyOS — ArkTS; xlog only, for now
```

Then the same three steps on every platform: open an appender once when the app
starts, write through it, and flush before you read or upload its files.

```rust
use marsrs::xlog::{appender_close, appender_flush_sync, appender_open, appender_write, XLogConfig};

let mut config = XLogConfig::default();
config.logdir = std::path::PathBuf::from("/tmp/mars-log");
config.nameprefix = "marsrs".to_owned();
appender_open(config)?;

appender_write(None, "hello from mars");

appender_flush_sync();   // the records are on disk when this returns
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

xlog.flush(sync = true)  // before the app reads or uploads the files
```

The file is `<logDir>/<namePrefix>_YYYYMMDD.xlog` — `marsrs_20260927.xlog` above.
The default mode is async, so a record can sit in the cache for a moment: flush
before the file is read or uploaded. Every option, and its name on each platform,
is on [the configuration page](https://orangeboychen.github.io/mars-rs/xlog/configuration);
what has to happen when the app goes away is on
[log files](https://orangeboychen.github.io/mars-rs/xlog/log-files).

## License

MIT, like the upstream project — see [LICENSE](LICENSE).
