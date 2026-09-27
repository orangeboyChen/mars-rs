# 快速开始

每个平台都是三步：进程或 App 启动时**打开**一个 appender，往里**写**记录，读文件或上传前**flush**。

## 1. 加依赖

::: code-group

```bash [Rust]
cargo add marsrs          # 整个端口：xlog、stn、sdt
cargo add marsrs-xlog     # 只有 xlog —— 日志，别的都没有
```

```swift [SwiftPM]
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")

// 在要用的 target 里：
.product(name: "MarsRSXlog", package: "mars-rs")
```

```kotlin [Android]
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen:mars-rs-xlog:0.1.0")  // 只有 xlog
```

```kotlin [Kotlin Multiplatform]
// 共享模块的 build.gradle.kts
implementation("io.github.orangeboychen:mars-rs-xlog-kmp:0.1.0")
```

```text [C]
mars-rs-<version>-<host>.tar.gz   （Linux、macOS）
mars-rs-<version>-<host>.zip      （Windows）
    include/mars_xlog.h
    include/mars_sdt.h
    include/mars_stn.h
    marsrs-ffi 的静态库和动态库
```

:::

## 2. 打开、写、flush

::: code-group

```rust [Rust]
use marsrs::xlog::{appender_close, appender_flush_sync, appender_open, appender_write, XLogConfig};

let mut config = XLogConfig::default();
config.logdir = std::path::PathBuf::from("/tmp/mars-log");
config.nameprefix = "Ham".to_owned();
appender_open(config)?;

appender_write(None, "hello from mars");

appender_flush_sync();   // 返回时记录已经在磁盘上了
appender_close();
```

```swift [Swift]
import MarsRSXlog

let log = try Xlog(
    XlogConfig(
        logDirectory: logDirectory.path,
        namePrefix: "Ham",
        level: .info
    )
)
log.isConsoleLogEnabled = true

log.info(message: "hello from mars", tag: "startup")

log.flush(sync: true)    // 返回时记录已经在磁盘上了
```

```kotlin [Android]
val xlog = Xlog(
    XlogConfig(
        logDir = File(context.filesDir, "xlog/log").path,
        namePrefix = "Ham",
        level = LogLevel.INFO,
    )
)
xlog.consoleLogEnabled = BuildConfig.DEBUG

xlog.i("startup", "hello from mars")

xlog.flush(sync = true)  // 返回时记录已经在磁盘上了
```

```kotlin [Kotlin Multiplatform]
Xlog.open(
    XlogConfig(
        logDir = logDirectory,
        namePrefix = "Ham",
        level = LogLevel.Info,
    )
)

Xlog.write(LogLevel.Info, "startup", "hello from mars")

Xlog.flush(sync = true)  // 返回时记录已经在磁盘上了
Xlog.close()
```

```c [C]
#include <mars_xlog.h>

MarsXLogConfig config = {
    .mode = MarsAppenderAsync,
    .log_dir = "/tmp/mars-log",
    .name_prefix = "Ham",
    .compress_mode = MarsCompressZlib,
};
if (mars_xlog_open(&config) != MARS_XLOG_OK) { /* 看返回码 */ }

mars_xlog_write(MarsLevelInfo, "startup", __FILE__, __func__, __LINE__, "hello from mars");

mars_xlog_flush_sync();  // 返回时记录已经在磁盘上了
mars_xlog_close();
```

:::

## 3. 文件在哪

appender 把日志写进你给的那个目录，名字是 `<namePrefix>_YYYYMMDD.xlog` —— 上面就是
`/tmp/mars-log/Ham_20260927.xlog`。默认模式是异步，一条记录可能在缓存里待一会儿：
**读文件、上传、以及进程退出前都要 flush**。见[日志文件](/zh/log-files)。

## 接下来

- [配置项](/zh/configuration) —— 每个配置项和它的默认值。
- [日志文件](/zh/log-files) —— 在哪、叫什么、怎么读回来。
- [你那个平台](/zh/platforms/rust)的页面 —— 完整 API、级别与模式、包里还有什么。
