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
written here:

| the question | what STN wants | Rust | Swift | shared Kotlin | Android | C |
|---|---|---|---|---|---|---|
| `req2Buf` | the bytes the task sends | `req2buf` | `.req2Buf` | `Question.Kind.Req2Buf` | `req2Buf` | `MarsStnQuestionReq2Buf` |
| `buf2Resp` | what the answer meant | `buf2resp` | `.buf2Resp` | `Question.Kind.Buf2Resp` | `buf2Resp` | `MarsStnQuestionBuf2Resp` |
| `onTaskEnd` | what to do about the end | `on_task_end` | `.onTaskEnd` | `Question.Kind.OnTaskEnd` | `onTaskEnd` | `MarsStnQuestionOnTaskEnd` |
| `onPush` | what the server sent down the long link | `on_push` | `.onPush` | `Question.Kind.OnPush` | `onPush` | `MarsStnQuestionOnPush` |
| `onNewDns` | the addresses of a host | `on_new_dns` | `.onNewDns` | `Question.Kind.OnNewDns` | `onNewDns` | `MarsStnQuestionOnNewDns` |
| `isAuthed` | whether this user is logged in | `is_authed` | `.isAuthed` | `Question.Kind.IsAuthed` | `isAuthed` | `MarsStnQuestionIsAuthed` |
| `getAppInfo` | the account and the device | `get_app_info` | `.getAppInfo` | `Question.Kind.GetAppInfo` | `getAppInfo` | `MarsStnQuestionGetAppInfo` |

A question the app does not answer is a question with the default answer, and
every platform but Android carries one for all eighteen — so an app writes the
ones it cares about: `default: return .nothing` in Swift, `else -> Answer.None`
in the shared Kotlin, an `App` with three methods in Rust.

Android's `ICallBack` is the plain interface the C++ project's Java declared, and
it has no defaults: the object is the app's to finish. The shared Kotlin of
`marsrs-kmp` is asked all eighteen on Kotlin/Native and thirteen on Android —
the two network errors, the long link's status change, the task limit and the DNS
profile are five the JNI bridge answers itself.

## The boot, on Android

Android is the one platform where the app also starts the bridges and hands over
what STN asks about the device:

```kotlin
Mars.init(context, Handler(Looper.getMainLooper()))
Mars.onCreate(true)

AppLogic.setCallBack(object : AppLogic.ICallBack { /* the account and the device */ })
StnLogic.setCallBack(object : StnLogic.ICallBack { /* the fourteen questions */ })

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
