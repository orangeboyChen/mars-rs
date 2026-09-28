# 从 mars 迁移

跑着整个 C++ 项目的应用要搬三样东西：日志库、任务链路、网络诊断。三样这里都有，用的
是这个移植写的名字；而日志库写出来的就是 C++ 写出来的那个文件 —— 同样的帧结构、同样
的压缩、同样的加密 —— 所以你已经收集到的 `.xlog` 文件，现有的工具照样能读，迁移不丢
任何历史。

日志库是三样里的一样，它有自己一页 —— [从 mars-xlog 迁移](/zh/migrating-from-mars-xlog)
—— 给只用了日志库的应用。这一页是另外两样，以及搬它们时应用要接手的两件事。

## 三块分别对应什么

| 你在用的 | 这里用什么 | 写在哪一页 |
|---|---|---|
| `mars/xlog` —— 日志库 | `xlog`：crates.io 上的 `marsrs-xlog`、JitPack 上的 `xlog`、共享 Kotlin 模块的 `xlog-kmp`、Apple 上的 `MarsRSXlog` | [从 mars-xlog 迁移](/zh/migrating-from-mars-xlog) |
| `mars/stn` —— 任务链路 | `marsrs`，以及它的两个孪生包 `marsrs-kmp` 和 `MarsRSNet` | [任务链路](/zh/stn) |
| `mars/sdt` —— 网络诊断 | 同一个 `marsrs`，和链路在同一个包里 | [网络诊断](/zh/sdt) |

Rust 的 crate、JitPack 和共享 Kotlin 模块做的是这个切分：一个包只有日志库，另一个包是
整个移植，而整个移植里带着的是同一个日志库 —— 只有名字不同。Apple 是切成三份的：
`MarsRSXlog` 是日志库，`MarsRSNet` 是链路和诊断、没有日志库，`MarsRS` 是两份都有。
Flutter 和 React Native 的两个包里都只有日志库，所以这两个平台上的应用没有链路、也
没有诊断要搬 —— 见 [Flutter](/zh/platforms/flutter#里面没有什么)和
[React Native](/zh/platforms/react-native#里面没有什么)。只打日志的应用用日志库
那个包，还要跑任务或诊断的用带着它们的那个。

## 应用接手的两件事

**队列是靠调用排空的，不是靠线程。** 任务和诊断都跑在队列上，C++ 用自己
的线程跑这些队列 —— 一个消息队列线程，以及诊断跑在其上的 `__RunOn` 线程。这个
移植里没有线程，所以本来会是线程的东西是宿主调的一次调用：任务用 `run_pending()`，
诊断用 `runChecks`，两个都带一个答案 —— *我下一次调用最多能等多久*。整个东西就是
一个循环：

::: code-group

```rust [Rust]
while let Some(wait) = stn.due_delay() {      // 这一趟最多能等多久，毫秒
    std::thread::sleep(std::time::Duration::from_millis(wait));
    stn.run_pending();
}
```

```kotlin [Android]
var due = StnLogic.dueTime()                  // -1 是“没什么要等的”
while (due >= 0) {
    Thread.sleep(due)
    StnLogic.runPending()
    due = StnLogic.dueTime()
}
```

:::

上面两种写法是[任务链路](/zh/stn)的；诊断是同一个形状，只是没有 sleep —— `runChecks`
会去问它的探针，问完才返回，所以应用跑一次就可以拿报告。

**socket 是应用自己的。** 短连接、长连接，以及诊断的每一个探针，都是 socket，而这个
移植一个都不拥有：C++ 把自己的网络实现链进了各个检查器里，这里 socket 是什么归调用
方。诊断会一个一个地问你要一次解析、一次连接、一次 HTTP 请求、一次 ICMP echo，你的
答案就是它记进报告的结果 —— 这也是为什么一台没有无线电的机器上也能跑一次诊断。任务
的连接在 Rust 里是同样接的，通过创建连接时给的那个 `SocketOperator`；Kotlin、Swift
和 C 的绑定没有给它留接缝，所以在 Rust 之外启动的任务结束在一个 socket 错误里，而不是
发出去 —— 这些平台上的应用今天能用什么，是[任务链路](/zh/stn)那页说的。

## 迁移任务链路

任务是同一个 struct —— id、命令、路径、要走的链路、主机、超时、重试 —— 跑的过程中应用
回答的还是那些问题。有两个默认值不是 `Task::Task()` 给的，两个都是 C++ 项目的 Java
给它的 `Task` 的那个：`channel_select` 是 `CHANNEL_BOTH` 而不是 `0`，后者是 `NetCore`
会判失败的值；`need_authed` 是 `true` 而不是 `false`。除此之外变的是答案怎么到 STN，
以及谁来排空队列。

| C++ | Rust | Android | Swift | C |
|---|---|---|---|---|
| `mars::stn::StartTask` | `stn.start_task(task)` | `StnLogic.startTask(task)` | `MarsStn.start(task)` | `mars_stn_start_task(&task)` |
| `mars::stn::StopTask` | `stn.stop_task(id)` | `StnLogic.stopTask(id)` | `MarsStn.stop(taskID:)` | `mars_stn_stop_task(id)` |
| `mars::stn::HasTask` | `stn.has_task(id)` | `StnLogic.hasTask(id)` | `MarsStn.hasTask(id)` | `mars_stn_has_task(id)` |
| `mars::stn::SetCallback` | `stn.set_callback(app)` | `StnLogic.setCallBack(cb)` | `MarsStn.setApp { … }` | `mars_stn_set_app(ctx, ask)` |
| `mars::stn::MakesureLonglinkConnected` | `stn.make_sure_long_link_connected("default")` | `StnLogic.makesureLongLinkConnected()` | `MarsStn.makeSureLongLinkConnected()` | `mars_stn_makesure_longlink_connected()` |
| 队列的那个线程 | `stn.run_pending()` | `StnLogic.runPending()` | `MarsStn.runPending()` | `mars_stn_run_pending()` |

C++ 作为 `mars::stn::Callback` 的虚函数问的那十八个问题，在 Rust 里是一个 `App` trait，
在 Kotlin 里是一个 `ICallBack`，在 Swift 里是一个闭包；三个都为应用不回答的每个问题
准备了默认值 —— 只有 Android 的不是，它是 C++ 项目的 Java 声明的那个普通 interface，
所以那个对象要应用自己写完。名字还是 C++ 用的那些：任务要发的字节是 `req2Buf`，回来
的答案是 `buf2Resp`，结束是 `onTaskEnd`，服务器顺着长连接推下来的是 `onPush`，一个
主机的地址是 `onNewDns`。

任务怎么结束没变：一个说*在哪*失败的 error **type** —— Rust 的 `ErrCmdType`，其他
地方的 `errType`；一个说*出了什么事*的 error code；以及一份耗时 profile。Android 像
以前一样把那些 code 命名在 `StnLogic` 上。

Android 上的启动也一样：`Mars.init(context, handler)` 和 `Mars.onCreate(true)`
开局，屏幕或网络变了用 `BaseEvent.onForeground` 和 `BaseEvent.onNetworkChange`，
STN 要问的账号和设备用 `AppLogic.setCallBack` —— 还是 C++ 项目的 Java 写的那些名字，
只是放在这个移植的包里。

而队列是上面说的那个新义务：一个启动了却从来没被排空的任务会一直留在它的队列里。
[任务链路](/zh/stn)是它的全部 —— 两条链路、任务的字段、以及长连接要应用做什么。

## 迁移网络诊断

一次诊断在 C++ 里和在这里是同样的三步：给出主机和要做的检查，跑，拿报告。

| 什么 | C++ | Rust | Android、共享 Kotlin | Swift | C |
|---|---|---|---|---|---|
| 主机和 mode | `StartActiveCheck` | `start_active_check(…)` | `startActiveCheck(…)` | `startActiveCheck(…)` | `mars_sdt_start_active_check(…)` |
| 跑这个计划 | `__RunOn` 线程 | `run_checks(&mut ask, net)` | `runChecks(net, probe)` | `runChecks(networkType:probe:)` | `mars_sdt_run_checks(ctx, probe, net)` |
| 停掉它 | `CancelActiveCheck` | `cancel_active_check()` | `cancelActiveCheck()` | `cancelActiveCheck()` | `mars_sdt_cancel_active_check()` |
| net-check 的 CGI | `SetHttpNetcheckCGI(cgi)` | `set_http_netcheck_cgi(cgi)` | `setHttpNetcheckCGI(cgi)` | `setHTTPNetCheckCGI(cgi)` | `mars_sdt_set_http_netcheck_cgi(cgi)` |
| 报告 | `Callback::ReportNetCheckResult` | `report_json(&results)` | `takeReport()`，或回调 | `takeReport()` | `mars_sdt_take_report(buf, len)` |

mode 还是那几个 bit —— ping 和 DNS，然后 TCP，然后是 net-check CGI 的 HTTP 检查 ——
报告还是同一份 `{"details":[ … ]}` 文档，每项检查一个对象，里面还是同样的
`detectType`、`errorCode` 和各项耗时，所以今天读报告的东西也读得懂这一份。

归应用的是它下面的网络：C++ 把自己的 socket 链进了检查器，这里一次检查是这一轮问你的
一个问题 —— 一个要解析的域名、一个要连的 ip 和端口、一个要取的 URL、一个要 ping 的
主机 —— 你的答案就是报告里记的东西。在 Android 和共享 Kotlin 里那是你交给
`runChecks` 的 `IProbe`；在 Rust 里是 `Ask`；在 Swift 和 C 里是那个闭包。

[网络诊断](/zh/sdt)是它的全部：mode、跑之前可以先看的计划，以及报告的每个字段。

## 接下来

- [从 mars-xlog 迁移](/zh/migrating-from-mars-xlog) —— 日志库：appender、它的配置
  项，以及它取代的那些 Java 和 Apple 调用点。
- [快速开始](/zh/getting-started) —— 每个平台的依赖和一段能跑的代码。
- [日志文件](/zh/log-files) —— 文件落在哪、怎么读回来。
