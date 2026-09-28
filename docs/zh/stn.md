# 任务链路（STN）

STN 是 mars 里跟服务器说话的那一半。**任务**是一个工作单位 —— 一个请求和它的回答
—— STN 管的是它等的队列、它出去时走的链路、重试、超时、DNS，以及它跑得怎么样的报告。
xlog 是另一半，两者互不依赖：只打日志的应用拿[日志那部分](/zh/log-files)，这里的一切都用不上。

两条链路，任务会说出它要哪一条：

| | **短连接** | **长连接** |
|---|---|---|
| 它是什么 | 一个请求，一个回答 | 应用一直保持着的一条连接 |
| 它承载什么 | 你发起的一个任务 | 你的任务、服务器推下来的消息、让连接保持的 noop |
| mars 怎么叫它 | `CHANNEL_SHORT` | `CHANNEL_LONG`、`CHANNEL_MINOR_LONG` |

`Task::new` 两条都要（`CHANNEL_BOTH`），在你有理由挑一条之前这就是你要的：STN 会把任务
发到第一条已经起来的链路上。

## 它在哪

| 你的应用是 | 什么带着 STN | 怎么用它 |
|---|---|---|
| Rust | `marsrs`（`marsrs-xlog` 里一点都没有） | [`marsrs::stn`](https://docs.rs/marsrs) —— 见 [Rust](/zh/platforms/rust#任务链路) |
| Swift | `MarsRSNet` 这个 product，或 `MarsRS` | `MarsStn` —— 见 [SwiftPM](/zh/platforms/swift#任务链路) |
| Android，Kotlin 或 Java | JitPack 上的 `marsrs`，不是 `xlog` | `io.github.orangeboychen.marsrs.stn.StnLogic` —— 见 [Android](/zh/platforms/android#任务链路) |
| Kotlin Multiplatform | `marsrs-kmp`，不是 `xlog-kmp` | `io.github.orangeboychen.marsrs.stn.StnLogic` —— 见 [Kotlin Multiplatform](/zh/platforms/kotlin-multiplatform#任务链路) |
| 任何能调 C 的 | `include/mars_stn.h` | `mars_stn_*` —— 见 [C ABI](/zh/platforms/c-abi#任务链路) |
| HarmonyOS | 同一个 `libmars_ffi.so` | `mars_stn_*`，通过你自己写的 NAPI shim —— 见 [HarmonyOS](/zh/platforms/harmonyos#mars-的另一半) |

Flutter 插件和 React Native 模块今天就只有日志：两个都不认识任务，在它们下面的 Dart 和
TypeScript 认识之前也不会认识 —— 见 [Flutter](/zh/platforms/flutter#里面没有什么)和
[React Native](/zh/platforms/react-native#里面没有什么)。

::: warning socket 是你自己的，这个端口一个都不开
这个端口里的一条链路是连接的**模型**，不是连接本身：`ShortLink` 和 `LongLink` 建起来时
没有 `SocketOperator` —— 也就是 C++ 里 `socketOperator_` 的那个 trait —— 而没有它的链路，
对一个任务开头的那次连接回答 `SocketFd::INVALID`，于是任务以一次 socket 错误
（`ErrCmdType::Socket`）结束，而不是真的发出去。这个端口里没有任何一处实现那个 trait：
树里每个 `impl SocketOperator` 都是测试用的，C、JNI、Swift 和 Kotlin 的绑定也都不装，所以
除 Rust 之外，今天还没有放 socket 的地方。

在 Rust 里接线是你自己的事，通过 C++ 里 `net_channel_factory.cc` 那两个钩子：

```rust
use marsrs::stn::{ShortLink, StnLogic};

let mut stn = StnLogic::new();
stn.set_callback(MyApp);
stn.create();
let core = stn.net_core().expect("create() has been through");
core.factory().set_create_shortlink(|task, use_proxy| {
    let mut link = ShortLink::new(task, use_proxy);
    link.set_socket_operator(MySockets); // 你自己的 connect / send / recv / close
    link
});
```

socket **周围**的一切都在，而且测过：队列、链路的选择、连接前的 DNS、重试、超时，以及最后
的 profile 和报告。
:::

## 三件事，按这个顺序

**1. 装上应用。** 一个任务跑着的时候，STN 会问应用十八个问题 —— *这个用户登录了吗？*，
*这个任务要发哪些字节？*，*那个回答是什么意思？* —— 由应用来回答。Rust、Swift 和 C 把
十八个都收到一个 trait、一个闭包和一个函数指针里，每个问题都有默认值，所以只关心其中
一个问题的应用就只写一个。Android 的 `StnLogic.ICallBack` 是 C++ 那个 Java interface
声明的十四个，而且它是个普通 interface：十四个都由应用写。`marsrs-kmp` 的共享 Kotlin
是一个 `ask` —— 一个问题进，一个回答出 —— 它在 Kotlin/Native 上会被问到全部十八个，在
Android 上是十三个：JNI 桥自己回答的那五个 —— 两个网络错误、长连接的状态变化、任务上限
和 DNS profile —— 在那里永远到不了应用。

**2. 发起一个任务。** `start_task` 接过去，挑一条链路，然后立刻返回：任务跑在队列上，
不在调用它的线程上。

**3. 驱动队列。** 这是这个端口唯一一处要求调用方做、而 C++ 不做的事。C++ 用自己的线程跑
队列和长连接；这个端口里面没有线程，所以本该是一个线程的地方，是宿主的一次调用 ——
`run_pending()`，以及告诉你这一趟最多还能等多久的 `due_time()`。一个发起了却从没被排空的
任务，会一直待在它的队列里。

::: code-group

```rust [Rust]
use marsrs::stn::{gen_task_id, App, ErrCmdType, StnLogic, Task, TaskFailHandleType};

// 1. 应用：每个问题都有默认值，所以这就是一个会编码请求、读回答的应用的全部
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

```swift [Swift]
import MarsRSNet

// 1. 应用就是一个闭包：一个问题进，一个回答出
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

```kotlin [Android]
// 1. 应用，以及它要问的那个平台
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
    //    所以这个对象得由应用补完
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

```kotlin [Kotlin Multiplatform]
import io.github.orangeboychen.marsrs.stn.Answer
import io.github.orangeboychen.marsrs.stn.FailHandle
import io.github.orangeboychen.marsrs.stn.Question
import io.github.orangeboychen.marsrs.stn.StnLogic
import io.github.orangeboychen.marsrs.stn.Task

// 1. 应用就是一个 `ask`：一个问题进，一个回答出
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

```c [C]
#include <mars_stn.h>

// 1. 应用就是一个 C 回调
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

:::

::: warning 没有谁替你驱动队列
`run_pending` / `due_time` —— 上面每一种写法里 —— 是把任务从它的队列里挪出去的东西，
而这里的任何平台都不会用自己的线程去调它：C++ 有一个 message-queue 线程和一个 `__RunOn`
线程，这个端口两个都没有。每个带着 STN 的平台都把这一对露出来，所以这个循环要由应用
自己写；一个发起了却从没被排空的任务会一直坐在它的队列里，直到进程结束 —— 对一个哪儿
也去不了的任务，`has_task` 照样回答 `true`。

`due_time` —— 上面每一种写法里 —— 是距离下一趟还有多久，单位是毫秒：`0` 是已经到期的
一趟，队列里等着的一个 follow-up 就是。它是一个时长，不是一个时刻，这正是跨 ABI 的调用方
需要的：tick 从一个只有本进程读得到的原点起算，所以告诉宿主的是还要等多久。它也不是什么
承诺 —— 一个不停空转调 `run_pending` 的循环照样能工作，只是会白烧掉一个核。Rust 那一半
把两者分开写：`StnLogic::due_time` 是 `gettickcount()` 的读数，同一进程里的调用方可以拿它
跟自己的钟比，`due_delay` 才是那个等待；其余每个平台回答的都是那个等待，别的什么都没有。
:::

## 任务

一个任务就是一个 struct，每个字段都有 C++ 给的那个默认值。应用会设的是这些：

| 它是什么 | Rust | Swift | Android、KMP | C |
|---|---|---|---|---|
| 你停它时用的 id | `taskid` | `taskID` | `taskID` | `taskid` |
| 服务器怎么叫它 | `cmdid` | `cmdid` | `cmdID` | `cmdid` |
| 它去的那个路径 | `cgi` | `cgi` | `cgi` | `cgi` |
| 走哪条（些）链路 | `channel_select` | `channelSelect` | `channelSelect` | `channel_select` |
| 要试的 hosts | `shortlink_host_list`、`longlink_host_list` | `shortLinkHosts`、`longLinkHosts` | `shortLinkHostList` | `shortlink_host_list` |
| 多长时间，毫秒 | `total_timeout` | `totalTimeout` | `totalTimeout` | `total_timeout` |
| 重试几次 | `retry_count` | `retryCount` | `retryCount` | `retry_count` |
| headers | `headers` | `headers` | `headers` | `headers` / `header_count` |

Rust 的 `Task::new`、Swift 的 `StnTask(channelSelect:)`、共享 Kotlin 的 `Task()` 会把这个
形状给你画好；想在任务之前先拿到一个 id 时，有 `gen_task_id()` /
`MarsStn.generateTaskID()` / `StnLogic.genTaskID()`。

## 一个任务怎么结束

应用是把结束当一个问题听到的 —— 每种写法里都叫 `onTaskEnd` —— 带着两个数字和一个 profile：

- **一个错误类型** —— 失败*在哪*：Rust 里是 `ErrCmdType`，其余是 `errType`。`Ok`（0），
  或者 `Dns`、`Socket`、`Http`、`Server`、`Local`、`Canceld` …
- **一个错误码** —— *出了什么*问题，它是负的：`-500` 第一个包一直没来，`-501` 包之间有
  断档，`-502` 一次读或写超时，`-503` 整个任务超时，`-10086` 网络在它下面变了。Android
  把这些名字放在 `StnLogic` 上（`FIRSTPKGTIMEOUT`、`TASKTIMEOUT` …）。
- **一个 profile** —— 这一趟的耗时：DNS 什么时候开始、连接什么时候建完、rtt。Rust 和
  Kotlin 里是 `CgiProfile`，Swift 里是 `StnQuestion.CgiProfile`，C 里是 `MarsStnCgiProfile`。

`buf2Resp` 除了一个 code 还要回答一个 **fail handle**，这是应用告诉 STN 对一个坏回答该
怎么办、而不只是说它是坏的：`Normal`、`RetryAllTasks`、`SessionTimeout`、`TaskEnd`、
`TaskTimeout`。

## 长连接

长连接是一条连接，不是一个请求，而且它由应用来保持：

| 什么 | Rust | Swift | Android、KMP | C |
|---|---|---|---|---|
| 它连到哪里 | `set_longlink_svr_addr` | `setLongLinkServerAddress` | `setLonglinkSvrAddr` | `mars_stn_set_longlink_svr_addr` |
| 短连接去哪里 | `set_shortlink_svr_addr` | `setShortLinkServerAddress` | `setShortlinkSvrAddr` | `mars_stn_set_shortlink_svr_addr` |
| 强制连一次 | `make_sure_long_link_connected` | `makeSureLongLinkConnected` | `makesureLongLinkConnected` | `mars_stn_makesure_longlink_connected` |
| 保活 | `keep_signalling` / `stop_signalling` | `keepSignalling` / `stopSignalling` | `keepSignalling` / `stopSignalling` | `mars_stn_keep_signalling` / `mars_stn_stop_signalling` |
| 现在就发一个 noop | — | `triggerNooping` | `trigNooping` | `mars_stn_trig_nooping` |
| 丢掉重来 | `reset` / `reset_with_encoder` | `reset` / `resetAndInitEncoderVersion` | `reset` / `resetAndInitEncoderVersion` | `mars_stn_reset` / `mars_stn_reset_and_init_encoder_version` |

网络变了、应用进后台了、进程要走了 —— 这些都要告诉 STN：Android 上通过 `BaseEvent`
（`onNetworkChange`、`onForeground`），其余平台通过 `reset` 和上面那些地址。

## 接着看

- [网络诊断](/zh/sdt) —— mars 这边的另一半，也是某个 host 不再回答时 STN 会调的那一个。
- [你那个平台](/zh/platforms/rust) —— 完整的 API 面，xlog 也在里面。
