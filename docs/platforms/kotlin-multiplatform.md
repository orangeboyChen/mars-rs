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

xlog.flush(sync = true)   // before the app reads or uploads the files
xlog.close()
```

`logDir` is the one option with no default — the rest are on
[the configuration page](/configuration).

This is the `Xlog` of [Android](/platforms/android): `Xlog.open(config)` is a
factory over the constructor, with the same members and the same names, so a
shared module that moves between `xlog-kmp` and `xlog` renames nothing.

## Writing

The write is `android.util.Log`'s shape — `v`/`d`/`i`/`w`/`e`/`f`, each of them a
tag and a message, and `log(level, tag, message)` when the level is not known
until the call.

```kotlin
xlog.v("net", "…")
xlog.d("net", "…")
xlog.i("startup", "…")
xlog.w("net", "…")
xlog.e("login", "…")
xlog.f("login", "…")

xlog.log(LogLevel.DEBUG, "net", "…")
```

A record below the level the appender was opened at is dropped before anything is
formatted. A message that is expensive to build is worth asking about first,
because a record the level drops still costs the caller the `String`:

```kotlin
if (xlog.isLoggable(LogLevel.DEBUG)) {
    xlog.d("net", expensiveDescription())
}
```

## While it is open

| what | how |
|---|---|
| move the level | `xlog.level = LogLevel.WARNING` |
| switch async / sync | `xlog.mode = AppenderMode.SYNC` |
| mirror records to the console | `xlog.consoleLogEnabled = true` |
| close a file at a size | `xlog.maxFileSizeBytes = 8 * 1024 * 1024` |
| drop a file at an age | `xlog.maxAliveTimeSeconds = 10 * 24 * 3600` |
| is it still open | `xlog.isOpen` |
| drain the cache | `xlog.flush(sync = true)` |

`close()` drains what is left and closes the appender. Two `Xlog`s of one
`namePrefix` are one appender, so closing one of them closes what the other
writes through — give a part of the app whose logs are read apart from the rest a
prefix of its own.

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
declaration can only be. The process-wide appender of the C ABI —
`mars_xlog_open`, `mars_xlog_close`, `mars_xlog_current_log_path` — is not in it,
because the JNI bridge exports no equivalent: a caller who wants it is a caller
of one platform, and writes it in that platform's source set.
