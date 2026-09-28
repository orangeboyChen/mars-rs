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

两个默认值是 C++ 项目自己的 Java 给它的 `Task` 的那两个：`channel_select` 是
`CHANNEL_BOTH` 而不是 `0` —— 后者会被 net core 判为失败 —— 以及 `need_authed` 是
`true`。

## 发起、停下，以及它还在不在

| 什么 | Rust | Swift | Android、KMP | C |
|---|---|---|---|---|
| 发起 | `stn.start_task(task)` | `MarsStn.start(task)` | `StnLogic.startTask(task)` | `mars_stn_start_task(&task)` |
| 停下 | `stn.stop_task(id)` | `MarsStn.stop(taskID:)` | `StnLogic.stopTask(id)` | `mars_stn_stop_task(id)` |
| 还在不在 | `stn.has_task(id)` | `MarsStn.hasTask(id)` | `StnLogic.hasTask(id)` | `mars_stn_has_task(id)` |

`start_task` 立刻返回：任务跑在队列上，不在调用它的线程上。`has_task` 对一个哪儿
也去不了的任务也回答 `true` —— 一个发起了却从没被排空的那个。

## 一个任务怎么结束

App 是把结束当一个问题听到的 —— 每种写法里都叫 `onTaskEnd` —— 带着两个数字和一个
profile：

- **一个错误类型** —— 失败*在哪*：Rust 里是 `ErrCmdType`，其余是 `errType`。`Ok`
  （0），或者 `Dns`、`Socket`、`Http`、`Server`、`Local`、`Canceld` …
- **一个错误码** —— *出了什么*问题，它是负的：`-500` 第一个包一直没来，`-501` 包
  之间有  断档，`-502` 一次读或写超时，`-503` 整个任务超时，`-10086` 网络在它底下变了。Android 把这些名字放在 `StnLogic` 上（`FIRSTPKGTIMEOUT`、`TASKTIMEOUT` …）。
- **一个 profile** —— 这一趟的耗时：DNS 什么时候开始、连接什么时候建完、rtt。Rust
  和 Kotlin 里是 `CgiProfile`，Swift 里是 `StnQuestion.CgiProfile`，C 里是
  `MarsStnCgiProfile`。

`buf2Resp` 除了一个 code 还要回答一个 **fail handle**，这是 App 告诉 STN 对一个坏
回答该怎么办、而不只是说它是坏的：`Normal`、`RetryAllTasks`、`SessionTimeout`、
`TaskEnd`、`TaskTimeout`。

## 一个任务跑在什么上面

没有谁替你排空队列。`run_pending` / `due_time` —— [快速开始](/zh/stn/getting-started)
那页上每种写法里都有 —— 是把任务从队列里挪出去的东西，而这个移植不会起自己的线程
去调它们：那个循环是 App 的，一个发起了却从没被排空的任务会一直坐在队列里，直到
进程结束。

`due_time` 是距离下一趟还有多久，单位是毫秒：`0` 是已经到期的一趟，队列里等着的一
个 follow-up 就是。它是一个时长，不是一个时刻，这正是跨 ABI 的调用方需要的：tick 从一个只有本进程读得到的原点起算，所以给宿主的是还要等多久。它也不是什么承诺 ——
一个不停空转调 `run_pending` 的循环照样能工作，只是会白烧掉一个核。

在 Android 和共享 Kotlin 上，这个循环是 App 唯一能看到两座桥对一个形状意见不一致
的地方：`StnLogic.dueTime()` 回答 `Long?` —— 没有可等的东西时是 `null` —— 因为
JNI 桥回答 `-1`，而 C ABI 回答它自己的 `MARS_STN_ERR_NO_DUE`。
