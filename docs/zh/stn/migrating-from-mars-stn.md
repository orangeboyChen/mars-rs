# 从 mars-stn 迁移

App 从 C++ 项目的任务链路迁过来，要挪的有两样东西：发起任务的那几个调用，以及发起
时要接过去的那两件事。一个任务还是那个 struct，App 在它跑着的时候回答的还是那些问
题，一个任务怎么结束也没有变 —— 一个错误类型、一个错误码和一个 profile。

| 你用的那一块 | 这里拿什么 | 写在哪一页 |
|---|---|---|
| `mars/stn` | JitPack 上的 `marsrs` 和同名的 crate（crates.io 还没发布），共享 Kotlin 模块的 `marsrs-kmp`，Apple 上的 `MarsRSNet` | [快速开始](/zh/stn/getting-started) |
| `mars/xlog` | `xlog` —— 日志，在自己的一个包里 | [从 mars-xlog 迁移](/zh/xlog/migrating-from-mars-xlog) |
| `mars/sdt` | 同一个 `marsrs`，和链路在同一个包里 | [从 mars-sdt 迁移](/zh/sdt/migrating-from-mars-sdt) |

## App 接过去的那两件事

**队列是靠一次调用排空的，不是靠一个线程。** C++ 用自己的一个 message-queue 线程跑
队列；这个移植里没有线程，所以本该是一个线程的地方，是宿主的一次调用 ——
`run_pending()`，以及 `due_time()`，告诉它这一趟最多还能等多久。一个循环就是全部：

::: code-group

```rust [Rust]
while let Some(wait) = stn.due_delay() {      // 这一趟还能等多久，毫秒
    std::thread::sleep(std::time::Duration::from_millis(wait));
    stn.run_pending();
}
```

```kotlin [Android]
var due = StnLogic.dueTime()                  // -1 是"没有可等的东西"
while (due >= 0) {
    Thread.sleep(due)
    StnLogic.runPending()
    due = StnLogic.dueTime()
}
```

```kotlin [Kotlin Multiplatform]
var due = StnLogic.dueTime()                  // null 是"没有可等的东西"
while (due != null) {
    wait(due)                                 // App 自己的：共享 Kotlin 里没有 sleep
    StnLogic.runPending()
    due = StnLogic.dueTime()
}
```

:::

共享 Kotlin 里那个等也是 App 自己的：共享 Kotlin 没有自己的 sleep，所以上面的
`wait` 是 App 写的一个 `expect` —— Android 上 `Thread.sleep(due)`，native 目标上
`usleep(due * 1000)`。

在 Rust 里一个调用就能起一个：`Driver::spawn(stn)` 起这个 crate 的一个线程排空
队列 —— 到点了 `run_pending()`，其间睡 `due_delay()` —— `Driver` 被 drop 的时候把它
join 掉。要 await 一个任务又没有自己循环的 App 用它；有循环的 App 留着那个循环，
因为一趟把某个任务跑完的时候，await 它的人会被唤醒。

一个发起了却一直没排空的任务会一直待在它的队列里 —— `has_task` 对一个哪儿也去不了
的任务照样回答 `true`。

**socket 是 App 的。** 短连接和长连接就是 socket，而这个移植一个都不拥有。在 Rust
里，接线就是造链路时给它一个 `SocketOperator`，走 net core 的 factory —— 也就是
C++ 里 `net_channel_factory.cc` 那两个钩子。Kotlin、Swift 和 C 的绑定都没有给它留
口子，所以在 Rust 之外，一个任务是以一次 socket 错误（`ErrCmdType::Socket`）结束
的，而不是真的发出去：那些平台上的 App 能拿到的是队列、链路的选择、连接前的 DNS、
重试、超时，以及最后的报告。

## 那几个调用叫什么

| C++ | Rust | Android | Swift | C |
|---|---|---|---|---|
| `mars::stn::StartTask` | `stn.send(task, body)` —— `start_task(task)` 还能发起没人 await 的任务，它带 deprecated | `StnLogic.startTask(task)` | `MarsStn.start(task)` | `mars_stn_start_task(&task)` |
| `mars::stn::StopTask` | `stn.stop_task(id)` | `StnLogic.stopTask(id)` | `MarsStn.stop(taskID:)` | `mars_stn_stop_task(id)` |
| `mars::stn::HasTask` | `stn.has_task(id)` | `StnLogic.hasTask(id)` | `MarsStn.hasTask(id)` | `mars_stn_has_task(id)` |
| `mars::stn::SetCallback` | `stn.set_callback(app)` | `StnLogic.setCallBack(cb)` | `MarsStn.setApp { … }` | `mars_stn_set_app(ctx, ask)` |
| `mars::stn::MakesureLonglinkConnected` | `stn.make_sure_long_link_connected("default")` | `StnLogic.makesureLongLinkConnected()` | `MarsStn.makeSureLongLinkConnected()` | `mars_stn_makesure_longlink_connected()` |
| `mars::stn::CreateLonglink_ext` | `stn.create_long_link(config)` | `StnLogic.createLonglink(config)` | `MarsStn.createLongLink(config)` | `mars_stn_create_longlink(&config)` |
| `mars::stn::DestroyLonglink_ext` | `stn.destroy_long_link(name)` | `StnLogic.destroyLonglink(name)` | `MarsStn.destroyLongLink(name)` | `mars_stn_destroy_longlink(name)` |
| `mars::stn::MarkMainLonglink_ext` | `stn.mark_main_longlink(name)` | `StnLogic.markMainLonglink(name)` | `MarsStn.markMainLongLink(name)` | `mars_stn_mark_main_longlink(name)` |
| 队列那个线程 | `stn.run_pending()` | `StnLogic.runPending()` | `MarsStn.runPending()` | `mars_stn_run_pending()` |

Rust 是唯一一个 `StartTask` 旁边还站着一个更新的调用的平台：`send` 发起任务，并把
它的回答交回来让你 await，而 `start_task` —— 上游那个调用，给没人 await 的任务用的
—— 才是带着 deprecated 的那个。Android、Swift 和 C 的 `start` 是上游的调用，没有
deprecated。

共享 Kotlin 模块里这几个也是 `StnLogic` 的，名字跟 Android 的一样，只有 App 那个不
一样：`StnLogic.setApp { question -> … }` 接的是一个闭包而不是 `ICallBack`，
`dueTime()` 回答 `null` 而不是 `-1`。

## 那些问题

C++ 把那十八个问题写成 `mars::stn::Callback` 的虚函数，每个平台上换一种形状：
Rust 里一个 `App` trait，Android 上一个 `ICallBack`，共享 Kotlin 模块里一个 `ask`
闭包，Swift 里一个闭包，C 里一个回调。每一个都为 App 没回答的问题准备了默认值 ——
只有 Android 那个例外，它是 C++ 项目的 Java 声明出来的一个普通 interface，所以那个
对象要 App 自己补全。

名字是 C++ 用的那几个：`req2Buf` 是任务要发的字节，`buf2Resp` 是回答，`onTaskEnd`
是结束，`onPush` 是服务器从长连接上推下来的东西，`onNewDns` 是一个 host 的地址。见
[那些问题](/zh/stn/callbacks)。

任务的两个默认值不是 `Task::Task()` 给的那两个，其中一个还不是任何 Java 给的：
`need_authed` 是 `true` 而不是 `false`，那是 C++ 项目的 Java 的回答；`channel_select`
是 `CHANNEL_BOTH` 而不是 `0` —— 后者 net core 会判为失败 —— 而 C++ 的 Java 把通道当作
一个参数，让 App 自己说，所以 `CHANNEL_BOTH` 是这个移植自己的。

## Android 上的启动

启动这一步也一样：`Mars.init(context, handler)` 和 `Mars.onCreate(true)` 起两座
桥，屏幕或网络变了的时候 `BaseEvent.onForeground` 和 `BaseEvent.onNetworkChange`，
`AppLogic.setCallBack` 给的是 STN 问的账号和设备 —— C++ 项目的 Java 拼的那几个名字，
在这个移植的包里。

## 接着看

- [快速开始](/zh/stn/getting-started) —— 每个带着 STN 的平台上的依赖和一个跑起来的
  任务。
- [从 mars-xlog 迁移](/zh/xlog/migrating-from-mars-xlog) —— 日志，给那些除了任务
  链路还拿了别的东西的应用。
- [从 mars-sdt 迁移](/zh/sdt/migrating-from-mars-sdt) —— 网络诊断，和链路在同一个
  包里。
