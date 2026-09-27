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

val xlog = Xlog(
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
同样的名字，所以在 `xlog-kmp` 和 `xlog` 之间搬动的共享模块什么都不用改。

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

编译共享模块时不编译任何 Rust：每个平台链接的是 release 已经为它发布的静态库。

## 不在这里面的

这个 API 面是两座桥的交集，也是 `common` 声明唯一能是的东西。C ABI 那个进程级的
appender —— `mars_xlog_open`、`mars_xlog_close`、`mars_xlog_current_log_path` —— 不在
里面，因为 JNI 桥没有对应的东西：要用它们就是某个单一平台的调用方，写在那平台的
source set 里。
