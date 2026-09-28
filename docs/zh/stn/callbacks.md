# 那些问题

一个任务不会自己跑起来：它跑着的时候，STN 会问 App 十八个问题，由 App 来回答。
*这个用户登录了吗？* *这个任务要发哪些字节？* *那个回答是什么意思？* *设备的网络是
什么？* —— 以及其余那些。每个平台都把十八个收进一个东西里：

| 你的 App 是 | 谁来回答 |
|---|---|
| Rust | `impl App for MyApp`，交给 `stn.set_callback(MyApp)` |
| iOS / watchOS，Swift | 一个闭包，交给 `MarsStn.setApp { … }` |
| Android，Kotlin 或 Java | `StnLogic.ICallBack`，交给 `StnLogic.setCallBack(…)` |
| Kotlin Multiplatform | 一个 `ask`，交给 `StnLogic.setApp { … }` |
| 有 C FFI 的任何东西 | 一个函数指针，交给 `mars_stn_set_app(ctx, ask)` |
| HarmonyOS | 同一个函数指针，通过你自己写的 NAPI shim |

名字是 C++ 项目用的那几个，所以在那边写过的回答在这里也写得出来：

| 问题 | STN 想要什么 | Rust | Swift | 共享 Kotlin | Android | C |
|---|---|---|---|---|---|---|
| `req2Buf` | 这个任务要发的字节 | `req2buf` | `.req2Buf` | `Question.Kind.Req2Buf` | `req2Buf` | `MarsStnQuestionReq2Buf` |
| `buf2Resp` | 那个回答是什么意思 | `buf2resp` | `.buf2Resp` | `Question.Kind.Buf2Resp` | `buf2Resp` | `MarsStnQuestionBuf2Resp` |
| `onTaskEnd` | 这个结束该怎么办 | `on_task_end` | `.onTaskEnd` | `Question.Kind.OnTaskEnd` | `onTaskEnd` | `MarsStnQuestionOnTaskEnd` |
| `onPush` | 服务器从长连接上推下来的东西 | `on_push` | `.onPush` | `Question.Kind.OnPush` | `onPush` | `MarsStnQuestionOnPush` |
| `onNewDns` | 一个 host 的地址 | `on_new_dns` | `.onNewDns` | `Question.Kind.OnNewDns` | `onNewDns` | `MarsStnQuestionOnNewDns` |
| `isAuthed` | 这个用户登录了吗 | `is_authed` | `.isAuthed` | `Question.Kind.IsAuthed` | `isAuthed` | `MarsStnQuestionIsAuthed` |
| `getAppInfo` | 账号和设备 | `get_app_info` | `.getAppInfo` | `Question.Kind.GetAppInfo` | `getAppInfo` | `MarsStnQuestionGetAppInfo` |

App 没回答的问题就是拿了默认答案的问题，除 Android 之外每个平台都为全部十八个准备
了默认 —— 所以 App 只写它关心的那几个：Swift 里 `default: return .nothing`，共享
Kotlin 里 `else -> Answer.None`，Rust 里一个带三个方法的 `App`。

Android 的 `ICallBack` 是 C++ 项目的 Java 声明的那个普通 interface，它没有默认实
现：那个对象得由 App 补完。`marsrs-kmp` 的共享 Kotlin 在 Kotlin/Native 上会被问到
全部十八个，在 Android 上是十三个 —— 两个网络错误、长连接的状态变化、任务上限和
DNS profile 是 JNI 桥自己回答的那五个。

## Android 上的启动

Android 是唯一一个 App 还要自己把两座桥起起来、并把 STN 问的设备信息递过去的平台：

```kotlin
Mars.init(context, Handler(Looper.getMainLooper()))
Mars.onCreate(true)

AppLogic.setCallBack(object : AppLogic.ICallBack { /* 账号和设备 */ })
StnLogic.setCallBack(object : StnLogic.ICallBack { /* 那十四个问题 */ })

// 屏幕或网络变了的时候
BaseEvent.onForeground(true)
BaseEvent.onNetworkChange()
```

`AppLogic` 是账号和设备；`BaseEvent` 是屏幕和网络。其余每个平台都没有启动这一步：
Rust 里的 `stn.create()`，加上[长连接](/zh/stn/long-link)那几个地址和 `reset`，就是
App 要做的全部。

## 接着看

- [任务](/zh/stn/tasks) —— 一个任务由什么构成，以及它是怎么结束的。
- [长连接](/zh/stn/long-link) —— App 保持着的那条连接。
- [快速开始](/zh/stn/getting-started) —— 每个带着 STN 的平台上的依赖和一个跑起来
  的任务。
