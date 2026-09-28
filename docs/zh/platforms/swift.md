# SwiftPM

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")

// 在要用的 target 里：
.product(name: "MarsRSXlog", package: "mars-rs")
```

三个 product，每个都用背后那个 module 的名字：product 是 manifest 里依赖的名字，module
是 `import` 写的名字，这里两者同名，所以 `Package.swift` 里的 `MarsRSXlog` 就是代码里的
`import MarsRSXlog`。

| product | 拿到什么 |
|---|---|
| `MarsRSXlog` | 日志：`Xlog`、`XlogConfig`、`LogLevel`、`AppenderMode`、`CompressMode` |
| `MarsRSNet` | STN 任务链路和 SDT 网络诊断 |
| `MarsRS` | 两半都重新导出 |

包本身叫 `mars-rs`，跟着拉取它的 URL 走；每次 release 发布的两个文件是
`marsrs-xlog.xcframework.zip` 和 `marsrs-net.xcframework.zip` —— 文件名对编译器没有要求，
SwiftPM 按 binary target 的名字解压 zip，再读里面的 module map。

framework 有四个 slice —— `ios-arm64`、`ios-arm64_x86_64-simulator`、
`watchos-arm64_arm64_32`、`watchos-arm64-simulator` —— 两个平台的 App target 都能解析到。

## 打开、写、flush

```swift
import MarsRSXlog

let log = try Xlog.open(
    XlogConfig(
        logDirectory: logDirectory.path,
        cacheDirectory: cacheDirectory.path,
        namePrefix: "marsrs",
        level: .info
    )
)
log.isConsoleLogEnabled = true

log.info(message: "cold start in \(elapsedMillis) ms", tag: "startup")
log.error(message: "login failed\n\(error)", tag: "login")

log.flush(sync: true)    // 读文件或上传前
```

`XlogConfig(logDirectory:)` 是简写形式，其余字段在之后赋值，都在
[配置项](/zh/configuration)那页：

```swift
var config = XlogConfig(logDirectory: logDirectory.path)
config.mode = .sync
config.compression = .zstd
config.compressionLevel = 6
config.publicKey = publicKey      // 留空写出的文件不加密
```

appender 拒绝这份配置时，`Xlog.open(...)` 会抛 `XlogError` —— 空的日志目录、空的
前缀、压缩器不接受的压缩级别、负的 `cacheDays`。

## 写

一个级别一个方法，都是一条消息加一个 tag；`file`、`function`、`line` 取的是调用处，
所以调用方不用自己填，记录里也带着它是在哪写的：

```swift
log.verbose(message: "…", tag: "net")
log.debug(message: "…", tag: "net")
log.info(message: "…", tag: "startup")
log.warning(message: "…", tag: "net")
log.error(message: "…", tag: "login")
log.fatal(message: "…", tag: "login")

log.log(.debug, message: "…", tag: "net")   // 级别要到调用时才知道
```

级别低于 appender 的记录，在格式化之前就被丢掉了。构造起来很贵的消息值得先问一句：

```swift
if log.isEnabled(for: .debug) {
    log.debug(message: "\(expensiveDescription())", tag: "net")
}
```

## 开着的时候

| 作用 | 怎么写 |
|---|---|
| 改级别 | `log.level = .warning` |
| 切异步 / 同步 | `log.mode = .sync` |
| 同时打到控制台 | `log.isConsoleLogEnabled = true` |
| 到某个大小换文件 | `log.maxFileSizeBytes = 8 * 1024 * 1024` |
| 到某个时间删文件 | `log.maxAliveTimeSeconds = 10 * 24 * 3600` |
| 还开着吗 | `log.isOpen` |
| 当前文件在哪 | `Xlog.currentLogPath` |

`close()` 排空剩下的内容并丢掉这个 appender；之后再写就什么都不写了。同一个
`namePrefix` 的两个 `Xlog` 是同一个 appender（C ABI 返回它已经有的那个 handle），
所以关掉其中一个，另一个也就写不了了。

C 的那些符号也在：`import MarsRSXlog` 重新导出了 `MarsRSFFI`，所以
`mars_xlog_open`、`mars_xlog_write` 这些照样能直接调。

## App 退出的时候

**什么都不用调用。** App 离开屏幕时，`Xlog` 自己会 flush：它监听 `didEnterBackground`
—— 有 scene 的 App 监听的是 scene 的那个，因为这类 App 根本收不到 `UIApplication` 的
生命周期通知 —— 以及 `willTerminate`，两个通知都会当场跑一次 `flush(sync: true)`。

那是排空的时刻，因为它是 iOS 最后一次开口：从后台被杀掉的 App 一个通知都收不到，之后再
没有 `flush` 能跑的地方。没有它也什么都不丢 —— 记录留在缓存文件里，下一个同 `namePrefix`
的 `Xlog` 打开时会把它们排进日志文件 —— 但本次会话那个文件要等它跑完才算完整。
见[日志文件](/zh/log-files)。
