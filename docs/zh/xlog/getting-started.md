# 快速开始

每个平台都是三步：App 或进程启动时**打开**一个 appender，往里**写**记录，读文件或
上传之前**排空**它。排空是三个调用，不是每个平台三个都有：`signalFlush()` 请写线程去
排空、自己立刻返回，`flushNow()` 在调用方线程上排空，`await flush()` 等的是同一次排
空，但不占着一个线程。

## 它在哪里

| 你的 App 是 | 谁带着 xlog | 怎么拿到它 |
|---|---|---|
| Rust | `marsrs`，或 `marsrs-xlog` | `marsrs::xlog` |
| iOS / watchOS，Swift | `MarsRSXlog` 这个 SwiftPM product | `import MarsRSXlog` |
| iOS / watchOS，Swift 或 Objective-C | `MarsRSXlog` 这个 pod | `import MarsRSXlog` / `@import MarsRSXlog;` |
| Android，Kotlin 或 Java | `xlog`，或 `marsrs` | `io.github.orangeboychen.marsrs.xlog.Xlog` |
| Kotlin Multiplatform | `xlog-kmp`，或 `marsrs-kmp` | `io.github.orangeboychen.marsrs.xlog.Xlog` |
| Flutter | `marsrs_xlog`，或 `marsrs` | `package:marsrs_xlog/marsrs_xlog.dart` |
| React Native | `marsrs-react-native-xlog`，或 `marsrs-react-native` | `marsrs-react-native-xlog` |
| 有 C FFI 的任何东西 | `include/mars_xlog.h` | `mars_xlog_*` |
| C++ | `include/mars_xlog.hpp` | `marsrs::xlog::Xlog` |
| HarmonyOS | `marsrs-harmonyos-xlog` | HAR 里的 `Xlog` |

**`xlog` 是只有日志，`marsrs` 是整个移植** —— 日志加上[任务链路](/zh/stn/getting-started)和
[网络诊断](/zh/sdt/getting-started)。只打日志的 App 拿前者；每个平台都是这样分
的，两个包之间换一个不会改动文件本身：`marsrs` 里的日志就是 `xlog` 里的日志。

Apple 那边有三种而不是两种 —— `MarsRSXlog` 是日志，`MarsRSNet` 是另外两块、不带
日志，`MarsRS` 是三块都有。Flutter 和 React Native 目前两个包里带的都是日志。

## Rust

```bash
cargo add marsrs          # 整个移植：xlog、stn、sdt
cargo add marsrs-xlog     # 只有 xlog —— 日志，别的都没有
```

`marsrs` 给 mars 的每一块一个模块 —— `xlog`、`stn`、`sdt`、`comm` —— 每块有自己
的 feature，默认全开。只打日志的 App 拿 `marsrs-xlog`，或者把 `marsrs` 里其余的
关掉：

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

xlog.flush_now();   // 返回时记录已经在磁盘上
// xlog.flush().await 是同一场排空，只是不在本线程上
```

`Xlog::open` 回答的那个 appender，就是 Kotlin、Dart 和 TypeScript 的
`Xlog.open(config)`，也是 Swift 的 `Xlog(config:)` —— 一个对象，拿着它，对着它写；
第二个 logger 就是第二个 `Xlog`，给它自己的 prefix。每个选项都在
[配置项](/zh/xlog/configuration)那页。一条记录可以不止带一句话 ——
`xlog.log_with_info(Some(&info), message)` 收一个 `XLoggerInfo`，里面有级别、tag
和调用处的文件、函数、行号，而 `xlog.log(level, tag, message)` 是写空的那三个的
短写法，Kotlin 写的也是这样 —— Rust 没有 `#file` 可以填进去。

已经开着的 prefix 再 `Xlog::open` 一次，回答的是那个已经开着的 appender 而不是第
二个，而且第二次传进去的 config 会被忽略 —— 级别、目录都在内 —— 因为那个 appender
是用第一次的 config 开的。所以一个 `Xlog` 上 `close()` 会关掉同 prefix 的另一个
`Xlog` 正在写的那个：它们共用同一个 appender、同一个文件、同一个级别。这个移植的每个
平台都是这样，`mars_xlog.hpp` 里的 C++ 也是。`Xlog` 被 drop 时
自己会关，所以一个活到进程结束的 `Xlog` 根本不需要 `close()`。

## SwiftPM

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")

// 在要用的 target 里：
.product(name: "MarsRSXlog", package: "mars-rs")
```

| product | 拿到什么 |
|---|---|
| `MarsRSXlog` | 日志：`Xlog`、`XlogConfig`、`LogLevel`、`AppenderMode`、`CompressMode` |
| `MarsRSNet` | [任务链路](/zh/stn/getting-started)和[网络诊断](/zh/sdt/getting-started) |
| `MarsRS` | 两半都有，重新导出 |

framework 里有四个 slice —— `ios-arm64`、`ios-arm64_x86_64-simulator`、
`watchos-arm64_arm64_32` 和 `watchos-arm64-simulator`。

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

log.flushNow()    // 返回时记录已经在磁盘上
```

`XlogConfig(logDirectory:)` 是简写形式 —— 其余字段之后在它上面设，每个都在
[配置项](/zh/xlog/configuration)那页。`Xlog.open` 在 appender 不接受这个 config 时
抛 `XlogError`。记录的文件、函数和行号是在调用处填进去的，所以一条记录不说也知道
它是在哪儿写的。`import MarsRSXlog` 会把 `MarsRSFFI` 一并导出，所以想直接用 C 符号
的人也能拿到。

## CocoaPods

```ruby
# Podfile
platform :ios, '12.0'
use_frameworks!

pod 'MarsRSXlog', :podspec => 'https://raw.githubusercontent.com/orangeboyChen/mars-rs/v0.1.0/MarsRSXlog.podspec'
```

三个 pod，名字就是上面三个 product：`MarsRSXlog` 是日志，`MarsRSNet` 是任务链路和
网络诊断，`MarsRS` 是两者。这一行的重点是 `:podspec` —— 这些 pod 不在 spec repo
上，podspec 才是给 release 发布的那个压缩包命名的东西。只写 `pod 'MarsRSXlog'` 的
App 是在向 trunk CDN 要一个并不在上面的 pod。

Swift 这边就是 SwiftPM 包里的那一套 —— 一次 `import`，同样的 `Xlog`、
`XlogConfig`、`LogLevel`，因为 pod 和 package 是同一个 framework 上的同一份
Swift。Objective-C 也没有 Swift 源码可拿：`Xlog`、`XlogConfig` 和那三个枚举都是
`@objc` 的，Objective-C 文件 import 的是编译器从它们写出来的头文件。

```objc
@import MarsRSXlog;

XlogConfig *config = [[XlogConfig alloc] initWithLogDirectory:logDirectory.path];
config.namePrefix = @"marsrs";

NSError *error = nil;
Xlog *log = [[Xlog alloc] initWithConfig:config error:&error];
log.isConsoleLogEnabled = YES;

[log writeWithLevel:LogLevelInfo message:@"hello from mars" tag:@"startup"];

[log flushNow];   // 在 App 读文件或上传之前
```

Objective-C 没有 `#file` 可填，所以这里写的记录带的是空文件名和行号 0，除非用长形
式把它们写上 —— `[log log:message:tag:file:function:line:]`。Objective-C 拿不到的
是网络那一半：`MarsStn` 和 `MarsSdt` 是 Swift 类型，要跑任务或诊断的 App 得把那
部分写在 Swift 里。

## Android

```kotlin
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen.marsrs:xlog:0.1.0")    // 只有 xlog
implementation("io.github.orangeboychen.marsrs:marsrs:0.1.0")  // + STN 和 SDT
```

两个 AAR 带的是同一个 `libmarsrsxlog.so`，覆盖 `arm64-v8a`、`armeabi-v7a` 和
`x86_64`。两个都带着 R8 规则，把 Kotlin 和 native 两边互相调用用到的名字留住，所
以开了 `minifyEnabled true` 的 release 构建不需要自己再加规则。

```kotlin
val xlog = Xlog.open(
    XlogConfig(
        logDir = File(context.filesDir, "xlog/log").path,
        namePrefix = "marsrs",
        level = LogLevel.INFO,
    ),
    context,     // App 界面离开时自己 flush
)
xlog.consoleLogEnabled = BuildConfig.DEBUG

xlog.i("startup", "hello from mars")

xlog.flushNow()  // 返回时记录已经在磁盘上
```

写是 `android.util.Log` 的形状 —— `xlog.v`、`d`、`i`、`w`、`e`、`f`，每个都是一个
tag 和一句话。Java 写的也是这些，没有别的东西要学。每个成员在任何线程上调用都是安
全的，一条记录是一次 JNI 调用。`Context` 是可选的：不给就不注册任何东西，也不会丢
东西 —— 见[日志文件](/zh/xlog/log-files)。

## Kotlin Multiplatform

```kotlin
// settings.gradle.kts
maven {
    url = uri("https://maven.pkg.github.com/orangeboyChen/mars-rs")
    credentials { username = "<user>"; password = "<有 read:packages 的 token>" }
}

// 共享模块的 build.gradle.kts
implementation("io.github.orangeboychen.marsrs:xlog-kmp:0.1.0")    // 只有 xlog
implementation("io.github.orangeboychen.marsrs:marsrs-kmp:0.1.0")  // + STN 和 SDT
```

`commonMain` 里一个依赖，每个平台各自编译自己的一半 —— `androidMain` 走 JNI
bridge，`nativeMain` 通过 cinterop 走 C ABI —— 所以共享代码里同样这几个调用，在
Android、iOS、watchOS、tvOS、macOS、Linux 和 Windows 上写出的是同一个 `.xlog`。
不想去 GitHub Packages 认证的 App 可以改用 release 里的
`marsrs-kmp-maven.zip`：解压，然后加 `maven { url = uri("<目录>") }`。

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

xlog.flushNow()  // 返回时记录已经在磁盘上
xlog.close()
```

这就是 Android 的那个 `Xlog` —— 成员一样、名字一样 —— 所以在 `xlog-kmp` 和 `xlog`
之间换的共享模块什么都不用改名。`common` 声明只能是两座桥的交集：C ABI 那些进程级
调用不在里面，因为 JNI bridge 没有导出对应的东西，要它们的调用方，写在那个平台的
source set 里。

## Flutter

```bash
flutter pub add marsrs_xlog    # 要整个移植就 marsrs
```

插件在 pub.dev 上，所以 `flutter pub add` 拿的是那里最新的版本。Android 不需要额外
配置 —— 它从 JitPack 解析 `io.github.orangeboychen.marsrs:xlog` —— `pod install`
是 `flutter build ios` 自己的事：这个插件是带 podspec 的普通 Flutter 插件，App 的
`Podfile` 里不需要提到它。两个包只拿一个，不要都拿：两个带的是同一个 native 库，
一个 App 里放两份是编不过的。

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

await xlog.flush();      // 等它返回时记录已经在磁盘上
await xlog.close();
```

这个插件是 method channel 而不是 `dart:ffi`，所以过到平台线程上的是一条消息而不是
一次调用：写、一个设置和 `signalFlush()` 都是把消息递过去就返回，只有四个回答
`Future`，也只有它们有 App 要等的东西 —— `Xlog.open` 打开的那个
appender，`await flush()` 和 `close()` 等的那个 drain，以及 `isLoggable` 给的答案。
Dart 这边没有阻塞式排空的那个面：channel 阻塞不了 Dart 这一侧，所以不等的那一下是
`signalFlush()`，要等的那一下是 `await xlog.flush()`。
[任务链路](/zh/stn/getting-started)和
[网络诊断](/zh/sdt/getting-started)都还没进 Dart：这个插件是日志，在它的两个包里都
是。

## React Native

```bash
npm install marsrs-react-native-xlog       # 要整个移植就 marsrs-react-native
cd ios && pod install
```

包在 npm 上，所以 `npm install` 拿的是那里最新的版本。App 里不需要提到这个模块：
autolinking 会找到 `android/` 里的 `ReactPackage` 和 `ios/` 里的 pod，这也就是把
`Xlog` 放进 `TurboModuleRegistry` 的东西。它是 TurboModule，这正是新架构要的 ——
React Native 0.74 或更新，并且关掉 bridge —— 这也是这层对 App 唯一的要求。

```ts
const xlog = Xlog.open({
    logDir: `${directory}/xlog`,
    namePrefix: 'marsrs',
    level: LogLevel.info,
});
xlog.consoleLogEnabled = __DEV__;

xlog.i('startup', 'hello from mars');

xlog.flushNow();        // 返回时记录已经在磁盘上
xlog.close();
```

模块的方法都在 JS 线程上调用，也从那里返回，所以 App 调的没有哪个会长时间堵住它：
`Xlog.open(config)` 直接回答那个 appender，`xlog.i(tag, message)` 在它返回时就已经落地
了。排空是唯一可能比 JS 线程该等的时间更长的那件事，所以它有三个写法 ——
`xlog.signalFlush()` 通知一声就返回，`xlog.flushNow()` 在当前线程上排空，
`await xlog.flush()` 把它交给模块自己的线程，排空结束时才落地。[任务链路](/zh/stn/getting-started)和[网络诊断](/zh/sdt/getting-started)都
还没进 TypeScript。

## The C ABI {#c-abi}

```text
marsrs-<version>-<host>.tar.gz   （Linux、macOS）
marsrs-<version>-<host>.zip      （Windows）
    include/mars_xlog.h     日志
    include/mars_xlog.hpp   日志的 C++ 写法
    include/mars_sdt.h      网络诊断
    include/mars_stn.h      任务链路
    libmars_ffi.a / libmars_ffi.so（.dylib、.dll）
```

为 `x86_64-unknown-linux-gnu`、`aarch64-apple-darwin` 和
`x86_64-pc-windows-msvc` 构建，任务链路和网络诊断都在里面。

```c
#include <mars_xlog.h>

MarsXLogConfig config = {
    .mode = MarsAppenderAsync,
    .log_dir = "/tmp/mars-log",
    .name_prefix = "marsrs",
    .compress_mode = MarsCompressZlib,
};
if (mars_xlog_open(&config) != MARS_XLOG_OK) { /* 看返回码 */ }

mars_xlog_write(MarsLevelInfo, "startup", __FILE__, __func__, __LINE__, "hello from mars");

mars_xlog_flush_now_instance(0);  /* 返回时记录已经在磁盘上 */
mars_xlog_close();
```

`MarsXLogConfig` 里每个指针都得以 NUL 结尾的 UTF-8，或者是 `NULL`；`NULL` 就是
“空”，只有 `log_dir` 例外，它是必填的。`mars_xlog_open` 装上一个进程级 appender，
`mars_xlog_write` 往它里面写。第二个是
`mars_xlog_new_instance(&config, level)`，它不接受 config 时回答 `0`；`*_instance`
那一族收下它回答的 handle。

```bash
cc -I include -o app app.c libmars_ffi.a -lpthread -ldl     # 静态
cc -I include -o app app.c -L. -lmars_ffi                  # 动态
```

每个返回 `int` 的调用回答 `MARS_XLOG_OK`（0）或一个负的 `MARS_XLOG_ERR_*`，C ABI
里没有任何东西会把栈展开到 C 里。

## C++

```cpp
#include <mars_xlog.hpp>

marsrs::xlog::XlogConfig config;
config.logDir = "/tmp/mars-log";
config.namePrefix = "marsrs";

auto log = marsrs::xlog::Xlog::open(config);   // 抛出 marsrs::xlog::XlogError
log.setConsoleLogEnabled(true);

log.i("startup", "hello from mars");

log.flushNow();     // 返回时记录已经在磁盘上
log.close();
```

`mars_xlog.hpp` 是 C ABI 的 C++ 写法：一个头文件，架在同一个库上，所以 App 要链
的东西和上面 C 那一节是同一个。`Xlog::open(config)` 就是 Kotlin、Dart 和
TypeScript 的 `Xlog.open(config)`，成员也是同一批成员 —— `v`、`d`、`i`、`w`、
`e`、`f` 收一个 tag 和一条消息，`level`、`mode`、`consoleLogEnabled`、
`maxFileSizeBytes` 和 `maxAliveTimeSeconds` 既能设也能读回来，`isLoggable` 是那
句在拼一条很贵的消息之前该问的问题，而落盘还是那三个调用：`signalFlush()` 通知
写线程然后立刻返回，`flushNow()` 在这个线程上做，`flush()` 回答一个
`std::future<void>`，把同一次落盘交给别的线程。

有两样是 C++ 自己的。`Xlog` 只能移动不能复制 —— 一个 prefix 就是一个 appender，
一个 handle 的两份副本会是同一次关闭的两个主人 —— 而它的析构函数会关掉这个
appender，所以一个自动存储期的 `Xlog` 在作用域末尾不需要 `close()`。还有：记录
里的文件、函数和行号是空的，除非那个长的 `log` 把它们写出来，Kotlin 写的也是这
样 —— C++ 没有 `#file` 可以填进去，所以调用点的 `__FILE__`、
`__PRETTY_FUNCTION__` 和 `__LINE__` 交给
`log(level, tag, message, file, function, line)`。

这个头文件要 C++17：`Xlog::setConsoleSink` 的控制台 sink 收到的是
`std::string_view`，`flush()` 回答的是 `std::future`。

```bash
c++ -std=c++17 -I include -o app app.cpp libmars_ffi.a -lpthread -ldl
```

## HarmonyOS

```bash
ohpm install marsrs-harmonyos-xlog
```

包还没发到 ohpm 上，在那之前只能从 release 拿：从那里取
`marsrs-harmonyos-xlog-<version>.har`，`ohpm install` 那个文件。HAR 里带的是
`libmarsrs_xlog.so` —— C ABI 的 staticlib 外面包了一层 NAPI 模块 —— 覆盖
`arm64-v8a`、`armeabi-v7a` 和 `x86_64`，所以拿到它的 App 不需要再解析别的。要自己
写 NAPI 的 App 改用 `marsrs-harmony-<version>.tar.gz`：三个 `libmars_ffi.so` 和
`mars_xlog.h`，外面什么都没包。

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

xlog.flushNow();   // 返回时记录已经在磁盘上
xlog.close();
```

NAPI 模块的每个方法都是同步的，所以调用从发起它的线程返回，不回答 promise。
`LogLevel` 是 `Verbose` / `Debug` / `Info` / `Warning` / `Error` / `Fatal` /
`None` —— PascalCase，不是 Kotlin 那个 `INFO`，因为写 ArkTS 的 App 里满屏都是
HarmonyOS 的枚举。这个包的 ArkTS 没有伸到
[任务链路](/zh/stn/getting-started)或[网络诊断](/zh/sdt/getting-started)：HAR 是日
志。

## 接着看

- [配置项](/zh/xlog/configuration) —— 每个选项和它的默认值，以及每个平台上的写法。
- [日志文件](/zh/xlog/log-files) —— 文件落在哪里、叫什么名字、怎么读回来。
- [xlog 命令行](/zh/xlog/cli) —— 在 shell 里读一个 `.xlog`，以及造出决定谁能读的
  那对密钥。
