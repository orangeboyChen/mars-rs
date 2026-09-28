# The task pipeline (STN)

STN is the half of mars that talks to a server. A **task** is one unit of work —
a request and its answer — and STN is what holds it: the queue it waits in, the
link it goes out on, the retry, the timeout, the DNS, and the report of how it
went. xlog is the other half, and the two do not depend on each other: an app
that only logs takes [the logger](/log-files) and none of this.

Two links, and a task names the one it wants:

| | a **short link** | a **long link** |
|---|---|---|
| what it is | one request, one answer | one connection the app keeps |
| what it carries | a task you started | your tasks, the server's pushes, the noop that keeps the connection up |
| what mars calls it | `CHANNEL_SHORT` | `CHANNEL_LONG`, `CHANNEL_MINOR_LONG` |

`Task::new` asks for both (`CHANNEL_BOTH`), which is what you want until you have
a reason to pick: STN sends the task on the first link that is up.

## Where it is

| your app is | what carries STN | how you reach it |
|---|---|---|
| Rust | `marsrs` (`marsrs-xlog` has none of it) | [`marsrs::stn`](https://docs.rs/marsrs) — see [Rust](/platforms/rust#the-task-pipeline) |
| Swift | the `MarsRSNet` product, or `MarsRS` | `MarsStn` — see [SwiftPM](/platforms/swift#the-task-pipeline) |
| Android, Kotlin or Java | `marsrs` on JitPack, not `xlog` | `io.github.orangeboychen.marsrs.stn.StnLogic` — see [Android](/platforms/android#the-task-pipeline) |
| Kotlin Multiplatform | `marsrs-kmp`, not `xlog-kmp` | `io.github.orangeboychen.marsrs.stn.StnLogic` — see [Kotlin Multiplatform](/platforms/kotlin-multiplatform#the-task-pipeline) |
| anything with a C FFI | `include/mars_stn.h` | `mars_stn_*` — see [the C ABI](/platforms/c-abi#the-task-pipeline) |
| HarmonyOS | the same `libmars_ffi.so` | `mars_stn_*` through a NAPI shim of your own — see [HarmonyOS](/platforms/harmonyos#the-mars-half) |

The Flutter plugin and the React Native module are the logger and nothing else
today: neither names a task, and neither will until the Dart and the TypeScript
below them do — see [Flutter](/platforms/flutter#what-is-not-in-it) and
[React Native](/platforms/react-native#what-is-not-in-it).

## Three things, in this order

**1. Install the app.** STN asks the app eighteen questions while a task runs —
*is this user logged in?*, *what bytes does this task send?*, *what did the
answer mean?* — and the app answers them. Rust, Swift and C funnel all eighteen
through one trait, one closure and one function pointer, each with a default for
every question, so an app that cares about one of them writes one. Android's
`StnLogic.ICallBack` is the fourteen the C++'s Java interface declares, and it is
a plain interface: all fourteen are the app's. The shared Kotlin of
`marsrs-kmp` is one `ask` — a question in, an answer out — and it is asked all
eighteen on Kotlin/Native and thirteen on Android: the five the JNI bridge
answers itself — the two network errors, the long link's status change, the task
limit and the DNS profile — never reach the app there.

**2. Start a task.** `start_task` takes it, picks a link and returns at once: the
task runs on the queue, not on the calling thread.

**3. Drive the queue.** This is the one thing this port asks of the caller that
the C++ did not. The C++ runs the queues and the long links on threads of its
own; this port has no threads in it, so what would have been a thread is a call
the host makes — `run_pending()`, and `due_time()` to know how long it may wait
before making it. A task that is started and never drained stays in its queue.

::: code-group

```rust [Rust]
use marsrs::stn::{gen_task_id, App, ErrCmdType, StnLogic, Task, TaskFailHandleType};

// 1. the app: every question has a default, so this is the whole of one that
//    encodes a request and reads an answer.
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
        Ok(b"...".to_vec()) // Err(code) ends the task with that code
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
stn.create(); // builds the net core; nothing above works before this

// 2. one task
let mut task = Task::new(gen_task_id(), 100);
task.cgi = "/cgi-bin/hello".to_owned();
task.shortlink_host_list = vec!["example.com".to_owned()];
task.total_timeout = 10_000;
stn.start_task(task);

// 3. the run loop: `due_delay()` is how long the pass may wait
while let Some(wait) = stn.due_delay() {
    std::thread::sleep(std::time::Duration::from_millis(wait));
    stn.run_pending();
}
```

```swift [Swift]
import MarsRSNet

// 1. the app is one closure: a question in, an answer out
MarsStn.setApp { question in
    switch question.kind {
    case .req2Buf:  return .encoded(try! encode(question.task!))
    case .buf2Resp: handle(question.body); return .decoded(errorCode: 0, handle: .normal)
    case .onTaskEnd: return .ended(errorCode: 0)
    default:        return .nothing
    }
}

// 2. one task
var task = StnTask(channelSelect: .short)
task.taskID = MarsStn.generateTaskID()
task.cgi = "/cgi-bin/hello"
task.shortLinkHosts = ["example.com"]
task.totalTimeout = 10_000
MarsStn.start(task)

// 3. the run loop — `dueTime` is how many milliseconds the pass may wait,
//    and it is `nil` when there is nothing to wait for
while let wait = MarsStn.dueTime {
    Thread.sleep(forTimeInterval: Double(wait) / 1000)
    MarsStn.runPending()
}
```

```kotlin [Android]
// 1. the app, and the platform it asks about
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
    // … and the other eleven: `ICallBack` is an interface with no defaults,
    //    so this object is the app's to finish
})

// 2. one task
val task = StnLogic.Task(
    StnLogic.Task.E_BOTH, 100, "/cgi-bin/hello", arrayListOf("example.com")
)
task.totalTimeout = 10_000
StnLogic.setShortlinkSvrAddr(443)
StnLogic.startTask(task)

// 3. the run loop — `dueTime()` is how many milliseconds the pass may wait,
//    and it answers -1 when there is nothing to wait for
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

// 1. the app is one `ask`: a question in, an answer out
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

// 2. one task
val task = Task().apply {
    taskID = StnLogic.genTaskID()
    channelSelect = Task.E_BOTH
    cmdID = 100
    cgi = "/cgi-bin/hello"
    shortLinkHostList = listOf("example.com")
    totalTimeout = 10_000
}
StnLogic.startTask(task)

// 3. the run loop — `dueTime()` is how many milliseconds the pass may wait,
//    and it answers `null` when there is nothing to wait for
var due = StnLogic.dueTime()
while (due != null) {
    StnLogic.runPending()
    due = StnLogic.dueTime()
}
```

```c [C]
#include <mars_stn.h>

// 1. the app is one C callback
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

// 2. one task
MarsStnTask task;
memset(&task, 0, sizeof task);
task.taskid = mars_stn_gen_task_id();
task.cmdid = 100;
task.channel_select = 0x3;              /* both */
task.cgi = "/cgi-bin/hello";
const char *hosts[] = { "example.com" };
task.shortlink_host_list.items = hosts;
task.shortlink_host_list.count = 1;
if (mars_stn_start_task(&task) != MARS_STN_OK) { /* refused or panicked */ }

/* 3. the run loop — the answer is how many milliseconds the pass may wait,
   and MARS_STN_ERR_NO_DUE is what "nothing to wait for" is */
long long due = mars_stn_due_time();
while (due >= 0) {
    usleep(due * 1000);
    mars_stn_run_pending();
    due = mars_stn_due_time();
}
```

:::

::: warning Nothing drives the queue for you
`run_pending` / `due_time` — in every spelling above — is what moves a task out
of its queue, and no platform here calls it on a thread of its own: the C++ has
a message-queue thread and a `__RunOn` thread, and this port has neither. Every
platform that carries STN exposes the pair, so the loop is the app's to write,
and a task that is started and never drained sits in its queue until the process
ends — `has_task` answers `true` for a task that is going nowhere.

`due_time` — in every spelling above — is how long the host may wait until the
next pass is due, in milliseconds: `0` is a pass that is already due, which a
follow-up waiting in the queue is. It is a duration and not a time of day, which
is what a caller across an ABI needs: a tick is measured from an origin only this
process can read, so the host is told the wait instead. What it is not is a
promise — a loop that calls `run_pending` in a tight spin works too, and burns a
core doing it. The Rust half spells the two apart: `StnLogic::due_time` is the
`gettickcount()` reading, which a caller in the same process compares against its
own clock, and `due_delay` is the wait. Every other platform answers the wait and
nothing else.
:::

## The task

A task is one struct, and every field has the default the C++ gives it. The ones
an app sets:

| what it is | Rust | Swift | Android, KMP | C |
|---|---|---|---|---|
| the id you stop it by | `taskid` | `taskID` | `taskID` | `taskid` |
| what the server calls it | `cmdid` | `cmdid` | `cmdID` | `cmdid` |
| the path it goes to | `cgi` | `cgi` | `cgi` | `cgi` |
| which link(s) | `channel_select` | `channelSelect` | `channelSelect` | `channel_select` |
| the hosts to try | `shortlink_host_list`, `longlink_host_list` | `shortLinkHosts`, `longLinkHosts` | `shortLinkHostList` | `shortlink_host_list` |
| how long, in ms | `total_timeout` | `totalTimeout` | `totalTimeout` | `total_timeout` |
| how many retries | `retry_count` | `retryCount` | `retryCount` | `retry_count` |
| headers | `headers` | `headers` | `headers` | `headers` / `header_count` |

`Task::new` in Rust — and `StnTask(channelSelect:)` in Swift, and `Task()` in
the shared Kotlin — draws the shape for you; `gen_task_id()` /
`MarsStn.generateTaskID()` / `StnLogic.genTaskID()` is there when you want an id
before the task.

## How a task ends

The app hears the end as a question — `onTaskEnd` in every spelling — carrying
two numbers and a profile:

- **an error type** — *where* it failed: `ErrCmdType` in Rust, `errType` on the
  others. `Ok` (0), or `Dns`, `Socket`, `Http`, `Server`, `Local`, `Canceld` …
- **an error code** — *what* went wrong, and it is negative: `-500` a first
  packet that never came, `-501` a gap between packets, `-502` a read or a write
  that timed out, `-503` the task as a whole, `-10086` the network changed under
  it. Android names these on `StnLogic` (`FIRSTPKGTIMEOUT`, `TASKTIMEOUT`, …).
- **a profile** — the timings of the run: when the DNS started, when the connect
  finished, the rtt. `CgiProfile` in Rust and Kotlin, `StnQuestion.CgiProfile` in
  Swift, `MarsStnCgiProfile` in C.

`buf2Resp` answers with a **fail handle** as well as a code, which is how the app
tells STN what to do about a bad answer rather than only that it was one:
`Normal`, `RetryAllTasks`, `SessionTimeout`, `TaskEnd`, `TaskTimeout`.

## The long link

A long link is a connection, not a request, and it is the app's to keep up:

| what | Rust | Swift | Android, KMP | C |
|---|---|---|---|---|
| where it connects | `set_longlink_svr_addr` | `setLongLinkServerAddress` | `setLonglinkSvrAddr` | `mars_stn_set_longlink_svr_addr` |
| where the short link goes | `set_shortlink_svr_addr` | `setShortLinkServerAddress` | `setShortlinkSvrAddr` | `mars_stn_set_shortlink_svr_addr` |
| force a connect | `make_sure_long_link_connected` | `makeSureLongLinkConnected` | `makesureLongLinkConnected` | `mars_stn_makesure_longlink_connected` |
| the keepalive | `keep_signalling` / `stop_signalling` | `keepSignalling` / `stopSignalling` | `keepSignalling` / `stopSignalling` | `mars_stn_keep_signalling` / `mars_stn_stop_signalling` |
| send a noop now | — | `triggerNooping` | `trigNooping` | `mars_stn_trig_nooping` |
| throw it away and start over | `reset` / `reset_with_encoder` | `reset` / `resetAndInitEncoderVersion` | `reset` / `resetAndInitEncoderVersion` | `mars_stn_reset` / `mars_stn_reset_and_init_encoder_version` |

The network changed, the app went to the background, the process is going away —
STN is told, on Android through `BaseEvent` (`onNetworkChange`, `onForeground`),
on the others through `reset` and the addresses above.

## Where to go next

- [The network diagnosis](/sdt) — the other half of the mars side, and the one
  STN calls when a host stops answering.
- [Your platform](/platforms/rust) — the whole surface, xlog included.
