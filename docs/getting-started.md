# Getting started

Three steps on every platform: **open** an appender once when the process or the
app starts, **write** records through it, and **flush** before you read or upload
its files.

Every block below is a group of platforms: pick yours once and the rest of these
pages open on it — which one you picked is remembered.

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

```ruby [CocoaPods]
# Podfile
platform :ios, '12.0'
use_frameworks!

pod 'MarsRSXlog', :podspec => 'https://raw.githubusercontent.com/orangeboyChen/mars-rs/v0.1.0/MarsRSXlog.podspec'
```

```kotlin [Android]
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen.marsrs:xlog:0.1.0")  // xlog alone
```

```kotlin [Kotlin Multiplatform]
// build.gradle.kts of the shared module
implementation("io.github.orangeboychen.marsrs:xlog-kmp:0.1.0")
```

```bash [Flutter]
flutter pub add marsrs_xlog    # marsrs, for the whole port
```

```bash [React Native]
npm install marsrs-react-native-xlog       # marsrs-react-native, for the whole port
cd ios && pod install          # autolinking finds the module
```

```text [C]
marsrs-<version>-<host>.tar.gz   (Linux, macOS)
marsrs-<version>-<host>.zip      (Windows)
    include/mars_xlog.h
    include/mars_sdt.h
    include/mars_stn.h
    the static and shared libraries of marsrs-ffi
```

```bash [HarmonyOS]
ohpm install ./marsrs-harmonyos-xlog-<version>.har
# off a release: ohpm is not published to yet — see /platforms/harmonyos
```

:::

## 2. Open, write, flush

::: code-group

```rust [Rust]
use marsrs::xlog::{appender_close, appender_flush_sync, appender_open, appender_write, XLogConfig};

let mut config = XLogConfig::default();
config.logdir = std::path::PathBuf::from("/tmp/mars-log");
config.nameprefix = "marsrs".to_owned();
appender_open(config)?;

appender_write(None, "hello from mars");

appender_flush_sync();   // the records are on disk when this returns
appender_close();
```

```swift [Swift]
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

log.flush(sync: true)    // the records are on disk when this returns
```

```kotlin [Android]
val xlog = Xlog.open(
    XlogConfig(
        logDir = File(context.filesDir, "xlog/log").path,
        namePrefix = "marsrs",
        level = LogLevel.INFO,
    )
)
xlog.consoleLogEnabled = BuildConfig.DEBUG

xlog.i("startup", "hello from mars")

xlog.flush(sync = true)  // the records are on disk when this returns
```

```kotlin [Kotlin Multiplatform]
val xlog = Xlog.open(
    XlogConfig(
        logDir = logDirectory,
        namePrefix = "marsrs",
        level = LogLevel.INFO,
    )
)
xlog.consoleLogEnabled = isDebug

xlog.i("startup", "hello from mars")

xlog.flush(sync = true)  // the records are on disk when this returns
xlog.close()
```

```dart [Flutter]
final xlog = await Xlog.open(
    XlogConfig(
        logDir: "${directory.path}/xlog",
        namePrefix: "marsrs",
        level: LogLevel.info,
    ),
);
xlog.consoleLogEnabled = kDebugMode;

xlog.i("startup", "hello from mars");

await xlog.flush(sync: true);  // the records are on disk when this returns
await xlog.close();
```

```ts [React Native]
const xlog = Xlog.open({
    logDir: `${directory}/xlog`,
    namePrefix: "marsrs",
    level: LogLevel.info,
});
xlog.consoleLogEnabled = __DEV__;

xlog.i("startup", "hello from mars");

xlog.flush(true);        // the records are on disk when this returns
xlog.close();
```

```c [C]
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

```typescript [HarmonyOS]
import { LogLevel, Xlog } from 'marsrs-harmonyos-xlog';

const xlog: Xlog = Xlog.open({
    logDir: `${getContext().filesDir}/xlog/log`,
    namePrefix: 'marsrs',
    level: LogLevel.Info,
});
xlog.consoleLogEnabled = true;

xlog.i('startup', 'hello from mars');

xlog.flush(true);   // the records are on disk when this returns
xlog.close();
```

:::

## 3. Find the files

An appender writes `<namePrefix>_YYYYMMDD.xlog` into the log directory you gave
it — `/tmp/mars-log/marsrs_20260927.xlog` above. The default mode is async, so a
record can sit in the cache for a moment: **flush before you read or upload**.
Nothing has to be called when the app goes away — see [log files](/log-files).

## Where to go next

- [Configuration](/configuration) — every option and its default.
- [Log files](/log-files) — where they are, what they are called, how to read
  them back.
- The page of [your platform](/platforms/rust) — the full API, the levels and
  the modes, and what else the package carries.
- [The task pipeline](/stn) and [the network diagnosis](/sdt) — the half of the
  port that is not the logger, for the platforms that carry it.
- [Migrating from mars-xlog](/migrating-from-mars-xlog) — for an app that
  already writes through the C++ implementation's logger.
- [Migrating from mars](/migrating-from-mars) — the same for the task pipeline
  and the network diagnosis.
