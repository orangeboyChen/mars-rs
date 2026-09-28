# Migrating from mars

An app that already runs the C++ implementation has three things to move: the
logger, the task pipeline and the network diagnosis. All three are here under the
names this port spells them, and what the logger writes is the file the C++ one
writes — same framing, same compression, same encryption — so the `.xlog` files
you have already collected are read by the tooling you already have, and a
migration costs you no history.

What it does cost is two things: the calls are spelled the way each platform
spells them, and two pieces the C++ kept inside itself are the app's here. The
rest of this page is one chapter per piece, and those two first.

## What maps to what

| the piece you use | what you take here | where it is written down |
|---|---|---|
| `mars/xlog` — the logger | `xlog`: `marsrs-xlog` on crates.io, `xlog` on JitPack, `xlog-kmp` for a shared Kotlin module, `MarsRSXlog` on Apple | [Getting started](/getting-started) |
| `mars/stn` — the task pipeline | `marsrs`, and its twins `marsrs-kmp` and `MarsRSNet` | [The task pipeline](/stn) |
| `mars/sdt` — the network diagnosis | the same `marsrs`, in the same package as the pipeline | [The network diagnosis](/sdt) |

The split is the one every platform makes: the logger is one package and the
whole port is the other, and the whole port carries the same logger — only the
name differs. An app that only logs takes the first, an app that also runs a task
or a diagnosis takes the second, and an app that is not sure starts with the
logger: `marsrs` is the logger plus the other two pieces, so moving from one
package to the other renames nothing about the file.

## The two things the app takes over

**A queue is drained by a call and not by a thread.** A task and a diagnosis both
run on a queue, and the C++ runs those queues on threads of its own — a
message-queue thread, and the `__RunOn` thread a diagnosis runs on. This port has
no threads in it, so what would have been a thread is a call the host makes:
`run_pending()` for a task and `runChecks` for a diagnosis, each with the answer
to *how long may I wait before I make it*. A loop is the whole of it:

::: code-group

```rust [Rust]
while let Some(wait) = stn.due_delay() {      // how long the pass may wait, in ms
    std::thread::sleep(std::time::Duration::from_millis(wait));
    stn.run_pending();
}
```

```kotlin [Android]
var due = StnLogic.dueTime()                  // -1 is "nothing to wait for"
while (due >= 0) {
    Thread.sleep(due)
    StnLogic.runPending()
    due = StnLogic.dueTime()
}
```

:::

The two spellings above are [the task pipeline](/stn)'s; the diagnosis has the
same shape and no sleep — `runChecks` asks its probes and does not come back
until they have answered, so an app runs it once and takes the report.

**The socket is the app's.** A short link, a long link and every probe of a
diagnosis are sockets, and this port owns none: the C++ links its own networking
into the checkers, and here what a socket is belongs to the caller. A diagnosis
asks you for a resolve, a connect, an HTTP request and an ICMP echo, one at a
time, and your answer is the result — which is also what makes one testable from
a machine with no radio. A task's link is wired the same way in Rust, through the
`SocketOperator` a link is made with; outside Rust the bindings expose no seam
for one yet, so [the task pipeline](/stn) is the page that says where that
leaves an app today.

## Migrating the logger

The appender is the same object: one process-wide writer per prefix, opened once
when the app starts, written through, and flushed before its file is read or
uploaded. What changes is who holds it — the C++ installs one process-wide
appender behind free functions, and here you hold the `Xlog` you opened and write
through it.

### From the C++ headers

The appender is one struct of options and a handful of free calls in
`mars/xlog/appender.h`, and it is one struct and a handful of calls here:

| the C++ | Rust | the C ABI |
|---|---|---|
| `appender_open(const XLogConfig&)` | `appender_open(XLogConfig)` | `mars_xlog_open(&config)` |
| `xlogger_Write(info, log)`, or the `xinfo2` family | `appender_write(info, message)` | `mars_xlog_write(...)` |
| `appender_flush()` | `appender_flush()` | `mars_xlog_flush()` |
| `appender_flush_sync()` | `appender_flush_sync()` | `mars_xlog_flush_sync()` |
| `appender_close()` | `appender_close()` | `mars_xlog_close()` |
| `xlogger_SetLevel(level)` | `set_level(handle, level)` | `mars_xlog_set_level(level)` |
| `appender_setmode(mode)` | `appender_set_mode(mode)` | `mars_xlog_set_mode(mode)` |
| `appender_set_console_log(bool)` | `appender_set_console_log(bool)` | `mars_xlog_set_console_log(on)` |
| `appender_set_max_file_size(bytes)` | `appender_set_max_file_size(bytes)` | `mars_xlog_set_max_file_size(bytes)` |
| `appender_set_max_alive_duration(secs)` | `appender_set_max_alive_duration(secs)` | `mars_xlog_set_max_alive_duration(secs)` |
| `appender_get_current_log_path(out, len)` | `appender_get_current_log_path()` | `mars_xlog_current_log_path(out, len)` |

The config is one struct in all three, and the eight fields are the same eight:
`mode_`, `logdir_`, `nameprefix_`, `pub_key_`, `compress_mode_`,
`compress_level_`, `cachedir_` and `cache_days_` in the C++, which are `mode`,
`logdir`, `nameprefix`, `pub_key`, `compress_mode`, `compress_level`, `cachedir`
and `cache_days` here. Every one of them is on
[the configuration page](/configuration), in the spelling of every platform.
`TAppenderMode` is `AppenderMode`, `TCompressMode` is `CompressMode` and
`TLogLevel` is `LogLevel`.

The macros are the other half of what goes. `XLOGGER_TAG` and the
`xverbose2` / `xdebug2` / `xinfo2` / `xwarn2` / `xerror2` / `xfatal2` family
carried the level, the tag and the call site in one line of C++; what replaces
them is a method per level — `xlog.i(tag, message)` in Kotlin,
`log.info(message:tag:)` in Swift, `appender_write(None, message)` in Rust —
with the file, the function and the line taken from the call site instead of
named at it.

A second appender is where the shapes differ most. The C++ project's Java opened
one with `Log.openLogInstance(level, mode, cacheDir, logDir, nameprefix,
cacheDays)` and threaded the handle it answered through every call after it;
here an appender you hold *is* the instance, so a second one is a second `Xlog`
of a prefix of its own, or the `*_instance` family in Rust —
`appender_open_instance(config)` answers a handle, and `appender_write_instance`,
`appender_flush_instance` and `appender_close_instance` take it.

### From the C++ project's Java

The Android package is the one place the old spelling is still there:
`Xlog.open` with its seven arguments, `XLogConfig`, `XLoggerInfo`, `logWrite` and
the `LEVEL_*` constants all work, and every one of them is deprecated with the
spelling that replaces it. `Log.setLogImp(Xlog())` and `Log.d(tag, message)`
still write through the same appender `Xlog.open` installs, so a migration can go
one call site at a time:

```kotlin
// before
Log.setLogImp(Xlog())
Log.d("net", "…")

// after
val xlog = Xlog.open(XlogConfig(logDir = dir, namePrefix = "marsrs"))
xlog.d("net", "…")
```

What the new spelling buys is a `Context`: `Xlog.open(config, context)` flushes
itself when the app leaves the screen, which is the last moment Android says
anything before it can end the process without another word — see
[Android](/platforms/android#when-the-app-goes-away).

### From the Apple headers

There is no Objective-C wrapper to move: what an app on Apple called was the C++
out of an Objective-C++ file — `xlogger_SetLevel`, `appender_set_console_log`,
an `XLogConfig` filled in field by field and `appender_open(config)`. That file
becomes one that imports the module and holds the appender it opened:

::: code-group

```objc [before]
XLogConfig config;
config.mode_ = kAppenderAsync;
config.logdir_ = [logPath UTF8String];
config.nameprefix_ = "Test";
config.pub_key_ = "";
config.compress_mode_ = kZlib;
config.compress_level_ = 0;
config.cachedir_ = "";
config.cache_days_ = 0;
appender_open(config);
```

```objc [after]
@import MarsRSXlog;

XlogConfig *config = [[XlogConfig alloc] initWithLogDirectory:logPath.path];
config.namePrefix = @"marsrs";

NSError *error = nil;
Xlog *log = [[Xlog alloc] initWithConfig:config error:&error];
[log writeWithLevel:LogLevelInfo message:@"cold start" tag:@"startup"];
[log flushWithSync:YES];        // before the app reads or uploads the files
```

:::

Swift fills the file, the function and the line in at the call site;
Objective-C has no `#file` to fill one with, so a record written here carries an
empty file and the line 0 unless the long form names them —
`[log log:message:tag:file:function:line:]`.

What replaces the header is `MarsRSXlog` — a SwiftPM product and a pod of the
same name, over the same framework, with an `@objc` surface under it.
[SwiftPM](/platforms/swift) and [CocoaPods](/platforms/cocoapods) are the two
pages of it.

## Migrating the task pipeline

A task is the same struct with the same defaults — the id, the command, the path,
the link it wants, the hosts, the timeout and the retries — and the app answers
the same questions while it runs. What changes is how the answers reach STN, and
who drains the queue.

| the C++ | Rust | Android | Swift | C |
|---|---|---|---|---|
| `mars::stn::StartTask` | `stn.start_task(task)` | `StnLogic.startTask(task)` | `MarsStn.start(task)` | `mars_stn_start_task(&task)` |
| `mars::stn::StopTask` | `stn.stop_task(id)` | `StnLogic.stopTask(id)` | `MarsStn.stop(taskID:)` | `mars_stn_stop_task(id)` |
| `mars::stn::HasTask` | `stn.has_task(id)` | `StnLogic.hasTask(id)` | `MarsStn.hasTask(id)` | `mars_stn_has_task(id)` |
| `mars::stn::SetCallback` | `stn.set_callback(app)` | `StnLogic.setCallBack(cb)` | `MarsStn.setApp { … }` | `mars_stn_set_app(ctx, ask)` |
| `mars::stn::MakesureLonglinkConnected` | `stn.make_sure_long_link_connected("default")` | `StnLogic.makesureLongLinkConnected()` | `MarsStn.makeSureLongLinkConnected()` | `mars_stn_makesure_longlink_connected()` |
| the queue's thread | `stn.run_pending()` | `StnLogic.runPending()` | `MarsStn.runPending()` | `mars_stn_run_pending()` |

The eighteen questions the C++ asks as virtuals of `mars::stn::Callback` are one
`App` trait in Rust, one `ICallBack` in Kotlin and one closure in Swift, and
each of them has a default for every question the app does not answer — except
Android's, which is the plain interface the C++ project's Java declared, so the
object is the app's to finish. The names are the ones the C++ used: `req2Buf`
for the bytes a task sends, `buf2Resp` for the answer, `onTaskEnd` for the end,
`onPush` for what the server sends down the long link, `onNewDns` for the
addresses of a host.

How a task ends is unchanged: an error *type* that says where it failed —
`ErrCmdType` in Rust, `errType` on the others — an error *code* that says what
went wrong, and a profile of the timings. Android names the codes on `StnLogic`
the way it did before.

So does the boot, on Android: `Mars.init(context, handler)` and
`Mars.onCreate(true)` to start, `BaseEvent.onForeground` and
`BaseEvent.onNetworkChange` when the screen or the network changes, and
`AppLogic.setCallBack` for the account and the device STN asks about — the names
the C++ project's Java spelled, in the package of this port.

And the queue is the one new obligation, above: a task that is started and never
drained stays in its queue. [The task pipeline](/stn) is the whole of it — the
two links, the fields of a task, and what a long link asks of an app.

## Migrating the network diagnosis

A diagnosis is the same three steps in the C++ and here: name the hosts and the
checks, run them, take the report.

| what | the C++ | Rust | Android, shared Kotlin | Swift | C |
|---|---|---|---|---|---|
| the hosts and the mode | `StartActiveCheck` | `start_active_check(…)` | `startActiveCheck(…)` | `startActiveCheck(…)` | `mars_sdt_start_active_check(…)` |
| run the plan | the `__RunOn` thread | `run_checks(&mut ask, net)` | `runChecks(net, probe)` | `runChecks(networkType:probe:)` | `mars_sdt_run_checks(ctx, probe, net)` |
| stop it | `CancelActiveCheck` | `cancel_active_check()` | `cancelActiveCheck()` | `cancelActiveCheck()` | `mars_sdt_cancel_active_check()` |
| the net-check CGI | `SetHttpNetcheckCGI(cgi)` | `set_http_netcheck_cgi(cgi)` | `setHttpNetcheckCGI(cgi)` | `setHTTPNetCheckCGI(cgi)` | `mars_sdt_set_http_netcheck_cgi(cgi)` |
| the report | `Callback::ReportNetCheckResult` | `report_json(&results)` | `takeReport()`, or the callback | `takeReport()` | `mars_sdt_take_report(buf, len)` |

The mode is the same bit set — ping and DNS, then TCP, then the HTTP check of
the net-check CGI — and the report is the same `{"details":[ … ]}` document, one
object per check with the same `detectType`, `errorCode` and timings in it, so
whatever reads a report today reads this one.

What is the app's is the network underneath: the C++ links its own sockets into
the checkers, and here a check is a question the run asks you — a domain to
resolve, an ip and a port to connect to, a URL to fetch, a host to ping — and
your answer is what the report records. On Android and in the shared Kotlin that
is the `IProbe` you hand to `runChecks`; in Rust it is the `Ask`; in Swift and in
C it is the closure.

[The network diagnosis](/sdt) is the whole of it: the mode, the plan you can read
before you run it, and every field of the report.

## Where to go next

- [Getting started](/getting-started) — the dependency and a running example for
  each platform.
- [Configuration](/configuration) — every option of the appender and its default,
  in the spelling of every platform.
- [Log files](/log-files) — where the file lands and how it is read back, for an
  app whose upload path already knows the C++ one.
