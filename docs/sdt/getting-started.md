# Getting started

SDT answers one question: *why can this phone not reach the server?* It takes the
hosts of the two links and runs **ping**, **DNS**, **TCP** and **HTTP** checks
against them, and hands the app a JSON report of what each one recorded. Where
STN is what an app runs all the time, SDT is what it runs when it wants to know
why STN is not working.

## Where it is

| your app is | what carries SDT | how you reach it |
|---|---|---|
| Rust | `marsrs` (`marsrs-xlog` has none of it) | `marsrs::sdt` |
| iOS / watchOS, Swift or Objective-C | the `MarsRSNet` product or pod, or `MarsRS` | `MarsSdt` |
| Android, Kotlin or Java | `marsrs` on JitPack, not `xlog` | `io.github.orangeboychen.marsrs.sdt.SdtLogic` |
| Kotlin Multiplatform | `marsrs-kmp`, not `xlog-kmp` | `io.github.orangeboychen.marsrs.sdt.SdtLogic` |
| anything with a C FFI | `include/mars_sdt.h` | `mars_sdt_*` |
| HarmonyOS | the same `libmars_ffi.so` | `mars_sdt_*` through a NAPI shim of your own |

The Flutter plugin and the React Native module are the logger and nothing else
today: neither can start a check.

## Three calls

1. **Start** the diagnosis with the hosts of the two links, a mode and a timeout.
   Nothing is probed yet — the call only writes the plan, and
   [the checks page](/sdt/checks) is what the mode turns into.
2. **Run** the plan, which is where your probes are asked — one per check, in
   order, on the calling thread. See [the probes](/sdt/probes).
3. **Take the report**, and send it wherever your logs go — or install a callback
   and let the run hand it to you instead. See [the report](/sdt/report).

In Rust the three are one call — `sdt.diagnose(…)` — and the mode has names.

## Rust

```bash
# The crate is not on crates.io yet — publication is pending — so a Rust app
# takes it off the tag: `cargo add marsrs` on its own resolves nothing.
cargo add marsrs --git https://github.com/orangeboyChen/mars-rs --tag v0.1.0-alpha.3
```

```rust
use marsrs::sdt::checkimpl::{Answer, Ask, Query};
use marsrs::sdt::{report_json, CheckIPPort, CheckIPPorts, Mode, SdtLogic};

let mut sdt = SdtLogic::new();
sdt.set_http_netcheck_cgi("http://example.com/netcheck");

// 1. the hosts of the two links, keyed by the name they are known under
let mut longlink = CheckIPPorts::new();
longlink.insert("default".to_owned(), vec![CheckIPPort::new("1.2.3.4", 80)]);
let shortlink = CheckIPPorts::new();

// 2. the plan, over the network `ask` answers with
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

// 3. the whole diagnosis: ping and dns, then tcp, over the network `1` names,
//    which is what `comm::getNetInfo()` is in the C++. `None` is a check that
//    is already in flight.
let results = sdt
    .diagnose(
        &longlink, &shortlink, Mode::BASIC | Mode::LONG, 10_000, &mut ask, 1,
    )
    .expect("no check was in flight");

// 4. the report
println!("{}", report_json(&results));
```

`sdt.plan()` hands the plan back in the order the checks will run, before the run
asks anything. A run borrows the logic exclusively in Rust, so cancelling one
takes a `CancelHandle` made before it — see [the report](/sdt/report).

`Mode` is the `NET_CHECK_*` bits with names — `Mode::NONE`, `BASIC`, `LONG`,
`SHORT` and `ALL` — and `|` puts them together; `Mode::of(bits)` and
`mode.bits()` are there for an app that counts. `start_active_check` still takes
the raw `i32` and carries no deprecation: a host that drives the run across
threads — starting the check on one and reporting it on another — needs the
three calls apart.

`diagnose` is not a future: every check is a probe that runs to its own timeout
on the thread that asks for it. An app that wants the diagnosis off that thread
puts it there — `std::thread::spawn` around the call, or an executor's own
`spawn_blocking`. What it hands back is also handed to the callback an app set
with `set_callback`, the way it is when the three calls are made apart.

## Swift

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0-alpha.3")

// and, in the target that takes it:
.product(name: "MarsRSNet", package: "mars-rs")   // or MarsRS, for both halves
```

```swift
import MarsRSNet

// 1. the hosts of the two links
let longLink = [MarsSdt.Link(
    name: "default",
    ports: [MarsSdt.HostPort(host: "1.2.3.4", port: 80)]
)]
MarsSdt.setHTTPNetCheckCGI("http://example.com/netcheck")
// [.basic, .long] is ping and dns, then tcp
MarsSdt.startActiveCheck(longLink: longLink, shortLink: [], mode: [.basic, .long], timeout: 10_000)

// 2. the plan, over the network `probe` answers with
MarsSdt.runChecks(networkType: 1) { query in
    switch query.probe {
    case .dns:   return .dns(errorCode: 0, rtt: 12, addresses: ["1.2.3.4"])
    case .tcp:   return .tcp(errorCode: 0, rtt: 30, noop: MarsSdt.Noop(sent: 0, received: 0, isNoopResponse: true))
    case .http:  return .http(errorCode: 0, rtt: 40, statusCode: 200)
    case .ping:  return .ping(errorCode: 0, rtt: 20, lossRate: 0, averageRTT: 18)
    default:     return .nothing()
    }
}

// 3. the report
if let report = MarsSdt.takeReport() { send(report) }
```

`MarsSdt.takeReport()` starts its buffer at 4 KB and doubles up to 1 MB, so an
app that takes the report asks for it once and does not size anything.

`MarsSdt` is a class over the same framework, so an app written in Objective-C
runs the same three calls out of `MarsRSNet-Swift.h`: the probe is a block, and
the five answers are the five class methods of `MarsSdtResult`. The one thing it
does not reach is `MarsSdt.plan` — an array of enums has no Objective-C type,
so [the plan](/sdt/checks) stays Swift's.

## Android

```kotlin
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen.marsrs:marsrs:0.1.0-alpha.3")   // xlog alone has none of it
```

```kotlin
import io.github.orangeboychen.marsrs.sdt.SdtLogic

// 0. where the report goes
SdtLogic.setCallBack(object : SdtLogic.ICallBack {
    override fun reportSignalDetectResults(resultsJson: String?) { send(resultsJson) }
})

// 1. the hosts of the two links, and the mode: ping and dns, then tcp
val longLink = arrayOf(SdtLogic.Link("default", arrayOf("1.2.3.4"), intArrayOf(80)))
SdtLogic.setHttpNetcheckCGI("http://example.com/netcheck")
SdtLogic.startActiveCheck(
    longLink,
    emptyArray(),
    SdtLogic.CheckMode.K_BASIC or SdtLogic.CheckMode.K_LONG,
    10_000,
)

// 2. the plan, over the network `probe` answers with
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

// 3. nothing: `runChecks` has already handed the report to the callback above.
//    `takeReport()` is the other way to get it — for an app that installs no
//    callback — and asking for it as well as being told is being sent it twice.
```

## Kotlin Multiplatform

```kotlin
// build.gradle.kts of the shared module
implementation("io.github.orangeboychen.marsrs:marsrs-kmp:0.1.0-alpha.3")   // not xlog-kmp
```

```kotlin
import io.github.orangeboychen.marsrs.sdt.CheckMode
import io.github.orangeboychen.marsrs.sdt.Link
import io.github.orangeboychen.marsrs.sdt.ProbeAnswer
import io.github.orangeboychen.marsrs.sdt.SdtLogic

// 1. the hosts of the two links, and the mode: ping and dns, then tcp
val longLink = arrayOf(Link("default", arrayOf("1.2.3.4"), intArrayOf(80)))
SdtLogic.setHttpNetcheckCGI("http://example.com/netcheck")
SdtLogic.startActiveCheck(longLink, emptyArray(), CheckMode.K_BASIC or CheckMode.K_LONG, 10_000)

// 2. the plan, over the network `probe` answers with
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

// 3. the report, asked for rather than handed over: this one installs no
//    callback, so `takeReport()` is what has it — one document, once
SdtLogic.takeReport()?.let { send(it) }
```

## The C ABI {#c-abi}

```text
marsrs-<version>-<host>.tar.gz   (Linux, macOS)
marsrs-<version>-<host>.zip      (Windows)
    include/mars_sdt.h      the network diagnosis
    libmars_ffi.a / libmars_ffi.so (.dylib, .dll)
```

```c
#include <mars_sdt.h>

/* 1. the hosts of the two links */
MarsSdtIpPort port = { "1.2.3.4", 80 };
MarsSdtHosts longlink[] = { { "default", &port, 1 } };
mars_sdt_set_http_netcheck_cgi("http://example.com/netcheck");
mars_sdt_start_active_check(longlink, 1, NULL, 0, NET_CHECK_BASIC | NET_CHECK_LONG, 10000);

/* 2. the plan, over the network `probe` answers with */
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

/* 3. the report */
char buffer[4096];
if (mars_sdt_take_report(buffer, sizeof buffer) >= 0) { send(buffer); }
```

## HarmonyOS

An app that takes `marsrs-harmonyos-xlog` gets the logger: no ArkTS of that
package reaches SDT. What does carry it is the same `libmars_ffi.so`, built with
the `sdt` feature on top of the default `xlog` — so an app that wants a diagnosis
takes `marsrs-harmony-<version>.tar.gz`, drops `libmars_ffi.so` under the
module's `libs/<abi>/`, and writes the NAPI shim that reaches `mars_sdt.h`. It is
the same C ABI [its section](#c-abi) publishes for Linux, macOS and Windows.

## Where to go next

- [The checks](/sdt/checks) — the mode, and the plan it turns into before
  anything is probed.
- [The probes](/sdt/probes) — the four checks, and what each one asks the app
  for.
- [The report](/sdt/report) — the JSON of a run, and how to stop one.
