# Android

```kotlin
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen.marsrs:xlog:0.1.0")    // 只有 xlog
implementation("io.github.orangeboychen.marsrs:marsrs:0.1.0")  // 整个端口：还有 STN、SDT
```

两个 AAR 带的是同一个 `libmarsrsxlog.so`，覆盖 `arm64-v8a`、`armeabi-v7a`、`x86_64`。
只打日志就用 `xlog`。Kotlin 和 Java 的包名是 `io.github.orangeboychen.marsrs`。

## 打开、写、flush

```kotlin
import io.github.orangeboychen.marsrs.xlog.AppenderMode
import io.github.orangeboychen.marsrs.xlog.LogLevel
import io.github.orangeboychen.marsrs.xlog.Xlog
import io.github.orangeboychen.marsrs.xlog.XlogConfig

val xlog = Xlog.open(
    XlogConfig(
        logDir = File(context.filesDir, "xlog/log").path,
        cacheDir = File(context.filesDir, "xlog/cache").path,
        namePrefix = "marsrs",
        level = LogLevel.INFO,
        mode = AppenderMode.ASYNC,
    ),
    context,     // App 的 UI 离开屏幕时自己 flush
)
xlog.consoleLogEnabled = BuildConfig.DEBUG

xlog.i("startup", "cold start in $elapsedMillis ms")
xlog.e("login", "login failed\n${cause.stackTraceToString()}")

xlog.flush(sync = true)   // 读文件或上传前
```

`logDir` 是唯一没有默认值的选项，其余都在[配置项](/zh/configuration)那页。

## 写

写入的形状和 `android.util.Log` 一样：`v`/`d`/`i`/`w`/`e`/`f`，都是 tag 加消息；
级别要到调用时才知道就写 `log(level, tag, message)`。

```kotlin
xlog.v("net", "…")
xlog.d("net", "…")
xlog.i("startup", "…")
xlog.w("net", "…")
xlog.e("login", "…")
xlog.f("login", "…")

xlog.log(LogLevel.DEBUG, "net", "…")
```

Java 写的完全一样 —— `Xlog.open(config)` 然后 `xlog.i(tag, message)` —— 不用再学别的。

构造起来很贵的消息值得先问一句：被级别丢掉的记录，那串 `String` 你照样已经付过了。

```kotlin
if (xlog.isLoggable(LogLevel.DEBUG)) {
    xlog.d("net", expensiveDescription())
}
```

每个成员都可以从任意线程调用，一条记录就是一次 JNI 调用：pid 和 tid 是从系统取的，
不是从 Java 取的。

## 开着的时候

| 作用 | 怎么写 |
|---|---|
| 改级别 | `xlog.level = LogLevel.WARNING` |
| 切异步 / 同步 | `xlog.mode = AppenderMode.SYNC` |
| 同时打到 logcat | `xlog.consoleLogEnabled = true` |
| 到某个大小换文件 | `xlog.maxFileSizeBytes = 8 * 1024 * 1024` |
| 到某个时间删文件 | `xlog.maxAliveTimeSeconds = 10 * 24 * 3600` |
| 还开着吗 | `xlog.isOpen` |
| 排空缓存 | `xlog.flush(sync = true)` |

`close()` 排空剩下的内容并关掉 appender。同一个 `namePrefix` 的两个 `Xlog` 是同一个
appender，关掉一个，另一个也就写不了了 —— 哪一块日志要单独读取，就给它一个自己的前缀。

## App 退出的时候

把 App 任意一个 `Context` 交给 `Xlog`，它就自己 flush —— 退出时什么都不用调用：

```kotlin
val xlog = Xlog(config, context)
val opened = Xlog.open(config, context)     // 同一个调用，另一个名字
```

Android 没有"App 要退出了"这件事：`Application.onTerminate` 在真机上从来不会被调用，
而系统结束一个进程时，一个字都不会说。剩下能用的是
`ComponentCallbacks2.onTrimMemory`，`Xlog` 注册的就是它 —— 从
`TRIM_MEMORY_UI_HIDDEN` 往上，说明 App 的每个 activity 都已经被别的界面挡住了，
也就是一个退到后台的 App 在被杀掉之前待的地方。`onLowMemory()` 和 `close()` 也会排空。

不给 `Context` —— `Xlog(config)` —— 就什么都不注册，也什么都不丢：记录留在缓存文件里，
下一个同 `namePrefix` 的 `Xlog` 打开时会把它们排进日志文件。见[日志文件](/zh/log-files)。

## 从 C++ 版 Java 迁移过来

C++ 项目 Java 那套 API 还在，也还能用 —— 七个参数的 `Xlog.open`、`XLogConfig`、
`XLoggerInfo`、`logWrite`、`LEVEL_*` 常量和 `Log` 门面 —— 每一个都标了 deprecated，
并写明替代它的写法。`Xlog.open` 装的就是 `Log` 写进的那个 appender，所以两种写法落在
同一个文件里，迁移可以一个调用点一个调用点地做：

```kotlin
// 以前
Log.setLogImp(Xlog())
Log.d("net", "…")

// 现在
val xlog = Xlog.open(XlogConfig(logDir = dir, namePrefix = "marsrs"))
xlog.d("net", "…")
```

## 任务链路

`marsrs` 带着这个移植里跟服务器说话的那半，在
`io.github.orangeboychen.marsrs.stn.StnLogic` 下面 —— 一个装静态成员的 object，架在 JNI
桥上，就像 `com.tencent.mars.stn.StnLogic` 曾经是一个装静态成员的 class、架在 C++ 自己
那套上一样。

```kotlin
import io.github.orangeboychen.marsrs.stn.StnLogic

StnLogic.setCallBack(object : StnLogic.ICallBack { /* 那十四个问题 */ })

val task = StnLogic.Task(StnLogic.Task.E_BOTH, 100, "/cgi-bin/hello", arrayListOf("example.com"))
task.totalTimeout = 10_000
StnLogic.setShortlinkSvrAddr(443)
StnLogic.startTask(task)

// 这个移植没有线程：队列由谁调用谁来排空
var due = StnLogic.dueTime()
while (due >= 0) {          // -1 是"没有要等的东西"
    StnLogic.runPending()
    due = StnLogic.dueTime()
}
```

`ICallBack` 是 STN 跑一个任务时问的那十四个问题，它是一个没有默认实现的普通 interface，
所以那个 object 要 App 自己填完。`dueTime()` 是这一趟还能等多少毫秒，没有要等的东西时回答 `-1`，`runPending()`
就是那一趟：启动了却从来没被排空过的任务，就一直留在它的队列里。

[任务链路](/zh/stn)是它的全部 —— 两条连接、一个任务的各个字段、任务怎么结束、长连接要
App 做什么。

## 网络诊断

```kotlin
import io.github.orangeboychen.marsrs.sdt.SdtLogic

SdtLogic.setCallBack(object : SdtLogic.ICallBack {           // 报告送到哪
    override fun reportSignalDetectResults(resultsJson: String?) { send(resultsJson) }
})
SdtLogic.setHttpNetcheckCGI("http://example.com/netcheck")
SdtLogic.startActiveCheck(
    arrayOf(SdtLogic.Link("default", arrayOf("1.2.3.4"), intArrayOf(80))),
    emptyArray(),
    SdtLogic.CheckMode.K_BASIC or SdtLogic.CheckMode.K_LONG, // ping 和 dns，然后 tcp
    10_000, // 超时
)
SdtLogic.runChecks(1, object : SdtLogic.IProbe {
    override fun dns(host: String, timeoutMs: Int) =
        SdtLogic.Answer.dns(errorCode = 0, rtt = 12, ips = arrayOf("1.2.3.4"))
    override fun tcp(host: String, port: Int, timeoutMs: Int) =
        SdtLogic.Answer.tcp(sent = 0, received = 0, isNoopResponse = true, rtt = 30)
    override fun http(url: String, timeoutMs: Int) =
        SdtLogic.Answer.http(errorCode = 0, statusCode = 200, rtt = 40)
    override fun ping(host: String, timeoutSec: Int) =
        SdtLogic.Answer.ping(errorCode = 0, rtt = 20, lossRate = 0f, averageRTT = 18f)
})
// 报告已经送到上面的回调了；`takeReport()` 是拿到它的另一条路 —— 给不装回调的应用用 ——
// 一份文档，不是两份
```

四个探针是 App 的：这个移植不持有任何 socket，所以一次检查是向交给 `runChecks` 的那个
`IProbe` 一个一个地问，都在调用它的那个线程上，而且这个调用要等每个探针都回答了才返回，
所以一次只有一个诊断在跑。C++ 是在自己的线程上开始一次诊断的 ——
`com.tencent.mars.sdt.SdtLogic` 没有声明任何 start 方法 —— 所以这个面是这个移植自己的，
不是对齐出来的。

[网络诊断](/zh/sdt)是它的全部：那个模式、那份计划、以及报告的那份 JSON。

## 缩小发布包

什么都不用加。`libmarsrsxlog.so` 和调用它的 Kotlin 互相按名字引用 —— native 的符号是
`Java_io_github_orangeboychen_marsrs_xlog_Xlog_write`，Rust 读的字段是 `GetFieldID`
按名字问的那个字段 —— 而 R8 会把两半都改名。两个 AAR 都自带保住这些名字的规则，所以
开了 `minifyEnabled true` 的 app 不需要自己的 `proguard-rules.pro`：规则以 AAR 的
`proguard.txt` 进来，AGP 把它们并进 app 自己的规则里。
