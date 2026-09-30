# 快速开始

SDT 回答一个问题：*这台手机为什么连不上服务器？* 它拿两条链路的 hosts，对它们跑
**ping**、**DNS**、**TCP** 和 **HTTP** 四项检查，然后把 JSON 报告交给 App，报告里
记着每一项查到了什么。STN 是 App 一直在跑的东西，SDT 是它想知道 STN 为什么不工作
的时候才跑的。

## 它在哪里

| 你的 App 是 | 谁带着 SDT | 怎么拿到它 |
|---|---|---|
| Rust | `marsrs`（`marsrs-xlog` 里一点都没有） | `marsrs::sdt` |
| iOS / watchOS，Swift 或 Objective-C | `MarsRSNet` 这个 product 或 pod，或 `MarsRS` | `MarsSdt` |
| Android，Kotlin 或 Java | JitPack 上的 `marsrs`，不是 `xlog` | `io.github.orangeboychen.marsrs.sdt.SdtLogic` |
| Kotlin Multiplatform | `marsrs-kmp`，不是 `xlog-kmp` | `io.github.orangeboychen.marsrs.sdt.SdtLogic` |
| 有 C FFI 的任何东西 | `include/mars_sdt.h` | `mars_sdt_*` |
| HarmonyOS | 同一个 `libmars_ffi.so` | `mars_sdt_*`，通过你自己写的 NAPI shim |

Flutter 插件和 React Native 模块目前都只有日志：两个都起不了一次检查。

## 三个调用

1. **开启**一次诊断，给它两条链路的 hosts、一个 mode 和一个超时。这时还没有探测任
   何东西 —— 这个调用只是写下计划，[检查项](/zh/sdt/checks)那页讲的是这个 mode 会
   变成什么。
2. **跑**这个计划，你的探测是在这里问的 —— 一项检查一个，按顺序，在调用它的那个
   线程上。见[探测](/zh/sdt/probes)。
3. **取报告**，把它送到你送日志的地方 —— 或者装一个回调，让这一趟跑完直接递给你。
   见[报告](/zh/sdt/report)。

在 Rust 里这三步是一个调用 —— `sdt.diagnose(…)` —— 而且 mode 有名字。

## Rust

```bash
# 这个 crate 还没上 crates.io —— 发布还在进行中 —— 所以 Rust 应用现在从
# tag 上取：只写 `cargo add marsrs` 是解析不到东西的。
cargo add marsrs --git https://github.com/orangeboyChen/mars-rs --tag v0.1.0-alpha.3
```

```rust
use marsrs::sdt::checkimpl::{Answer, Ask, Query};
use marsrs::sdt::{report_json, CheckIPPort, CheckIPPorts, Mode, SdtLogic};

let mut sdt = SdtLogic::new();
sdt.set_http_netcheck_cgi("http://example.com/netcheck");

// 1. 两条链路的 hosts，按它们被叫的名字分组
let mut longlink = CheckIPPorts::new();
longlink.insert("default".to_owned(), vec![CheckIPPort::new("1.2.3.4", 80)]);
let shortlink = CheckIPPorts::new();

// 2. 计划，跑在 `ask` 回答的那个网络之上
let mut ask = Ask::new(|query| match query {
    Query::Dns { domain, .. } => Answer::Dns {
        error_code: 0, rtt: 12, ips: vec!["1.2.3.4".to_owned()],
        local_dns: String::new(),
    },
    Query::Tcp { .. } => Answer::Tcp {
        sent: 0, received: 0, is_noop_resp: true, conntime: 0, rtt: 30,
    },
    Query::Http { .. } => Answer::Http { error_code: 0, status_code: 200, rtt: 40 },
    Query::Ping { .. } => Answer::Ping { error_code: 0, rtt: 20, status: None },
});

// 3. 一整趟诊断：先 ping 和 dns，再 tcp，跑在 `1` 说的那个网络之上 —— 也就是 C++
//    里的 `comm::getNetInfo()`。`None` 是这一趟没启动：已经有一趟在跑，或者 mode
//    三个 bit 一个都没有 —— `is_checking()` 能把这两种分开。
let results = sdt
    .diagnose(
        &longlink, &shortlink, Mode::BASIC | Mode::LONG, 10_000, &mut ask, 1,
    )
    .expect("没有正在跑的检查");

// 4. 报告
println!("{}", report_json(&results));
```

`sdt.plan()` 在这一趟问任何东西之前，按检查要跑的顺序把计划交回来。在 Rust 里一趟
会独占借用这个 logic，所以取消一趟要用 `CancelHandle`，而且得在这一趟之前就造好 ——
见[报告](/zh/sdt/report)。

`Mode` 是那几个 `NET_CHECK_*` bit 有了名字 —— `Mode::NONE`、`BASIC`、`LONG`、
`SHORT` 和 `ALL` —— 用 `|` 拼起来；`Mode::of(bits)` 和 `mode.bits()` 是给还在数数
的 App 留的。`start_active_check` 依然吃那个裸的 `i32`，也没有带 deprecated：一个
要跨线程驱动这趟跑的宿主 —— 在一个线程上开检查，在另一个线程上取报告 —— 还是要把
这三个调用分开用。

`diagnose` 不是一个 future：每一项检查都是一次探测，在问它的那个线程上跑到自己的
超时。想让这趟诊断离开当前线程的 App 自己把它挪过去 —— 在这个调用外面套一层
`std::thread::spawn`，或者用 executor 自己的 `spawn_blocking`。它交回来的结果也照样
递给 App 用 `set_callback` 装的那个回调，跟三个调用分开用的时候一样。

## Swift

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0-alpha.3")

// 在要用的 target 里：
.product(name: "MarsRSNet", package: "mars-rs")   // 要两半都有就 MarsRS
```

```swift
import MarsRSNet

// 1. 两条链路的 hosts
let longLink = [MarsSdt.Link(
    name: "default",
    ports: [MarsSdt.HostPort(host: "1.2.3.4", port: 80)]
)]
MarsSdt.setHTTPNetCheckCGI("http://example.com/netcheck")
// [.basic, .long] 是先 ping 和 dns，再 tcp
MarsSdt.startActiveCheck(longLink: longLink, shortLink: [], mode: [.basic, .long], timeout: 10_000)

// 2. 计划，跑在 `probe` 回答的那个网络之上
MarsSdt.runChecks(networkType: 1) { query in
    switch query.probe {
    case .dns:   return .dns(errorCode: 0, rtt: 12, addresses: ["1.2.3.4"])
    case .tcp:   return .tcp(errorCode: 0, rtt: 30, noop: MarsSdt.Noop(sent: 0, received: 0, isNoopResponse: true))
    case .http:  return .http(errorCode: 0, rtt: 40, statusCode: 200)
    case .ping:  return .ping(errorCode: 0, rtt: 20, lossRate: 0, averageRTT: 18)
    default:     return .nothing()
    }
}

// 3. 报告
if let report = MarsSdt.takeReport() { send(report) }
```

`MarsSdt.takeReport()` 的 buffer 从 4 KB 起、翻倍到 1 MB，所以取报告的 App 问一次
就行，不用估大小。

`MarsSdt` 是同一份 framework 上的类，所以 Objective-C 写的 App 也是从
`MarsRSNet-Swift.h` 跑这三个调用：probe 是一个 block，五种回答是 `MarsSdtResult`
的五个类方法。它摸不到的只有 `MarsSdt.plan` —— 枚举数组没有对应的 Objective-C
类型，所以 [计划](/zh/sdt/checks) 仍归 Swift。

## Android

```kotlin
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen.marsrs:marsrs:0.1.0-alpha.3")   // 只有 xlog 的那个没有
```

```kotlin
import io.github.orangeboychen.marsrs.sdt.SdtLogic

// 0. 报告去哪里
SdtLogic.setCallBack(object : SdtLogic.ICallBack {
    override fun reportSignalDetectResults(resultsJson: String?) { send(resultsJson) }
})

// 1. 两条链路的 hosts，以及 mode：先 ping 和 dns，再 tcp
val longLink = arrayOf(SdtLogic.Link("default", arrayOf("1.2.3.4"), intArrayOf(80)))
SdtLogic.setHttpNetcheckCGI("http://example.com/netcheck")
SdtLogic.startActiveCheck(
    longLink,
    emptyArray(),
    SdtLogic.CheckMode.K_BASIC or SdtLogic.CheckMode.K_LONG,
    10_000,
)

// 2. 计划，跑在 `probe` 回答的那个网络之上
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

// 3. 没有了：`runChecks` 已经把报告交给上面那个回调。
//    `takeReport()` 是拿到它的另一条路 —— 给不装回调的 App ——
//    又要又要的话，同一份会被送两次。
```

## Kotlin Multiplatform

```kotlin
// 共享模块的 build.gradle.kts
implementation("io.github.orangeboychen.marsrs:marsrs-kmp:0.1.0-alpha.3")   // 不是 xlog-kmp
```

```kotlin
import io.github.orangeboychen.marsrs.sdt.CheckMode
import io.github.orangeboychen.marsrs.sdt.Link
import io.github.orangeboychen.marsrs.sdt.ProbeAnswer
import io.github.orangeboychen.marsrs.sdt.SdtLogic

// 1. 两条链路的 hosts，以及 mode：先 ping 和 dns，再 tcp
val longLink = arrayOf(Link("default", arrayOf("1.2.3.4"), intArrayOf(80)))
SdtLogic.setHttpNetcheckCGI("http://example.com/netcheck")
SdtLogic.startActiveCheck(longLink, emptyArray(), CheckMode.K_BASIC or CheckMode.K_LONG, 10_000)

// 2. 计划，跑在 `probe` 回答的那个网络之上
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

// 3. 报告，是问出来的而不是递过来的：这里没装回调，所以 `takeReport()` 才有它 ——
//    一份文档，一次
SdtLogic.takeReport()?.let { send(it) }
```

## The C ABI {#c-abi}

```text
marsrs-<version>-<host>.tar.gz   （Linux、macOS）
marsrs-<version>-<host>.zip      （Windows）
    include/mars_sdt.h      网络诊断
    libmars_ffi.a / libmars_ffi.so（.dylib、.dll）
```

```c
#include <mars_sdt.h>

/* 1. 两条链路的 hosts */
MarsSdtIpPort port = { "1.2.3.4", 80 };
MarsSdtHosts longlink[] = { { "default", &port, 1 } };
mars_sdt_set_http_netcheck_cgi("http://example.com/netcheck");
mars_sdt_start_active_check(longlink, 1, NULL, 0, NET_CHECK_BASIC | NET_CHECK_LONG, 10000);

/* 2. 计划，跑在 `probe` 回答的那个网络之上 */
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

## HarmonyOS

拿 `marsrs-harmonyos-xlog` 的 App 拿到的是日志：那个包的 ArkTS 没有伸到 SDT。带着
它的是同一个 `libmars_ffi.so`，在默认的 `xlog` 之上开了 `sdt` feature —— 所以要跑
诊断的 App 取 `marsrs-harmony-<version>.tar.gz`，把 `libmars_ffi.so` 放进模块的
`libs/<abi>/`，再写那个伸到 `mars_sdt.h` 的 NAPI shim。它就是[那一节](#c-abi)为
Linux、macOS 和 Windows 发布的那个 C ABI。

## 接着看

- [检查项](/zh/sdt/checks) —— 那个 mode，以及在探测任何东西之前它变成的计划。
- [探测](/zh/sdt/probes) —— 那四项检查，以及每一项要 App 给什么。
- [报告](/zh/sdt/report) —— 一趟跑出来的 JSON，以及怎么停下一趟。
