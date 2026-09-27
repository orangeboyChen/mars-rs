# mars, in Rust

Rust implementation of [Tencent/mars](https://github.com/Tencent/mars): the
**xlog** logging pipeline, the **STN** task model and the **SDT** network
diagnosis. xlog is the half that is pinned to a file format — it writes and
reads exactly the `.xlog` the C++ implementation produced, encrypted or plain,
zlib- or zstd-compressed, sync or async — and this is the whole stack:

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
project publishes as `mars-core` and `mars-xlog`, and this one as `MarsRS` and
`MarsRSXlog`. The seven between them and `marsrs-ffi` are implementation details
of the pair, and are on crates.io only because a published crate cannot depend
on a crate that is not: crates.io resolves a dependency out of the registry and
not out of a path.

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
cargo build --release -p marsrs-jni --target aarch64-linux-android
```

`unsafe` appears in exactly three places: the C ABI shims of `marsrs-ffi`,
the single `memmap2::MmapOptions::map_mut` call of `marsrs-appender` (there
is no safe API for creating a mapping) and the one `JString::from_raw` of
`marsrs-jni`, which re-wraps a borrowed local ref. All three carry SAFETY notes.

## Lint

One gate per language, and each of them fails on the first finding:
`.github/workflows/lint-swift.yml` and `lint-kotlin.yml` run the Swift and the
Kotlin gate, `rust.yml` runs the Rust one. Every run starts, but a job of it
skips its work when the commit touches nothing that job reads: a pull request
with no Swift in it does not pay for a macOS runner, and one with no Kotlin in
it does not pay for a JDK. The filter is on the jobs and not on the run because
a workflow that never starts reports no status at all, and the checks the
ruleset requires would be left waiting for one — as #108 was, for two days —
while a skipped job answers them.

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
says why. The twelve crates are held to the same rule: no `#[allow]` answers a lint
in them, and the one `#[allow]` left in the tree is `unsafe_code` on the single
`mmap` of `marsrs-appender` — a crate that denies `unsafe_code` outright, with the
invariant the mapping needs argued beside it.

## The format is pinned by golden files

The 16 `.xlog` files in `crates/marsrs-compat/fixtures` were written by the
*original C++* encoders — one per combination of zlib/zstd, sync/async,
encryption on/off and flush policy — and `cargo test -p marsrs-compat`
decodes every one of them back to the exact text that went in. That is what
keeps the port readable/writable against files produced by the C++ it replaced.

Two differences are known and accepted:

* **zlib** — the port uses `zlib-rs`, whose output is not byte-identical to
  system zlib above a few KB (still decodable by both).
* **zstd** — the C++ build vendored 1.4.4, this one uses the `zstd` crate
  1.5.x, so compressed sizes differ.

## The two implementations read each other's files

`cargo test -p marsrs-compat` proves one half of that: the 16 `.xlog` files under
`fixtures/` were written by the C++ encoders and the Rust decoder reads every one
back. The other half — a file this port wrote, read by the C++ — needs the C++
in the tree, so it lives in a script instead of a test:

```bash
sh scripts/compat/cross.sh
```

It builds upstream's own decoder (`mars/xlog/crypt/decode_log_file_c_impl/
decode_log_file.c`, patched with the fixtures' ECDH keys) and an encoder
(`scripts/compat/upstream_encode.cpp`) that mirrors `xlog-compat encode`
option for option, then runs the same 16 combinations — zlib/zstd × sync/async ×
crypt on/off × flush policy — in both directions: encode here and decode there,
encode there and decode here, byte for byte against `fixtures/expected.bin`.
It then does the same through the real appenders, so what is decoded is a
`<prefix>_YYYYMMDD.xlog` an app would ship and not just the block bytes.

| layer | rust → C++ | C++ → rust |
|---|---|---|
| record layer, 16 combinations | 12 ok, 2 `cpp-decoder`, 2 fail | 16 ok |
| appender layer, 8 combinations | 8 ok | 8 ok |

`cpp-decoder` is two of the four `zstd-async` cases, and it is not a difference
between the encoders: upstream's decoder reads both encoders' `zstd-async`
files the same wrong way — 4501 of 4510 bytes with one flush — while the Rust
decoder reads both whole. The script runs that control itself and says so
instead of failing, but only when the two wrong outputs are the *same* wrong
bytes; a Rust file that decodes to something else fails.

The two that fail are the `zstd-async` cases that flush once per record. There
the Rust encoder's zstd frame for the 4096-byte record is one byte shorter than
upstream's — one raw literal before the match where upstream writes two, two
spellings of the same 4096 bytes, which the Rust decoder reads back whole from
both files. It only shows up because upstream's decoder returns 17 and 18 bytes
instead of 4096 for that block, so the two files come out of it 431 and 432
bytes long. Everything else — all 8 appender combinations, including async and
zstd — is byte for byte.

Two things the run turns up that are worth knowing:

* upstream's `decode_log_file.c` **does not compile as it stands** —
  `zstdDecompress` reads a `lastPos` it never declares (line 212). The build
  patches one `size_t` into its copy under `target/`.
* the Rust appender's `appender_write` takes a `&str`, so a record that is not
  UTF-8 cannot be logged through it; the C++ `xlogger_Write` takes bytes. The
  appender layer of the test checks the records that are UTF-8 and the record
  layer carries the rest through both encoders unchanged.

## What a record costs, measured against the C++

xlog writes a record on the thread that logs it, so what one record costs is
what the app pays on its hot path. The table below is one measurement of it: one
harness per implementation, the same scenarios and the same constant payload on
both, a discarded warm-up run per scenario and then five timed ones, the fastest
of the five reported. Neither harness is in this tree — the C++ half needs a
clone of upstream `Tencent/mars` to build at all, and what they compare is two
code paths, not a property of this repository. Both hand the logger a payload
that is already formatted, because what is being compared is the logging path —
header, compression, crypt, file — and not `snprintf` against `format!`.

The scenarios are one record through `log_formater` into a 16 KiB buffer, and
`appender_open` → *N* threads × 20 000 `appender_write`s →
`appender_flush_sync` → `appender_close` for every combination of sync/async,
zlib/zstd and 1/8 threads. Nanoseconds per record, macOS/arm64, one run of
each harness (run-to-run spread is about 10%):

| scenario | Tencent/mars (C++) | mars-rs | rust is |
|---|---|---|---|
| format one record | 519 ns | 104 ns | 4.96x faster |
| append sync/zlib/1t | 2038 ns | 830 ns | 2.45x faster |
| append sync/zlib/8t | 3708 ns | 2111 ns | 1.76x faster |
| append sync/zstd/1t | 2180 ns | 1052 ns | 2.07x faster |
| append sync/zstd/8t | 3929 ns | 2198 ns | 1.79x faster |
| append async/zlib/1t | 2793 ns | 1654 ns | 1.69x faster |
| append async/zlib/8t | 4818 ns | 2979 ns | 1.62x faster |
| append async/zstd/1t | 1700 ns | 848 ns | 2.01x faster |
| append async/zstd/8t | 3295 ns | 2570 ns | 1.28x faster |

Async is the *slower* of the two modes per record, in both implementations: a
record that goes to the cache is compressed into the mapping, copied back out of
it and handed to another thread, where the sync path writes it straight through.
What async buys is that the `write` syscall is not on the logging thread, which
is worth it when the file is the bottleneck and not when the record is.

### What the C++ spends a record on that the port does not

| | upstream `Tencent/mars` | this port |
|---|---|---|
| local time | **four `localtime` calls per record** — `formater.cc:90`, both halves of the day check in `__OpenLogFile` (`appender.cc:750-751`, one of them on `openfiletime_`, which never changes) and, with crypt on, `log_crypt.cc:191` | one per **second**, cached per thread (`marsrs_core::localtime`) |
| the scratch buffer | `char temp[16 * 1024] = {0}` per call — 16 KiB of stores — in `__WriteSync` / `__WriteAsync` (`appender.cc:960`, `972`) | one `Vec` per thread, grown once (`RECORD`) |
| the thread id | `syscall(SYS_gettid)` per record (`comm/unix/xlogger_threadinfo.cc:39`); only the pid next to it is a cached static | cached per thread, keyed on the pid so that a `fork` re-reads it (`sys.rs`) |
| the open appender | `sg_default_appender`, read with no lock at all | a per-thread `Arc` tagged with a generation counter: no lock per record, and no shared atomic write either |
| locking | `mutex_buffer_async_` and `mutex_log_file_`, taken several times per flush | one `Mutex` plus one bounded channel to the writer thread |
| file writes | stdio `FILE*` buffering: a failed `fwrite` has already consumed the batch | an explicit `pending` batch, which a refused write keeps, so it can be retried |

The one thing both do the same way — and the reason the port's numbers are not
better still — is that neither formats under a lock: the record is built on the
calling thread and only its bytes cross into the shared state.

### The optimizations behind the table

Measured the same way, against the port as it stood before them:
format 360 → 104 ns, `append sync/zlib/1t` 1150 → 830 ns, at the other end
`append async/zlib/8t` 3259 → 2979 ns. In the order they pay:

1. **`marsrs_core::localtime`** — a per-thread snapshot of the second (offset,
   hour, date) that every caller of `localtime` in a record shares. This is the
   one that moves the needle: it is what turns four conversions per record into
   one per second.
2. **The timestamp text is rendered once per second** — `"yyyy-mm-dd +8.0
   hh:mm:ss."` is decided by `tv_sec` alone, so a second of logging renders it
   once instead of paying six integer conversions and a float per record
   (`LocalStamp` in `formater.rs`).
3. **Integers are written without `write!`** — `push_decimal` / `push_signed`
   put digits into the buffer directly, which is what a `Formatter` cannot do
   without a `Display` impl per integer width.
4. **Local fields by integer arithmetic** — `DateTime::from_timestamp(secs +
   gmtoff, 0)` *is* the local time, and costs ~2 ns where `Local` is a time-zone
   lookup.
5. **The day the file was opened on is stamped once** — `open_file_day`, so the
   per-record roll-over check compares against a cached `local_time` of today
   instead of converting two timestamps, one of which is a constant.
6. **The open appender is cached per thread** — a generation counter says
   whether the cached `Arc` is still current, so a record costs one shared
   atomic *load* instead of a turn through a mutex every logging thread in the
   process contends for.
7. **`flush_sync` takes the lock once** — it took it five times for one flush,
   and nothing in between was observable by anyone else.
8. **`RecursionGuard::drop` uses `try_with`** — no `catch_unwind` around a
   decrement that cannot panic, on the way out of every single record.

Three things were measured and left alone on purpose: `LOG_FLUSH_THRESHOLD`
stays at 4 KiB (it is `st_blksize`, and a bigger batch trades durability for
throughput — a `write` of 4 KiB is ~5 µs, half a sync record); the crypt path's
`localtime` is not threaded through (`set_header_info` would have to take the
second, for ~8%); and `LogBuffer::clear` keeps its 150 KiB `memset`, which is
what the C++ does before unmapping.

## Consumers

Every release ships a package per platform; `.github/workflows/release.yml`
builds them when it is run by hand (Actions → Release → Run workflow) with a
version, or with a bump and a channel — `v1.2.3-alpha.1`, `v1.2.3-beta.2`,
`v1.2.3`. Anything with a suffix is published as a GitHub pre-release.

### Rust (crates.io)

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

### SwiftPM

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")
```

Three products: `MarsRS` is the whole port, `MarsRSXlog` is the logging half of
it and `MarsRSNet` is the half that is not logging — the diagnosis and the task
pipeline — for an app that runs tasks and does not log. The two halves are two
artifacts and not one library split at the Swift layer: `MarsRSXlog` is Swift
over the `MarsRSFFI` binary target, `MarsRSNet` over `MarsRSNetFFI`, and each is
a static xcframework published with the release.

What either one holds is the feature set it is named for.
`scripts/build_xcframework.sh` builds `-p marsrs-ffi --no-default-features
--features xlog` for the first and `--no-default-features --features sdt,stn`
for the second — SDT and STN reach the C ABI behind features of their own, and a
set that is spelled out does not inherit them — and then fails if a slice of
either one exports a symbol of the port's that is not its own. That is what makes
the split worth having: an app that only logs downloads xlog's bytes and nothing
else, and an app that takes both links xlog's symbols exactly once. Two Rust
static libraries out of one crate do link side by side, which is not obvious —
the symbols they share are the crate's own, and rustc emits those private extern,
so the second copy is not a duplicate definition.

That is also how the C++ project answers it — orangeboyChen/mars publishes one
SPM product, `MarsXlog`, over a `MarsXlog.xcframework` built from the xlog subset
of its sources, and its Android build has an `--xlog-only` mode for the same
reason — and it is why the xlog artifact is named after xlog here and not after
the port. The tag and the SPM checksums are written into `Package.swift` by the
release workflow on a `chore/package-swift-<tag>` branch, proposed as a pull
request; there is one checksum per binary target, because a checksum is bound to
the zip it was computed from. A module is what a consumer imports, so the xlog
one keeps the name `MarsRSFFI` until a release carries the renamed asset — it
cannot be renamed under one — while the net one is new and named after itself.

The framework carries four slices — `ios-arm64`, `ios-arm64_x86_64-simulator`,
`watchos-arm64_arm64_32` and `watchos-arm64-simulator` — so an app target of
either platform resolves it. The watchOS device slice holds two architectures:
`arm64_32`, which is what a watch running watchOS 10 to 25 links, and `arm64`,
which is what watchOS 26 moved its watches onto. `arm64_32` is a tier 3 target
no channel ships a std for, so it is built out of a nightly's sources with
`-Z build-std`; the arm64 half needs no such thing, and is built for
watchOS 26 because its std is. `x86_64-apple-watchos-sim` is left out — a
watchOS simulator is arm64.

What an app pays for the xlog framework is about 1.0 MB of `__TEXT` on arm64,
measured by linking a slice into an otherwise empty executable with
`-dead_strip`: the 21 MB archive of a slice is the shelf the linker picks from,
not what lands in the app. Each archive is stripped of its local symbols before
it is packaged — 31 % off the zip a consumer downloads, and nothing off the
link, because every symbol the artifact is named for is an external symbol and
stays.

`Sources/MarsRSXlog/Xlog.swift` and `Sources/MarsRSNet/` are what the port
exposes: the Swift over the 28 `mars_xlog_*` symbols, and the Swift over the
`mars_sdt_*` and `mars_stn_*` of the diagnosis and the task pipeline —
`MarsSdt`, `MarsStn` and the `StnTask`, `StnQuestion` and `StnAnswer` they ask
and answer with. `MarsRS` is the module that re-exports both halves, which is
why it exists as a module of its own and not just as a name for xlog.
orangeboyChen/mars ships a single `MarsXlog` product because xlog is all its
package has; here the xlog-only import is `import MarsRSXlog` and the net-only
one is `import MarsRSNet`.

An app writes through an `Xlog` of its own — `let log = try Xlog(XlogConfig(
logDirectory: dir))`, then `log.info(message: "hello", tag: "Net")` — the two
steps the Android `Xlog` is, with `XlogConfig`, `LogLevel`, `AppenderMode` and
`CompressMode` spelled the way the Kotlin API spells them, so one app reads the
same either way. `file`, `function` and `line` of a record come from the call
site: `#file` costs Swift nothing, and the C ABI carries them anyway. Nothing
here is deprecated, because there is no older Swift API to keep — the
process-wide appender is a set of C symbols, and `@_exported import MarsRSFFI`
reaches them.

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
| `mars-core.aar` | `mars-rs` | every Kotlin class whose natives `marsrs-jni` exports — `xlog`, `stn`, `sdt`, `app`, `comm` and `BaseEvent`/`Mars` — plus `libmarsxlog.so` for `arm64-v8a`, `armeabi-v7a` and `x86_64` |
| `mars-xlog.aar` | `mars-rs-xlog` | `xlog.Xlog` — with `XlogConfig`, `LogLevel`, `AppenderMode` and `CompressMode` — and the `xlog.Log` facade over it, plus the same `libmarsxlog.so` |

Take `mars-rs` when you want STN or SDT, `mars-rs-xlog` when the app only logs;
both carry the whole library, because there is one `.so` and it is not split.
The Kotlin and Java package is `io.github.orangeboychen.marsrs`, the package
whose natives `marsrs-jni` exports, so the two are renamed together. The AAR's face
is Kotlin — `Xlog`, `Log`, `StnLogic`, `SdtLogic` and the rest — written so that
an app in Java sees the same statics the C++ project's Java had. A repository is also
reachable on JitPack as `com.github.<owner>.<repo>`, which is the spelling its
badge prints; the release workflow asks jitpack.io to build the tag under both,
reports which one answered, and then checks that both AARs resolve.

JitPack has an Android SDK but neither an NDK nor a Rust toolchain, so it
downloads `mars-android-native.zip` of the same release first — see
`jitpack.yml`.

### Kotlin Multiplatform

One dependency in `commonMain`, and each platform compiles its own half of it:
the `androidMain` of the module talks to the JNI bridge of `mars-jni`, the
`nativeMain` of it talks to the C ABI of `mars-ffi` through cinterop.

```kotlin
// settings.gradle.kts
maven {
    url = uri("https://maven.pkg.github.com/orangeboyChen/mars-rs")
    credentials { username = "<user>"; password = "<token with read:packages>" }
}

// build.gradle.kts of the shared module
implementation("io.github.orangeboychen:mars-rs-kmp:0.1.0")       // the whole port
implementation("io.github.orangeboychen:mars-rs-xlog-kmp:0.1.0")  // xlog alone
```

| artifact | what is in it |
|---|---|
| `mars-rs-kmp` | `mars-rs-xlog-kmp`, re-exported — the pair `mars-rs`/`mars-rs-xlog` are on Android and `MarsRS`/`MarsRSXlog` are in Swift. Its own declarations are six `typealias`es under `io.github.orangeboychen.marsrs`, one per name below: `mars-ffi` is an xlog C ABI today, so the whole port and its logging half are the same module. Taking this coordinate is what lets STN and SDT arrive without a rename. |
| `mars-rs-xlog-kmp` | `Xlog`, `XlogConfig`, `LogLevel` and the `Log` facade, over the JNI bridge on Android and over the C ABI of `mars-ffi` everywhere else |

Fourteen targets: Android — the AAR carries `libmarsxlog.so` for `arm64-v8a`,
`armeabi-v7a` and `x86_64` — plus `iosArm64`, `iosX64`, `iosSimulatorArm64`,
`macosX64`, `macosArm64`, `watchosArm64`, `watchosDeviceArm64`,
`watchosSimulatorArm64`, `tvosArm64`, `tvosSimulatorArm64`, `linuxX64`,
`linuxArm64` and `mingwX64`. The two Kotlin targets missing from the list are
the x86_64 simulators, `watchosX64` and `tvosX64`, and both are missing because
Rust has no triple for them: `x86_64-apple-watchos-sim` and `x86_64-apple-tvos`
are not targets `rustup` knows.

Nothing is compiled from Rust when the Kotlin module is built. Neither a
consumer nor a CI host has the toolchain for thirteen triples, and an Apple
archive cannot be cross-compiled from Linux at all — so the release builds them
(`scripts/build_kmp_native.sh`) and publishes `mars-kmp-native.zip`, one
`libmars_ffi.a` per Kotlin target plus the `.so` files of the JNI bridge, and
`mars-kmp-maven.zip`, which is the repository itself. An app that would rather
not authenticate to GitHub Packages takes the second: unzip it and add
`maven { url = uri("<dir>") }`.

The AARs on JitPack stay the packaging for an app that is Android only; this one
is for a `commonMain` that compiles for more than one platform.

### The Android API

Two steps: build the appender once when the app starts, then write through it
from wherever there is something to say.

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
xlog.consoleLogEnabled = BuildConfig.DEBUG

xlog.i("startup", "cold start in $elapsedMillis ms")
xlog.e("login", "login failed\n${cause.stackTraceToString()}")
```

The write is `android.util.Log`'s shape — `v`/`d`/`i`/`w`/`e`/`f`, each of them
a tag and a message, and `log(level, tag, message)` when the level is not known
until the call — so Java writes `new Xlog(config)` and `xlog.i(tag, message)`
with nothing else to learn. A record costs one JNI call: `marsrs-jni` is what
drops a record the appender's level is above, before anything is formatted, and
what fills the pid and the tid in from the OS. A message that is expensive to
build is worth an `if (xlog.isLoggable(LogLevel.DEBUG))` first — a record the
level drops costs the caller the `String` either way.

`XlogConfig` is the Kotlin face of the appender's options — `LogLevel`,
`AppenderMode` and `CompressMode` are enums over the numbers `marsrs-jni` speaks
— and it refuses a config the `.so` cannot honour here, in Kotlin, because
`marsrs-jni` answers a config it does not like by opening nothing. A part of an
app whose logs are read apart from the rest gets an appender of its own out of
a second `Xlog(XlogConfig(...))` with a `namePrefix` of its own, and
`xlog.flush(sync = true)` before the app reads or uploads its files: a record of
the default `ASYNC` mode sits in a memory-mapped cache until a writer thread
takes it to the log file.

`Xlog` also still carries what the C++ project's Java spelled — the
seven-argument `open`, `XLogConfig`, `XLoggerInfo`, `logWrite`, the `LEVEL_*`
constants and the `Log` facade — so an app that already calls any of them keeps
working, and every one of them is deprecated with the spelling that replaces
it. `Xlog.open` installs `Xlog` into `Log` as well, so the two spellings write
through one appender and land in the same file, and a migration can go one call
site at a time.

### The C ABI

`mars-rs-<version>-<host>.tar.gz` (Linux, macOS) and `.zip` (Windows) hold
`include/mars_xlog.h`, `include/mars_sdt.h`, `include/mars_stn.h` and the static
and shared libraries of `marsrs-ffi` — built `--features sdt,stn`, the task
pipeline and the diagnosis included, which is what the Android `.so` carries —
for `x86_64-unknown-linux-gnu`,
`aarch64-apple-darwin` and `x86_64-pc-windows-msvc`.

### HarmonyOS

HarmonyOS is reached through the OpenHarmony targets —
`aarch64-unknown-linux-ohos`, `armv7-unknown-linux-ohos` and
`x86_64-unknown-linux-ohos` — and what builds for them today is the C ABI:

```bash
scripts/build_harmony.sh dist/harmony   # <abi>/libmars_ffi.so
```

There is no HarmonyOS package yet, and no ArkTS: a HarmonyOS app would load
`libmars_ffi.so` and call `mars_xlog_*` through NAPI itself. The three targets
are tier 2 with host tools, so a std ships for them and the build is a stable
one; the SDK it needs is the public OpenHarmony one, which the script downloads
when `OHOS_SDK_HOME` is not set.

### Building the packages

```bash
scripts/build_xcframework.sh 0.1.0 dist   # MarsRSXlog.xcframework.zip, MarsRSNet.xcframework.zip
scripts/build_android.sh dist/native      # <abi>/libmarsxlog.so
# the .so of dist/native has to be under android/<module>/libs first
(cd android && ./gradlew :mars-core:assembleRelease :mars-xlog:assembleRelease)
# <kotlin-target>/libmars_ffi.a into kmp/native, plus dist/native's .so into
# kmp/native/android — on macOS for the Apple ones, on Linux for the rest
scripts/build_kmp_native.sh kmp/native
(cd kmp && ./gradlew :mars-core:publishToMavenLocal :mars-xlog:publishToMavenLocal)
```

`kmp` is a Gradle build of its own and not two modules of `android`, because the
two need different Android Gradle Plugins: 8.13 here, where the Kotlin plugin
still owns the `kotlin` extension an Android target needs, and 9 in `../android`.

The scripts that only a workflow runs — resolving the version, installing the
NDK on a runner, waiting for JitPack, rewriting `Package.swift`, naming the
files a commit touched — are in `.github/scripts/`.

## License

MIT, like the upstream project — see [LICENSE](LICENSE).
