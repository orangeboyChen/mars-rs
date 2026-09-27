# Kotlin Multiplatform

One dependency in `commonMain`, and each platform compiles its own half of it:
`androidMain` talks to the JNI bridge, `nativeMain` talks to the C ABI through
cinterop — so the same calls in shared code write the same `.xlog` on Android,
iOS, watchOS, tvOS, macOS, Linux and Windows.

```kotlin
// settings.gradle.kts
maven {
    url = uri("https://maven.pkg.github.com/orangeboyChen/mars-rs")
    credentials { username = "<user>"; password = "<token with read:packages>" }
}

// build.gradle.kts of the shared module
implementation("io.github.orangeboychen.marsrs:xlog-kmp:0.1.0")    // xlog alone
implementation("io.github.orangeboychen.marsrs:marsrs-kmp:0.1.0")  // the whole port
```

An app that would rather not authenticate to GitHub Packages takes
`marsrs-kmp-maven.zip` of the release instead: unzip it and add
`maven { url = uri("<dir>") }`.

## Open, write, flush

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

Xlog.flush(sync = true)   // before the app reads or uploads the files
Xlog.close()
```

`logDir` is the one option with no default — the rest are on
[the configuration page](/configuration).

## Writing

`Xlog.write` is one record: a level, a tag and a message, plus the file,
function and line of the call site when the shared code has them.

```kotlin
Xlog.write(LogLevel.Debug, "net", "…")
Xlog.write(LogLevel.Error, "login", "…")

Xlog.write(LogLevel.Debug, "net", "…", file = "Net.kt", function = "fetch", line = 42)
```

A record below the level the appender was opened at is dropped before anything is
formatted.

## The `Log` facade

`Log` is the C++ project's facade — `Log.d(tag, message)` and friends over one
appender — for shared code that would rather write it that way:

```kotlin
Log.setLevel(LogLevel.Info)
Log.v("net", "…")
Log.d("net", "…")
Log.i("startup", "…")
Log.w("net", "…")
Log.e("login", "…")
Log.f("login", "…")
```

## What compiles

Fourteen Kotlin targets: Android — the AAR carries `libmarsrsxlog.so` for
`arm64-v8a`, `armeabi-v7a` and `x86_64` — plus `iosArm64`, `iosX64`,
`iosSimulatorArm64`, `macosX64`, `macosArm64`, `watchosArm64`,
`watchosDeviceArm64`, `watchosSimulatorArm64`, `tvosArm64`, `tvosSimulatorArm64`,
`linuxX64`, `linuxArm64` and `mingwX64`.

Nothing is compiled from Rust when the shared module is built: each platform
links the archive the release published for it.

## What is not in it

The surface is the intersection of the two bridges, which is what a `common`
declaration can only be. The named instances of the C ABI —
`mars_xlog_new_instance`, `mars_xlog_current_log_path` — and the per-instance
`Xlog` of Android are not in it; a caller who wants them is a caller of one
platform, and writes it in that platform's source set.
