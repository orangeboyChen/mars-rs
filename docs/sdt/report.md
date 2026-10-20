# The report

Every host a check probes writes one `CheckResultProfile`, and the report is those
as JSON — `{"details":[ … ]}`, one object per host and not one per check: a ping
and a dns walk both links, so each of them writes one per host of the long link
and one per host of the short link.

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
| which check | `detectType` | one of the integers of [the checks page](/sdt/checks) |
| how the probe went | `errorCode` | `0` and above is one that worked |
| the network it ran on | `networkType` | the `comm::getNetInfo()` you handed to the run |
| what was probed | `detectIP`, `port`, `dnsDomain` | the host, and the port for a tcp check |
| the timings | `conntime`, `rtt`, `rttStr` | ms; `rttStr` is the `rtt` as a string |
| the HTTP check | `httpStatusCode` | what the net-check CGI answered |
| the ping | `pingCheckCount`, `pingLossRate` | how many went out, and what share was lost |
| the dns | `localDns`, `dnsIP1`, `dnsIP2` | the resolver, and the first two addresses |

## Taking it

| what | Rust | Swift | shared Kotlin, Android | C |
|---|---|---|---|---|
| the report | `report_json(&results)` | `MarsSdt.takeReport()` | `SdtLogic.takeReport()` | `mars_sdt_take_report(buf, len)` |
| a callback instead | `sdt.set_callback(…)` | — | `SdtLogic.setCallBack(object : SdtLogic.ICallBack { … })` | — |

The two carry the same document, so an app uses one of them: on Android and in the
shared Kotlin a run hands the report to the callback *and* keeps it for a
`takeReport()` that comes later, so an app that does both sends the same diagnosis
twice. Taking it empties it — the next call reports what happened since.

On Android and in the shared Kotlin the report is a `String?`, so there is no
buffer to size. The Swift `takeReport()` starts at 4 KB and doubles up to 1 MB,
and answers `nil` in three cases: there was nothing to take, the C ABI answered
a code other than `MARS_SDT_ERR_NO_SPACE` — a panic, or a NULL buffer — or the
report still did not fit 1 MB. Only the first of them is "no diagnosis yet", so
an app that treats every `nil` that way loses a report it already had.
`mars_sdt_take_report` answers `MARS_SDT_ERR_NO_SPACE` when the report does not
fit the buffer — and **keeps the results**, so a caller that asks again with a
bigger one gets the diagnosis rather than an empty one.

## Cancelling, and starting over

| what | Rust | Swift | Kotlin | C |
|---|---|---|---|---|
| stop the run | `cancel_active_check` / `cancel_handle` | `cancelActiveCheck` | `cancelActiveCheck` | `mars_sdt_cancel_active_check` |
| is one in flight | `is_checking` | `isChecking` | `isChecking` | `mars_sdt_is_checking` |
| throw it all away | a new `SdtLogic` | `reset` | `reset` | `mars_sdt_reset` |

A run borrows the logic exclusively in Rust, so `cancel_active_check` cannot be
called while `run_checks` is on the stack: take a `CancelHandle` with
`cancel_handle()` first and hand it to whoever has to stop the run — the probe
closure, which owns the socket that has to give up.

There is no `reset` on `SdtLogic` — the `SdtCore` behind it is a private field —
so the way Rust throws a diagnosis away is a new logic: drop this one, make
another, and set the callback on that one again. A run that finished needs none
of it: its end clears the plan and the request, and `run_checks` hands the
results back instead of keeping them, so the next `start_active_check` is a
diagnosis made from nothing.

Everywhere else the cancel needs no handle: it sets the flag the run reads and
takes no lock the run holds, so it lands from another thread while the probes are
still being asked — which is the only moment cancelling means anything. What it
does is keep the rest of the plan from running; a probe that is already out is not
interrupted.

And a run is one at a time: the diagnosis is one process-wide value, so a second
`runChecks` does not run beside the first. On Android it waits — the second call
does not come back until the first is over. On the other Kotlin targets it is
refused: `runChecks` answers `false` and runs nothing.

## Where to go next

- [The probes](/sdt/probes) — the four checks, and what each one asks the app for.
- [Getting started](/sdt/getting-started) — the dependency and a whole diagnosis.
