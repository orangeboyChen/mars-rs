# 任务

一个任务就是一个 struct，每个字段都有 mars 给它的默认值。App 会设的是这些：

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

Rust 的 `Task::new`、Swift 的 `StnTask(channelSelect:)`、共享 Kotlin 的 `Task()`
会把这个形状给你画好，所以 App 要说的是路径、hosts 和超时。想先拿到 id 的时候有
`gen_task_id()` / `MarsStn.generateTaskID()` / `StnLogic.genTaskID()`。

通道是 App 第一个要说的字段：`channel_select` 为 `0` 的任务哪也去不了，因为 net
core 一见这个值就判失败 —— 上面三个里只有 Rust 的 `Task::new` 会填上它，填的是
`CHANNEL_BOTH`。`StnTask` 一开始一条通道都没有，共享 Kotlin 的 `Task()` 是 `0`，也
就是 C++ 的 `Task::Task()` 给的值，它的 Java 让 App 自己设的也是这个值，所以这两处
要 App 自己说：`MarsStn.Channel.both`、`Task.E_BOTH`，Android 上则是
`Task(channelselect, cmdid, cgi, hostList)` 的第一个参数。

`need_authed` 在 Rust 和 Swift 里是 `true`，在 Android 那个四参数的 `Task` 里也是
—— C++ 项目自己的 Java 在那里同样回答 `true`。App 用无参 `Task()` 建出来的那个把它
留在 `false`。

## 发起、停下，以及它还在不在

| 什么 | Rust | Swift | Android、KMP | C |
|---|---|---|---|---|
| 发起 | `stn.start_task(task)` | `MarsStn.start(task)` | `StnLogic.startTask(task)` | `mars_stn_start_task(&task)` |
| 发起并 await 它的回答 | `stn.send(task, body)` | — | — | — |
| 停下 | `stn.stop_task(id)` | `MarsStn.stop(taskID:)` | `StnLogic.stopTask(id)` | `mars_stn_stop_task(id)` |
| 还在不在 | `stn.has_task(id)` | `MarsStn.hasTask(id)` | `StnLogic.hasTask(id)` | `mars_stn_has_task(id)` |

`start_task` 立刻返回：任务跑在队列上，不在调用它的线程上。`has_task` 对一个哪儿
也去不了的任务也回答 `true` —— 一个发起了却一直没排空的任务。

`send` 是 Rust 才有的，想要任务回答的时候就用它：它发起任务，并交回一个 `Sent` ——
一个 future，输出是这个任务的回答，或者这一趟的失败。`body` 是这个任务要发的字节，
也就是本来要问 `req2buf` 的东西；`send_at(now, task, body)` 是同一个调用，只是时钟
读数由你递进去。Rust 里 `start_task` 让位给 `send`，带着 deprecated；`start_task_at`
不带，其他平台的 `start` 也不带 —— 那是上游的调用，给没人 await 的任务用的。

## 一个任务怎么结束

结束是 App 听到的一个问题 —— 每种写法里都叫 `onTaskEnd` —— 带着两个数字和一个
profile：

- **一个错误类型** —— 失败*在哪*：Rust 里是 `ErrCmdType`，其余是 `errType`。`Ok`
  （0），或者 `Dns`、`Socket`、`Http`、`Server`、`Local`、`Canceld` …
- **一个错误码** —— *出了什么*问题，它是负的：`-500` 第一个包一直没来，`-501` 包
  之间有 断档，`-502` 一次读或写超时，`-503` 整个任务超时，`-10086` 网络在它底下变了。Android 把这些名字放在 `StnLogic` 上（`FIRSTPKGTIMEOUT`、`TASKTIMEOUT` …）。
- **一个 profile** —— 这一趟的耗时：DNS 什么时候开始、连接什么时候建完、rtt。Rust
  和 Kotlin 里是 `CgiProfile`，Swift 里是 `StnQuestion.CgiProfile`，C 里是
  `MarsStnCgiProfile`。

`buf2Resp` 除了一个 code 还要回答一个 **fail handle**，这是 App 告诉 STN 对一个坏
回答该怎么办、而不只是说它是坏的：`Normal`、`RetryAllTasks`、`SessionTimeout`、
`TaskEnd`、`TaskTimeout`。

### 改成 await 它，在 Rust 里

在 Rust 里，一个任务除了被听见，还可以被 await。`send` 交回的是一个 `Sent`，它的
`Output` 是：

- **`Ok(Answer)`** —— `body`，服务器回答的那些字节，也就是本来要递给 `buf2Resp` 的
  东西；以及 `profile`，这次连接的耗时。
- **`Err(Failure)`** —— 为什么没有回答。`NotCreated` 是还没有 net core：
  `create()` 还没走。`Refused` 是一个已经释放的 core，它什么都不启动、也什么都不
  上报。`Ended { err_type, err_code, profile }` 是失败在哪、用的什么错误码 —— 就是
  `on_task_end` 带着的那两个数 —— 一个在入口就被 core 拒掉的任务也是这么报的。

一个 `Sent` 不借任何东西，所以它可以在 App 自己的 executor 放到哪就在哪 await，而
那个 logic 底下的锁要在 `.await` 之前放开：把锁握过 `.await`，就是卡住了本来要回答
它的那一趟。一个没人排空的 `Sent` 一直是 `Pending`，就像一个发起了却从没排空的任务
一直待在队列里。

那些问题并没有停止：一个被 await 的任务跑着的时候，App 照旧被问到每一个问题，包括
`on_task_end` —— await 是 App 拿到的一个值，不是一个 App 从此不再被问的问题。会落回
App 的，是一个请求和它的回答本来要问的那两个 —— `req2buf` 和 `buf2resp` —— 在没人
await 的任务上。见[那些问题](/zh/stn/callbacks)。

## 一个任务跑在什么上面

没有谁替你排空队列。`run_pending` / `due_time` —— [快速开始](/zh/stn/getting-started)
那页上每种写法里都有 —— 就是把任务从队列里挪出去，而 App 不要求就一个线程也不起：
那个循环是 App 的，一个发起了却一直没排空的任务会一直坐在队列里，直到进程结束。

在 Rust 里一个调用就够了：`Driver::spawn(stn)` 起这个 crate 的一个线程，做的正是
宿主那个循环做的事 —— 到点了 `run_pending()`，其间睡 `due_delay()` —— `Driver`
被 drop 的时候这个线程被 join。那个 logic 是共享的、不是被搬走的，所以 App 留着自己
的 `Arc`，继续通过它发任务。已经有 `run_pending` 循环的宿主留着它就好，不需要
`Driver`：一趟把某个任务跑完的时候，await 它的人会被唤醒，所以一个 `Driver` 和宿主
自己的循环可以一起用。

`due_time` 是距离下一趟还有多久，单位是毫秒：`0` 是已经到期的一趟，队列里等着的一
个 follow-up 就是。它是一个时长，不是一个时刻，跨 ABI 的调用方要的正是这个：tick
的原点只有本进程读得到，所以给宿主的只是还要等多久。它也不是什么承诺 —— 一个不停
空转调 `run_pending` 的循环照样能工作，只是会白烧掉一个核。

在 Android 和共享 Kotlin 上，只有在这个循环里 App 才看得到两座桥对一个形状意见不
一致：`StnLogic.dueTime()` 回答 `Long?` —— 没有可等的东西时是 `null` —— 因为 JNI
桥回答 `-1`，而 C ABI 回答它自己的 `MARS_STN_ERR_NO_DUE`。
