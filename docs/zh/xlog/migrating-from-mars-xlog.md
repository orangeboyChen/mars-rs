# 从 mars-xlog 迁移

只用了 C++ 项目日志库 —— 没用别的 —— 的应用要搬一样东西：appender。它写出来的就是
C++ 写出来的那个文件 —— 同样的帧结构、同样的压缩、同样的加密 —— 所以你已经收集到的
`.xlog` 文件，现有的工具照样能读，迁移不丢任何历史。

## 你用哪个包

| 你在用的 | 这里用什么 | 写在哪一页 |
|---|---|---|
| `mars/xlog` | `xlog`：crates.io 上的 `marsrs-xlog`、JitPack 上的 `xlog`、共享 Kotlin 模块的 `xlog-kmp`、Apple 上的 `MarsRSXlog` | [快速开始](/zh/xlog/getting-started) |

Rust 的 crate、JitPack 和共享 Kotlin 模块做的是这个切分：一个包只有日志库，另一个包是
整个移植，而整个移植里带着的是同一个日志库 —— 只有名字不同。Apple 是切成三份的：
`MarsRSXlog` 是日志库，`MarsRSNet` 是链路和诊断、完全没有日志库，`MarsRS` 是两份都
有。Flutter 和 React Native 的两个包里都只有日志库 —— 见
[Flutter](/zh/xlog/getting-started#flutter)和
[React Native](/zh/xlog/getting-started#react-native)。

只打日志的应用用日志库那个包，还要跑任务或诊断的用带着它们的那个；拿不准的从日志库
开始：`marsrs` 是日志库加另外两块，所以从一个包换到另一个包，文件这件事什么都不用改。

## appender

appender 是同一个东西：每个前缀一个进程级写入器，应用启动时开一次，往里写，读文件或
上传前 flush。变的是谁拿着它 —— C++ 在一组自由函数背后装一个进程级 appender，这里
你拿着自己打开的那个 `Xlog`，通过它写。

### 从 C++ 头文件来

appender 在 `mars/xlog/appender.h` 里是一组选项构成的一个 struct 加几个自由函数，在
这里也是一个 struct 加几个调用：

| C++ | Rust | C ABI |
|---|---|---|
| `appender_open(const XLogConfig&)` | `appender_open(XLogConfig)` | `mars_xlog_open(&config)` |
| `xlogger_Write(info, log)`，或 `xinfo2` 那一族 | `appender_write(info, message)` | `mars_xlog_write(...)` |
| `appender_flush()` | `appender_flush()` | `mars_xlog_flush()` |
| `appender_flush_sync()` | `appender_flush_sync()` | `mars_xlog_flush_sync()` |
| `appender_close()` | `appender_close()` | `mars_xlog_close()` |
| `xlogger_SetLevel(level)` | `set_level(handle, level)` | `mars_xlog_set_level(level)` |
| `appender_setmode(mode)` | `appender_set_mode(mode)` | `mars_xlog_set_mode(mode)` |
| `appender_set_console_log(bool)` | `appender_set_console_log(bool)` | `mars_xlog_set_console_log(on)` |
| `appender_set_max_file_size(bytes)` | `appender_set_max_file_size(bytes)` | `mars_xlog_set_max_file_size(bytes)` |
| `appender_set_max_alive_duration(secs)` | `appender_set_max_alive_duration(secs)` | `mars_xlog_set_max_alive_duration(secs)` |
| `appender_get_current_log_path(out, len)` | `appender_get_current_log_path()` | `mars_xlog_current_log_path(out, len)` |

config 在三种写法里都是一个 struct，八个字段还是那八个：C++ 里是 `mode_`、
`logdir_`、`nameprefix_`、`pub_key_`、`compress_mode_`、`compress_level_`、
`cachedir_`、`cache_days_`，这里是 `mode`、`logdir`、`nameprefix`、`pub_key`、
`compress_mode`、`compress_level`、`cachedir`、`cache_days`。每一个都在
[配置项](/zh/xlog/configuration)那页，按每个平台的写法列着。`TAppenderMode` 是
`AppenderMode`，`TCompressMode` 是 `CompressMode`，`TLogLevel` 是 `LogLevel`。

另一半要搬走的是宏。`XLOGGER_TAG` 和 `xverbose2` / `xdebug2` / `xinfo2` / `xwarn2` /
`xerror2` / `xfatal2` 那一族，把级别、tag 和调用点塞在一行 C++ 里；取代它们的是每个
级别一个方法 —— Kotlin 的 `xlog.i(tag, message)`、Swift 的 `log.info(message:tag:)`、
Rust 的 `appender_write(None, message)`。

调用点是三样里唯一不是每个平台都跟着走的那个。Swift 从 `#file`、`#function` 和 `#line`
填上文件、函数和行号，所以从 `log.info(message:tag:)` 写出的记录知道自己是哪儿写的。
Kotlin 的 write 只接 handle、级别、tag 和消息，没有别的，所以从 `xlog.i(tag, message)`
写出的记录里文件是空的、行号是 0 —— 也就是 C++ 项目自己的 `Log` 一直传的那两个值。要
把调用点写进记录的应用有两条路：旧的 `logWrite` 接的那个 `XLoggerInfo`，或者在 Rust
里给 `appender_write` 传一个而不是 `None`：

```rust
appender_write(
    Some(&XLoggerInfo {
        level: LogLevel::Info,
        tag: Some("startup".into()),
        filename: Some(file!().into()),
        func_name: Some("main".into()),
        line: line!() as i32,
        ..Default::default()
    }),
    "cold start in 412 ms",
);
```

第二个 appender 是形状差别最大的地方。C++ 项目的 Java 是用
`Log.openLogInstance(level, mode, cacheDir, logDir, nameprefix, cacheDays)` 开第二个，
然后把它返回的 handle 一路传下去；这里你拿着一个 appender 就*是*拿到一个实例，所以第
二个就是自己的另一个前缀的第二个 `Xlog`，或者 Rust 里的 `*_instance` 那一族 ——
`appender_open_instance(config)` 返回 handle，`appender_write_instance`、
`appender_flush_instance` 和 `appender_close_instance` 接它。

### 从 C++ 项目的 Java 来

Android 包是唯一还留着旧写法的地方：七个参数的 `Xlog.open`、`XLogConfig`、
`XLoggerInfo`、`logWrite` 和 `LEVEL_*` 常量都还能用。带着 `@Deprecated` 和取代它的写法
的是七个参数的 `Xlog.open`、`logWrite`、无参的 `Xlog()` 以及围着它们的 `Log` 门面；
`XLogConfig`、`XLoggerInfo` 和 `LEVEL_*` 常量没有这个标记，所以用到它们的调用点照样
编译，没有任何东西把它指向新写法，找出来是应用自己的 grep。`Log.setLogImp(Xlog())`
和 `Log.d(tag, message)` 仍然写进 `Xlog.open` 装上的那个 appender，所以其余部分可以
一个调用点一个调用点地走：

```kotlin
// 之前
Log.setLogImp(Xlog())
Log.d("net", "…")

// 之后
val xlog = Xlog.open(XlogConfig(logDir = dir, namePrefix = "marsrs"))
xlog.d("net", "…")
```

新写法多给一样东西：`Context`。`Xlog.open(config, context)` 会在应用离开屏幕时自己
flush，那是 Android 在可以不打招呼就结束进程之前最后一个还会说话的时刻 —— 见
[Android](/zh/xlog/log-files#app-退出的时候)。

### 从 Apple 的头文件来

没有 Objective-C 封装要搬：Apple 上的应用调的是 Objective-C++ 文件里的 C++ ——
`xlogger_SetLevel`、`appender_set_console_log`、一个字段一个字段填的 `XLogConfig`，
以及 `appender_open(config)`。那个文件变成导入模块、拿着自己打开的 appender 的文件：

::: code-group

```objc [之前]
XLogConfig config;
config.mode_ = kAppenderAsync;
config.logdir_ = [logPath UTF8String];
config.nameprefix_ = "Test";
config.pub_key_ = "";
config.compress_mode_ = kZlib;
config.compress_level_ = 0;
config.cachedir_ = "";
config.cache_days_ = 0;
appender_open(config);
```

```objc [之后]
@import MarsRSXlog;

XlogConfig *config = [[XlogConfig alloc] initWithLogDirectory:logPath.path];
config.namePrefix = @"marsrs";

NSError *error = nil;
Xlog *log = [[Xlog alloc] initWithConfig:config error:&error];
[log writeWithLevel:LogLevelInfo message:@"cold start" tag:@"startup"];
[log flushWithSync:YES];        // 读文件或上传前
```

:::

Swift 会在调用点填上文件、函数和行号；Objective-C 没有 `#file` 可填，所以这里写的记录
里文件是空的、行号是 0，除非用长形式把它们写上 ——
`[log log:message:tag:file:function:line:]`。

取代那个头文件的是 `MarsRSXlog` —— 一个 SwiftPM product 和一个同名的 pod，同一个
framework，下面是 `@objc` 的接口。[SwiftPM](/zh/xlog/getting-started#swiftpm)和
[CocoaPods](/zh/xlog/getting-started#cocoapods)是它的两页。

## 接下来

- [配置项](/zh/xlog/configuration) —— appender 的每个配置项和默认值，按每个平台的写法。
- [日志文件](/zh/xlog/log-files) —— 文件落在哪、怎么读回来，给上传路径已经认识 C++
  那个文件的应用。
- [从 mars-stn 迁移](/zh/stn/migrating-from-mars-stn)和
  [从 mars-sdt 迁移](/zh/sdt/migrating-from-mars-sdt) —— 给日志库不是它唯一拿的
  那块的应用：任务链路和网络诊断。
