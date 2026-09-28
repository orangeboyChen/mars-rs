# 快速开始

每个平台都是三步：进程或 App 启动时**打开**一个 appender，往里**写**记录，读文件或上传前**flush**。

## 1. 加依赖

::: code-group

```bash [Rust]
cargo add marsrs          # 整个移植：xlog、stn、sdt
cargo add marsrs-xlog     # 只有 xlog —— 日志，别的都没有
```

```swift [SwiftPM]
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")

// 在要用的 target 里：
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
implementation("io.github.orangeboychen.marsrs:xlog:0.1.0")  // 只有 xlog
```

```kotlin [Kotlin Multiplatform]
// 共享模块的 build.gradle.kts
implementation("io.github.orangeboychen.marsrs:xlog-kmp:0.1.0")
```

```bash [Flutter]
flutter pub add marsrs_xlog          # 要整个端口就 marsrs
```

```bash [React Native]
npm install marsrs-react-native-xlog         # 要整个移植就 marsrs-react-native
cd ios && pod install          # autolinking 会找到这个模块
```

```text [C]
marsrs-<version>-<host>.tar.gz   （Linux、macOS）
marsrs-<version>-<host>.zip      （Windows）
    include/mars_xlog.h
    include/mars_sdt.h
    include/mars_stn.h
    marsrs-ffi 的静态库和动态库
```

```bash [HarmonyOS]
ohpm install ./marsrs-harmonyos-xlog-<version>.har
# 从 release 取：ohpm 还没开始发布 —— 见 /zh/platforms/harmonyos
```

:::

## 2. 打开、写、flush

::: code-group

```rust [Rust]
use marsrs::xlog::{appender_close, appender_flush_sync, appender_open, appender_write, XLogConfig};

let mut config = XLogConfig::default();
config.logdir = std::path::PathBuf::from("/tmp/mars-log");
config.nameprefix = "marsrs".to_owned();
appender_open(config)?;

appender_write(None, "hello from mars");

appender_flush_sync();   // 返回时记录已经在磁盘上了
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

log.flush(sync: true)    // 返回时记录已经在磁盘上了
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

xlog.flush(sync = true)  // 返回时记录已经在磁盘上了
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

xlog.flush(sync = true)  // 返回时记录已经在磁盘上了
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

await xlog.flush(sync: true);  // 返回时记录已经在磁盘上了
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

xlog.flush(true);        // 返回时记录已经在磁盘上了
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
if (mars_xlog_open(&config) != MARS_XLOG_OK) { /* 看返回码 */ }

mars_xlog_write(MarsLevelInfo, "startup", __FILE__, __func__, __LINE__, "hello from mars");

mars_xlog_flush_sync();  // 返回时记录已经在磁盘上了
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

xlog.flush(true);   // 返回时记录已经在磁盘上了
xlog.close();
```

:::

## 3. 文件在哪

appender 把日志写进你给的那个目录，名字是 `<namePrefix>_YYYYMMDD.xlog` —— 上面就是
`/tmp/mars-log/marsrs_20260927.xlog`。默认模式是异步，一条记录可能在缓存里待一会儿：
**读文件或上传前 flush**。App 退出时什么都不用调用 —— 见[日志文件](/zh/log-files)。

## 接下来

- [配置项](/zh/configuration) —— 每个配置项和它的默认值。
- [日志文件](/zh/log-files) —— 在哪、叫什么、怎么读回来。
- [你那个平台](/zh/platforms/rust)的页面 —— 完整 API、级别与模式、包里还有什么。
- [任务链路](/zh/stn)和[网络诊断](/zh/sdt) —— 这个移植里不是日志的那半，给带着它的那些平台。
- [从 mars-xlog 迁移](/zh/migrating-from-mars-xlog) —— 已经在用 C++ 实现的日志库写
  日志的 App 看这里。
- [从 mars 迁移](/zh/migrating-from-mars) —— 除了日志库还要搬任务链路和网络诊断的
  App 看这里。
