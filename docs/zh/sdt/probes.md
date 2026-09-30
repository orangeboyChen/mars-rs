# 探测

这一件事要先知道。一项检查就是一个 **socket** —— 一次解析、一次连接、一个
HTTP 请求、一个 ICMP echo —— 而这个移植一个都不拥有。所以一次诊断是一个一个地向
App 要网络，App 回答什么，报告就记下什么：

| 检查项 | 它要什么 | 你回答什么 |
|---|---|---|
| ping | host，一个以**秒**计的超时 | 花了多久，以及丢了多少比例的 ping |
| dns | 域名，一个以毫秒计的超时 | 它解析出来的那些地址 |
| tcp | ip 和端口，一个以毫秒计的超时 | noop 有没有发出去、回来的是什么、以及它是不是那个回答 |
| http | net-check CGI 的 URL | HTTP 状态码，以及这个请求花了多久 |

这也正是一次诊断**可以测**的原因，也正是一个不是手机的机器也能跑一趟的原因：在设备
上回答的那个闭包，在测试里、在一台没有无线电的机器上，回答的是同一个东西。

## App 把探测交到哪里

| 你的 App 是 | 探测交给谁 |
|---|---|
| Rust | `sdt.run_checks(&mut ask, net)` 的那个 `Ask` |
| iOS / watchOS，Swift 或 Objective-C | `MarsSdt.runChecks(networkType:) { … }` 的那个闭包，或者说它变成的那个 block |
| Android | `SdtLogic.runChecks(net, probe)` 的那个 `SdtLogic.IProbe` |
| Kotlin Multiplatform | 同一个 `IProbe`，交给 `SdtLogic.runChecks(1, probe)` |
| 有 C FFI 的任何东西 | `mars_sdt_run_checks(ctx, probe, net)` 的那个函数指针 |
| HarmonyOS | 同一个函数指针，通过你自己写的 NAPI shim |

这一趟在调用它的那个线程上，按计划的顺序一个一个地问，所有探测都回答了它才返回 ——
所以不像[任务](/zh/stn/getting-started)那样要 sleep、要循环。一个没什么可说的探测就
回答“没什么” —— Swift 里 `.nothing()`，共享 Kotlin 里 `ProbeAnswer.None`，C 里
`MarsSdtNothing` —— 问它的那一项检查就被记成失败的一项，而一项失败的检查会结束这一趟：
排在它后面的就不查了。ping 是唯一的例外：一次没发出去的 ping 是一项没跑的检查，报告里
没有它，排在它后面的那一项照查。

## 那些回答

| 检查项 | Rust | Swift | 共享 Kotlin | Android | C |
|---|---|---|---|---|---|
| ping | `Answer::Ping { error_code, rtt, status }` | `.ping(errorCode:rtt:lossRate:averageRTT:)` | `ProbeAnswer.Ping` | `SdtLogic.Answer.ping(…)` | `MarsSdtPing` |
| dns | `Answer::Dns { error_code, rtt, local_dns, ips }` | `.dns(errorCode:rtt:addresses:)` | `ProbeAnswer.Dns` | `SdtLogic.Answer.dns(…)` | `MarsSdtDns` |
| tcp | `Answer::Tcp { sent, received, is_noop_resp, conntime, rtt }` | `.tcp(errorCode:rtt:noop:)` | `ProbeAnswer.Tcp` | `SdtLogic.Answer.tcp(…)` | `MarsSdtTcp` |
| http | `Answer::Http { error_code, status_code, rtt }` | `.http(errorCode:rtt:statusCode:)` | `ProbeAnswer.Http` | `SdtLogic.Answer.http(…)` | `MarsSdtHttp` |

在 C 里这四个是一个 struct —— `MarsSdtAnswer` —— 里面的 `kind` 说明它是哪一个：
最后一列那几个名字是 `MarsSdtKind` 的四个值，不是四个类型。Rust 那两个字段又是
Rust 独有的 —— `local_dns` 和 `conntime`，别的 seam 都没有对应的字段 —— 所以在那些
seam 上回答的探测，报告里的 `localDns` 是空的，`conntime` 是 `0`。

一个探测如果不是这一趟问的那个，它的回答就不算：报告记下的是跑起来的那一项检查
带回来的东西。

## 接着看

- [检查项](/zh/sdt/checks) —— 那个 mode，以及这一趟按顺序走的计划。
- [报告](/zh/sdt/report) —— 探测回答了什么的那个 JSON。
