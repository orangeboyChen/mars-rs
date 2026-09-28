# The network diagnosis (SDT)

SDT ("smart diagnosis tool") answers one question: *why can this phone not reach
the server?* It takes the hosts of the two links and runs **ping**, **DNS**,
**TCP** and **HTTP** checks against them, and hands the app a JSON report of what
each one recorded. Where STN is what an app runs all the time, SDT is what it
runs when it wants to know why STN is not working.

## Where it is

| your app is | what carries SDT | how you reach it |
|---|---|---|
| Rust | `marsrs` (`marsrs-xlog` has none of it) | [`marsrs::sdt`](https://docs.rs/marsrs) — see [Rust](/platforms/rust#the-network-diagnosis) |
| Swift | the `MarsRSNet` product, or `MarsRS` | `MarsSdt` — see [SwiftPM](/platforms/swift#the-network-diagnosis) |
| Android, Kotlin or Java | `marsrs` on JitPack, not `xlog` | `io.github.orangeboychen.marsrs.sdt.SdtLogic` — see [Android](/platforms/android#the-network-diagnosis) |
| Kotlin Multiplatform | `marsrs-kmp`, not `xlog-kmp` | `io.github.orangeboychen.marsrs.sdt.SdtLogic` — see [Kotlin Multiplatform](/platforms/kotlin-multiplatform#the-network-diagnosis) |
| anything with a C FFI | `include/mars_sdt.h` | `mars_sdt_*` — see [the C ABI](/platforms/c-abi#the-network-diagnosis) |
| HarmonyOS | the same `libmars_ffi.so` | `mars_sdt_*` through a NAPI shim of your own — see [HarmonyOS](/platforms/harmonyos#the-mars-half) |

The Flutter plugin and the React Native module are the logger and nothing else
today: neither can start a check, and neither will until the Dart and the
TypeScript below them do — see [Flutter](/platforms/flutter#what-is-not-in-it)
and [React Native](/platforms/react-native#what-is-not-in-it).

## The probes are yours

This is the one thing to know before the rest of it. A check is a **socket** — a
resolve, a connect, an HTTP request, an ICMP echo — and this port owns none.
The C++ links `dnsquery.cc` and the rest straight into the checkers, so a
diagnosis there can only ever run against the platform's own sockets. Here the
app hands the network in, as one closure that answers one probe at a time:

| the check | what it asks for | what you answer |
|---|---|---|
| ping | the host, a timeout in **seconds** | how long it took, and what share of the pings were lost |
| dns | the domain, a timeout in ms | the addresses it resolved to |
| tcp | the ip and port, a timeout in ms | whether the noop went out, what came back, and whether it was the answer to it |
| http | the URL of the net-check CGI | the HTTP status and how long the request took |

Which is what makes a diagnosis **testable**, and what lets a host that is not a
phone run one at all: the same `Ask` that answers on a device answers from a
test, or from a machine with no radio.

## Three calls

**1. Start** the diagnosis with the hosts of the two links, a mode and a
timeout. Nothing is probed yet: the call only writes the plan.

**2. Run** the plan, which is where your probes are asked — one per check, in
order, on the calling thread. This port has no threads, so it is a call the host
makes.

**3. Take the report**, and send it wherever your logs go — or install an
`ICallBack` and let the run hand it to you instead. The two carry the same
document, so an app uses one of them: on Android and in the shared Kotlin a run
hands the report to the callback *and* keeps it for a `takeReport()` that comes
later, so an app that does both sends the same diagnosis twice. Taking it empties
it: the next call reports what happened since.

::: code-group

```rust [Rust]
use marsrs::sdt::checkimpl::{Answer, Ask, Query};
use marsrs::sdt::{report_json, CheckIPPort, CheckIPPorts, SdtLogic, NET_CHECK_BASIC, NET_CHECK_LONG};

let mut sdt = SdtLogic::new();
sdt.set_http_netcheck_cgi("http://example.com/netcheck");

// 1. the hosts of the two links, keyed by the name they are known under
let mut longlink = CheckIPPorts::new();
longlink.insert("default".to_owned(), vec![CheckIPPort::new("1.2.3.4", 80)]);
let shortlink = CheckIPPorts::new();
// the mode is the checks to run: ping and dns, then tcp. `0` is none of them.
sdt.start_active_check(&longlink, &shortlink, NET_CHECK_BASIC | NET_CHECK_LONG, 10_000);

// 2. the plan, over the network `ask` answers with
let mut ask = Ask::new(|query| match query {
    Query::Dns { domain, .. } => Answer::Dns {
        error_code: 0, rtt: 12, ips: vec!["1.2.3.4".to_owned()],
    },
    Query::Tcp { .. } => Answer::Tcp { sent: 0, received: 0, is_noop_resp: true, rtt: 30 },
    Query::Http { .. } => Answer::Http { error_code: 0, status_code: 200, rtt: 40 },
    Query::Ping { .. } => Answer::Ping { error_code: 0, rtt: 20, status: None },
});
let results = sdt.run_checks(&mut ask, 1 /* comm::getNetInfo() */);

// 3. the report
println!("{}", report_json(&results));
```

```swift [Swift]
import MarsRSNet

// 1. the hosts of the two links
let longLink = [MarsSdt.Link(
    name: "default",
    ports: [MarsSdt.HostPort(host: "1.2.3.4", port: 80)]
)]
MarsSdt.setHTTPNetCheckCGI("http://example.com/netcheck")
// 1 | 2 is NET_CHECK_BASIC | NET_CHECK_LONG: ping and dns, then tcp
MarsSdt.startActiveCheck(longLink: longLink, shortLink: [], mode: 1 | 2, timeout: 10_000)

// 2. the plan, over the network `probe` answers with
MarsSdt.runChecks(networkType: 1) { query in
    switch query.probe {
    case .dns:   return .dns(errorCode: 0, rtt: 12, addresses: ["1.2.3.4"])
    case .tcp:   return .tcp(errorCode: 0, rtt: 30, noop: MarsSdt.Noop(sent: 0, received: 0, isNoopResponse: true))
    case .http:  return .http(errorCode: 0, rtt: 40, statusCode: 200)
    case .ping:  return .ping(errorCode: 0, rtt: 20, lossRate: 0, averageRTT: 18)
    default:     return .nothing
    }
}

// 3. the report
if let report = MarsSdt.takeReport() { send(report) }
```

```kotlin [Android]
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

```kotlin [Kotlin Multiplatform]
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

```c [C]
#include <mars_sdt.h>

/* 1. the hosts of the two links */
MarsSdtIpPort port = { "1.2.3.4", 80 };
MarsSdtHosts longlink[] = { { "default", &port, 1 } };
mars_sdt_set_http_netcheck_cgi("http://example.com/netcheck");
/* 1 | 2 is NET_CHECK_BASIC | NET_CHECK_LONG: ping and dns, then tcp */
mars_sdt_start_active_check(longlink, 1, NULL, 0, 1 | 2, 10000);

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

:::

## The mode and the plan

`start_active_check` takes a **mode**, which is a bit set of the checks to run,
and turns it into a plan you can read before you run it:

| the check | Rust | Swift | shared Kotlin | Android | C |
|---|---|---|---|---|---|
| ping | `NetCheckType::PingCheck` | `.ping` | `Check.Ping` | `0` | `MarsSdtCheckPing` |
| dns | `NetCheckType::DnsCheck` | `.dns` | `Check.Dns` | `1` | `MarsSdtCheckDns` |
| dns against another server, to compare with | `NetCheckType::NewDnsCheck` | `.newDns` | `Check.NewDns` | `2` | `MarsSdtCheckNewDns` |
| tcp | `NetCheckType::TcpCheck` | `.tcp` | `Check.Tcp` | `3` | `MarsSdtCheckTcp` |
| http | `NetCheckType::HttpCheck` | `.http` | `Check.Http` | `4` | `MarsSdtCheckHttp` |
| traceroute — planned, no probe asked for it yet | `NetCheckType::TracerouteCheck` | `.traceroute` | `Check.Traceroute` | `5` | `MarsSdtCheckTraceroute` |
| the request's own buffer — same | `NetCheckType::ReqBufCheck` | `.reqBuf` | `Check.ReqBuf` | `6` | `MarsSdtCheckReqBuf` |

`sdt.plan()` (Rust), `MarsSdt.plan` (Swift), `SdtLogic.plan()` (Kotlin) and
`mars_sdt_plan` (C) hand the plan back in the order the checks will run — the
shared Kotlin as `Check`s and Android as the integers themselves — and the
integers of the plan are the `detectType` of every entry of the report, so one
vocabulary names both.

The bits the mode is made of:

| the bit | Rust | shared Kotlin, Android | Swift, C | what it puts in the plan |
|---|---|---|---|---|
| `NET_CHECK_BASIC` | `NET_CHECK_BASIC` | `CheckMode.K_BASIC` | `1` | a ping and a dns check — the two the C++ starts with |
| `NET_CHECK_LONG` | `NET_CHECK_LONG` | `K_LONG` | `2` | a tcp check: a noop out to the long link's hosts |
| `NET_CHECK_SHORT` | `NET_CHECK_SHORT` | `K_SHORT` | `4` | an http check: the net-check CGI, and the short link's hosts |

They OR together — `NET_CHECK_BASIC | NET_CHECK_LONG` is a run of three checks —
and `0` is **no checks at all**: the plan is empty, the run asks no probe and the
report is `{"details":[]}`. It is not "run everything", which is `1 | 2 | 4`.
`NET_CHECK_SHORT` is the one that needs its hosts: that bit with a `shortLink` of
nothing is a plan of an http check with nothing to check — and a run whose report
says nothing about the short link.

## The report

Every check writes one `CheckResultProfile`, and the report is those as JSON —
`{"details":[ … ]}`, one object per check:

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

| what | the field | what it is |
|---|---|---|
| which check | `detectType` | one of the integers above |
| how the probe went | `errorCode` | `0` and above is one that worked |
| the network it ran on | `networkType` | the `comm::getNetInfo()` you handed to the run |
| what was probed | `detectIP`, `port`, `dnsDomain` | the host, and the port for a tcp check |
| the timings | `conntime`, `rtt`, `rttStr` | ms; `rttStr` is the `rtt` as a string |
| the HTTP check | `httpStatusCode` | what the net-check CGI answered |
| the ping | `pingCheckCount`, `pingLossRate` | how many went out, and what share was lost |
| the dns | `localDns`, `dnsIP1`, `dnsIP2` | the resolver, and the first two addresses |

`mars_sdt_take_report` and `MarsSdt.takeReport()` answer `MARS_SDT_ERR_NO_SPACE`
when the report does not fit the buffer, and **keep the results** — so a caller
that asks again with a bigger buffer gets the diagnosis rather than an empty one.
The Swift `takeReport()` does that for you: it starts at 4 KB and doubles up to
1 MB, and answers `nil` only when there was nothing to take. On Android and in
the shared Kotlin the report is a `String?`, so there is no buffer to size.

## Cancelling, and starting over

| what | Rust | Swift | Kotlin | C |
|---|---|---|---|---|
| stop the run | `cancel_active_check` / `cancel_handle` | `cancelActiveCheck` | `cancelActiveCheck` | `mars_sdt_cancel_active_check` |
| is one in flight | `is_checking` | `isChecking` | `isChecking` | `mars_sdt_is_checking` |
| throw it all away | `SdtCore::reset` | `reset` | `reset` | `mars_sdt_reset` |

A run borrows the logic exclusively in Rust, so `cancel_active_check` cannot be
called while `run_checks` is on the stack: take a `CancelHandle` with
`cancel_handle()` first and hand it to whoever has to stop the run — the probe
closure, which owns the socket that has to give up.

Everywhere else the cancel needs no handle: it sets the flag the run reads and
takes no lock the run holds, so it lands from another thread while the probes are
still being asked — which is the only moment cancelling means anything. What it
does is keep the rest of the plan from running; a probe that is already out is
not interrupted.

And a run is one at a time: the diagnosis is one process-wide value, so a second
`runChecks` waits for the first instead of running beside it — on Android and in
the shared Kotlin the second call does not come back until the first is over.

## Where to go next

- [The task pipeline](/stn) — the half that calls SDT when a host stops
  answering, and whose long link the tcp check noops.
- [Your platform](/platforms/rust) — the whole surface, xlog included.
