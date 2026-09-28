# 探测

先要知道一件事：一项检查就是一个 **socket** —— 一次解析、一次连接、一个 HTTP
请求、一个 ICMP echo —— 而这个库一个都不持有。所以一趟诊断是一个一个地向 App
要网络，App 回答什么，报告就记下什么：

| 检查项 | 它要什么 | 你回答什么 |
|---|---|---|
| ping | host，一个以**秒**计的超时 | 花了多久，以及丢了多少比例的 ping |
| dns | 域名，一个以毫秒计的超时 | 它解析出来的那些地址 |
| tcp | ip 和端口，一个以毫秒计的超时 | noop 有没有发出去、回来的是什么、以及它是不是那个回答 |
| http | net-check CGI 的 URL | HTTP 状态码，以及这个请求花了多久 |

这也就是一趟诊断**测得了**的原因，也是一台不是手机的机器也能跑一趟的原因：在设备上
回答的那个闭包，换到测试里、换到一台没有无线电的机器上，回答的还是同一个东西。

## App 把探测交到哪里

| 你的 App 是 | 谁来回答探测 |
|---|---|
| Rust | `sdt.run_checks(&mut ask, net)` 的那个 `Ask` |
| iOS / watchOS，Swift | `MarsSdt.runChecks(networkType:) { … }` 的那个闭包 |
| Android | `SdtLogic.runChecks(net, probe)` 的那个 `SdtLogic.IProbe` |
| Kotlin Multiplatform | 同一个 `IProbe`，交给 `SdtLogic.runChecks(1, probe)` |
| 有 C FFI 的任何东西 | `mars_sdt_run_checks(ctx, probe, net)` 的那个函数指针 |
| HarmonyOS | 同一个函数指针，通过你自己写的 NAPI shim |

这一趟在调用它的那个线程上、按计划的顺序一个一个地问，所有探测都回答了它才返回
—— 所以不像[任务](/zh/stn/getting-started)那样要 sleep、要循环。一个没什么可说的
探测就回答“没什么” —— Swift 里 `.nothing`，共享 Kotlin 里 `Answer.None`，C 里
`MarsSdtAnswerNothing` —— 那一项检查就记成没跑的一项。

## 那些回答

| 检查项 | Rust | Swift | 共享 Kotlin | Android | C |
|---|---|---|---|---|---|
| ping | `Answer::Ping { error_code, rtt, status }` | `.ping(errorCode:rtt:lossRate:averageRTT:)` | `ProbeAnswer.Ping` | `SdtLogic.Answer.ping(…)` | `MarsSdtPing` |
| dns | `Answer::Dns { error_code, rtt, ips }` | `.dns(errorCode:rtt:addresses:)` | `ProbeAnswer.Dns` | `SdtLogic.Answer.dns(…)` | `MarsSdtDns` |
| tcp | `Answer::Tcp { sent, received, is_noop_resp, rtt }` | `.tcp(errorCode:rtt:noop:)` | `ProbeAnswer.Tcp` | `SdtLogic.Answer.tcp(…)` | `MarsSdtTcp` |
| http | `Answer::Http { error_code, status_code, rtt }` | `.http(errorCode:rtt:statusCode:)` | `ProbeAnswer.Http` | `SdtLogic.Answer.http(…)` | `MarsSdtHttp` |

答了别的探测，等于没答：报告记的只是这一项检查拿回来的东西。

## 接着看

- [检查项](/zh/sdt/checks) —— 那个 mode，以及这一趟按顺序走的计划。
- [报告](/zh/sdt/report) —— 探测回答了什么的那个 JSON。
