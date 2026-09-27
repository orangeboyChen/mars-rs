# 配置项

开一个 appender 只需要一份配置，各平台用各自的拼法带着同样的选项。只有
`logdir` / `logDir` / `logDirectory` 没有默认值 —— 没有写的地方就开不了 appender。

| 作用 | Rust | Swift | Android | Kotlin Multiplatform | Flutter / React Native | C | 默认值 |
|---|---|---|---|---|---|---|---|
| `.xlog` 文件写到哪；目录不存在会创建 | `logdir` | `logDirectory` | `logDir` | `logDir` | `logDir` | `log_dir` | **必填**（Rust 里是 `./log`） |
| 每个文件名的开头，也是这个 appender 的名字 | `nameprefix` | `namePrefix` | `namePrefix` | `namePrefix` | `namePrefix` | `name_prefix` | `xlog`（Rust 里是 `Mars`） |
| 记录要达到的级别 | 见[级别](#级别) | `level` | `level` | `level` | `level` | `mars_xlog_set_level` | `info` |
| 写入是否等落盘 | `mode` | `mode` | `mode` | `mode` | `mode` | `mode` | 异步 |
| 异步缓存文件放哪 | `cachedir` | `cacheDirectory` | `cacheDir` | `cacheDir` | `cacheDir` | `cache_dir` | 和日志文件同一个目录 |
| 缓存文件保留几天 | `cache_days` | `cacheDays` | `cacheDays` | `cacheDays` | `cacheDays` | `cache_days` | `0` —— 都留着 |
| 关闭的文件用什么压缩 | `compress_mode` | `compression` | `compressMode` | `compressMode` | `compressMode` | `compress_mode` | zlib |
| 压缩到什么程度 | `compress_level` | `compressionLevel` | `compressLevel` | `compressLevel` | `compressLevel` | `compress_level` | `0` —— 用压缩器自己的（zlib 是 6） |
| 加密用的公钥 | `pub_key` | `publicKey` | `pubKey` | `pubKey` | `pubKey` | `pub_key` | 空 —— 不加密 |

一份 appender 没法接受的配置会在你构造它的地方就被拒绝，而不是被库悄悄吞掉：
Swift 抛 `XlogError`，Kotlin 抛 `IllegalArgumentException`，Rust 返回 `Err`，C 返回负的
`MARS_XLOG_ERR_*`；两个要跨桥的平台上，打开它的那次调用失败 —— Dart 是
`PlatformException`，TypeScript 是被 reject 的 `Promise`。

## 级别

appender 的级别不高于记录的级别时，这条记录才会被写进去：开在 `info` 的
appender 会留下 `warning`，丢掉 `debug`。

| 级别 | Rust | Swift | Android | KMP | Flutter / React Native | C |
|---|---|---|---|---|---|---|
| 0 | `LogLevel::Verbose` | `.verbose` | `LogLevel.VERBOSE` | `LogLevel.VERBOSE` | `LogLevel.verbose` | `MarsLevelVerbose` |
| 1 | `LogLevel::Debug` | `.debug` | `LogLevel.DEBUG` | `LogLevel.DEBUG` | `LogLevel.debug` | `MarsLevelDebug` |
| 2 | `LogLevel::Info` | `.info` | `LogLevel.INFO` | `LogLevel.INFO` | `LogLevel.info` | `MarsLevelInfo` |
| 3 | `LogLevel::Warn` | `.warning` | `LogLevel.WARNING` | `LogLevel.WARNING` | `LogLevel.warning` | `MarsLevelWarn` |
| 4 | `LogLevel::Error` | `.error` | `LogLevel.ERROR` | `LogLevel.ERROR` | `LogLevel.error` | `MarsLevelError` |
| 5 | `LogLevel::Fatal` | `.fatal` | `LogLevel.FATAL` | `LogLevel.FATAL` | `LogLevel.fatal` | `MarsLevelFatal` |
| 6 | `LogLevel::None` | `.none` | `LogLevel.NONE` | `LogLevel.NONE` | `LogLevel.none` | `MARS_LEVEL_NONE` |

两个 Kotlin 列是故意写成一样的：`xlog-kmp` 和 `xlog` 发布的是同一个 API，所以在两者
之间搬动的共享模块什么都不用改。跟着它们的那两列也一样 —— `marsrs_flutter_xlog` 和
`marsrs-react-native-xlog` 用的是同一套选项的同一个拼法，就是 Kotlin 的那套。

`none` 什么都不写，连 `fatal` 也不写 —— 这是不关掉 appender 而让它安静下来的办法。

构造起来很贵的消息，值得先问一句：被级别丢掉的记录，那串字符串你照样已经付过了。

::: code-group

```swift [Swift]
if log.isEnabled(for: .debug) {
    log.debug(message: "\(expensiveDescription())", tag: "net")
}
```

```kotlin [Android]
if (xlog.isLoggable(LogLevel.DEBUG)) {
    xlog.d("net", expensiveDescription())
}
```

```dart [Flutter]
if (await xlog.isLoggable(LogLevel.debug)) {
  await xlog.d("net", expensiveDescription());
}
```

```ts [React Native]
if (await xlog.isLoggable(LogLevel.debug)) {
  await xlog.d("net", expensiveDescription());
}
```

:::

## 异步还是同步

| | 异步（默认） | 同步 |
|---|---|---|
| 一次写入做了什么 | 压缩进 mmap 缓存，交给写线程 | 直接写进文件 |
| 换来什么 | `write` 系统调用不在打日志的线程上 | 打日志的线程要等文件 |
| 你要做什么 | 读文件或上传前、进程退出前 `flush(sync: true)` | 不用做什么 |

默认是异步，因为文件比一条记录慢；同步用在"下一条代码跑之前必须落盘"的地方 ——
崩溃日志，或者退出前的最后几行。

## 压缩与加密

默认 `zlib`，`zstd` 压得更紧；两者都是按文件、在文件关闭时生效。级别是压缩器自己的旋钮：
`0` 用默认值（zlib 是 6），zlib 的上限是 `9`，zstd 是 `22`。

给了 `pubKey`，每条记录的正文会用 ECDH + AES-GCM 加密。这里填的是**公**钥，
读日志用的私钥是它的另一半；留空则写出的文件任何 mars 日志读取工具都能打开。

## 打开之后

下面这些不在配置里，而是 appender 上的 setter，都从下一条记录开始生效：

| 作用 | Rust | Swift | Android | Kotlin Multiplatform | Flutter / React Native | C |
|---|---|---|---|---|---|---|
| 改级别 | `set_level` | `log.level` | `xlog.level` | `xlog.level` | `await xlog.setLevel(…)` | `mars_xlog_set_level` |
| 切异步 / 同步 | `appender_set_mode` | `log.mode` | `xlog.mode` | `xlog.mode` | `await xlog.setMode(…)` | `mars_xlog_set_mode` |
| 同时打到控制台 | `appender_set_console_log` | `log.isConsoleLogEnabled` | `xlog.consoleLogEnabled` | `xlog.consoleLogEnabled` | `await xlog.setConsoleLogEnabled(…)` | `mars_xlog_set_console_log` |
| 到 N 字节就换文件 | `appender_set_max_file_size` | `log.maxFileSizeBytes` | `xlog.maxFileSizeBytes` | `xlog.maxFileSizeBytes` | `await xlog.setMaxFileSize(…)` | `mars_xlog_set_max_file_size` |
| 超过 N 秒就删文件 | `appender_set_max_alive_duration` | `log.maxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` | `await xlog.setMaxAliveTime(…)` | `mars_xlog_set_max_alive_duration` |
| 当前文件在哪 | `appender_get_current_log_path` | `Xlog.currentLogPath` | — | — | — | `mars_xlog_current_log_path` |

大小和时间的 `0` 都表示"不限制"：文件永不切分、永不删除 —— C++ 那边自己保留十天。
