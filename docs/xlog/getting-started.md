# Getting started

Three steps on every platform: **open** an appender once, when the app or the
process starts; **write** records through it; **flush** before its file is read
or uploaded. The drain is three calls, and not every platform carries all three.
What separates them is who waits and who learns when the drain is over:
`requestFlush()` asks for it and returns at once, and nothing answers when it
happened; `flushNow()` drains on the calling thread, so the records are on disk
when it returns; `await flush()` hands the drain to another thread and answers
when it is over.

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
| C++ | `include/mars_xlog.hpp` | `marsrs::xlog::Xlog` |
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
use marsrs::xlog::{LogLevel, XLogConfig, Xlog};

let mut config = XLogConfig::default();
config.logdir = std::path::PathBuf::from("/tmp/mars-log");
config.nameprefix = "marsrs".to_owned();

let xlog = Xlog::open(config, LogLevel::Info)?;
xlog.i("startup", "hello from mars");

xlog.flush_now();   // the records are on disk when this returns
// xlog.flush().await is the same drain off this thread
```

`Xlog::open` answers the appender, which is the `Xlog.open(config)` of Kotlin, of
Dart and of TypeScript and the `Xlog(config)` of Swift: one object, held, and
written through — a second logger is a second `Xlog` of a prefix of its own.
Every option is on [the configuration page](/xlog/configuration). A record is a
level, a tag and a message — `xlog.log(level, tag, message)`, or `xlog.i(tag,
message)` at a level of its own — and nothing of the call site: Rust has no
`#file` to fill the file, the function and the line in with, so a record written
here carries the empty ones, as Kotlin's does.

`Xlog::open` for a prefix that is already open answers the appender that is open
and not a second one, and the config that second call hands in is ignored —
level, directory and all — because the appender was opened with the first one's.
Which is why `close()` on one `Xlog` closes what another `Xlog` of the same
prefix writes through: they share the appender, the file and the level. On every
platform of the port, and in the C++ of `mars_xlog.hpp` too. An `Xlog` closes
itself when it is dropped, so one held for the life of the process needs no
`close()` at all.

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
declaration can only be is the intersection of the two bridges, and both of
them are the whole API: the JNI bridge and the cinterop one export the same
symbols, so there is nothing a platform source set has to write for itself.

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
`requestFlush()` hand the message over and return, and only the seven that have
something an app can act on answer a `Future` — the appender `Xlog.open` opens,
the drain `await flush()` waits for, `close()`, the answer `isLoggable` gives,
and the three that name files: `currentLogPath()`, `logFiles(daysAgo)` and
`logFileNames(daysAgo)`. What Dart has no face for is the blocking drain: a
channel cannot block this side of it, so what Dart gets is the other two:
`requestFlush()`, which asks for the drain and returns at once with nothing to
say about when it is over, and `await xlog.flush()`, which answers when it is.
Neither
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
it is three calls — `xlog.requestFlush()` asks for it and returns at once, and
nothing answers when it is over; `xlog.flushNow()` does it on this thread, so the
records are on disk when it returns; and `await xlog.flush()` hands it to a
thread of the module's own and answers when it is over. Neither
[the task pipeline](/stn/getting-started) nor
[the network diagnosis](/sdt/getting-started) is in the TypeScript yet.

## The C ABI {#c-abi}

```text
marsrs-<version>-<host>.tar.gz   (Linux, macOS)
marsrs-<version>-<host>.zip      (Windows)
    include/mars_xlog.h     the logger
    include/mars_xlog.hpp   the logger in C++
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
long long xlog = mars_xlog_new_instance(&config, MarsLevelVerbose);
if (xlog == 0) { /* the config was refused */ }

mars_xlog_write_instance(xlog, MarsLevelInfo, "startup", __FILE__, __func__, __LINE__,
                         "hello from mars");
mars_xlog_flush_now_instance(xlog);  /* the records are on disk when this returns */
mars_xlog_release_instance("marsrs");
```

Every pointer in `MarsXLogConfig` has to be NUL-terminated UTF-8 or `NULL`;
`NULL` is "empty", except for `log_dir`, which is mandatory. What an app holds
is an **instance**: `mars_xlog_new_instance(&config, level)` answers its handle,
or `0` when it refuses the config, and every `*_instance` call takes that
handle. There is no process-wide appender to open from C — an app that wants a
second logger gives it a second prefix.

```bash
cc -I include -o app app.c libmars_ffi.a -lpthread -ldl     # static
cc -I include -o app app.c -L. -lmars_ffi                  # shared
```

Every call that answers an `int` answers `MARS_XLOG_OK` (0) or a negative
`MARS_XLOG_ERR_*`, and nothing in the C ABI unwinds into C. Three calls answer
an `int` that is not a status: the three that name a path answer the number of
bytes they wrote, `mars_xlog_is_enabled_for` answers 1 or 0, and
`mars_xlog_get_level` answers the level itself — or `-1` for a handle that
names no appender, which is not an error code.

## C++

```cpp
#include <mars_xlog.hpp>

marsrs::xlog::XlogConfig config;
config.logDir = "/tmp/mars-log";
config.namePrefix = "marsrs";

auto log = marsrs::xlog::Xlog::open(config);   // throws marsrs::xlog::XlogError
log.setConsoleLogEnabled(true);

log.i("startup", "hello from mars");

log.flushNow();     // the records are on disk when this returns
log.close();
```

`mars_xlog.hpp` is the C ABI in C++: one header over the same library, so what
an app links is what the C section above links. `Xlog::open(config)` is the
`Xlog.open(config)` of Kotlin, of Dart and of TypeScript, and the members are
the same members — `v`, `d`, `i`, `w`, `e` and `f` take a tag and a message,
`level`, `mode`, `consoleLogEnabled`, `maxFileSizeBytes` and
`maxAliveTimeSeconds` are set and read back, `isLoggable` is the question to ask
before building a message that is expensive to build, and the drain is the same
three calls: `requestFlush()` asks for the drain and returns at once, and nothing
answers when it is over; `flushNow()` does it on this thread, so the records are
on disk when it returns; and `flush()` answers a `std::future<void>` for the same
drain off it.

Two things are C++'s own. An `Xlog` is move-only — a prefix is one appender, and
two copies of one handle would be two owners of one close — and its destructor
closes the appender, so an `Xlog` of automatic storage needs no `close()` at the
end of the scope. And a record carries an empty file, an empty function and the
line 0 unless the long `log` names them, which is what Kotlin writes too: C++
has no `#file` to fill one in with, so `__FILE__`, `__PRETTY_FUNCTION__` and
`__LINE__` of the call site go to
`log(level, tag, message, file, function, line)`.

The header is C++17: it refuses to compile below that, and `std::future` is what
`flush()` answers.

```bash
c++ -std=c++17 -I include -o app app.cpp libmars_ffi.a -lpthread -ldl
```

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
