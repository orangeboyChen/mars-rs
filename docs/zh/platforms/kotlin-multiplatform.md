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
implementation("io.github.orangeboychen:mars-rs-xlog-kmp:0.1.0")  // 只有 xlog
implementation("io.github.orangeboychen:mars-rs-kmp:0.1.0")       // 整个端口
```

不想为 GitHub Packages 配认证的应用，可以拿 release 里的 `mars-kmp-maven.zip`：
解压后加 `maven { url = uri("<dir>") }`。

## 打开、写、flush

```kotlin
import io.github.orangeboychen.marsrs.xlog.AppenderMode
import io.github.orangeboychen.marsrs.xlog.LogLevel
import io.github.orangeboychen.marsrs.xlog.Xlog
import io.github.orangeboychen.marsrs.xlog.XlogConfig

Xlog.open(
    XlogConfig(
        logDir = logDirectory,
        namePrefix = "Ham",
        level = LogLevel.Info,
        mode = AppenderMode.Async,
    )
)

Xlog.write(LogLevel.Info, "startup", "cold start in $elapsedMillis ms")

Xlog.flush(sync = true)   // 读文件或上传前
Xlog.close()
```

`logDir` 是唯一没有默认值的选项，其余都在[配置项](/zh/configuration)那页。

## 写

`Xlog.write` 就是一条记录：级别、tag、消息，共享代码里有的话再加上调用处的文件、
函数和行号。

```kotlin
Xlog.write(LogLevel.Debug, "net", "…")
Xlog.write(LogLevel.Error, "login", "…")

Xlog.write(LogLevel.Debug, "net", "…", file = "Net.kt", function = "fetch", line = 42)
```

级别低于 appender 打开时那个级别的记录，在格式化之前就被丢掉了。

## `Log` 门面

`Log` 是 C++ 项目那套门面 —— `Log.d(tag, message)` 这些，写进同一个 appender ——
给更愿意这么写的共享代码用：

```kotlin
Log.setLevel(LogLevel.Info)
Log.v("net", "…")
Log.d("net", "…")
Log.i("startup", "…")
Log.w("net", "…")
Log.e("login", "…")
Log.f("login", "…")
```

## 哪些平台

十四个 Kotlin target：Android（AAR 里带 `arm64-v8a`、`armeabi-v7a`、`x86_64` 的
`libmarsxlog.so`），加上 `iosArm64`、`iosX64`、`iosSimulatorArm64`、`macosX64`、
`macosArm64`、`watchosArm64`、`watchosDeviceArm64`、`watchosSimulatorArm64`、
`tvosArm64`、`tvosSimulatorArm64`、`linuxX64`、`linuxArm64`、`mingwX64`。

编译共享模块时不编译任何 Rust：每个平台链接的是 release 已经为它发布的静态库。

## 不在这里面的

这个 API 面是两座桥的交集，也是 `common` 声明唯一能是的东西。C ABI 的具名实例
（`mars_xlog_new_instance`、`mars_xlog_current_log_path`）和 Android 那个按实例的
`Xlog` 都不在里面；要用它们就是某个单一平台的调用方，写在那平台的 source set 里。
