# The questions

A task does not run on its own: while it does, STN asks the app eighteen
questions and the app answers them. *Is this user logged in?* *What bytes does
this task send?* *What did the answer mean?* *What is the device's network?* —
and the rest. Every platform funnels all eighteen through one thing:

| your app is | what answers them |
|---|---|
| Rust | `impl App for MyApp`, given to `stn.set_callback(MyApp)` |
| iOS / watchOS, Swift | one closure, given to `MarsStn.setApp { … }` |
| Android, Kotlin or Java | `StnLogic.ICallBack`, given to `StnLogic.setCallBack(…)` |
| Kotlin Multiplatform | one `ask`, given to `StnLogic.setApp { … }` |
| anything with a C FFI | one function pointer, given to `mars_stn_set_app(ctx, ask)` |
| HarmonyOS | the same function pointer, through a NAPI shim of your own |

The names are the ones the C++ project used, so an answer written there is
written here — all eighteen of them, in the order STN numbers them:

| # | the question | what STN wants | Rust | Swift | shared Kotlin | Android | C |
|---|---|---|---|---|---|---|---|
| 1 | `makesureAuthed` | whether this user is logged in | `makesure_authed` | `.makesureAuthed` | `Question.Kind.MakesureAuthed` | `makesureAuthed` | `MarsStnQuestionMakesureAuthed` |
| 2 | `trafficData` | how much went out and came in | `traffic_data` | `.trafficData` | `Question.Kind.TrafficData` | `trafficData` | `MarsStnQuestionTrafficData` |
| 3 | `onNewDns` | the addresses of a host | `on_new_dns` | `.onNewDns` | `Question.Kind.OnNewDns` | `onNewDns` | `MarsStnQuestionOnNewDns` |
| 4 | `onPush` | the server sent this, and no task asked for it | `on_push` | `.onPush` | `Question.Kind.OnPush` | `onPush` | `MarsStnQuestionOnPush` |
| 5 | `req2Buf` | the bytes the task sends | `req2buf` | `.req2Buf` | `Question.Kind.Req2Buf` | `req2Buf` | `MarsStnQuestionReq2Buf` |
| 6 | `buf2Resp` | what the answer meant | `buf2resp` | `.buf2Resp` | `Question.Kind.Buf2Resp` | `buf2Resp` | `MarsStnQuestionBuf2Resp` |
| 7 | `onTaskEnd` | what to do about the end | `on_task_end` | `.onTaskEnd` | `Question.Kind.OnTaskEnd` | `onTaskEnd` | `MarsStnQuestionOnTaskEnd` |
| 8 | `reportConnectStatus` | the network, as the app is to see it | `report_connect_status` | `.reportConnectStatus` | `Question.Kind.ReportConnectStatus` | `reportConnectInfo` | `MarsStnQuestionReportConnectStatus` |
| 9 | `onLongLinkNetworkError` | the main long link failed | `on_long_link_network_error` | `.longLinkNetworkError` | `Question.Kind.LongLinkNetworkError` | — | `MarsStnQuestionLongLinkNetworkError` |
| 10 | `onShortLinkNetworkError` | a short link failed | `on_short_link_network_error` | `.shortLinkNetworkError` | `Question.Kind.ShortLinkNetworkError` | — | `MarsStnQuestionShortLinkNetworkError` |
| 11 | `onLongLinkStatusChange` | where the default long link is | `on_long_link_status_change` | `.longLinkStatusChange` | `Question.Kind.LongLinkStatusChange` | — | `MarsStnQuestionLongLinkStatusChange` |
| 12 | `getLonglinkIdentifyCheckBuffer` | the check a new link is used with | `identify_check_buffer` | `.identifyCheckBuffer` | `Question.Kind.IdentifyCheckBuffer` | `getLongLinkIdentifyCheckBuffer` | `MarsStnQuestionIdentifyCheckBuffer` |
| 13 | `onLonglinkIdentifyResponse` | whether the answer is the one the check asked for | `identify_response` | `.identifyResponse` | `Question.Kind.IdentifyResponse` | `onLongLinkIdentifyResp` | `MarsStnQuestionIdentifyResponse` |
| 14 | `requestSync` | the app is asked to sync | `request_sync` | `.requestSync` | `Question.Kind.RequestSync` | `requestDoSync` | `MarsStnQuestionRequestSync` |
| 15 | `requestNetCheckShortLinkHosts` | the hosts the network check may probe | `net_check_shortlink_hosts` | `.netCheckShortLinkHosts` | `Question.Kind.NetCheckShortLinkHosts` | `requestNetCheckShortLinkHosts` | `MarsStnQuestionNetCheckShortLinkHosts` |
| 16 | `reportTaskProfile` | everything a task left behind | `report_task_profile` | `.reportTaskProfile` | `Question.Kind.ReportTaskProfile` | `reportTaskProfile` | `MarsStnQuestionReportTaskProfile` |
| 17 | `reportTaskLimited` | a task the two gates refused: which gate, and its reading | `report_task_limited` | `.reportTaskLimited` | `Question.Kind.ReportTaskLimited` | — | `MarsStnQuestionReportTaskLimited` |
| 18 | `reportDnsProfile` | how a dns question went | `report_dns_profile` | `.reportDnsProfile` | `Question.Kind.ReportDnsProfile` | — | `MarsStnQuestionReportDnsProfile` |

A question the app does not answer is a question with the default answer, and
every platform but Android carries one for all eighteen — so an app writes the
ones it cares about: `default: return .nothing` in Swift, `else -> Answer.None`
in the shared Kotlin, an `App` with three methods in Rust.

Android's `ICallBack` is the plain interface the C++ project's Java declared, and
it has no defaults: the object is the app's to finish. It is asked thirteen of
the eighteen — the five marked `—` above are ones the Java api has no method
for, so STN takes its own answers for them — plus one of its own, `isLogoned`,
which nothing asks.

`marsrs-kmp`'s shared Kotlin is asked all eighteen on Kotlin/Native and
thirteen on Android, the same five.

`reportTaskLimited` is the one question only a refused task is asked for: the
two anti-avalanche gates weigh every task that goes out, and a task they refuse
never reaches a queue, so nothing else the app hears carries it. What the app is
handed is which gate refused it and the reading it weighed the task against —
how long ago the same body went out, or how many bytes the funnel would not
take — and what it answers is that reading, changed or not, which is what ends
up on the task's profile. The reading is `limit` on the question, and
`Answer.Limit` or `.limit(…)` is how the shared Kotlin and Swift hand it back;
in C it is the `limit` of `MarsStnQuestion` and a `MarsStnAnswerLimit`.
See [the long link](/stn/long-link) for the link a task goes out on.

The account and the device are not one of the eighteen: on Android they are
`AppLogic.ICallBack`, and on every other platform they are nothing STN asks
about.

## The boot, on Android

Android is the one platform where the app also starts the bridges and hands over
what STN asks about the device:

```kotlin
Mars.init(context, Handler(Looper.getMainLooper()))
Mars.onCreate(true)

AppLogic.setCallBack(object : AppLogic.ICallBack { /* the account and the device */ })
StnLogic.setCallBack(object : StnLogic.ICallBack { /* the thirteen it is asked, and `isLogoned` */ })

// when the screen or the network changes
BaseEvent.onForeground(true)
BaseEvent.onNetworkChange()
```

`AppLogic` is the account and the device; `BaseEvent` is the screen and the
network. On every other platform there is no boot: `stn.create()` in Rust, and
the address and `reset` calls of [the long link](/stn/long-link), are the whole
of what an app has to make.

## Where to go next

- [The task](/stn/tasks) — what a task is made of, and how one ends.
- [The long link](/stn/long-link) — the connection an app keeps up.
- [Getting started](/stn/getting-started) — the dependency and a running task,
  on every platform that carries STN.
