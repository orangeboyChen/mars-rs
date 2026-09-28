# 快速开始

STN 是 mars 里跟服务器说话的那一半。**任务**是一个工作单位 —— 一个请求和它的回答
—— STN 管的是它等的队列、它出去时走的链路、重试、超时、DNS，以及它跑得怎么样的
报告。xlog 是另一半，两者互不依赖：只打日志的 App 拿
[日志那部分](/zh/xlog/getting-started)，这里的一切都用不上。

两条链路，任务会说出它要哪一条：

| | **短连接** | **长连接** |
|---|---|---|
| 它是什么 | 一个请求，一个回答 | App 一直保持着的一条连接 |
| 它承载什么 | 你发起的一个任务 | 你的任务、服务器推下来的消息、让连接保持的 noop |
| mars 怎么叫它 | `CHANNEL_SHORT` | `CHANNEL_LONG`、`CHANNEL_MINOR_LONG` |

`Task::new` 两条都要（`CHANNEL_BOTH`），在你有理由挑一条之前这就是你要的：STN 会
把任务发到第一条已经起来的链路上。

## 它在哪里

| 你的 App 是 | 谁带着 STN | 怎么拿到它 |
|---|---|---|
| Rust | `marsrs`（`marsrs-xlog` 里一点都没有） | `marsrs::stn` |
| iOS / watchOS，Swift | `MarsRSNet` 这个 product，或 `MarsRS` | `MarsStn` |
| Android，Kotlin 或 Java | JitPack 上的 `marsrs`，不是 `xlog` | `io.github.orangeboychen.marsrs.stn.StnLogic` |
| Kotlin Multiplatform | `marsrs-kmp`，不是 `xlog-kmp` | `io.github.orangeboychen.marsrs.stn.StnLogic` |
| 有 C FFI 的任何东西 | `include/mars_stn.h` | `mars_stn_*` |
| HarmonyOS | 同一个 `libmars_ffi.so` | `mars_stn_*`，通过你自己写的 NAPI shim |

Flutter 插件和 React Native 模块目前都只有日志：两个都不认识任务。它们下面的
native 一侧在 Android 上倒是带着 STN —— 整个移植的那个 AAR —— 所以这两边的 App
从平台自己的代码里发起一个任务，再自己把它递到 Dart 或 JS 那边。

## 三件事，按这个顺序

1. **回答那些问题。** 一个任务跑着的时候，STN 会问 App 十八个问题 —— *这个用户登录
   了吗？*，*这个任务要发哪些字节？*，*那个回答是什么意思？* —— 由 App 来回答：
   Rust 里一个 trait，Swift 里一个闭包，共享 Kotlin 里一个 `ask`，C 里一个回调，
   Android 上是 App 实现的那个 interface。见[那些问题](/zh/stn/callbacks)。
2. **发起一个任务。** 它被接过去，挑一条链路，放进队列，调用立刻返回。
3. **驱动队列。** 这是这个移植对调用方唯一的要求。本该是一个线程的地方，是 App 的
   一对调用 —— `run_pending()`，以及告诉它这一趟最多还能等多久的 `due_time()`。
   一个发起了却从没被排空的任务，会一直待在它的队列里。

## Rust

```bash
cargo add marsrs          # 整个移植：xlog、stn、sdt
```

```rust
use marsrs::stn::{gen_task_id, App, ErrCmdType, StnLogic, Task, TaskFailHandleType};

// 1. App：每个问题都有默认值，所以这就是一个会编码请求、读回答的 App 的全部
struct MyApp;

impl App for MyApp {
    fn req2buf(
        &mut self,
        _taskid: u32,
        _user_id: &str,
        _channel_select: i32,
        _host: &str,
        _sequence: u16,
    ) -> Result<Vec<u8>, i32> {
        Ok(b"...".to_vec()) // Err(code) 用那个 code 结束这个任务
    }

    fn buf2resp(
        &mut self,
        _taskid: u32,
        _user_id: &str,
        body: &[u8],
        _channel_select: i32,
    ) -> (i32, TaskFailHandleType) {
        handle(body);
        (0, TaskFailHandleType::Normal)
    }

    fn on_task_end(
        &mut self,
        _taskid: u32,
        _user_id: &str,
        _err_type: ErrCmdType,
        _err_code: i32,
        _profile: &marsrs::stn::CgiProfile,
    ) -> i32 {
        0
    }
}

let mut stn = StnLogic::new();
stn.set_callback(MyApp);
stn.create(); // 建出 net core；这之前上面这些都不起作用

// 2. 一个任务
let mut task = Task::new(gen_task_id(), 100);
task.cgi = "/cgi-bin/hello".to_owned();
task.shortlink_host_list = vec!["example.com".to_owned()];
task.total_timeout = 10_000;
stn.start_task(task);

// 3. 循环：`due_delay()` 是这一趟还能等多久
while let Some(wait) = stn.due_delay() {
    std::thread::sleep(std::time::Duration::from_millis(wait));
    stn.run_pending();
}
```

`stn.create()` 建出 net core，这之前什么都不起作用。`due_delay()` 回答这一趟还能
等多少毫秒，没有可等的东西时回答 `None`；`StnLogic::due_time` 是那个 tick 读数，
同一进程里的调用方可以拿它跟自己的钟比。

这里的一条链路是连接的**模型**而不是连接本身，所以 socket 是 App 的：一个任务的
连接、发送、接收和关闭就是造链路时给它的那个 `SocketOperator`，在 Rust 里通过 net
core 的 factory 接上去：

```rust
use marsrs::stn::{ShortLink, StnLogic};

let core = stn.net_core().expect("create() has been through");
core.factory().set_create_shortlink(|task, use_proxy| {
    let mut link = ShortLink::new(task, use_proxy);
    link.set_socket_operator(MySockets); // 你自己的 connect / send / recv / close
    link
});
```

socket 周围的一切都在 —— 队列、链路的选择、连接前的 DNS、重试、超时，以及最后的
profile 和报告。

## Swift

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")

// 在要用的 target 里：
.product(name: "MarsRSNet", package: "mars-rs")   // 要两半都有就 MarsRS
```

```swift
import MarsRSNet

// 1. App 就是一个闭包：一个问题进，一个回答出
MarsStn.setApp { question in
    switch question.kind {
    case .req2Buf:  return .encoded(try! encode(question.task!))
    case .buf2Resp: handle(question.body); return .decoded(errorCode: 0, handle: .normal)
    case .onTaskEnd: return .ended(errorCode: 0)
    default:        return .nothing
    }
}

// 2. 一个任务
var task = StnTask(channelSelect: .short)
task.taskID = MarsStn.generateTaskID()
task.cgi = "/cgi-bin/hello"
task.shortLinkHosts = ["example.com"]
task.totalTimeout = 10_000
MarsStn.start(task)

// 3. 循环 —— `dueTime` 是这一趟还能等多少毫秒，没有可等的东西时是 `nil`
while let wait = MarsStn.dueTime {
    Thread.sleep(forTimeInterval: Double(wait) / 1000)
    MarsStn.runPending()
}
```

`MarsStn` 是一个 `enum`，装的是 C ABI 那套 `mars_stn_*` 上的 static，pod 是同一个
framework 上的同一份 Swift。Objective-C 看不见它：一个只有 static 成员的 Swift
`enum` 不是 Objective-C 能 import 的类型，所以跑任务的 App 得把那部分写在 Swift
里。

## Android

```kotlin
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen.marsrs:marsrs:0.1.0")   // 只有 xlog 的那个没有
```

```kotlin
// 1. App，以及它要问的那个平台
Mars.init(context, Handler(Looper.getMainLooper()))
Mars.onCreate(true)

AppLogic.setCallBack(object : AppLogic.ICallBack { /* … */ })
StnLogic.setCallBack(object : StnLogic.ICallBack {
    override fun req2Buf(
        taskID: Int, userContext: Any?, reqBuffer: ByteArrayOutputStream?,
        errCode: IntArray?, channelSelect: Int, host: String?, sequence: Int,
    ): Boolean { reqBuffer?.write(encode(taskID)); return true }
    override fun buf2Resp(
        taskID: Int, userContext: Any?, respBuffer: ByteArray?,
        errCode: IntArray?, channelSelect: Int, sequence: IntArray?,
    ): Int { handle(respBuffer); return StnLogic.RESP_FAIL_HANDLE_NORMAL }

    override fun onTaskEnd(
        taskID: Int, userContext: Any?, errType: Int, errCode: Int,
        profile: StnLogic.CgiProfile?,
    ): Int = 0
    // … 还有另外十一个：`ICallBack` 是没有默认实现的 interface，
    //    所以这个对象得由 App 补完
})

// 2. 一个任务
val task = StnLogic.Task(
    StnLogic.Task.E_BOTH, 100, "/cgi-bin/hello", arrayListOf("example.com")
)
task.totalTimeout = 10_000
StnLogic.setShortlinkSvrAddr(443)
StnLogic.startTask(task)

// 3. 循环 —— `dueTime()` 是这一趟还能等多少毫秒，没有可等的东西时回答 -1
var due = StnLogic.dueTime()
while (due >= 0) {
    Thread.sleep(due)
    StnLogic.runPending()
    due = StnLogic.dueTime()
}
```

`Mars.init` 和 `Mars.onCreate` 起两座桥，`AppLogic.setCallBack` 是 STN 问的账号和
设备的来源 —— 见[那些问题](/zh/stn/callbacks)。`ICallBack` 是个没有默认实现的普通
interface，所以那个对象得由 App 补完。

## Kotlin Multiplatform

```kotlin
// 共享模块的 build.gradle.kts
implementation("io.github.orangeboychen.marsrs:marsrs-kmp:0.1.0")   // 不是 xlog-kmp
```

```kotlin
import io.github.orangeboychen.marsrs.stn.Answer
import io.github.orangeboychen.marsrs.stn.FailHandle
import io.github.orangeboychen.marsrs.stn.Question
import io.github.orangeboychen.marsrs.stn.StnLogic
import io.github.orangeboychen.marsrs.stn.Task

// 1. App 就是一个 `ask`：一个问题进，一个回答出
StnLogic.setApp { question ->
    when (question.kind) {
        Question.Kind.Req2Buf -> Answer.Encoded(encode(question.task))
        Question.Kind.Buf2Resp -> {
            handle(question.body)
            Answer.Decoded(errorCode = 0, handle = FailHandle.Normal)
        }
        Question.Kind.OnTaskEnd -> Answer.Ended(errorCode = 0)
        else -> Answer.None
    }
}

// 2. 一个任务
val task = Task().apply {
    taskID = StnLogic.genTaskID()
    channelSelect = Task.E_BOTH
    cmdID = 100
    cgi = "/cgi-bin/hello"
    shortLinkHostList = listOf("example.com")
    totalTimeout = 10_000
}
StnLogic.startTask(task)

// 3. 循环 —— `dueTime()` 是这一趟还能等多少毫秒，没有可等的东西时回答 `null`
var due = StnLogic.dueTime()
while (due != null) {
    StnLogic.runPending()
    due = StnLogic.dueTime()
}
```

`StnLogic` 是 `commonMain` 里一个 `expect object`，每个平台族有一个 `actual` ——
Android 上走 JNI 桥，每个 Kotlin/Native target 上通过 cinterop 走 C ABI。一个
`common` 声明只能是两者都能说出来的东西：Android 上那座桥自己回答十八个问题里的
五个，所以 `setApp` 在那里被问到十三个，在 Kotlin/Native 上是全部十八个。

## The C ABI {#c-abi}

```text
marsrs-<version>-<host>.tar.gz   （Linux、macOS）
marsrs-<version>-<host>.zip      （Windows）
    include/mars_stn.h      任务链路
    libmars_ffi.a / libmars_ffi.so（.dylib、.dll）
```

```c
#include <mars_stn.h>

// 1. App 就是一个 C 回调
static void ask(void *ctx, const MarsStnQuestion *q, MarsStnAnswer *a) {
    switch (q->kind) {
    case MarsStnQuestionReq2Buf:
        a->kind = MarsStnAnswerEncoded; a->bytes = body; a->byte_count = n; break;
    case MarsStnQuestionBuf2Resp:
        a->kind = MarsStnAnswerDecoded; a->error_code = 0; a->handle = 0; break;
    default: a->kind = MarsStnAnswerNothing; break;
    }
}
mars_stn_set_app(NULL, ask);

// 2. 一个任务
MarsStnTask task;
memset(&task, 0, sizeof task);
task.taskid = mars_stn_gen_task_id();
task.cmdid = 100;
task.channel_select = 0x3;              /* 两条都要 */
task.cgi = "/cgi-bin/hello";
const char *hosts[] = { "example.com" };
task.shortlink_host_list.items = hosts;
task.shortlink_host_list.count = 1;
if (mars_stn_start_task(&task) != MARS_STN_OK) { /* 被拒，或 panic 了 */ }

/* 3. 循环 —— 回答是这一趟还能等多少毫秒，"没有可等的东西" 就是 MARS_STN_ERR_NO_DUE */
long long due = mars_stn_due_time();
while (due >= 0) {
    usleep(due * 1000);
    mars_stn_run_pending();
    due = mars_stn_due_time();
}
```

头文件里每个 `int` 回答 `MARS_STN_OK`（0）或一个负的 `MARS_STN_ERR_*`，C ABI 里没
有任何东西会把栈展开到 C 里。

## HarmonyOS

拿 `marsrs-harmonyos-xlog` 的 App 拿到的是日志：那个包的 ArkTS 没有伸到 STN。带着
它的是同一个 `libmars_ffi.so`，在默认的 `xlog` 之上开了 `stn` feature —— 所以要跑
任务的 App 取 `marsrs-harmony-<version>.tar.gz`，把 `libmars_ffi.so` 放进模块的
`libs/<abi>/`，再写那个伸到 `mars_stn.h` 的 NAPI shim：

```c
#include <mars_stn.h>

mars_stn_set_app(NULL, ask);          /* 十八个问题，一个回调 */
mars_stn_start_task(&task);

/* 回答是这一趟还能等多少毫秒 */
long long due = mars_stn_due_time();
while (due >= 0) {                    /* App 排空队列 */
    usleep(due * 1000);
    mars_stn_run_pending();
    due = mars_stn_due_time();
}
```

HarmonyOS 上的 C ABI 就是[那一节](#c-abi)为 Linux、macOS 和 Windows 发布的那个，
也是 `MarsRSNet`、Android 的 AAR 和 `marsrs-kmp` 写在上面的那个。

## 接着看

- [任务](/zh/stn/tasks) —— 一个任务由什么构成，以及它是怎么结束的。
- [那些问题](/zh/stn/callbacks) —— App 在一个任务跑着的时候回答的那十八个，以及每个
  平台怎么称呼它们。
- [长连接](/zh/stn/long-link) —— App 保持着的那条连接，以及网络或屏幕变了的时候要
  告诉 STN 什么。
