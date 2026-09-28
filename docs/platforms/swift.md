# SwiftPM

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")

// and, in the target that takes it:
.product(name: "MarsRSXlog", package: "mars-rs")
```

Three products, and each one is named what the module behind it is: a product is
what a manifest declares and a module is what `import` names, and here they are
one name, so `MarsRSXlog` in `Package.swift` is `import MarsRSXlog` in the file
that takes it.

| product | what you get |
|---|---|
| `MarsRSXlog` | the logger: `Xlog`, `XlogConfig`, `LogLevel`, `AppenderMode`, `CompressMode` |
| `MarsRSNet` | the STN task pipeline and the SDT network diagnosis |
| `MarsRS` | both halves, re-exported |

The package itself is `mars-rs`, after the URL it is fetched from, and the two
files a release publishes are `marsrs-xlog.xcframework.zip` and
`marsrs-net.xcframework.zip`: a file name asks nothing of a compiler, and SwiftPM
extracts a zip under its binary target's name and reads the module map inside it.

The framework carries four slices — `ios-arm64`, `ios-arm64_x86_64-simulator`,
`watchos-arm64_arm64_32` and `watchos-arm64-simulator` — so an app target of
either platform resolves it.

## Open, write, flush

```swift
import MarsRSXlog

let log = try Xlog.open(
    XlogConfig(
        logDirectory: logDirectory.path,
        cacheDirectory: cacheDirectory.path,
        namePrefix: "marsrs",
        level: .info
    )
)
log.isConsoleLogEnabled = true

log.info(message: "cold start in \(elapsedMillis) ms", tag: "startup")
log.error(message: "login failed\n\(error)", tag: "login")

log.flush(sync: true)    // before the app reads or uploads the files
```

`XlogConfig(logDirectory:)` is the short form; the other fields are set on it
afterwards, and every one of them is on [the configuration page](/configuration):

```swift
var config = XlogConfig(logDirectory: logDirectory.path)
config.mode = .sync
config.compression = .zstd
config.compressionLevel = 6
config.publicKey = publicKey      // empty writes an unencrypted file
```

`Xlog.open(_:)` is a static factory over the constructor, and it throws an
`XlogError` when the appender refuses the config — an empty log directory, an
empty prefix, a compression level the compressor does not take, a negative
`cacheDays`.

## Writing

One method per level, each of them a message and a tag; `file`, `function` and
`line` come from the call site, so a record carries where it was written without
the caller naming it:

```swift
log.verbose(message: "…", tag: "net")
log.debug(message: "…", tag: "net")
log.info(message: "…", tag: "startup")
log.warning(message: "…", tag: "net")
log.error(message: "…", tag: "login")
log.fatal(message: "…", tag: "login")

log.log(.debug, message: "…", tag: "net")   // when the level is not known until the call
```

A record whose level is below the appender's is dropped before anything is
formatted. A message that is expensive to build is worth asking about first:

```swift
if log.isEnabled(for: .debug) {
    log.debug(message: "\(expensiveDescription())", tag: "net")
}
```

## While it is open

| what | how |
|---|---|
| move the level | `log.level = .warning` |
| switch async / sync | `log.mode = .sync` |
| mirror records to the console | `log.isConsoleLogEnabled = true` |
| close a file at a size | `log.maxFileSizeBytes = 8 * 1024 * 1024` |
| drop a file at an age | `log.maxAliveTimeSeconds = 10 * 24 * 3600` |
| is it still open | `log.isOpen` |
| where the current file is | `Xlog.currentLogPath` |

`close()` drains what is left and drops the appender; writing through the `Xlog`
afterwards writes nothing. Two `Xlog`s of one `namePrefix` are one appender — the
C ABI answers the handle it already has — so closing one of them closes what the
other writes through.

The C symbols are reachable too: `import MarsRSXlog` re-exports `MarsRSFFI`, so
`mars_xlog_open`, `mars_xlog_write` and the rest are there for whoever prefers
them.

## When the app goes away

**Nothing to call.** An `Xlog` flushes itself when the app leaves the screen: it
watches for `didEnterBackground` — the scene's, when the app has scenes, because
an app with them is sent no `UIApplication` lifecycle notification at all — and
for `willTerminate`, and each of them runs a `flush(sync: true)` on the spot. On
watchOS there is no UIKit, so what it watches is `WKExtension`'s
`applicationDidEnterBackground` instead.

That is the moment to drain because it is the last one the OS says anything: an
app killed out of the background is told nothing, so there is no later place a
`flush` could run. Nothing is lost without it either — the records stay in the
cache file, and the next `Xlog` of the same `namePrefix` drains them into its log
file when it opens — but the file of the session that is ending is complete only
once this has run. See [log files](/log-files#when-the-app-goes-away).
## The task pipeline

`MarsRSNet` carries the half of the port that talks to a server: `MarsStn` is
one `enum` of statics over the `mars_stn_*` of the C ABI.

```swift
import MarsRSNet

MarsStn.setApp { question in        // one closure answers the eighteen questions
    switch question.kind {
    case .req2Buf:  return .encoded(try! encode(question.task!))
    case .buf2Resp: handle(question.body); return .decoded(errorCode: 0, handle: .normal)
    case .onTaskEnd: return .ended(errorCode: 0)
    default:        return .nothing
    }
}

var task = StnTask(channelSelect: .short)
task.taskID = MarsStn.generateTaskID()
task.cgi = "/cgi-bin/hello"
MarsStn.start(task)

while MarsStn.dueTime != nil {      // nothing drains the queue on a thread of its own
    MarsStn.runPending()
}
```

`MarsStn.dueTime` is how long the pass may wait, in milliseconds — `0` is one
that is already due — and `MarsStn.runPending()` is the pass: the C++ runs them on
a message-queue thread and this port has none, so the loop is the app's. A task
that is started and never drained stays in its queue.

[The task pipeline](/stn) is the whole of it — the two links, the fields of a
task, how a task ends, and what a long link asks of an app.

## The network diagnosis

```swift
import MarsRSNet

MarsSdt.setHTTPNetCheckCGI("http://example.com/netcheck")
MarsSdt.startActiveCheck(longLink: longLink, shortLink: [], mode: 1 | 2, timeout: 10_000)
// 1 | 2 is NET_CHECK_BASIC | NET_CHECK_LONG: ping and dns, then tcp. `0` is no checks.

MarsSdt.runChecks(networkType: 1) { query in
    switch query.probe {
    case .dns:  return .dns(errorCode: 0, rtt: 12, addresses: ["1.2.3.4"])
    case .ping: return .ping(errorCode: 0, rtt: 20, lossRate: 0, averageRTT: 18)
    default:    return .nothing
    }
}

if let report = MarsSdt.takeReport() { send(report) }
```

The four probes are the app's — this port owns no sockets, so a check is asked of
the closure you hand to `runChecks`, one at a time, on the calling thread.

[The network diagnosis](/sdt) is the whole of it: the mode, the plan, and the
JSON of the report.
