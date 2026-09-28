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
links the archive the release published for it — and `marsrs-kmp` links two per
target, `libmars_ffi.a` for xlog and `libmars_net_ffi.a` for STN and SDT, which
is what lets an app that takes `xlog-kmp` carry no byte of the task pipeline.

## What is not in it

The surface is the intersection of the two bridges, which is what a `common`
declaration can only be. The process-wide appender of the C ABI —
`mars_xlog_open`, `mars_xlog_close`, `mars_xlog_current_log_path` — is not in it,
because the JNI bridge exports no equivalent: a caller who wants it is a caller
of one platform, and writes it in that platform's source set.

Four places the two `actual`s answer differently, and the shared API is what
both can say:

- `StnLogic.dueTime()` answers `Long?` — how many milliseconds the next pass may
  wait, and `null` when there is nothing to wait for — because the JNI bridge
  answers `-1` and the C ABI its own `MARS_STN_ERR_NO_DUE`.
- `StnLogic.makesureLongLinkConnected()` answers nothing: the C ABI's symbol
  answers 1 or 0 and the JNI one answers `void`, so a caller who wants to know
  reads `Question.linkStatus` instead.
- A `CgiProfile` an app reads on Android carries `0` and `""` for the two
  readings only the C ABI has — `sendPacketFinishedTime` and `netType`.
- `setApp` is asked all eighteen questions on Kotlin/Native and thirteen on
  Android: the two network errors, the long link's status change, the task limit
  and the DNS profile are five the JNI bridge answers itself.

## The task pipeline

`marsrs-kmp` carries the half of the port that talks to a server: `StnLogic` is
one `expect object` in `commonMain`, with one `actual` per platform family — over
the JNI bridge of `crates/marsrs-jni` on Android, over the C ABI of
`crates/marsrs-ffi` through cinterop on every Kotlin/Native target.

```kotlin
import io.github.orangeboychen.marsrs.stn.Answer
import io.github.orangeboychen.marsrs.stn.FailHandle
import io.github.orangeboychen.marsrs.stn.Question
import io.github.orangeboychen.marsrs.stn.StnLogic
import io.github.orangeboychen.marsrs.stn.Task

// one `ask` answers the thirteen questions that reach the app
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

// no threads in the port: the queues are drained by whoever calls this
while (StnLogic.dueTime() != null) {   // null is "nothing to wait for"
    StnLogic.runPending()
}
```

[The task pipeline](/stn) is the whole of it — the two links, the fields of a
task, how a task ends, and what a long link asks of an app.

## The network diagnosis

```kotlin
import io.github.orangeboychen.marsrs.sdt.CheckMode
import io.github.orangeboychen.marsrs.sdt.Link
import io.github.orangeboychen.marsrs.sdt.ProbeAnswer
import io.github.orangeboychen.marsrs.sdt.SdtLogic

SdtLogic.setCallBack { resultsJson -> send(resultsJson) }   // where the report goes
SdtLogic.setHttpNetcheckCGI("http://example.com/netcheck")
SdtLogic.startActiveCheck(
    arrayOf(Link("default", arrayOf("1.2.3.4"), intArrayOf(80))),
    emptyArray(),
    CheckMode.K_BASIC or CheckMode.K_LONG, // ping and dns, then tcp
    10_000, // the timeout
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
// the report has already gone to the callback above; `takeReport()` is the other
// way to get it, for an app that installs no callback — one document, not two
```

The four probes are the app's: this port owns no sockets, so a check is asked of
the `IProbe` you hand to `runChecks`, one at a time, on the thread that called it
— and that call does not come back until every probe has answered, so one
diagnosis runs at a time.

[The network diagnosis](/sdt) is the whole of it: the mode, the plan, and the
JSON of the report.
