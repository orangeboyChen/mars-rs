# mars, in Rust

Rust implementation of [Tencent/mars](https://github.com/Tencent/mars): the
**xlog** logging pipeline, the **STN** task model and the **SDT** network
diagnosis. xlog is the half that is pinned to a file format — it writes and
reads exactly the `.xlog` the C++ implementation produced, encrypted or plain,
zlib- or zstd-compressed, sync or async — and this is the whole stack:

| crate                 | what it is                                                        |
|-----------------------|-------------------------------------------------------------------|
| `mars-core`      | block buffer, log file framing, zlib/zstd helpers                 |
| `mars-comm`      | common utilities: `strutil`, `tickcount`, thread, message queue, alarm |
| `mars-stn`       | the task model and the anti-avalanche / dynamic-timeout policies   |
| `mars-sdt`       | the network diagnosis: check profiles, the plan a mode turns into  |
| `mars-crypt`     | ECDH + AES-GCM record encryption                                  |
| `mars-buffer`    | the mmap append buffer (`LogZlibBuffer` / `LogZstdBuffer`)        |
| `mars-appender`  | the process-wide appender and per-instance loggers                |
| `mars-ffi`       | C ABI (`cdylib` + `staticlib`) and its hand-written header        |
| `mars-jni`       | JNI bindings of `io.github.orangeboychen.marsrs`: `Xlog`, `StnLogic`, `SdtLogic`|
| `mars-compat`    | CLI plus the golden `.xlog` files that pin the wire format        |

## Build and test

```bash
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Cross-compiling needs the usual toolchain plus, for Android, an NDK (only for
the linker):

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
export ANDROID_HOME=...            # .cargo/config.toml forces 16 KiB pages
cargo build --release -p mars-jni --target aarch64-linux-android
```

`unsafe` appears in exactly three places: the C ABI shims of `mars-ffi`,
the single `memmap2::MmapOptions::map_mut` call of `mars-appender` (there
is no safe API for creating a mapping) and the one `JString::from_raw` of
`mars-jni`, which re-wraps a borrowed local ref. All three carry SAFETY notes.

## Lint

One gate per language, and each of them fails on the first finding.
`.github/workflows/lint.yml` runs the Swift and the Kotlin gate on every push
and pull request; `rust.yml` runs the Rust one.

| Language | Tools | Configuration |
| --- | --- | --- |
| Rust | `rustfmt`, `clippy` | `rustfmt.toml`, `clippy.toml`, `[workspace.lints]` of `Cargo.toml` |
| Swift | SwiftLint 0.65.1 | `.swiftlint.yml` |
| Kotlin | ktlint 1.8.0, detekt 1.23.8 | `.editorconfig`, `detekt.yml` |

```bash
# Swift: `--strict` turns every warning into an error.
swiftlint lint --strict

# Kotlin: the style and the line length of .editorconfig, then the analysis
# detekt.yml configures on top of detekt's own defaults.
ktlint --relative 'android/**/*.kt' 'android/**/*.kts'
java -jar detekt-cli-1.23.8-all.jar \
  --input android/mars-core/src/main/kotlin,android/mars-xlog/src/main/kotlin \
  --config detekt.yml --build-upon-default-config
```

The tool versions are pinned, and each of the three configurations says what it
turns on beyond the tool's own defaults and why — `detekt.yml` in particular is
an override file: every rule it names is either a threshold the default sets too
low or one the port cannot satisfy without changing what it does, and it says
which. Every finding is answered in the source or it fails the job, so there is
no suppression comment anywhere: the constants of the AAR's Kotlin are spelled
the way Kotlin spells a constant — `kPingCheck` of the C++ project's Java is
`K_PING_CHECK` here — rather than kept under a `ktlint-disable`, and the JNI
reaches a constant by the number it carries, not by its name. `detekt.yml` is
the only place a rule a tool turns on by default is off, and each of the seven
says why. The ten crates are held to the same rule: no `#[allow]` answers a lint
in them, and the one `#[allow]` left in the tree is `unsafe_code` on the single
`mmap` of `mars-appender` — a crate that denies `unsafe_code` outright, with the
invariant the mapping needs argued beside it.

## The format is pinned by golden files

The 16 `.xlog` files in `crates/mars-compat/fixtures` were written by the
*original C++* encoders — one per combination of zlib/zstd, sync/async,
encryption on/off and flush policy — and `cargo test -p mars-compat`
decodes every one of them back to the exact text that went in. That is what
keeps the port readable/writable against files produced by the C++ it replaced.

Two differences are known and accepted:

* **zlib** — the port uses `zlib-rs`, whose output is not byte-identical to
  system zlib above a few KB (still decodable by both).
* **zstd** — the C++ build vendored 1.4.4, this one uses the `zstd` crate
  1.5.x, so compressed sizes differ.

## Consumers

Every release ships a package per platform; `.github/workflows/release.yml`
builds them when it is run by hand (Actions → Release → Run workflow) with a
version, or with a bump and a channel — `v1.2.3-alpha.1`, `v1.2.3-beta.2`,
`v1.2.3`. Anything with a suffix is published as a GitHub pre-release.

### SwiftPM

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")
```

Two products, the pair the Android packages make of `mars-rs` and
`mars-rs-xlog`: `MarsRS` is the whole port, `MarsRSXlog` is the logging half
of it, for an app that only logs. Both are Swift over the `MarsRSFFI` binary
target — the static `MarsRS.xcframework.zip` published with the release — so
taking the smaller one drops nothing but the promise of STN and SDT. The tag
and the SPM checksum are written into `Package.swift` by the release workflow
on a `chore/package-swift-<tag>` branch, proposed as a pull request.

`Sources/MarsRSXlog/Xlog.swift` is what the port exposes today: `mars-ffi` is
an xlog C ABI (21 `mars_xlog_*` symbols, nothing else), so xlog is all the Swift
layer can reach and `MarsRS` re-exports `MarsRSXlog` and nothing more.
`Stn.swift` and `Sdt.swift` join the umbrella when the C ABI carries them —
that is why `MarsRS` exists as a module of its own and not just as a name for
xlog. orangeboyChen/mars ships a single `MarsXlog` product for the same reason;
here the xlog-only import is `import MarsRSXlog`.

### Android (JitPack)

Two AARs over the same `libmarsxlog.so` — the pair the C++ project publishes
as `mars-core` and `mars-xlog`:

```kotlin
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen:mars-rs:0.1.0")       // the whole port
implementation("io.github.orangeboychen:mars-rs-xlog:0.1.0")  // xlog alone
```

| AAR | artifact | what is in it |
|---|---|---|
| `mars-core.aar` | `mars-rs` | every Kotlin class whose natives `mars-jni` exports — `xlog`, `stn`, `sdt`, `app`, `comm` and `BaseEvent`/`Mars` — plus `libmarsxlog.so` for `arm64-v8a`, `armeabi-v7a` and `x86_64` |
| `mars-xlog.aar` | `mars-rs-xlog` | `xlog.Xlog` and the `xlog.Log` facade over it, plus the same `libmarsxlog.so` |

Take `mars-rs` when you want STN or SDT, `mars-rs-xlog` when the app only logs;
both carry the whole library, because there is one `.so` and it is not split.
The Kotlin and Java package is `io.github.orangeboychen.marsrs`, the package
whose natives `mars-jni` exports, so the two are renamed together. The AAR's face
is Kotlin — `Xlog`, `Log`, `StnLogic`, `SdtLogic` and the rest — written so that
an app in Java sees the same statics the C++ project's Java had. A repository is also
reachable on JitPack as `com.github.<owner>.<repo>`, which is the spelling its
badge prints; the release workflow asks jitpack.io to build the tag under both,
reports which one answered, and then checks that both AARs resolve.

JitPack has an Android SDK but neither an NDK nor a Rust toolchain, so it
downloads `mars-android-native.zip` of the same release first — see
`jitpack.yml`.

### The C ABI

`mars-rs-<version>-<host>.tar.gz` (Linux, macOS) and `.zip` (Windows) hold
`include/mars_xlog.h` and the static and shared libraries of `mars-ffi`, for
`x86_64-unknown-linux-gnu`, `aarch64-apple-darwin` and
`x86_64-pc-windows-msvc`.

### Building the packages

```bash
scripts/build_xcframework.sh 0.1.0 dist   # MarsRS.xcframework.zip
scripts/build_android.sh dist/native      # <abi>/libmarsxlog.so
# the .so of dist/native has to be under android/<module>/libs first
(cd android && ./gradlew :mars-core:assembleRelease :mars-xlog:assembleRelease)
```

## License

MIT, like the upstream project — see [LICENSE](LICENSE).
