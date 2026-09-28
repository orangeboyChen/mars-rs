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

val xlog = Xlog.open(
    XlogConfig(
        logDir = File(context.filesDir, "xlog/log").path,
        cacheDir = File(context.filesDir, "xlog/cache").path,
        namePrefix = "marsrs",
        level = LogLevel.INFO,
        mode = AppenderMode.ASYNC,
    ),
    context,     // flushes itself when the app's UI goes away
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

Java writes the same thing — `Xlog.open(config)` and `xlog.i(tag, message)` —
with nothing else to learn.

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

## When the app goes away

Hand the `Xlog` any `Context` of the app and it flushes itself — there is nothing
to call on the way out:

```kotlin
val xlog = Xlog(config, context)
val opened = Xlog.open(config, context)     // the same call, under the shared name
```

Android has no "the app is quitting": `Application.onTerminate` is never called
on a device, and a process the system ends is told nothing at all. What is left
is `ComponentCallbacks2.onTrimMemory`, and that is what the `Xlog` registers —
from `TRIM_MEMORY_UI_HIDDEN` up, every activity of the app is behind something
else, which is where a backgrounded app lives until it is killed. `onLowMemory()`
and `close()` flush as well.

Built without a `Context` — `Xlog(config)` — nothing is registered, and nothing
is lost: the records stay in the cache file, and the next `Xlog` of the same
`namePrefix` drains them into the log file when it opens. See
[log files](/log-files#when-the-app-goes-away).

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
val xlog = Xlog.open(XlogConfig(logDir = dir, namePrefix = "marsrs"))
xlog.d("net", "…")
```

## The task pipeline

`marsrs` carries the half of the port that talks to a server, under
`io.github.orangeboychen.marsrs.stn.StnLogic` — one object of statics over the
JNI bridge, the way `com.tencent.mars.stn.StnLogic` was one class of statics over
the C++'s own.

```kotlin
import io.github.orangeboychen.marsrs.stn.StnLogic

StnLogic.setCallBack(object : StnLogic.ICallBack { /* the fourteen questions */ })

val task = StnLogic.Task(StnLogic.Task.E_BOTH, 100, "/cgi-bin/hello", arrayListOf("example.com"))
task.totalTimeout = 10_000
StnLogic.setShortlinkSvrAddr(443)
StnLogic.startTask(task)

// no threads in the port: the queue is drained by whoever calls this
var due = StnLogic.dueTime()
while (due >= 0) {          // -1 is "nothing to wait for"
    StnLogic.runPending()
    due = StnLogic.dueTime()
}
```

`ICallBack` is the fourteen questions STN asks while a task runs, and it is a
plain interface with no defaults, so the object is the app's to finish.
`dueTime()` is how long the pass may wait, in milliseconds, and it answers `-1`
when there is nothing to wait for; `runPending()` is the pass: a task that is
started and never drained stays in its queue.

[The task pipeline](/stn) is the whole of it — the two links, the fields of a
task, how a task ends, and what a long link asks of an app.

## The network diagnosis

```kotlin
import io.github.orangeboychen.marsrs.sdt.SdtLogic

SdtLogic.setCallBack(object : SdtLogic.ICallBack {           // where the report goes
    override fun reportSignalDetectResults(resultsJson: String?) { send(resultsJson) }
})
SdtLogic.setHttpNetcheckCGI("http://example.com/netcheck")
SdtLogic.startActiveCheck(
    arrayOf(SdtLogic.Link("default", arrayOf("1.2.3.4"), intArrayOf(80))),
    emptyArray(),
    SdtLogic.CheckMode.K_BASIC or SdtLogic.CheckMode.K_LONG, // ping and dns, then tcp
    10_000, // the timeout
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
// the report has already gone to the callback above; `takeReport()` is the other
// way to get it, for an app that installs no callback — one document, not two
```

The four probes are the app's: this port owns no sockets, so a check is asked of
the `IProbe` you hand to `runChecks`, one at a time, on the thread that called
it — and that call does not come back until every probe has answered, so one
diagnosis runs at a time.

[The network diagnosis](/sdt) is the whole of it: the mode, the plan, and the
JSON of the report.

## Shrinking the release build

Nothing to add. `libmarsrsxlog.so` and the Kotlin that calls it name each other
— the symbol of a native is `Java_io_github_orangeboychen_marsrs_xlog_Xlog_write`,
and a field the Rust reads is the field `GetFieldID` asks for by name — and R8
renames both halves. Both AARs carry the rules that keep those names, so an app
whose release build sets `minifyEnabled true` needs no `proguard-rules.pro` of
its own: they reach it as the `proguard.txt` of the AAR, and AGP merges them
into the app's.
