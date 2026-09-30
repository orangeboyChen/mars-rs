# 那些问题

一个任务不会自己跑起来：它跑着的时候，STN 会问 App 十八个问题，App 来回答。
*这个用户登录了吗？* *这个任务要发哪些字节？* *那个回答是什么意思？* *设备的网络是
什么？* —— 以及其余那些。每个平台都把十八个收进一个东西里：

| 你的 App 是 | 谁来回答 |
|---|---|
| Rust | `impl App for MyApp`，交给 `stn.set_callback(MyApp)` |
| iOS / watchOS，Swift 或 Objective-C | 一个闭包交给 `MarsStn.setApp { … }`，或者 `[MarsStn setApp:]` 接的那个 block |
| Android，Kotlin 或 Java | `StnLogic.ICallBack`，交给 `StnLogic.setCallBack(…)` |
| Kotlin Multiplatform | 一个 `ask`，交给 `StnLogic.setApp { … }` |
| 有 C FFI 的任何东西 | 一个函数指针，交给 `mars_stn_set_app(ctx, ask)` |
| HarmonyOS | 同一个函数指针，通过你自己写的 NAPI shim |

名字是 C++ 项目用的那几个，所以在那边写过的回答在这里也写得出来 —— 十八个都在，按
STN 给它们编的号排：

| # | 问题 | STN 想要什么 | Rust | Swift | 共享 Kotlin | Android | C |
|---|---|---|---|---|---|---|---|
| 1 | `makesureAuthed` | 这个用户登录了吗 | `makesure_authed` | `.makesureAuthed` | `Question.Kind.MakesureAuthed` | `makesureAuthed` | `MarsStnQuestionMakesureAuthed` |
| 2 | `trafficData` | 发出去多少、收进来多少 | `traffic_data` | `.trafficData` | `Question.Kind.TrafficData` | `trafficData` | `MarsStnQuestionTrafficData` |
| 3 | `onNewDns` | 一个 host 的地址 | `on_new_dns` | `.onNewDns` | `Question.Kind.OnNewDns` | `onNewDns` | `MarsStnQuestionOnNewDns` |
| 4 | `onPush` | 服务器推下来的、没有任务要的东西 | `on_push` | `.onPush` | `Question.Kind.OnPush` | `onPush` | `MarsStnQuestionOnPush` |
| 5 | `req2Buf` | 这个任务要发的字节 | `req2buf` | `.req2Buf` | `Question.Kind.Req2Buf` | `req2Buf` | `MarsStnQuestionReq2Buf` |
| 6 | `buf2Resp` | 那个回答是什么意思 | `buf2resp` | `.buf2Resp` | `Question.Kind.Buf2Resp` | `buf2Resp` | `MarsStnQuestionBuf2Resp` |
| 7 | `onTaskEnd` | 这个结束该怎么办 | `on_task_end` | `.onTaskEnd` | `Question.Kind.OnTaskEnd` | `onTaskEnd` | `MarsStnQuestionOnTaskEnd` |
| 8 | `reportConnectStatus` | App 应当看到的那个网络 | `report_connect_status` | `.reportConnectStatus` | `Question.Kind.ReportConnectStatus` | `reportConnectInfo` | `MarsStnQuestionReportConnectStatus` |
| 9 | `onLongLinkNetworkError` | 主长连接坏了 | `on_long_link_network_error` | `.longLinkNetworkError` | `Question.Kind.LongLinkNetworkError` | — | `MarsStnQuestionLongLinkNetworkError` |
| 10 | `onShortLinkNetworkError` | 一条短连接坏了 | `on_short_link_network_error` | `.shortLinkNetworkError` | `Question.Kind.ShortLinkNetworkError` | — | `MarsStnQuestionShortLinkNetworkError` |
| 11 | `onLongLinkStatusChange` | 默认那条长连接到哪一步了 | `on_long_link_status_change` | `.longLinkStatusChange` | `Question.Kind.LongLinkStatusChange` | — | `MarsStnQuestionLongLinkStatusChange` |
| 12 | `getLonglinkIdentifyCheckBuffer` | 一条新连接要用的那个校验 | `identify_check_buffer` | `.identifyCheckBuffer` | `Question.Kind.IdentifyCheckBuffer` | `getLongLinkIdentifyCheckBuffer` | `MarsStnQuestionIdentifyCheckBuffer` |
| 13 | `onLonglinkIdentifyResponse` | 回来的回答是不是校验要的那个 | `identify_response` | `.identifyResponse` | `Question.Kind.IdentifyResponse` | `onLongLinkIdentifyResp` | `MarsStnQuestionIdentifyResponse` |
| 14 | `requestSync` | 要 App 同步一次 | `request_sync` | `.requestSync` | `Question.Kind.RequestSync` | `requestDoSync` | `MarsStnQuestionRequestSync` |
| 15 | `requestNetCheckShortLinkHosts` | 网络检查可以探的 host | `net_check_shortlink_hosts` | `.netCheckShortLinkHosts` | `Question.Kind.NetCheckShortLinkHosts` | `requestNetCheckShortLinkHosts` | `MarsStnQuestionNetCheckShortLinkHosts` |
| 16 | `reportTaskProfile` | 一个任务留下的全部 | `report_task_profile` | `.reportTaskProfile` | `Question.Kind.ReportTaskProfile` | `reportTaskProfile` | `MarsStnQuestionReportTaskProfile` |
| 17 | `reportTaskLimited` | 两道闸门拦下的任务：哪一道，以及它量出来的数 | `report_task_limited` | `.reportTaskLimited` | `Question.Kind.ReportTaskLimited` | — | `MarsStnQuestionReportTaskLimited` |
| 18 | `reportDnsProfile` | 一次 dns 问得怎么样 | `report_dns_profile` | `.reportDnsProfile` | `Question.Kind.ReportDnsProfile` | — | `MarsStnQuestionReportDnsProfile` |

App 不回答的问题，STN 都给它默认答案；除 Android 之外每个平台都为全部十八个准备了
默认 —— 所以 App 只写它关心的那几个：Swift 里 `default: return .nothing`，共享
Kotlin 里 `else -> Answer.None`，Rust 里一个带三个方法的 `App`。

在 Rust 里，十八个里有两个只在没人 await 的任务上才问 App：你交给
`stn.send(task, body)` 的 `body` 就是本来要问 `req2buf` 的东西，服务器回答的那些
字节就是本来要递给 `buf2resp` 的东西 —— 两个都替那个 `Sent` 留着。其余每一个问题照
旧问，包括 `on_task_end`；而没人 await 的任务，App 要答的是全部十八个。见
[任务](/zh/stn/tasks)。

Android 的 `ICallBack` 是 C++ 项目的 Java 声明出来的一个普通 interface，没有默认实
现：这个对象要 App 自己补全。十八个里 STN 会问它十三个 —— 上表标 `—` 的那五个，Java
api 没有对应的方法，于是 STN 用自己的答案 —— 外加一个它自己的 `isLogoned`，那个谁也
不会问。

`marsrs-kmp` 的共享 Kotlin 在 Kotlin/Native 上要答全部十八个，在 Android 上只答十
三个，少的还是上面那五个。

`reportTaskLimited` 是唯一一个只问闸门拦下的任务的问题：每个要发出去的
任务都要过两道防雪崩闸门，闸门拦下的那个根本到不了队列，所以 App 在别的事情里都听
不到它。App 拿到的是拦下它的那道闸门，以及那道闸门量出来的数 —— 同一份 body 多久
之前发过，或者 funnel 装不下的那几个字节；App 回答的也是这个数，改没改过，它最后
都记在任务的 profile 上。这个数在问题上是 `limit`，共享 Kotlin 和 Swift 用
`Answer.Limit` 和 `.limit(…)` 把它交回去；C 里它是 `MarsStnQuestion` 的 `limit`
和一个 `MarsStnAnswerLimit`。任务走的是哪条连接，见[长连接](/zh/stn/long-link)。

账号和设备不在这十八个里：Android 上它们是 `AppLogic.ICallBack`，其余平台上 STN
根本不问它们。

## Android 上的启动

Android 是唯一一个 App 还要自己把两座桥起起来、并把 STN 问的设备信息递过去的平台：

```kotlin
Mars.init(context, Handler(Looper.getMainLooper()))
Mars.onCreate(true)

AppLogic.setCallBack(object : AppLogic.ICallBack { /* 账号和设备 */ })
StnLogic.setCallBack(object : StnLogic.ICallBack { /* 会被问的那十三个，以及 `isLogoned` */ })

// 屏幕或网络变了的时候
BaseEvent.onForeground(true)
BaseEvent.onNetworkChange()
```

`AppLogic` 是账号和设备；`BaseEvent` 是屏幕和网络。屏幕和网络是每个平台都要报的那
两样，只是名字各平台写法不同 —— [长连接](/zh/stn/long-link)那一页把它们列了出来。
Android 独有的一步只有启动本身：`Mars.init` 和 `Mars.onCreate`。

## 接着看

- [任务](/zh/stn/tasks) —— 一个任务由什么构成，以及它是怎么结束的。
- [长连接](/zh/stn/long-link) —— App 保持着的那条连接。
- [快速开始](/zh/stn/getting-started) —— 每个带着 STN 的平台上的依赖和一个跑起来
  的任务。
