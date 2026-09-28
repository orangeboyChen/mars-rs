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
    )
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

## 缩小发布包

什么都不用加。`libmarsrsxlog.so` 和调用它的 Kotlin 互相按名字引用 —— native 的符号是
`Java_io_github_orangeboychen_marsrs_xlog_Xlog_write`，Rust 读的字段是 `GetFieldID`
按名字问的那个字段 —— 而 R8 会把两半都改名。两个 AAR 都自带保住这些名字的规则，所以
开了 `minifyEnabled true` 的 app 不需要自己的 `proguard-rules.pro`：规则以 AAR 的
`proguard.txt` 进来，AGP 把它们并进 app 自己的规则里。
