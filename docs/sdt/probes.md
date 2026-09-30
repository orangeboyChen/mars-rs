# The probes

This is the one thing to know before the rest of it. A check is a **socket** — a
resolve, a connect, an HTTP request, an ICMP echo — and this port owns none. So a
diagnosis asks the app for the network, one probe at a time, and what the app
answers is what the report records:

| the check | what it asks for | what you answer |
|---|---|---|
| ping | the host, a timeout in **seconds** | how long it took, and what share of the pings were lost |
| dns | the domain, a timeout in ms | the addresses it resolved to |
| tcp | the ip and port, a timeout in ms | whether the noop went out, what came back, and whether it was the answer to it |
| http | the URL of the net-check CGI | the HTTP status and how long the request took |

Which is what makes a diagnosis **testable**, and what lets a host that is not a
phone run one at all: the same closure that answers on a device answers from a
test, or from a machine with no radio.

## What the app hands in

| your app is | what the probes are asked of |
|---|---|
| Rust | the `Ask` of `sdt.run_checks(&mut ask, net)` |
| iOS / watchOS, Swift or Objective-C | the closure of `MarsSdt.runChecks(networkType:) { … }`, or the block it becomes |
| Android | the `SdtLogic.IProbe` of `SdtLogic.runChecks(net, probe)` |
| Kotlin Multiplatform | the same `IProbe`, of `SdtLogic.runChecks(1, probe)` |
| anything with a C FFI | the function pointer of `mars_sdt_run_checks(ctx, probe, net)` |
| HarmonyOS | the same function pointer, through a NAPI shim of your own |

The run asks them one at a time, in the order of the plan, on the thread that
called it — and the call does not come back until every probe has answered, so no
sleep and no loop is involved the way one is for
[a task](/stn/getting-started). A probe that has nothing to say answers
"nothing" — `.nothing` in Swift, `ProbeAnswer.None` in the shared Kotlin,
`MarsSdtNothing` in C — and the check that asked is recorded as one that
failed, which ends the run: what stands behind it in the plan is not checked.
A ping is the one exception — a ping nobody sent is a check that did not run,
so it is left out of the report and the run goes on behind it.

## The answers

| the check | Rust | Swift | shared Kotlin | Android | C |
|---|---|---|---|---|---|
| ping | `Answer::Ping { error_code, rtt, status }` | `.ping(errorCode:rtt:lossRate:averageRTT:)` | `ProbeAnswer.Ping` | `SdtLogic.Answer.ping(…)` | `MarsSdtPing` |
| dns | `Answer::Dns { error_code, rtt, local_dns, ips }` | `.dns(errorCode:rtt:addresses:)` | `ProbeAnswer.Dns` | `SdtLogic.Answer.dns(…)` | `MarsSdtDns` |
| tcp | `Answer::Tcp { sent, received, is_noop_resp, conntime, rtt }` | `.tcp(errorCode:rtt:noop:)` | `ProbeAnswer.Tcp` | `SdtLogic.Answer.tcp(…)` | `MarsSdtTcp` |
| http | `Answer::Http { error_code, status_code, rtt }` | `.http(errorCode:rtt:statusCode:)` | `ProbeAnswer.Http` | `SdtLogic.Answer.http(…)` | `MarsSdtHttp` |

The four are one struct in C — `MarsSdtAnswer` — and the `kind` in it is which
one it is: the names of the last column are the four `MarsSdtKind` values and
not four types. Two of the Rust's fields are the Rust's alone — `local_dns` and
`conntime`, which no other seam carries a field for — so a probe that answers on
one of them leaves the report's `localDns` empty and its `conntime` at `0`.

An answer of a probe that is not the one asked is not the answer to it: what the
report records is what came back for the check that was run.

## Where to go next

- [The checks](/sdt/checks) — the mode, and the plan the run walks in order.
- [The report](/sdt/report) — the JSON of what the probes answered.
