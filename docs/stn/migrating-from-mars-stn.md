# Migrating from mars-stn

An app that ran the C++ project's task pipeline has two things to move: the calls
it starts a task with, and the two things an app takes over when it does. A task
is the same struct, the app answers the same questions while it runs, and how a
task ends is unchanged — an error type, an error code and a profile.

| the piece you use | what you take here | where it is written down |
|---|---|---|
| `mars/stn` | `marsrs` on crates.io and JitPack, `marsrs-kmp` for a shared Kotlin module, `MarsRSNet` on Apple | [Getting started](/stn/getting-started) |
| `mars/xlog` | `xlog` — the logger, in a package of its own | [Migrating from mars-xlog](/xlog/migrating-from-mars-xlog) |
| `mars/sdt` | the same `marsrs`, in the same package as the pipeline | [Migrating from mars-sdt](/sdt/migrating-from-mars-sdt) |

## The two things the app takes over

**A queue is drained by a call and not by a thread.** The C++ runs the queues on a
message-queue thread of its own; this port has no threads in it, so what would
have been a thread is a call the host makes — `run_pending()`, with `due_time()`
for how long it may wait before making it. A loop is the whole of it:

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

```kotlin [Kotlin Multiplatform]
var due = StnLogic.dueTime()                  // null is "nothing to wait for"
while (due != null) {
    wait(due)                                 // the app's own: `common` Kotlin has no sleep
    StnLogic.runPending()
    due = StnLogic.dueTime()
}
```

:::

In a shared Kotlin module the wait is the app's own too: `common` Kotlin carries
no sleep of its own, so `wait` above is an `expect` the app writes —
`Thread.sleep(due)` on Android and `usleep(due * 1000)` on the native targets.

In Rust one call starts one: `Driver::spawn(stn)` starts a thread of this crate's
that drains the queues — `run_pending()` when it is due, sleeping `due_delay()`
meanwhile — and joins it when the `Driver` is dropped. An app that awaits a task
and has no loop of its own takes it; an app with a loop keeps the loop, because a
pass that ends a task wakes whoever awaited it.

A task that is started and never drained stays in its queue — `has_task` answers
`true` for one that is going nowhere.

**The socket is the app's.** A short link and a long link are sockets, and this
port owns none. In Rust the wiring is a `SocketOperator` a link is made with,
through the factory of the net core — the two hooks `net_channel_factory.cc` is
in the C++. The Kotlin, Swift and C bindings carry no seam for one, so outside
Rust a task ends in a socket error (`ErrCmdType::Socket`) instead of going out:
what is there for an app on those platforms is the queue, the choice of link, the
DNS ahead of the connect, the retry, the timeout, and the report at the end.

## What the calls are called

| the C++ | Rust | Android | Swift | C |
|---|---|---|---|---|
| `mars::stn::StartTask` | `stn.send(task, body)` — `start_task(task)` still starts a task nobody awaits, and it is deprecated | `StnLogic.startTask(task)` | `MarsStn.start(task)` | `mars_stn_start_task(&task)` |
| `mars::stn::StopTask` | `stn.stop_task(id)` | `StnLogic.stopTask(id)` | `MarsStn.stop(taskID:)` | `mars_stn_stop_task(id)` |
| `mars::stn::HasTask` | `stn.has_task(id)` | `StnLogic.hasTask(id)` | `MarsStn.hasTask(id)` | `mars_stn_has_task(id)` |
| `mars::stn::SetCallback` | `stn.set_callback(app)` | `StnLogic.setCallBack(cb)` | `MarsStn.setApp { … }` | `mars_stn_set_app(ctx, ask)` |
| `mars::stn::MakesureLonglinkConnected` | `stn.make_sure_long_link_connected("default")` | `StnLogic.makesureLongLinkConnected()` | `MarsStn.makeSureLongLinkConnected()` | `mars_stn_makesure_longlink_connected()` |
| `mars::stn::CreateLonglink_ext` | `stn.create_long_link(config)` | `StnLogic.createLonglink(config)` | `MarsStn.createLongLink(config)` | `mars_stn_create_longlink(&config)` |
| `mars::stn::DestroyLonglink_ext` | `stn.destroy_long_link(name)` | `StnLogic.destroyLonglink(name)` | `MarsStn.destroyLongLink(name)` | `mars_stn_destroy_longlink(name)` |
| `mars::stn::MarkMainLonglink_ext` | `stn.mark_main_longlink(name)` | `StnLogic.markMainLonglink(name)` | `MarsStn.markMainLongLink(name)` | `mars_stn_mark_main_longlink(name)` |
| the queue's thread | `stn.run_pending()` | `StnLogic.runPending()` | `MarsStn.runPending()` | `mars_stn_run_pending()` |

Rust is the only platform whose `StartTask` has a newer call beside it: `send`
starts the task and hands back the answer of it, to await, while `start_task` —
the upstream call, for a task nobody is awaiting — is the one that carries a
deprecation. The Android, Swift and C `start` calls are the upstream call and
carry none.

In a shared Kotlin module these are `StnLogic`'s as well, and the names are the
Android ones but for the app: `StnLogic.setApp { question -> … }` takes one
closure and not an `ICallBack`, and `dueTime()` answers `null` where the Android
one answers `-1`.

## The questions

The eighteen questions the C++ asks as virtuals of `mars::stn::Callback` are one
shape per platform: an `App` trait in Rust, an `ICallBack` on Android, an `ask`
closure in a shared Kotlin module, a closure in Swift, and one callback in C.
Each has a default for every question the app does not answer — except Android's,
which is the plain interface the C++ project's Java declared, so the object is
the app's to finish.

The names are the ones the C++ used: `req2Buf` for the bytes a task sends,
`buf2Resp` for the answer, `onTaskEnd` for the end, `onPush` for what the server
sends down the long link, `onNewDns` for the addresses of a host. See
[the questions](/stn/callbacks).

Two defaults of a task are not the ones `Task::Task()` gives it, and one of them
is not one any Java gives: `need_authed` is `true` and not `false`, which is
what the C++ project's Java answers, and `channel_select` is `CHANNEL_BOTH` and
not `0`, which a net core fails a task for — the C++'s Java takes the channel as
an argument and makes the app name it, so `CHANNEL_BOTH` is this port's own.

## The boot, on Android

The boot is the same: `Mars.init(context, handler)` and `Mars.onCreate(true)` to
start, `BaseEvent.onForeground` and `BaseEvent.onNetworkChange` when the screen or
the network changes, and `AppLogic.setCallBack` for the account and the device
STN asks about — the names the C++ project's Java spelled, in the package of this
port.

## Where to go next

- [Getting started](/stn/getting-started) — the dependency and a running task, on
  every platform that carries STN.
- [Migrating from mars-xlog](/xlog/migrating-from-mars-xlog) — the logger, for an
  app whose task pipeline was not the only piece it took.
- [Migrating from mars-sdt](/sdt/migrating-from-mars-sdt) — the network
  diagnosis, in the same package as the pipeline.
