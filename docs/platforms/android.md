# Android

```kotlin
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen.marsrs:xlog:0.1.0")    // xlog alone
implementation("io.github.orangeboychen.marsrs:marsrs:0.1.0")  // the whole port: + STN, SDT
```

Both AARs carry the same `libmarsrsxlog.so`, for `arm64-v8a`, `armeabi-v7a` and
`x86_64`. Take `xlog` when the app only logs. The Kotlin and Java package
is `io.github.orangeboychen.marsrs`.

## Open, write, flush

```kotlin
import io.github.orangeboychen.marsrs.xlog.AppenderMode
import io.github.orangeboychen.marsrs.xlog.LogLevel
import io.github.orangeboychen.marsrs.xlog.Xlog
import io.github.orangeboychen.marsrs.xlog.XlogConfig

val xlog = Xlog(
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

xlog.flush(sync = true)   // before the app reads or uploads the files
```

`logDir` is the one option with no default — everything else is on
[the configuration page](/configuration).

## Writing

The write is `android.util.Log`'s shape: `v`/`d`/`i`/`w`/`e`/`f`, each of them a
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

Java writes the same thing — `new Xlog(config)` and `xlog.i(tag, message)` — with
nothing else to learn.

A message that is expensive to build is worth asking about first, because a
record the level drops still costs the caller the `String`:

```kotlin
if (xlog.isLoggable(LogLevel.DEBUG)) {
    xlog.d("net", expensiveDescription())
}
```

Every member is safe to call from any thread, and a record costs one JNI call:
the pid and the tid are filled in from the OS, not from Java.

## While it is open

| what | how |
|---|---|
| move the level | `xlog.level = LogLevel.WARNING` |
| switch async / sync | `xlog.mode = AppenderMode.SYNC` |
| mirror records to logcat | `xlog.consoleLogEnabled = true` |
| close a file at a size | `xlog.maxFileSizeBytes = 8 * 1024 * 1024` |
| drop a file at an age | `xlog.maxAliveTimeSeconds = 10 * 24 * 3600` |
| is it still open | `xlog.isOpen` |
| drain the cache | `xlog.flush(sync = true)` |

`close()` drains what is left and closes the appender. Two `Xlog`s of one
`namePrefix` are one appender, so closing one of them closes what the other
writes through — give a part of the app whose logs are read apart from the rest
a prefix of its own.

## Coming from the C++ project's Java

The API the C++ project's Java spelled is still there and works — the
seven-argument `Xlog.open`, `XLogConfig`, `XLoggerInfo`, `logWrite`, the
`LEVEL_*` constants and the `Log` facade — and every one of them is deprecated
with the spelling that replaces it. `Xlog.open` installs the same appender
`Log` writes through, so the two spellings land in one file and a migration can
go one call site at a time:

```kotlin
// before
Log.setLogImp(Xlog())
Log.d("net", "…")

// after
val xlog = Xlog(XlogConfig(logDir = dir, namePrefix = "marsrs"))
xlog.d("net", "…")
```
