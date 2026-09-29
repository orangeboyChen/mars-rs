# Migrating from mars-sdt

An app that ran the C++ project's network diagnosis has one thing to move: what a
probe is. A diagnosis is the same three steps in the C++ and here — name the hosts
and the checks, run them, take the report — and the report is the same
`{"details":[ … ]}` document, one object per check with the same `detectType`,
`errorCode` and timings in it, so whatever reads a report today reads this one.

| the piece you use | what you take here | where it is written down |
|---|---|---|
| `mars/sdt` | `marsrs` on JitPack, and the crate of the same name — crates.io publication pending — `marsrs-kmp` for a shared Kotlin module, `MarsRSNet` on Apple | [Getting started](/sdt/getting-started) |
| `mars/xlog` | `xlog` — the logger, in a package of its own | [Migrating from mars-xlog](/xlog/migrating-from-mars-xlog) |
| `mars/stn` | the same `marsrs`, in the same package as the diagnosis | [Migrating from mars-stn](/stn/migrating-from-mars-stn) |

## What the calls are called

| what | the C++ | Rust | Android, shared Kotlin | Swift | C |
|---|---|---|---|---|---|
| the hosts and the mode | `StartActiveCheck` | `start_active_check(…)` | `startActiveCheck(…)` | `startActiveCheck(…)` | `mars_sdt_start_active_check(…)` |
| run the plan | the `__RunOn` thread | `run_checks(&mut ask, net)` | `runChecks(net, probe)` | `runChecks(networkType:probe:)` | `mars_sdt_run_checks(ctx, probe, net)` |
| stop it | `CancelActiveCheck` | `cancel_active_check()` | `cancelActiveCheck()` | `cancelActiveCheck()` | `mars_sdt_cancel_active_check()` |
| the net-check CGI | `SetHttpNetcheckCGI(cgi)` | `set_http_netcheck_cgi(cgi)` | `setHttpNetcheckCGI(cgi)` | `setHTTPNetCheckCGI(cgi)` | `mars_sdt_set_http_netcheck_cgi(cgi)` |
| the report | `Callback::ReportNetCheckResult` | `report_json(&results)` | `takeReport()`, or the callback | `takeReport()` | `mars_sdt_take_report(buf, len)` |

The mode is the same bit set — ping and DNS, then TCP, then the HTTP check of the
net-check CGI — and `0` is still no checks at all. In Rust the bits have names:
`Mode::NONE`, `BASIC`, `LONG`, `SHORT` and `ALL`, put together with `|`, and
`mode.bits()` hands back the `i32` the upstream call takes. See
[the checks](/sdt/checks).

In Rust the three steps are one call as well:
`sdt.diagnose(longlink, shortlink, mode, timeout, &mut ask, net)` is
`start_active_check`, `run_checks` and the report, and it answers `None` when a
check is already in flight — the `false` of the upstream call.
`start_active_check` carries no deprecation: a host that drives a run across
threads — starting the check on one and reporting it on another — still needs
the three calls apart.

## The two things the app takes over

**The probes are the app's.** The C++ links its own sockets into the checkers, so
a diagnosis there can only ever run against the platform's own networking. Here a
check is a question the run asks you — a domain to resolve, an ip and a port to
connect to, a URL to fetch, a host to ping — and your answer is what the report
records. On Android and in the shared Kotlin that is the `IProbe` you hand to
`runChecks`; in Rust it is the `Ask`; in Swift and in C it is the closure. See
[the probes](/sdt/probes).

**What would have been a thread is a call.** The C++ runs a diagnosis on its
`__RunOn` thread; this port has no threads in it, so `runChecks` asks its probes
on the thread that called it and does not come back until they have answered — an
app runs it once and takes the report, with no loop and no sleep. `diagnose`,
which is both calls in one, is the same and is not a future: an app that wants a
diagnosis off the thread it is on puts it there.

Taking the report empties it, and the buffer of `mars_sdt_take_report` keeps its
results when the report did not fit — see [the report](/sdt/report).

## Where to go next

- [Getting started](/sdt/getting-started) — the dependency and a whole diagnosis,
  on every platform that carries SDT.
- [Migrating from mars-xlog](/xlog/migrating-from-mars-xlog) — the logger, for an
  app whose diagnosis was not the only piece it took.
- [Migrating from mars-stn](/stn/migrating-from-mars-stn) — the task pipeline, in
  the same package as the diagnosis.
