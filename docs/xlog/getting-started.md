# Getting started

Three steps on every platform: **open** an appender once, when the app or the
process starts; **write** records through it; **flush** before its file is read
or uploaded. The drain is three calls, and not every platform carries all three:
`signalFlush()` asks the writer thread for it and returns at once, `flushNow()`
drains on the calling thread, and `await flush()` waits for the same drain
without holding one.

## Where it is

| your app is | what carries xlog | how you reach it |
|---|---|---|
| Rust | `marsrs`, or `marsrs-xlog` | `marsrs::xlog` |
| iOS / watchOS, Swift | the `MarsRSXlog` SwiftPM product | `import MarsRSXlog` |
| iOS / watchOS, Swift or Objective-C | the `MarsRSXlog` pod | `import MarsRSXlog` / `@import MarsRSXlog;` |
| Android, Kotlin or Java | `xlog`, or `marsrs` | `io.github.orangeboychen.marsrs.xlog.Xlog` |
| Kotlin Multiplatform | `xlog-kmp`, or `marsrs-kmp` | `io.github.orangeboychen.marsrs.xlog.Xlog` |
| Flutter | `marsrs_xlog`, or `marsrs` | `package:marsrs_xlog/marsrs_xlog.dart` |
| React Native | `marsrs-react-native-xlog`, or `marsrs-react-native` | `marsrs-react-native-xlog` |
| anything with a C FFI | `include/mars_xlog.h` | `mars_xlog_*` |
| HarmonyOS | `marsrs-harmonyos-xlog` | the `Xlog` of the HAR |

**`xlog` is the logger alone and `marsrs` is the whole port** — the logger plus
[the task pipeline](/stn/getting-started) and
[the network diagnosis](/sdt/getting-started). An app that only logs takes the
first; the same split is the one every platform makes, and moving from one
package to the other renames nothing about the file: the logger in `marsrs` is
the logger in `xlog`.

Apple makes it three ways instead — `MarsRSXlog` is the logger, `MarsRSNet` is
the other two pieces and no logger, and `MarsRS` is all three. Flutter and React
Native carry the logger in both of their packages today.

## Rust

```bash
cargo add marsrs          # the whole port: xlog, stn and sdt
cargo add marsrs-xlog     # xlog alone — the logger and nothing else
```

`marsrs` is one module per piece of mars — `xlog`, `stn`, `sdt`, `comm` — and
each is behind a feature of its own, all of them on by default. An app that only
logs takes `marsrs-xlog`, or `marsrs` with the rest turned off:

```toml
[dependencies]
marsrs = { version = "0.1", default-features = false, features = ["xlog"] }
```

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

`appender_open` installs one process-wide appender, which `appender_write` writes
through; every option is on [the configuration page](/xlog/configuration). A
record can carry more than a message — `appender_write` takes an `XLoggerInfo`
with the level, the tag and the file, function and line of the call site, and
`None` is the default one. An app that reads one part of its logs apart from the
rest gives that part an appender of its own, through the `*_instance` family —
`appender_open_instance(config)` answers a handle, and `appender_write_instance`,
`appender_flush_instance` and `appender_close_instance` take it.

## SwiftPM

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")

// and, in the target that takes it:
.product(name: "MarsRSXlog", package: "mars-rs")
```

| product | what you get |
|---|---|
| `MarsRSXlog` | the logger: `Xlog`, `XlogConfig`, `LogLevel`, `AppenderMode`, `CompressMode` |
| `MarsRSNet` | [the task pipeline](/stn/getting-started) and [the network diagnosis](/sdt/getting-started) |
| `MarsRS` | both halves, re-exported |

The framework carries four slices — `ios-arm64`, `ios-arm64_x86_64-simulator`,
`watchos-arm64_arm64_32` and `watchos-arm64-simulator`.

```swift
import MarsRSXlog

let log = try Xlog.open(
    XlogConfig(
        logDirectory: logDirectory.path,
        namePrefix: "marsrs",
        level: .info
    )
)
log.isConsoleLogEnabled = true

log.info(message: "hello from mars", tag: "startup")

log.flushNow()    // the records are on disk when this returns
```

`XlogConfig(logDirectory:)` is the short form — the other fields are set on it
afterwards, and every one of them is on
[the configuration page](/xlog/configuration). `Xlog.open` throws an `XlogError`
when the appender refuses the config. The file, the function and the line of a
record are filled in at the call site, so a record says where it was written
without the caller naming it. `import MarsRSXlog` re-exports `MarsRSFFI`, so the
C symbols are there too for whoever prefers them.

## CocoaPods

```ruby
# Podfile
platform :ios, '12.0'
use_frameworks!

pod 'MarsRSXlog', :podspec => 'https://raw.githubusercontent.com/orangeboyChen/mars-rs/v0.1.0/MarsRSXlog.podspec'
```

Three pods, under the names of the three products above: `MarsRSXlog` is the
logger, `MarsRSNet` is the task pipeline and the diagnosis, and `MarsRS` is both.
`:podspec` is the point of the line — the pods are not on a spec repo, and the
podspec is what names the archive a release publishes. An app that writes `pod
'MarsRSXlog'` alone is asking the trunk CDN for a pod that is not on it.

The Swift is the one of the SwiftPM package — one `import`, the same `Xlog`,
`XlogConfig` and `LogLevel`, because the pod and the package are the same Swift
over the same framework. Objective-C has no Swift source to take either: `Xlog`,
`XlogConfig` and the three enums are `@objc`, and what an Objective-C file
imports is the header the compiler writes out of them.

```objc
@import MarsRSXlog;

XlogConfig *config = [[XlogConfig alloc] initWithLogDirectory:logDirectory.path];
config.namePrefix = @"marsrs";

NSError *error = nil;
Xlog *log = [[Xlog alloc] initWithConfig:config error:&error];
log.isConsoleLogEnabled = YES;

[log writeWithLevel:LogLevelInfo message:@"hello from mars" tag:@"startup"];

[log flushNow];   // before the app reads or uploads the files
```

Objective-C has no `#file` to fill a call site with, so a record written here
carries an empty file and the line 0 unless the long form names them — `[log
log:message:tag:file:function:line:]`. What Objective-C does not get is the net
half: `MarsStn` and `MarsSdt` are Swift types, so an app that runs a task or a
diagnosis writes that part in Swift.

## Android

```kotlin
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen.marsrs:xlog:0.1.0")    // xlog alone
implementation("io.github.orangeboychen.marsrs:marsrs:0.1.0")  // + STN and SDT
```

Both AARs carry the same `libmarsrsxlog.so`, for `arm64-v8a`, `armeabi-v7a` and
`x86_64`. Both carry the R8 rules that keep the names the Kotlin and the native
half call each other by, so a release build that sets `minifyEnabled true` needs
no rules of its own.

```kotlin
val xlog = Xlog.open(
    XlogConfig(
        logDir = File(context.filesDir, "xlog/log").path,
        namePrefix = "marsrs",
        level = LogLevel.INFO,
    ),
    context,     // flushes itself when the app's UI goes away
)
xlog.consoleLogEnabled = BuildConfig.DEBUG

xlog.i("startup", "hello from mars")

xlog.flushNow()  // the records are on disk when this returns
```

The write is `android.util.Log`'s shape — `xlog.v`, `d`, `i`, `w`, `e` and `f`,
each of them a tag and a message. Java writes the same thing, with nothing else
to learn. Every member is safe to call from any thread, and a record costs one
JNI call. The `Context` is optional: without one nothing is registered and
nothing is lost — see [log files](/xlog/log-files).

## Kotlin Multiplatform

```kotlin
// settings.gradle.kts
maven {
    url = uri("https://maven.pkg.github.com/orangeboyChen/mars-rs")
    credentials { username = "<user>"; password = "<token with read:packages>" }
}

// build.gradle.kts of the shared module
implementation("io.github.orangeboychen.marsrs:xlog-kmp:0.1.0")    // xlog alone
implementation("io.github.orangeboychen.marsrs:marsrs-kmp:0.1.0")  // + STN and SDT
```

One dependency in `commonMain`, and each platform compiles its own half of it —
`androidMain` talks to the JNI bridge, `nativeMain` talks to the C ABI through
cinterop — so the same calls in shared code write the same `.xlog` on Android,
iOS, watchOS, tvOS, macOS, Linux and Windows. An app that would rather not
authenticate to GitHub Packages takes `marsrs-kmp-maven.zip` of the release
instead: unzip it and add `maven { url = uri("<dir>") }`.

```kotlin
val xlog = Xlog.open(
    XlogConfig(
        logDir = logDirectory,
        namePrefix = "marsrs",
        level = LogLevel.INFO,
    )
)
xlog.consoleLogEnabled = isDebug

xlog.i("startup", "hello from mars")

xlog.flushNow()  // the records are on disk when this returns
xlog.close()
```

This is the `Xlog` of Android — same members, same names — so a shared module
that moves between `xlog-kmp` and `xlog` renames nothing. What a `common`
declaration can only be is the intersection of the two bridges: the process-wide
calls of the C ABI are not in it, because the JNI bridge exports no equivalent,
and a caller who wants them writes them in that platform's source set.

## Flutter

```bash
flutter pub add marsrs_xlog    # or marsrs, for the whole port
```

The plugin is on pub.dev, so `flutter pub add` takes the newest version there.
Android needs nothing extra — it resolves `io.github.orangeboychen.marsrs:xlog`
from JitPack — and `pod install` is `flutter build ios`'s own business: the
plugin is a normal Flutter plugin with a podspec, and nothing in the app's
`Podfile` has to name it. Take one of the two packages and not both: both carry
the same native library, and an app with two of it does not build.

```dart
final dir = await getTemporaryDirectory();          // path_provider
final xlog = await Xlog.open(
  XlogConfig(
    logDir: '${dir.path}/xlog',
    namePrefix: 'marsrs',
    level: LogLevel.info,
  ),
);
xlog.consoleLogEnabled = kDebugMode;

xlog.i('startup', 'hello from mars');

await xlog.flush();      // the records are on disk when this resolves
await xlog.close();
```

The plugin is a method channel and not `dart:ffi`, so what crosses to the
platform thread is a message and not a call: a write, a setting and
`signalFlush()` hand the message over and return, and only the four that have
something an app can act on answer a `Future` — the appender `Xlog.open` opens,
the drain `await flush()` waits for, `close()`, and the answer `isLoggable`
gives. What Dart has no face for is the blocking drain: a channel cannot block
this side of it, so `signalFlush()` is the one that does not wait and
`await xlog.flush()` is the one that does. Neither
[the task pipeline](/stn/getting-started) nor
[the network diagnosis](/sdt/getting-started) is in the Dart yet: the plugin is
the logger, in both of its packages.

## React Native

```bash
npm install marsrs-react-native-xlog       # or marsrs-react-native, for the whole port
cd ios && pod install
```

The package is on npm, so `npm install` takes the newest version there. Nothing
in the app has to name the module: autolinking finds the `ReactPackage` in
`android/` and the pod in `ios/`, which is what puts `Xlog` in
`TurboModuleRegistry`. It is a TurboModule, which is what the New Architecture is
for — React Native 0.74 or newer, with the bridge switched off — and it is the
one thing this surface asks of the app.

```ts
const xlog = Xlog.open({
    logDir: `${directory}/xlog`,
    namePrefix: 'marsrs',
    level: LogLevel.info,
});
xlog.consoleLogEnabled = __DEV__;

xlog.i('startup', 'hello from mars');

xlog.flushNow();        // the records are on disk when this returns
xlog.close();
```

A method of the module is made on the JS thread and returns from there, so
nothing the app calls blocks it for long: `Xlog.open(config)` answers the
appender, and `xlog.i(tag, message)` has landed by the time it returns. The
drain is the one that can take longer than the JS thread should sit through, so
it is three calls — `xlog.signalFlush()` asks for it and returns,
`xlog.flushNow()` does it on this thread, and `await xlog.flush()` hands it to a
thread of the module's own and answers when it is over. Neither
[the task pipeline](/stn/getting-started) nor
[the network diagnosis](/sdt/getting-started) is in the TypeScript yet.

## The C ABI {#c-abi}

```text
marsrs-<version>-<host>.tar.gz   (Linux, macOS)
marsrs-<version>-<host>.zip      (Windows)
    include/mars_xlog.h     the logger
    include/mars_sdt.h      the network diagnosis
    include/mars_stn.h      the task pipeline
    libmars_ffi.a / libmars_ffi.so (.dylib, .dll)
```

built for `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin` and
`x86_64-pc-windows-msvc`, with the task pipeline and the diagnosis included.

```c
#include <mars_xlog.h>

MarsXLogConfig config = {
    .mode = MarsAppenderAsync,
    .log_dir = "/tmp/mars-log",
    .name_prefix = "marsrs",
    .compress_mode = MarsCompressZlib,
};
if (mars_xlog_open(&config) != MARS_XLOG_OK) { /* see the return code */ }

mars_xlog_write(MarsLevelInfo, "startup", __FILE__, __func__, __LINE__, "hello from mars");

mars_xlog_flush_sync();  /* the records are on disk when this returns */
mars_xlog_close();
```

Every pointer in `MarsXLogConfig` has to be NUL-terminated UTF-8 or `NULL`;
`NULL` is "empty", except for `log_dir`, which is mandatory. `mars_xlog_open`
installs one process-wide appender, which `mars_xlog_write` writes through. A
second one is `mars_xlog_new_instance(&config, level)`, which answers `0` when it
refuses the config; the `*_instance` calls take the handle it answered.

```bash
cc -I include -o app app.c libmars_ffi.a -lpthread -ldl     # static
cc -I include -o app app.c -L. -lmars_ffi                  # shared
```

Every call that returns an `int` answers `MARS_XLOG_OK` (0) or a negative
`MARS_XLOG_ERR_*`, and nothing in the C ABI unwinds into C.

## HarmonyOS

```bash
ohpm install marsrs-harmonyos-xlog
```

ohpm is not published to yet, so until it is, a release is where the package
comes from: take `marsrs-harmonyos-xlog-<version>.har` off it and `ohpm install`
that file. What the HAR carries is `libmarsrs_xlog.so` — the staticlib of the C
ABI with a NAPI module linked around it — for `arm64-v8a`, `armeabi-v7a` and
`x86_64`, so an app that takes it resolves nothing else. An app that writes NAPI
of its own takes `marsrs-harmony-<version>.tar.gz` instead: the three
`libmars_ffi.so` and `mars_xlog.h`, with nothing wrapped around them.

```typescript
import { AppenderMode, LogLevel, Xlog } from 'marsrs-harmonyos-xlog';

const xlog: Xlog = Xlog.open({
  logDir: `${getContext().filesDir}/xlog/log`,
  namePrefix: 'marsrs',
  level: LogLevel.Info,
  mode: AppenderMode.Async,
});
xlog.consoleLogEnabled = true;

xlog.i('startup', 'hello from mars');

xlog.flushNow();   // the records are on disk when this returns
xlog.close();
```

Every method of the NAPI module is synchronous, so a call returns from the thread
that made it and answers no promise. `LogLevel` is `Verbose` / `Debug` / `Info` /
`Warning` / `Error` / `Fatal` / `None` — PascalCase rather than the Kotlin's
`INFO`, because an ArkTS enum is written in an app full of HarmonyOS enums. No
ArkTS of this package reaches [the task pipeline](/stn/getting-started) or
[the network diagnosis](/sdt/getting-started): the HAR is the logger.

## Where to go next

- [Configuration](/xlog/configuration) — every option and its default, in the
  spelling of every platform.
- [Log files](/xlog/log-files) — where a file lands, what it is called, and how
  it is read back.
- [The xlog CLI](/xlog/cli) — reading a `.xlog` in a shell, and making the key
  pair that decides who can.
