# 配置项

开一个 appender 只需要一份配置：选项都一样，各平台拼法不同。只有
`logdir` / `logDir` / `logDirectory` 没有默认值 —— 没有写的地方就开不了 appender。

| 作用 | Rust | Swift | Android | Kotlin Multiplatform | Flutter / React Native | C | C++ | HarmonyOS | 默认值 |
|---|---|---|---|---|---|---|---|---|---|
| `.xlog` 文件写到哪；目录不存在会创建 | `logdir` | `logDirectory` | `logDir` | `logDir` | `logDir` | `log_dir` | `logDir` | `logDir` | **必填**（Rust 里是 `./log`） |
| 每个文件名的开头，也是这个 appender 的名字 | `nameprefix` | `namePrefix` | `namePrefix` | `namePrefix` | `namePrefix` | `name_prefix` | `namePrefix` | `namePrefix` | `xlog` |
| 记录要达到的级别 | 见[级别](#级别) | `level` | `level` | `level` | `level` | `mars_xlog_new_instance(&config, level)` | `level` | `level` | `info` |
| 写入是否等落盘 | `mode` | `mode` | `mode` | `mode` | `mode` | `mode` | `mode` | `mode` | 异步 |
| 异步缓存文件放哪 | `cachedir` | `cacheDirectory` | `cacheDir` | `cacheDir` | `cacheDir` | `cache_dir` | `cacheDir` | `cacheDir` | 和日志文件同一个目录 |
| 缓存文件保留几天 | `cache_days` | `cacheDays` | `cacheDays` | `cacheDays` | `cacheDays` | `cache_days` | `cacheDays` | `cacheDays` | `0` —— 都留着 |
| 关闭的文件用什么压缩 | `compress_mode` | `compression` | `compressMode` | `compressMode` | `compressMode` | `compress_mode` | `compressMode` | `compressMode` | zlib |
| 压缩到什么程度 | `compress_level` | `compressionLevel` | `compressLevel` | `compressLevel` | `compressLevel` | `compress_level` | `compressLevel` | `compressLevel` | `0` —— 用压缩器自己的（zlib 是 6） |
| 加密用的公钥 | `pub_key` | `publicKey` | `pubKey` | `pubKey` | `pubKey` | `pub_key` | `pubKey` | `pubKey` | 空 —— 不加密 |

appender 接受不了的配置，在构造它的地方就被拒绝，而不是被库悄悄吞掉：
Swift 和 C++ 抛 `XlogError`，Kotlin 抛 `IllegalArgumentException`，Rust 返回 `Err`，C 返回负的
`MARS_XLOG_ERR_*`，ArkTS 抛 `Error`；两个要跨桥的平台，失败的是打开它的那次调用 ——
Dart 是 `PlatformException`，TypeScript 是抛出的 `Error`。

## 级别

appender 的级别不高于记录的级别时，这条记录才会被写进去：开在 `info` 的
appender 会留下 `warning`，丢掉 `debug`。

| 级别 | Rust | Swift | Android | KMP | Flutter / React Native | C | C++ | HarmonyOS |
|---|---|---|---|---|---|---|---|---|
| 0 | `LogLevel::Verbose` | `.verbose` | `LogLevel.VERBOSE` | `LogLevel.VERBOSE` | `LogLevel.verbose` | `MarsLevelVerbose` | `LogLevel::Verbose` | `LogLevel.Verbose` |
| 1 | `LogLevel::Debug` | `.debug` | `LogLevel.DEBUG` | `LogLevel.DEBUG` | `LogLevel.debug` | `MarsLevelDebug` | `LogLevel::Debug` | `LogLevel.Debug` |
| 2 | `LogLevel::Info` | `.info` | `LogLevel.INFO` | `LogLevel.INFO` | `LogLevel.info` | `MarsLevelInfo` | `LogLevel::Info` | `LogLevel.Info` |
| 3 | `LogLevel::Warn` | `.warning` | `LogLevel.WARNING` | `LogLevel.WARNING` | `LogLevel.warning` | `MarsLevelWarn` | `LogLevel::Warning` | `LogLevel.Warning` |
| 4 | `LogLevel::Error` | `.error` | `LogLevel.ERROR` | `LogLevel.ERROR` | `LogLevel.error` | `MarsLevelError` | `LogLevel::Error` | `LogLevel.Error` |
| 5 | `LogLevel::Fatal` | `.fatal` | `LogLevel.FATAL` | `LogLevel.FATAL` | `LogLevel.fatal` | `MarsLevelFatal` | `LogLevel::Fatal` | `LogLevel.Fatal` |
| 6 | `LogLevel::None` | `.none` | `LogLevel.NONE` | `LogLevel.NONE` | `LogLevel.none` | `MARS_LEVEL_NONE` | `LogLevel::None` | `LogLevel.None` |

两个 Kotlin 列是故意写成一样的：`xlog-kmp` 和 `xlog` 发布的是同一个 API，所以在两者
之间搬动的共享模块什么都不用改。跟着它们的那两列也一样 —— `marsrs_xlog` 和
`marsrs-react-native-xlog` 用的是同一套选项的同一个拼法，就是 Kotlin 的那套。

HarmonyOS 那一列也是 Kotlin 的拼法，只是大小写不同：`LogLevel.Verbose`，而不是
`LogLevel.VERBOSE` —— ArkTS 的枚举是要写在一堆 HarmonyOS 枚举里的。

`none` 什么都不写，连 `fatal` 也不写 —— 想让 appender 安静下来又不关掉它，用这个。

移植里没有 assert：`mars_xlog_assert` 是进程级 appender 的，跟着它一起没了。想要的
App 自己写那条记录 —— `xlog.f(tag, message)` —— 写完再停进程，
`std::process::abort()` 或者平台自己的陷阱；上游写完这条记录之后会在 Android 上
`raise(SIGTRAP)`、在 Apple 上调 `__assert_rtn`，这里两个都不做。

构造起来很贵的消息值得先问一句：被级别挡掉的记录也是一样 —— 那串字符串你已经拼好了。

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
  xlog.d("net", expensiveDescription());
}
```

```ts [React Native]
if (xlog.isLoggable(LogLevel.debug)) {
  xlog.d("net", expensiveDescription());
}
```

```typescript [HarmonyOS]
if (xlog.isLoggable(LogLevel.Debug)) {
  xlog.d('net', expensiveDescription());
}
```

:::

## 异步还是同步

| | 异步（默认） | 同步 |
|---|---|---|
| 一次写入做了什么 | 压缩进 mmap 缓存，交给写线程 | 直接写进文件 |
| 换来什么 | `write` 系统调用不在打日志的线程上 | 打日志的线程要等文件 |
| 你要做什么 | 读文件或上传前 `flushNow()`；App 退出时什么都不用做（见[日志文件](/zh/xlog/log-files)） | 进程可能被杀之前 `close()` 或 `flushNow()` —— 还攒着的那截会跟着进程一起走 |

默认是异步，因为文件比一条记录慢；同步用在一条记录等不了写线程的地方 ——
崩溃日志，或者退出前的最后几行。

两种模式都不是每条记录都进内核：都先攒着，攒到大约 4 KiB，或者 `close()` /
`flushNow()` 的时候才交给系统。异步攒着不丢东西 —— 记录同时在内核手里那份
缓存文件里；同步后面什么都没有，进程被杀时攒着的那截就丢了。退出前要调的那一下，
就是为这一截。

## 压缩与加密

默认 `zlib`，`zstd` 压得更紧；两者都是按文件、在文件关闭时生效。级别是压缩器自己的旋钮：
`0` 用默认值（zlib 是 6），zlib 的上限是 `9`，zstd 是 `22`。

给了 `pubKey`，每条记录的正文会用 ECDH + TEA 加密 —— 也就是 C++ 实现用的那个算法，
密钥由写方和读方协商出来。这里填的是**公**钥，
读日志用的私钥是它的另一半；留空则写出的文件任何 mars 日志读取工具都能打开。

## 打开之后

下面这些不在配置里，而是 appender 上的 setter 或属性，都从下一条记录开始生效：

| 作用 | Rust | Swift | Android | Kotlin Multiplatform | Flutter | React Native | C | C++ | HarmonyOS |
|---|---|---|---|---|---|---|---|---|---|
| 改级别 | `xlog.set_level` | `log.level` | `xlog.level` | `xlog.level` | `xlog.level` | `xlog.level` | `mars_xlog_set_level_instance(handle, level)` | `log.setLevel` | `xlog.level` |
| 切异步 / 同步 | `xlog.set_mode` | `log.mode` | `xlog.mode` | `xlog.mode` | `xlog.mode` | `xlog.mode` | `mars_xlog_set_mode_instance(handle, mode)` | `log.setMode` | `xlog.mode` |
| 同时打到控制台 | `xlog.set_console_log_enabled` | `log.isConsoleLogEnabled` | `xlog.consoleLogEnabled` | `xlog.consoleLogEnabled` | `xlog.consoleLogEnabled` | `xlog.consoleLogEnabled` | `mars_xlog_set_console_log_instance(handle, on)` | `log.setConsoleLogEnabled` | `xlog.consoleLogEnabled` |
| 到 N 字节就换文件 | `xlog.set_max_file_size_bytes` | `log.maxFileSizeBytes` | `xlog.maxFileSizeBytes` | `xlog.maxFileSizeBytes` | `xlog.maxFileSizeBytes` | `xlog.maxFileSizeBytes` | `mars_xlog_set_max_file_size_instance(handle, bytes)` | `log.setMaxFileSizeBytes` | `xlog.maxFileSizeBytes` |
| 超过 N 秒就删文件 | `xlog.set_max_alive_time_seconds` | `log.maxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` | `mars_xlog_set_max_alive_duration_instance(handle, secs)` | `log.setMaxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` |
| 写到哪 | `xlog.current_log_path()` | `log.currentLogPath` | `xlog.currentLogPath` | `xlog.currentLogPath` | `await xlog.currentLogPath()` | `xlog.currentLogPath` | `mars_xlog_current_log_path_instance` | `log.currentLogPath()` | `xlog.currentLogPath` |
| 一天的文件在哪 | `xlog.log_files(1)` | `log.logFiles(daysAgo: 1)` | `xlog.logFiles(1L)` | `xlog.logFiles(1L)` | `await xlog.logFiles(1)` | `xlog.logFiles(1)` | `mars_xlog_getfilepath_from_timespan_instance` | `log.logFiles(1)` | `xlog.logFiles(1)` |

它给出的是**目录**而不是文件，也就是 C++ 的 `GetCurrentLogPath` 交回来的那个；当天的文件在下面一行。

大小的 `0` 是“不限制”：文件永不切分。时间的 `0` 不是 —— 不到一天的值一律被忽略，
`0` 也在内，于是 appender 保留它已有的那个期限：打开时的那十天，或者上次给它的那个
不少于一天的值。C++ 那边的默认也是这十天。

## 控制台那一副本去哪

`xlog.set_console_log_enabled(true)` —— 在那些这么拼的平台上写作
`xlog.consoleLogEnabled = true` —— 让每条记录除了进文件，还抄一份到控制台。通过 Rust 的
`Xlog` 写出的记录都带着一个 `XLoggerInfo` —— 调用时给的级别和 tag —— 所以控制台拿到的是
整条记录，而不只是那句话。

内置的出口是标准错误，每个平台、每个包都是 —— 这里没有任何一处写进 `os_log` 或
logcat，也没有 sink 可设了：原来那个坐在进程级 appender 上，而那个 appender 已经没了。
在 Apple 上系统日志是 App 自己调的那一次调用 —— `os_log` 是个宏，库碰不到它。

想把记录送到别处的 App 要么写两遍，要么从文件里读回来：控制台那份归标准错误，移植不把
它交到别处去。
