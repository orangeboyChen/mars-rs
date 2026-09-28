# Getting started

STN is the half of mars that talks to a server. A **task** is one unit of work —
a request and its answer — and STN is what holds it: the queue it waits in, the
link it goes out on, the retry, the timeout, the DNS, and the report of how it
went. xlog is the other half, and the two do not depend on each other: an app
that only logs takes [the logger](/xlog/getting-started) and none of this.

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
| Rust | `marsrs` (`marsrs-xlog` has none of it) | `marsrs::stn` |
| iOS / watchOS, Swift | the `MarsRSNet` product, or `MarsRS` | `MarsStn` |
| Android, Kotlin or Java | `marsrs` on JitPack, not `xlog` | `io.github.orangeboychen.marsrs.stn.StnLogic` |
| Kotlin Multiplatform | `marsrs-kmp`, not `xlog-kmp` | `io.github.orangeboychen.marsrs.stn.StnLogic` |
| anything with a C FFI | `include/mars_stn.h` | `mars_stn_*` |
| HarmonyOS | the same `libmars_ffi.so` | `mars_stn_*` through a NAPI shim of your own |

The Flutter plugin and the React Native module are the logger and nothing else
today: neither names a task. Their native side does carry STN on Android — the
whole-port AAR — so an app on either one starts a task from the platform's own
code and carries it across to the Dart or the JS itself.

## Three things, in this order

1. **Answer the questions.** STN asks the app eighteen of them while a task runs
   — *is this user logged in?*, *what bytes does this task send?*, *what did the
   answer mean?* — and the app answers them: one trait in Rust, one closure in
   Swift, one `ask` in the shared Kotlin, one callback in C, and on Android the
   interface the app implements. See [the questions](/stn/callbacks).
2. **Start a task.** It is taken, given a link and put on the queue, and the call
   returns at once.
3. **Drive the queue.** This is the one thing this port asks of the caller. What
   would have been a thread is a pair of calls the app makes — `run_pending()`,
   and `due_time()` to know how long it may wait before making it. A task that is
   started and never drained stays in its queue.

## Rust

```bash
cargo add marsrs          # the whole port: xlog, stn and sdt
```

```rust
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

`stn.create()` builds the net core, and nothing works before it. `due_delay()`
answers how long the pass may wait in milliseconds, and `None` when there is
nothing to wait for; `StnLogic::due_time` is the tick reading that a caller in
the same process compares against its own clock.

A link is a model of a connection here and not one, so the socket is the app's:
a task's connect, send, receive and close are the `SocketOperator` a link is made
with, wired in Rust through the factory of the net core:

```rust
use marsrs::stn::{ShortLink, StnLogic};

let core = stn.net_core().expect("create() has been through");
core.factory().set_create_shortlink(|task, use_proxy| {
    let mut link = ShortLink::new(task, use_proxy);
    link.set_socket_operator(MySockets); // your connect / send / recv / close
    link
});
```

Everything around the socket is there — the queue, the choice of link, the DNS
ahead of the connect, the retry, the timeout, and the profile and the report at
the end.

## Swift

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")

// and, in the target that takes it:
.product(name: "MarsRSNet", package: "mars-rs")   // or MarsRS, for both halves
```

```swift
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

`MarsStn` is one `enum` of statics over the `mars_stn_*` of the C ABI, and the
pod is the same Swift over the same framework. Objective-C does not see it: a
Swift `enum` of static members is not a type Objective-C can import, so an app
that runs a task writes that part in Swift.

## Android

```kotlin
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen.marsrs:marsrs:0.1.0")   // xlog alone has none of it
```

```kotlin
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

`Mars.init` and `Mars.onCreate` start the two bridges, and `AppLogic.setCallBack`
is where the account and the device STN asks about come from — see
[the questions](/stn/callbacks). `ICallBack` is a plain interface with no
defaults, so the object is the app's to finish.

## Kotlin Multiplatform

```kotlin
// build.gradle.kts of the shared module
implementation("io.github.orangeboychen.marsrs:marsrs-kmp:0.1.0")   // not xlog-kmp
```

```kotlin
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

`StnLogic` is one `expect object` in `commonMain`, with one `actual` per platform
family — over the JNI bridge on Android, over the C ABI through cinterop on every
Kotlin/Native target. What a `common` declaration can only be is what both can
say: on Android the bridge answers five of the eighteen questions itself, so
`setApp` is asked thirteen there and all eighteen on Kotlin/Native.

## The C ABI {#c-abi}

```text
marsrs-<version>-<host>.tar.gz   (Linux, macOS)
marsrs-<version>-<host>.zip      (Windows)
    include/mars_stn.h      the task pipeline
    libmars_ffi.a / libmars_ffi.so (.dylib, .dll)
```

One archive per host — Linux, macOS and Windows — on the release of the version
you take. It is the same archive [the logger's section](/xlog/getting-started#c-abi)
publishes: one library, all three headers in it.

```c
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

Every `int` of the header answers `MARS_STN_OK` (0) or a negative
`MARS_STN_ERR_*`, and nothing in the C ABI unwinds into C.

## HarmonyOS

An app that takes `marsrs-harmonyos-xlog` gets the logger: no ArkTS of that
package reaches STN. What does carry it is the same `libmars_ffi.so`, built with
the `stn` feature on top of the default `xlog` — so an app that wants a task
takes `marsrs-harmony-<version>.tar.gz`, drops `libmars_ffi.so` under the
module's `libs/<abi>/`, and writes the NAPI shim that reaches `mars_stn.h`:

```c
#include <mars_stn.h>

mars_stn_set_app(NULL, ask);          /* the eighteen questions, as one callback */
mars_stn_start_task(&task);

/* the answer is how many milliseconds the pass may wait */
long long due = mars_stn_due_time();
while (due >= 0) {                    /* the app drains the queues */
    usleep(due * 1000);
    mars_stn_run_pending();
    due = mars_stn_due_time();
}
```

The C ABI on HarmonyOS is the one [its section](#c-abi) publishes for Linux,
macOS and Windows, and the one `MarsRSNet`, the Android AARs and `marsrs-kmp` are
written over.

## Where to go next

- [The task](/stn/tasks) — what a task is made of, and how one ends.
- [The questions](/stn/callbacks) — the eighteen the app answers while a task
  runs, and what each platform calls them.
- [The long link](/stn/long-link) — the connection an app keeps up, and what STN
  is told when the network or the screen changes.
