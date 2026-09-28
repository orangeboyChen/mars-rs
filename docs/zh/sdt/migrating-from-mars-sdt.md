# 从 mars-sdt 迁移

跑过 C++ 项目网络诊断的 App 有一件事要挪：什么是探测。一次诊断在 C++ 和这里是同样
三步 —— 说出 hosts 和检查项、跑、取报告 —— 报告也是同一个 `{"details":[ … ]}` 文
档，一项检查一个对象，里面是同样的 `detectType`、`errorCode` 和那些耗时，所以今天
读报告的那个东西读得懂这份。

| 你用的那一块 | 这里拿什么 | 写在哪一页 |
|---|---|---|
| `mars/sdt` | crates.io 和 JitPack 上的 `marsrs`，共享 Kotlin 模块的 `marsrs-kmp`，Apple 上的 `MarsRSNet` | [快速开始](/zh/sdt/getting-started) |
| `mars/xlog` | `xlog` —— 日志，在自己的一个包里 | [从 mars-xlog 迁移](/zh/xlog/migrating-from-mars-xlog) |
| `mars/stn` | 同一个 `marsrs`，和诊断在同一个包里 | [从 mars-stn 迁移](/zh/stn/migrating-from-mars-stn) |

## 那几个调用叫什么

| 什么 | C++ | Rust | Android、共享 Kotlin | Swift | C |
|---|---|---|---|---|---|
| hosts 和 mode | `StartActiveCheck` | `start_active_check(…)` | `startActiveCheck(…)` | `startActiveCheck(…)` | `mars_sdt_start_active_check(…)` |
| 跑计划 | `__RunOn` 那个线程 | `run_checks(&mut ask, net)` | `runChecks(net, probe)` | `runChecks(networkType:probe:)` | `mars_sdt_run_checks(ctx, probe, net)` |
| 停掉 | `CancelActiveCheck` | `cancel_active_check()` | `cancelActiveCheck()` | `cancelActiveCheck()` | `mars_sdt_cancel_active_check()` |
| net-check 的 CGI | `SetHttpNetcheckCGI(cgi)` | `set_http_netcheck_cgi(cgi)` | `setHttpNetcheckCGI(cgi)` | `setHTTPNetCheckCGI(cgi)` | `mars_sdt_set_http_netcheck_cgi(cgi)` |
| 报告 | `Callback::ReportNetCheckResult` | `report_json(&results)` | `takeReport()`，或那个回调 | `takeReport()` | `mars_sdt_take_report(buf, len)` |

mode 是同一套 bit —— 先 ping 和 DNS，再 TCP，再 net-check CGI 的那次 HTTP —— `0`
还是一项都不跑。见[检查项](/zh/sdt/checks)。

## App 接过去的那两件事

**探测是 App 的。** C++ 把自己的 socket 链进那些 checker，所以那边的一次诊断只能对
着平台自己的网络跑。这里一项检查是这一趟问你的一个问题 —— 要解析的一个域名、要连的
一个 ip 和端口、要取的一个 URL、要 ping 的一个 host —— 你回答什么，报告就记下什么。
在 Android 和共享 Kotlin 上，这是你交给 `runChecks` 的那个 `IProbe`；在 Rust 里是那
个 `Ask`；在 Swift 和 C 里是那个闭包。见[探测](/zh/sdt/probes)。

**本该是一个线程的地方，是一次调用。** C++ 在它的 `__RunOn` 线程上跑诊断；这个库
里没有线程，所以 `runChecks` 在调用它的那个线程上问它的探测，所有探测回答了才返回
—— App 跑一趟、取报告，没有循环，也没有 sleep。

取走报告就清空了，`mars_sdt_take_report` 的 buffer 在报告塞不下时会把结果留着 ——
见[报告](/zh/sdt/report)。

## 接着看

- [快速开始](/zh/sdt/getting-started) —— 每个带着 SDT 的平台上的依赖和一整趟诊断。
- [从 mars-xlog 迁移](/zh/xlog/migrating-from-mars-xlog) —— 日志，给诊断不是它唯一
  拿的那块的应用。
- [从 mars-stn 迁移](/zh/stn/migrating-from-mars-stn) —— 任务链路，和诊断在同一个
  包里。
