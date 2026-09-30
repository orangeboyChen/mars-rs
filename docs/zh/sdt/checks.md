# 检查项

一次诊断跑的是一个 **mode** 里的那些检查，mode 是一个 bit set，在探测任何东西之前
它会变成一个可以读出来的计划。

## 那几项检查

| 检查项 | Rust | Swift | 共享 Kotlin | Android | C |
|---|---|---|---|---|---|
| ping | `NetCheckType::PingCheck` | `.ping` | `Check.Ping` | `0` | `MarsSdtCheckPing` |
| dns | `NetCheckType::DnsCheck` | `.dns` | `Check.Dns` | `1` | `MarsSdtCheckDns` |
| 对着另一个服务器做的 dns，用来比 | `NetCheckType::NewDnsCheck` | `.newDns` | `Check.NewDns` | `2` | `MarsSdtCheckNewDns` |
| tcp | `NetCheckType::TcpCheck` | `.tcp` | `Check.Tcp` | `3` | `MarsSdtCheckTcp` |
| http | `NetCheckType::HttpCheck` | `.http` | `Check.Http` | `4` | `MarsSdtCheckHttp` |
| traceroute —— 有计划，但还没有探测问它 | `NetCheckType::TracerouteCheck` | `.traceroute` | `Check.Traceroute` | — | `MarsSdtCheckTraceroute` |
| 请求自己的 buffer —— 同上 | `NetCheckType::ReqBufCheck` | `.reqBuf` | `Check.ReqBuf` | — | `MarsSdtCheckReqBuf` |

## mode 由哪些 bit 组成

| 那个 bit | Rust | 共享 Kotlin、Android | Swift | C | 它往计划里放什么 |
|---|---|---|---|---|---|
| `NET_CHECK_BASIC` | `NET_CHECK_BASIC` | `CheckMode.K_BASIC` | `.basic` | `NET_CHECK_BASIC` | 一次 ping 和一次 dns —— C++ 最先跑的就是这两项 |
| `NET_CHECK_LONG` | `NET_CHECK_LONG` | `K_LONG` | `.long` | `NET_CHECK_LONG` | 一次 tcp：往长连接的 hosts 发一个 noop |
| `NET_CHECK_SHORT` | `NET_CHECK_SHORT` | `K_SHORT` | `.short` | `NET_CHECK_SHORT` | 一次 http：那个 net-check CGI，以及短连接的 hosts |

它们按位或起来 —— `NET_CHECK_BASIC | NET_CHECK_LONG` 是跑三项的一趟 —— 而 `0` 是
**一项都不跑**：计划是空的，这一趟不问任何探测，报告是 `{"details":[]}`。它不是“全
部都跑”，全部都跑是 `1 | 2 | 4`，在 Swift 里是 `Mode.all`。

Swift 把它们当成一个集合来收，`mode: [.basic, .long]`；Objective-C 还是用那个整
数：`mode: NET_CHECK_BASIC | NET_CHECK_LONG`。

`NET_CHECK_SHORT` 是那个需要自己 hosts 的：这个 bit 配一个空的 `shortLink`，是一份
“http 检查但没有东西可检查”的计划，跑出来的报告关于短连接什么也没说。

## 先把计划读出来

| 什么 | Rust | Swift | 共享 Kotlin、Android | C |
|---|---|---|---|---|
| 计划，按检查要跑的顺序 | `sdt.plan()` | `MarsSdt.plan` | `SdtLogic.plan()` | `mars_sdt_plan` |

共享 Kotlin 交回来的是 `Check`，Android 是那些整数本身，而计划里的整数就是
[报告](/zh/sdt/report)每一项的 `detectType` —— 所以两边是同一套词。

## 接着看

- [探测](/zh/sdt/probes) —— 每一项检查要 App 给什么，以及一个回答由什么构成。
- [快速开始](/zh/sdt/getting-started) —— 每个带着 SDT 的平台上的依赖和一整趟诊断。
