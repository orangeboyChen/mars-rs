# Kotlin Multiplatform

`commonMain` 里加一个依赖，每个平台编译它自己的那一半：`androidMain` 走 JNI 桥，
`nativeMain` 通过 cinterop 走 C ABI —— 所以共享代码里的同一句调用，在 Android、iOS、
watchOS、tvOS、macOS、Linux、Windows 上写出的都是同一个 `.xlog`。

```kotlin
// settings.gradle.kts
maven {
    url = uri("https://maven.pkg.github.com/orangeboyChen/mars-rs")
    credentials { username = "<user>"; password = "<token with read:packages>" }
}

// 共享模块的 build.gradle.kts
implementation("io.github.orangeboychen.marsrs:xlog-kmp:0.1.0")    // 只有 xlog
implementation("io.github.orangeboychen.marsrs:marsrs-kmp:0.1.0")  // 整个端口
```

不想为 GitHub Packages 配认证的应用，可以拿 release 里的 `marsrs-kmp-maven.zip`：
解压后加 `maven { url = uri("<dir>") }`。

## 打开、写、flush

```kotlin
import io.github.orangeboychen.marsrs.xlog.AppenderMode
import io.github.orangeboychen.marsrs.xlog.LogLevel
import io.github.orangeboychen.marsrs.xlog.Xlog
import io.github.orangeboychen.marsrs.xlog.XlogConfig

val xlog = Xlog.open(
    XlogConfig(
        logDir = logDirectory,
        namePrefix = "marsrs",
        level = LogLevel.INFO,
        mode = AppenderMode.ASYNC,
    )
)
xlog.consoleLogEnabled = isDebug

xlog.i("startup", "cold start in $elapsedMillis ms")

xlog.flush(sync = true)   // 读文件或上传前
xlog.close()
```

`logDir` 是唯一没有默认值的选项，其余都在[配置项](/zh/configuration)那页。

这就是 [Android](/zh/platforms/android) 那个 `Xlog`：同一个构造函数、同样的成员、
同样的名字，应用调用的名字是 `Xlog.open(config)`，所以在 `xlog-kmp` 和 `xlog`
之间搬动的共享模块什么都不用改。

## 写

写是 `android.util.Log` 那个形状 —— `v`/`d`/`i`/`w`/`e`/`f`，每个都是 tag 加消息，
级别要到调用时才知道就用 `log(level, tag, message)`。

```kotlin
xlog.v("net", "…")
xlog.d("net", "…")
xlog.i("startup", "…")
xlog.w("net", "…")
xlog.e("login", "…")
xlog.f("login", "…")

xlog.log(LogLevel.DEBUG, "net", "…")
```

级别低于 appender 打开时那个级别的记录，在格式化之前就被丢掉了。构造起来很贵的消息
值得先问一句，因为被丢掉的记录仍然要调用方先把 `String` 拼出来：

```kotlin
if (xlog.isLoggable(LogLevel.DEBUG)) {
    xlog.d("net", expensiveDescription())
}
```

## 开着的时候

| 要什么 | 怎么写 |
|---|---|
| 改级别 | `xlog.level = LogLevel.WARNING` |
| 切异步 / 同步 | `xlog.mode = AppenderMode.SYNC` |
| 同时打到控制台 | `xlog.consoleLogEnabled = true` |
| 到某个大小就切文件 | `xlog.maxFileSizeBytes = 8 * 1024 * 1024` |
| 到某个年龄就删文件 | `xlog.maxAliveTimeSeconds = 10 * 24 * 3600` |
| 还开着吗 | `xlog.isOpen` |
| 把缓存排空 | `xlog.flush(sync = true)` |

`close()` 排空剩下的并关掉这个 appender。同一个 `namePrefix` 的两个 `Xlog` 是同一个
appender，所以关掉一个就关掉了另一个正在写的 —— 应用里日志要单独读的那部分，给它一个
自己的 prefix。

## 哪些平台

十四个 Kotlin target：Android（AAR 里带 `arm64-v8a`、`armeabi-v7a`、`x86_64` 的
`libmarsrsxlog.so`），加上 `iosArm64`、`iosX64`、`iosSimulatorArm64`、`macosX64`、
`macosArm64`、`watchosArm64`、`watchosDeviceArm64`、`watchosSimulatorArm64`、
`tvosArm64`、`tvosSimulatorArm64`、`linuxX64`、`linuxArm64`、`mingwX64`。

编译共享模块时不编译任何 Rust：每个平台链接的是 release 已经为它发布的静态库 ——
而 `marsrs-kmp` 每个 target 链两个，`libmars_ffi.a` 给 xlog，`libmars_net_ffi.a` 给
STN 和 SDT，这样只拿 `xlog-kmp` 的 App 才一个字节的任务链路都不带。

## 不在这里面的

这个 API 面是两座桥的交集，也是 `common` 声明唯一能是的东西。C ABI 那个进程级的
appender —— `mars_xlog_open`、`mars_xlog_close`、`mars_xlog_current_log_path` —— 不在
里面，因为 JNI 桥没有对应的东西：要用它们就是某个单一平台的调用方，写在那平台的
source set 里。

两个 `actual` 回答得不一样的有四处，共享 API 就是两边都能说的那个：

- `StnLogic.dueTime()` 回答 `Long?` —— 下一趟还能等多少毫秒，没有要等的东西时是 `null`
  —— 因为 JNI 桥回答
  `-1`，C ABI 回答它自己的 `MARS_STN_ERR_NO_DUE`。
- `StnLogic.makesureLongLinkConnected()` 什么都不回答：C ABI 那个符号回答 1 或 0，JNI
  那个回答 `void`，所以想知道的调用方去读 `Question.linkStatus`。
- Android 上 App 读到的 `CgiProfile`，只有 C ABI 有的那两个读数 ——
  `sendPacketFinishedTime` 和 `netType` —— 是 `0` 和 `""`。
- `setApp` 在 Kotlin/Native 上被问到全部十八个问题，在 Android 上是十三个：两个网络错误、
  长连接的状态变化、任务上限和 DNS profile 这五个，是 JNI 桥自己回答的。

## 任务链路

`marsrs-kmp` 带着这个移植里跟服务器说话的那半：`StnLogic` 是 `commonMain` 里一个
`expect object`，每个平台族一个 `actual` —— Android 上走在 `crates/marsrs-jni` 的 JNI
桥上，每个 Kotlin/Native target 上通过 cinterop 走在 `crates/marsrs-ffi` 的 C ABI 上。

```kotlin
import io.github.orangeboychen.marsrs.stn.Answer
import io.github.orangeboychen.marsrs.stn.FailHandle
import io.github.orangeboychen.marsrs.stn.Question
import io.github.orangeboychen.marsrs.stn.StnLogic
import io.github.orangeboychen.marsrs.stn.Task

// 一个 `ask` 回答到 App 这里的那十三个问题
StnLogic.setApp { question ->
    when (question.kind) {
        Question.Kind.Req2Buf -> Answer.Encoded(encode(question.task))
        Question.Kind.Buf2Resp -> {
            handle(question.body)
            Answer.Decoded(errorCode = 0, handle = FailHandle.Normal)
        }
        Question.Kind.OnTaskEnd -> Answer.Ended(errorCode = 0)
        else -> Answer.None
    }
}

val task = Task().apply {
    taskID = StnLogic.genTaskID()
    channelSelect = Task.E_BOTH
    cmdID = 100
    cgi = "/cgi-bin/hello"
    shortLinkHostList = listOf("example.com")
    totalTimeout = 10_000
}
StnLogic.startTask(task)

// 这个移植没有线程：队列由谁调用谁来排空
while (StnLogic.dueTime() != null) {   // null 是"没有要等的东西"
    StnLogic.runPending()
}
```

[任务链路](/zh/stn)是它的全部 —— 两条连接、一个任务的各个字段、任务怎么结束、长连接要
App 做什么。

## 网络诊断

```kotlin
import io.github.orangeboychen.marsrs.sdt.Link
import io.github.orangeboychen.marsrs.sdt.ProbeAnswer
import io.github.orangeboychen.marsrs.sdt.SdtLogic

SdtLogic.setCallBack { resultsJson -> send(resultsJson) }   // 报告送到哪
SdtLogic.setHttpNetcheckCGI("http://example.com/netcheck")
SdtLogic.startActiveCheck(
    arrayOf(Link("default", arrayOf("1.2.3.4"), intArrayOf(80))),
    emptyArray(),
    0,      // 模式
    10_000, // 超时
)
SdtLogic.runChecks(
    1,
    object : SdtLogic.IProbe {
        override fun dns(host: String, timeoutMs: Int) =
            ProbeAnswer.Dns(errorCode = 0, rtt = 12, addresses = listOf("1.2.3.4"))
        override fun tcp(host: String, port: Int, timeoutMs: Int) =
            ProbeAnswer.Tcp(sent = 0, received = 0, isNoopResponse = true, rtt = 30)
        override fun http(url: String, timeoutMs: Int) =
            ProbeAnswer.Http(errorCode = 0, statusCode = 200, rtt = 40)
        override fun ping(host: String, timeoutSec: Int) =
            ProbeAnswer.Ping(errorCode = 0, rtt = 20, lossRate = 0f, averageRTT = 18f)
    },
)
SdtLogic.takeReport()?.let { send(it) }   // 只回答一次，然后清空
```

四个探针是 App 的：这个移植不持有任何 socket，所以一次检查是向交给 `runChecks` 的那个
`IProbe` 一个一个地问，都在调用它的那个线程上，而且这个调用要等每个探针都回答了才返回，
所以一次只有一个诊断在跑。

[网络诊断](/zh/sdt)是它的全部：那个模式、那份计划、以及报告的那份 JSON。
