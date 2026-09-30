# The checks

A diagnosis runs the checks of a **mode**, which is a bit set, and the mode is
turned into a plan you can read before anything is probed.

## The checks

| the check | Rust | Swift | shared Kotlin | Android | C |
|---|---|---|---|---|---|
| ping | `NetCheckType::PingCheck` | `.ping` | `Check.Ping` | `0` | `MarsSdtCheckPing` |
| dns | `NetCheckType::DnsCheck` | `.dns` | `Check.Dns` | `1` | `MarsSdtCheckDns` |
| dns against another server, to compare with | `NetCheckType::NewDnsCheck` | `.newDns` | `Check.NewDns` | `2` | `MarsSdtCheckNewDns` |
| tcp | `NetCheckType::TcpCheck` | `.tcp` | `Check.Tcp` | `3` | `MarsSdtCheckTcp` |
| http | `NetCheckType::HttpCheck` | `.http` | `Check.Http` | `4` | `MarsSdtCheckHttp` |
| traceroute — planned, no probe asked for it yet | `NetCheckType::TracerouteCheck` | `.traceroute` | `Check.Traceroute` | — | `MarsSdtCheckTraceroute` |
| the request's own buffer — same | `NetCheckType::ReqBufCheck` | `.reqBuf` | `Check.ReqBuf` | — | `MarsSdtCheckReqBuf` |

## The bits the mode is made of

| the bit | Rust | shared Kotlin, Android | Swift, C | what it puts in the plan |
|---|---|---|---|---|
| `NET_CHECK_BASIC` | `NET_CHECK_BASIC` | `CheckMode.K_BASIC` | `1` | a ping and a dns check — the two the C++ starts with |
| `NET_CHECK_LONG` | `NET_CHECK_LONG` | `K_LONG` | `2` | a tcp check: a noop out to the long link's hosts |
| `NET_CHECK_SHORT` | `NET_CHECK_SHORT` | `K_SHORT` | `4` | an http check: the net-check CGI, and the short link's hosts |

They OR together — `NET_CHECK_BASIC | NET_CHECK_LONG` is a run of three checks —
and `0` is **no checks at all**: the plan is empty, the run asks no probe and the
report is `{"details":[]}`. It is not "run everything", which is `1 | 2 | 4`.

`NET_CHECK_SHORT` is the one that needs its hosts: that bit with a `shortLink` of
nothing is a plan of an http check with nothing to check, and a run whose report
says nothing about the short link.

## Reading the plan first

| what | Rust | Swift | shared Kotlin, Android | C |
|---|---|---|---|---|
| the plan, in the order the checks will run | `sdt.plan()` | `MarsSdt.plan` | `SdtLogic.plan()` | `mars_sdt_plan` |

The shared Kotlin hands it back as `Check`s and Android as the integers
themselves, and the integers of the plan are the `detectType` of every entry of
[the report](/sdt/report) — so one vocabulary names both.

## Where to go next

- [The probes](/sdt/probes) — what each check asks the app for, and what an
  answer is made of.
- [Getting started](/sdt/getting-started) — the dependency and a whole
  diagnosis, on every platform that carries SDT.
