# mars, in Rust

Rust implementation of [Tencent/mars](https://github.com/Tencent/mars): the
**xlog** logging pipeline, the **STN** task model and the **SDT** network
diagnosis. xlog is the half that is pinned to a file format — it writes and
reads exactly the `.xlog` the C++ implementation produced, encrypted or plain,
zlib- or zstd-compressed, sync or async.

**[Documentation — orangeboychen.github.io/mars-rs](https://orangeboychen.github.io/mars-rs/)**

## The crates

| crate                 | what it is                                                        |
|-----------------------|-------------------------------------------------------------------|
| `marsrs`           | the whole port, re-exported: `xlog`, `stn`, `sdt`, `comm`, `bytes` |
| `marsrs-xlog`      | xlog alone — the pair's other half, for an app that only logs      |
| `marsrs-core`      | block buffer, log file framing, zlib/zstd helpers                 |
| `marsrs-comm`      | common utilities: `strutil`, `tickcount`, thread, message queue, alarm |
| `marsrs-stn`       | the task model and the anti-avalanche / dynamic-timeout policies   |
| `marsrs-sdt`       | the network diagnosis: check profiles, the plan a mode turns into  |
| `marsrs-crypt`     | ECDH + AES-GCM record encryption                                  |
| `marsrs-buffer`    | the mmap append buffer (`LogZlibBuffer` / `LogZstdBuffer`)        |
| `marsrs-appender`  | the process-wide appender and per-instance loggers                |
| `marsrs-ffi`       | C ABI (`cdylib` + `staticlib`): `mars_xlog.h`, `mars_sdt.h` behind the `sdt` feature and `mars_stn.h` behind the `stn` one |
| `marsrs-jni`       | JNI bindings of `io.github.orangeboychen.marsrs`: `Xlog`, `StnLogic`, `SdtLogic`|
| `marsrs-compat`    | CLI plus the golden `.xlog` files that pin the wire format        |

The two at the top of that table are the crates a Rust caller depends on:
`marsrs`, the whole port, and `marsrs-xlog`, xlog alone — the pair the C++
project publishes as `mars-core` and `mars-xlog`, and this one as `marsrs` and
`marsrs-xlog`.

## Install

Every release ships a package per platform:

```bash
cargo add marsrs          # the whole port: xlog, stn and sdt
cargo add marsrs-xlog     # xlog alone
```

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")
.product(name: "marsrs-xlog", package: "mars-rs")
```

```kotlin
// build.gradle.kts — settings.gradle.kts: maven { url = uri("https://jitpack.io") }
implementation("io.github.orangeboychen:mars-rs:0.1.0")       // the whole port
implementation("io.github.orangeboychen:mars-rs-xlog:0.1.0")  // xlog alone
```

```kotlin
// build.gradle.kts of the shared module of a Kotlin Multiplatform project
implementation("io.github.orangeboychen:mars-rs-kmp:0.1.0")
implementation("io.github.orangeboychen:mars-rs-xlog-kmp:0.1.0")
```

## Documentation

The site is where the rest of it lives — it is built from `docs/` and published
to GitHub Pages on every push to `main` that touches it:

- [Getting started](https://orangeboychen.github.io/mars-rs/getting-started) —
  the dependency and the snippet for each platform.
- [Platforms](https://orangeboychen.github.io/mars-rs/platforms/rust) — Rust,
  SwiftPM, Android, Kotlin Multiplatform, the C ABI, HarmonyOS.
- [The .xlog format](https://orangeboychen.github.io/mars-rs/format/) — the 16
  golden files that pin it, and the cross-check against the C++ encoders.
- [What a record costs](https://orangeboychen.github.io/mars-rs/performance) —
  nanoseconds per record against upstream, and why.
- [Build, test and lint](https://orangeboychen.github.io/mars-rs/project/build) —
  the four commands and the three lint gates.
- [What a release ships](https://orangeboychen.github.io/mars-rs/project/packages).

## Build and test

```bash
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

The documentation site:

```bash
cd docs && npm ci && npm run dev    # http://localhost:5173/mars-rs/
```

## License

MIT, like the upstream project — see [LICENSE](LICENSE).
