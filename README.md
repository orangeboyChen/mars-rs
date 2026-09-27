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
| local time | **four `localtime` calls per record** — `formater.cc:90`, both halves of the day check in `__OpenLogFile` (`appender.cc:750-751`, one of them on `openfiletime_`, which never changes) and, with crypt on, `log_crypt.cc:191` | one per **second**, cached per thread (`mars_core::localtime`) |
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

1. **`mars_core::localtime`** — a per-thread snapshot of the second (offset,
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

The framework carries four slices — `ios-arm64`, `ios-arm64_x86_64-simulator`,
`watchos-arm64_arm64_32` and `watchos-arm64-simulator` — so an app target of
either platform resolves it. The watchOS device slice holds two architectures:
`arm64_32`, which is what a watch running watchOS 10 to 25 links, and `arm64`,
which is what watchOS 26 moved its watches onto. `arm64_32` is a tier 3 target
no channel ships a std for, so it is built out of a nightly's sources with
`-Z build-std`; the arm64 half needs no such thing, and is built for
watchOS 26 because its std is. `x86_64-apple-watchos-sim` is left out — a
watchOS simulator is arm64.

`Sources/MarsRSXlog/Xlog.swift` is what the port exposes today: `mars-ffi` is
an xlog C ABI (21 `mars_xlog_*` symbols, nothing else), so xlog is all the Swift
layer can reach and `MarsRS` re-exports `MarsRSXlog` and nothing more.
`Stn.swift` and `Sdt.swift` join the umbrella when the C ABI carries them —
that is why `MarsRS` exists as a module of its own and not just as a name for
xlog. orangeboyChen/mars ships a single `MarsXlog` product for the same reason;
here the xlog-only import is `import MarsRSXlog`.

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
| `mars-core.aar` | `mars-rs` | every Kotlin class whose natives `mars-jni` exports — `xlog`, `stn`, `sdt`, `app`, `comm` and `BaseEvent`/`Mars` — plus `libmarsxlog.so` for `arm64-v8a`, `armeabi-v7a` and `x86_64` |
| `mars-xlog.aar` | `mars-rs-xlog` | `xlog.Xlog` — with `XlogConfig`, `LogLevel`, `AppenderMode` and `CompressMode` — and the `xlog.Log` facade over it, plus the same `libmarsxlog.so` |

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
with nothing else to learn. A record costs one JNI call: `mars-jni` is what
drops a record the appender's level is above, before anything is formatted, and
what fills the pid and the tid in from the OS. A message that is expensive to
build is worth an `if (xlog.isLoggable(LogLevel.DEBUG))` first — a record the
level drops costs the caller the `String` either way.

`XlogConfig` is the Kotlin face of the appender's options — `LogLevel`,
`AppenderMode` and `CompressMode` are enums over the numbers `mars-jni` speaks
— and it refuses a config the `.so` cannot honour here, in Kotlin, because
`mars-jni` answers a config it does not like by opening nothing. A part of an
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
