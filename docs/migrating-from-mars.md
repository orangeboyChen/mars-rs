# Migrating from mars

An app that runs the whole C++ project has three things to move: the logger, the
task pipeline and the network diagnosis. All three are here under the names this
port spells them, and what the logger writes is the file the C++ one writes —
same framing, same compression, same encryption — so the `.xlog` files you have
already collected are read by the tooling you already have, and a migration costs
you no history.

The logger is one of the three and it has a page of its own —
[Migrating from mars-xlog](/migrating-from-mars-xlog) — for an app that took the
logger and nothing else. This page is the other two, and the two things an app
takes over when it moves them.

## What maps to what

| the piece you use | what you take here | where it is written down |
|---|---|---|
| `mars/xlog` — the logger | `xlog`: `marsrs-xlog` on crates.io, `xlog` on JitPack, `xlog-kmp` for a shared Kotlin module, `MarsRSXlog` on Apple | [Migrating from mars-xlog](/migrating-from-mars-xlog) |
| `mars/stn` — the task pipeline | `marsrs`, and its twins `marsrs-kmp` and `MarsRSNet` | [The task pipeline](/stn) |
| `mars/sdt` — the network diagnosis | the same `marsrs`, in the same package as the pipeline | [The network diagnosis](/sdt) |

The split is the one every platform makes: the logger is one package and the
whole port is the other, and the whole port carries the same logger — only the
name differs. An app that only logs takes the first, an app that also runs a task
or a diagnosis takes the second.

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

- [Migrating from mars-xlog](/migrating-from-mars-xlog) — the logger: the
  appender, its config, and the Java and Apple call sites it replaces.
- [Getting started](/getting-started) — the dependency and a running example for
  each platform.
- [Log files](/log-files) — where the file lands and how it is read back.
