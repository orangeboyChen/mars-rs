# 网络诊断（SDT）

SDT（“smart diagnosis tool”）回答一个问题：*这台手机为什么到不了服务器？* 它拿两条链路的
hosts，对它们跑 **ping**、**DNS**、**TCP** 和 **HTTP** 检查，然后交给应用一份 JSON 报告，
写清每一项记到了什么。STN 是应用一直在跑的东西，SDT 是它想知道 STN 为什么不工作的时候
才跑的。

## 它在哪

| 你的应用是 | 什么带着 SDT | 怎么用它 |
|---|---|---|
| Rust | `marsrs`（`marsrs-xlog` 里一点都没有） | [`marsrs::sdt`](https://docs.rs/marsrs) —— 见 [Rust](/zh/platforms/rust#网络诊断) |
| Swift | `MarsRSNet` 这个 product，或 `MarsRS` | `MarsSdt` —— 见 [SwiftPM](/zh/platforms/swift#网络诊断) |
| Android，Kotlin 或 Java | JitPack 上的 `marsrs`，不是 `xlog` | `io.github.orangeboychen.marsrs.sdt.SdtLogic` —— 见 [Android](/zh/platforms/android#网络诊断) |
| Kotlin Multiplatform | `marsrs-kmp`，不是 `xlog-kmp` | `io.github.orangeboychen.marsrs.sdt.SdtLogic` —— 见 [Kotlin Multiplatform](/zh/platforms/kotlin-multiplatform#网络诊断) |
| 任何能调 C 的 | `include/mars_sdt.h` | `mars_sdt_*` —— 见 [C ABI](/zh/platforms/c-abi#网络诊断) |
| HarmonyOS | 同一个 `libmars_ffi.so` | `mars_sdt_*`，通过你自己写的 NAPI shim —— 见 [HarmonyOS](/zh/platforms/harmonyos#mars-的另一半) |

Flutter 插件和 React Native 模块今天就只有日志：两个都发起不了一次检查，在它们下面的
Dart 和 TypeScript 能发起之前也不能 —— 见 [Flutter](/zh/platforms/flutter#里面没有什么)
和 [React Native](/zh/platforms/react-native#里面没有什么)。

## 探针是你自己的

这是其余内容之前要知道的唯一一件事。一次检查就是一个 **socket** —— 一次解析、一次连接、
一个 HTTP 请求、一次 ICMP 回显 —— 而这个移植一个都不拥有。C++ 把 `dnsquery.cc` 和其余几个
直接链进 checker 里，所以那里的诊断永远只能跑在平台自己的 socket 上。这里的应用是把网络
递进来 —— 一个每次回答一个探针的闭包：

| 哪一项检查 | 它要什么 | 你回答什么 |
|---|---|---|
| ping | host，以**秒**为单位的超时 | 花了多久，以及丢了多少比例的 ping |
| dns | 域名，以毫秒为单位的超时 | 解析出来的那些地址 |
| tcp | ip 和端口，以毫秒为单位的超时 | noop 有没有发出去、回来的是什么、以及它是不是那个 noop 的回答 |
| http | net-check CGI 的 URL | HTTP 状态码，以及这个请求花了多久 |

这也正是一次诊断**能被测试**的原因，也是一台不是手机的设备也能跑一次的原因：在设备上回答的
那个 `Ask`，同样能在测试里回答，或者在一台没有射频的机器上回答。

## 三次调用

**1. 开始**这次诊断：给两条链路的 hosts、一个 mode 和一个超时。此时还没有探针跑起来：
这个调用只写下计划。

**2. 跑**这个计划，你的探针就是在这里被问到 —— 每项检查一次，按顺序，在调用线程上。
这个移植没有线程，所以它是宿主的一次调用。

**3. 取报告**，送到你的日志去的地方 —— 或者装一个 `ICallBack`，让这次跑把它递给你。两者
拿到的是同一份文档，所以应用只用其中一个：在 Android 和共享 Kotlin 上，一次跑会把报告交给
回调，*同时*留着给之后来的 `takeReport()`，所以两个都用就是把同一次诊断送了两次。取走就把
它清空了：下一次调用报的是这之后发生的事。

::: code-group

```rust [Rust]
use marsrs::sdt::checkimpl::{Answer, Ask, Query};
use marsrs::sdt::{report_json, CheckIPPort, CheckIPPorts, SdtLogic, NET_CHECK_BASIC, NET_CHECK_LONG};

let mut sdt = SdtLogic::new();
sdt.set_http_netcheck_cgi("http://example.com/netcheck");

// 1. 两条链路的 hosts，按它们被叫的那个名字索引
let mut longlink = CheckIPPorts::new();
longlink.insert("default".to_owned(), vec![CheckIPPort::new("1.2.3.4", 80)]);
let shortlink = CheckIPPorts::new();
// mode 是要跑哪些检查：ping 和 dns，然后 tcp。`0` 是一样都不跑。
sdt.start_active_check(&longlink, &shortlink, NET_CHECK_BASIC | NET_CHECK_LONG, 10_000);

// 2. 计划，跑在 `ask` 回答的那个网络上
let mut ask = Ask::new(|query| match query {
    Query::Dns { domain, .. } => Answer::Dns {
        error_code: 0, rtt: 12, ips: vec!["1.2.3.4".to_owned()],
    },
    Query::Tcp { .. } => Answer::Tcp { sent: 0, received: 0, is_noop_resp: true, rtt: 30 },
    Query::Http { .. } => Answer::Http { error_code: 0, status_code: 200, rtt: 40 },
    Query::Ping { .. } => Answer::Ping { error_code: 0, rtt: 20, status: None },
});
let results = sdt.run_checks(&mut ask, 1 /* comm::getNetInfo() */);

// 3. 报告
println!("{}", report_json(&results));
```

```swift [Swift]
import MarsRSNet

// 1. 两条链路的 hosts
let longLink = [MarsSdt.Link(
    name: "default",
    ports: [MarsSdt.HostPort(host: "1.2.3.4", port: 80)]
)]
MarsSdt.setHTTPNetCheckCGI("http://example.com/netcheck")
// 1 | 2 是 NET_CHECK_BASIC | NET_CHECK_LONG：ping 和 dns，然后 tcp
MarsSdt.startActiveCheck(longLink: longLink, shortLink: [], mode: 1 | 2, timeout: 10_000)

// 2. 计划，跑在 `probe` 回答的那个网络上
MarsSdt.runChecks(networkType: 1) { query in
    switch query.probe {
    case .dns:   return .dns(errorCode: 0, rtt: 12, addresses: ["1.2.3.4"])
    case .tcp:   return .tcp(errorCode: 0, rtt: 30, noop: MarsSdt.Noop(sent: 0, received: 0, isNoopResponse: true))
    case .http:  return .http(errorCode: 0, rtt: 40, statusCode: 200)
    case .ping:  return .ping(errorCode: 0, rtt: 20, lossRate: 0, averageRTT: 18)
    default:     return .nothing
    }
}

// 3. 报告
if let report = MarsSdt.takeReport() { send(report) }
```

```kotlin [Android]
import io.github.orangeboychen.marsrs.sdt.SdtLogic

// 0. 报告送到哪
SdtLogic.setCallBack(object : SdtLogic.ICallBack {
    override fun reportSignalDetectResults(resultsJson: String?) { send(resultsJson) }
})

// 1. 两条链路的 hosts，以及 mode：ping 和 dns，然后 tcp
val longLink = arrayOf(SdtLogic.Link("default", arrayOf("1.2.3.4"), intArrayOf(80)))
SdtLogic.setHttpNetcheckCGI("http://example.com/netcheck")
SdtLogic.startActiveCheck(
    longLink,
    emptyArray(),
    SdtLogic.CheckMode.K_BASIC or SdtLogic.CheckMode.K_LONG,
    10_000,
)

// 2. 计划，跑在 `probe` 回答的那个网络上
SdtLogic.runChecks(
    1,
    object : SdtLogic.IProbe {
        override fun dns(host: String, timeoutMs: Int) =
            SdtLogic.Answer.dns(errorCode = 0, rtt = 12, ips = arrayOf("1.2.3.4"))
        override fun tcp(host: String, port: Int, timeoutMs: Int) =
            SdtLogic.Answer.tcp(sent = 0, received = 0, isNoopResponse = true, rtt = 30)
        override fun http(url: String, timeoutMs: Int) =
            SdtLogic.Answer.http(errorCode = 0, statusCode = 200, rtt = 40)
        override fun ping(host: String, timeoutSec: Int) =
            SdtLogic.Answer.ping(errorCode = 0, rtt = 20, lossRate = 0f, averageRTT = 18f)
    },
)

// 3. 不用做什么：上面的 `runChecks` 已经把报告交给回调了。`takeReport()` 是拿到它的另一
//    条路 —— 给不装回调的应用用 —— 既被通知又自己去问，是同一次诊断被送了两次。
```

```kotlin [Kotlin Multiplatform]
import io.github.orangeboychen.marsrs.sdt.CheckMode
import io.github.orangeboychen.marsrs.sdt.Link
import io.github.orangeboychen.marsrs.sdt.ProbeAnswer
import io.github.orangeboychen.marsrs.sdt.SdtLogic

// 1. 两条链路的 hosts，以及 mode：ping 和 dns，然后 tcp
val longLink = arrayOf(Link("default", arrayOf("1.2.3.4"), intArrayOf(80)))
SdtLogic.setHttpNetcheckCGI("http://example.com/netcheck")
SdtLogic.startActiveCheck(longLink, emptyArray(), CheckMode.K_BASIC or CheckMode.K_LONG, 10_000)

// 2. 计划，跑在 `probe` 回答的那个网络上
SdtLogic.runChecks(
    1,
    object : SdtLogic.IProbe {
        override fun dns(host: String, timeoutMs: Int) =
            ProbeAnswer.Dns(errorCode = 0, rtt = 12, addresses = listOf("1.2.3.4"))
        override fun tcp(host: String, port: Int, timeoutMs: Int) =
            ProbeAnswer.Tcp(sent = 0, received = 0, isNoopResponse = true, rtt = 30)
        override fun http(url: String, timeoutMs: Int) =
            ProbeAnswer.Http(errorCode = 0, statusCode = 200, rtt = 40)
        override fun ping(host: String, timeoutSec: Int) =
            ProbeAnswer.Ping(errorCode = 0, rtt = 20, lossRate = 0f, averageRTT = 18f)
    },
)

// 3. 报告，是自己来要的而不是被递过来的：这个例子没装回调，`takeReport()` 才是有它的
//    那个 —— 一份文档，一次
SdtLogic.takeReport()?.let { send(it) }
```

```c [C]
#include <mars_sdt.h>

/* 1. 两条链路的 hosts */
MarsSdtIpPort port = { "1.2.3.4", 80 };
MarsSdtHosts longlink[] = { { "default", &port, 1 } };
mars_sdt_set_http_netcheck_cgi("http://example.com/netcheck");
/* 1 | 2 是 NET_CHECK_BASIC | NET_CHECK_LONG：ping 和 dns，然后 tcp */
mars_sdt_start_active_check(longlink, 1, NULL, 0, 1 | 2, 10000);

/* 2. 计划，跑在 `probe` 回答的那个网络上 */
static void probe(void *ctx, const MarsSdtQuery *q, MarsSdtAnswer *a) {
    switch (q->kind) {
    case MarsSdtDns:  a->kind = MarsSdtDns; a->error_code = 0; a->rtt = 12; break;
    case MarsSdtTcp:  a->kind = MarsSdtTcp; a->sent = 0; a->received = 0; a->is_noop_resp = 1; break;
    case MarsSdtHttp: a->kind = MarsSdtHttp; a->error_code = 0; a->status_code = 200; break;
    case MarsSdtPing: a->kind = MarsSdtPing; a->error_code = 0; a->rtt = 20; break;
    default:          a->kind = MarsSdtNothing; break;
    }
}
mars_sdt_run_checks(NULL, probe, 1);

/* 3. 报告 */
char buffer[4096];
if (mars_sdt_take_report(buffer, sizeof buffer) >= 0) { send(buffer); }
```

:::

## mode 和计划

`start_active_check` 接一个 **mode**，它是要跑哪些检查的位集合，并把它变成一个你在跑之前
就能读的计划：

| 哪一项检查 | Rust | Swift | 共享 Kotlin | Android | C |
|---|---|---|---|---|---|
| ping | `NetCheckType::PingCheck` | `.ping` | `Check.Ping` | `0` | `MarsSdtCheckPing` |
| dns | `NetCheckType::DnsCheck` | `.dns` | `Check.Dns` | `1` | `MarsSdtCheckDns` |
| 换一个服务器再查一次 dns，用来对比 | `NetCheckType::NewDnsCheck` | `.newDns` | `Check.NewDns` | `2` | `MarsSdtCheckNewDns` |
| tcp | `NetCheckType::TcpCheck` | `.tcp` | `Check.Tcp` | `3` | `MarsSdtCheckTcp` |
| http | `NetCheckType::HttpCheck` | `.http` | `Check.Http` | `4` | `MarsSdtCheckHttp` |
| traceroute —— 有计划，还没有为它问探针 | `NetCheckType::TracerouteCheck` | `.traceroute` | `Check.Traceroute` | `5` | `MarsSdtCheckTraceroute` |
| 请求自己的 buffer —— 同上 | `NetCheckType::ReqBufCheck` | `.reqBuf` | `Check.ReqBuf` | `6` | `MarsSdtCheckReqBuf` |

`sdt.plan()`（Rust）、`MarsSdt.plan`（Swift）、`SdtLogic.plan()`（Kotlin）和 `mars_sdt_plan`
（C）按检查将要跑的顺序把计划交回来 —— 共享 Kotlin 是 `Check`，Android 是那些整数本身 ——
而计划里的整数就是报告每一行里的 `detectType`，所以两边用的是同一套词。

构成 mode 的那些位：

| 哪一位 | Rust | 共享 Kotlin、Android | Swift、C | 它往计划里放什么 |
|---|---|---|---|---|
| `NET_CHECK_BASIC` | `NET_CHECK_BASIC` | `CheckMode.K_BASIC` | `1` | 一次 ping 和一次 dns 检查 —— C++ 开头的那两项 |
| `NET_CHECK_LONG` | `NET_CHECK_LONG` | `K_LONG` | `2` | 一次 tcp 检查：往长连 hosts 发一个 noop |
| `NET_CHECK_SHORT` | `NET_CHECK_SHORT` | `K_SHORT` | `4` | 一次 http 检查：net-check CGI，以及短连的 hosts |

它们按位或在一起 —— `NET_CHECK_BASIC | NET_CHECK_LONG` 是三项检查的一次跑 —— 而 `0` 是
**一项都不查**：计划是空的，跑的时候一个探针都不问，报告是 `{"details":[]}`。它不是“全查”，
全查是 `1 | 2 | 4`。`NET_CHECK_SHORT` 是唯一需要自己 hosts 的那一位：这一位配上空的
`shortLink`，是一次没有东西可查的 http 检查。

## 报告

每项检查写一个 `CheckResultProfile`，报告就是它们组成的 JSON —— `{"details":[ … ]}`，
一项检查一个对象：

```json
{
  "details": [
    {
      "detectType": 1, "errorCode": 0, "networkType": 1,
      "detectIP": "1.2.3.4", "port": 80, "conntime": 30, "rtt": 42,
      "rttStr": "42ms", "httpStatusCode": 0, "pingCheckCount": 0,
      "pingLossRate": "", "dnsDomain": "example.com", "localDns": "",
      "dnsIP1": "1.2.3.4", "dnsIP2": ""
    }
  ]
}
```

| 什么 | 字段 | 它是什么 |
|---|---|---|
| 哪一项检查 | `detectType` | 上面那些整数之一 |
| 探针跑得怎么样 | `errorCode` | `0` 及以上是跑通了的 |
| 它跑在什么网络上 | `networkType` | 你交给 run 的那个 `comm::getNetInfo()` |
| 探的是什么 | `detectIP`、`port`、`dnsDomain` | host，tcp 检查还有端口 |
| 耗时 | `conntime`、`rtt`、`rttStr` | 毫秒；`rttStr` 是 `rtt` 的字符串形式 |
| HTTP 检查 | `httpStatusCode` | net-check CGI 回答的状态码 |
| ping | `pingCheckCount`、`pingLossRate` | 发出去多少个，丢了多少比例 |
| dns | `localDns`、`dnsIP1`、`dnsIP2` | 用的解析器，以及前两个地址 |

`mars_sdt_take_report` 和 `MarsSdt.takeReport()` 在报告装不进 buffer 时回答
`MARS_SDT_ERR_NO_SPACE`，并且**保留结果** —— 所以换一个更大的 buffer 再问一次的调用方，
拿到的是诊断结果而不是一份空的。Swift 的 `takeReport()` 替你做了这件事：从 4 KB 开始，
加倍到 1 MB，只有在没有任何东西可取时才回答 `nil`。Android 和共享 Kotlin 里报告是
`String?`，所以没有 buffer 要估大小。

## 取消，和重来

| 什么 | Rust | Swift | Kotlin | C |
|---|---|---|---|---|
| 停掉这次运行 | `cancel_active_check` / `cancel_handle` | `cancelActiveCheck` | `cancelActiveCheck` | `mars_sdt_cancel_active_check` |
| 有没有一次在跑 | `is_checking` | `isChecking` | `isChecking` | `mars_sdt_is_checking` |
| 全部丢掉 | `SdtCore::reset` | `reset` | `reset` | `mars_sdt_reset` |

Rust 里一次运行会独占借用这个 logic，所以 `run_checks` 还在栈上时调不了
`cancel_active_check`：先用 `cancel_handle()` 拿一个 `CancelHandle`，交给那个必须停掉这次
运行的人 —— 也就是握着那个必须放弃的 socket 的探针闭包。

其余每个地方都不需要 handle：它设的是运行自己读的那个标志，而且不拿运行持有的那把锁，
所以它在探针还在被问的时候就能从另一个线程落到那里 —— 而这正是取消唯一有意义的时刻。
它做的是让计划里剩下的部分不再跑；已经发出去的那个探针不会被打断。

另外，同时只能跑一次：诊断是一个进程级的值，所以第二次 `runChecks` 会等第一次，而不是
跟它并排跑 —— Android 和共享 Kotlin 上，第二次调用要等第一次结束才返回。

## 接着看

- [任务链路](/zh/stn) —— 某个 host 不再回答时调用 SDT 的那半，tcp 检查 noop 的正是它的
  长连接。
- [你那个平台](/zh/platforms/rust) —— 完整的 API 面，xlog 也在里面。
