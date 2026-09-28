# 从 mars 迁移

已经在用 C++ 实现的应用要搬三样东西：日志库、任务链路、网络诊断。三样这里都有，用的
是这个移植写的名字；而日志库写出来的就是 C++ 写出来的那个文件 —— 同样的帧结构、同样
的压缩、同样的加密 —— 所以迁移不丢任何历史：你已经收集到的 `.xlog` 文件，现有的工具
照样能读。

要花力气的是两件事：调用改成了各平台自己的写法，另外 C++ 自己收在里面的两块，在这里
归应用自己。下面先说这两块，然后一块一章。

## 三块分别对应什么

| 你在用的 | 这里用什么 | 写在哪一页 |
|---|---|---|
| `mars/xlog` —— 日志库 | `xlog`：crates.io 上的 `marsrs-xlog`、JitPack 上的 `xlog`、共享 Kotlin 模块的 `xlog-kmp`、Apple 上的 `MarsRSXlog` | [快速开始](/zh/getting-started) |
| `mars/stn` —— 任务链路 | `marsrs`，以及它的两个孪生包 `marsrs-kmp` 和 `MarsRSNet` | [任务链路](/zh/stn) |
| `mars/sdt` —— 网络诊断 | 同一个 `marsrs`，和链路在同一个包里 | [网络诊断](/zh/sdt) |

这个切分是每个平台都做的那个切分：一个包只有日志库，另一个包是整个移植，而整个移植
里带着的是同一个日志库 —— 只有名字不同。只打日志的应用用前者，还要跑任务或诊断的用
后者；拿不准的从日志库开始：`marsrs` 是日志库加另外两块，所以从一个包换到另一个包，
文件这件事什么都不用改。

## 应用接手的两件事

**队列是靠调用排空的，不是靠线程。** 任务和诊断都跑在队列上，C++ 用自己
的线程跑这些队列 —— 一个消息队列线程，以及诊断跑在其上的 `__RunOn` 线程。这个
移植里没有线程，所以本来会是线程的东西是宿主调的一次调用：任务用 `run_pending()`，
诊断用 `runChecks`，两个都带一个答案 —— *我下一次调用最多能等多久*。整个东西就是
一个循环：

::: code-group

```rust [Rust]
while let Some(wait) = stn.due_delay() {      // 这一趟最多能等多久，毫秒
    std::thread::sleep(std::time::Duration::from_millis(wait));
    stn.run_pending();
}
```

```kotlin [Android]
var due = StnLogic.dueTime()                  // -1 是“没什么要等的”
while (due >= 0) {
    Thread.sleep(due)
    StnLogic.runPending()
    due = StnLogic.dueTime()
}
```

:::

上面两种写法是[任务链路](/zh/stn)的；诊断是同一个形状，只是没有 sleep —— `runChecks`
会去问它的探针，问完才返回，所以应用跑一次就可以拿报告。

**socket 是应用自己的。** 短连接、长连接，以及诊断的每一个探针，都是 socket，而这个
移植一个都不拥有：C++ 把自己的网络实现链进了各个检查器里，这里 socket 是什么归调用
方。诊断会一个一个地问你要一次解析、一次连接、一次 HTTP 请求、一次 ICMP echo，你的
答案就是它记进报告的结果 —— 这也是为什么一台没有无线电的机器上也能跑一次诊断。任务
的连接在 Rust 里是同样接的，通过创建连接时给的那个 `SocketOperator`；Rust 之外各绑
定还没有给它留接缝，所以今天就走到哪一步，是[任务链路](/zh/stn)那页说的。

## 迁移日志库

appender 是同一个东西：每个前缀一个进程级写入器，应用启动时开一次，往里写，读文件或
上传前 flush。变的是谁拿着它 —— C++ 在一组自由函数背后装一个进程级 appender，这里
你拿着自己打开的那个 `Xlog`，通过它写。

### 从 C++ 头文件来

appender 在 `mars/xlog/appender.h` 里是一组选项构成的一个 struct 加几个自由函数，在
这里也是一个 struct 加几个调用：

| C++ | Rust | C ABI |
|---|---|---|
| `appender_open(const XLogConfig&)` | `appender_open(XLogConfig)` | `mars_xlog_open(&config)` |
| `xlogger_Write(info, log)`，或 `xinfo2` 那一族 | `appender_write(info, message)` | `mars_xlog_write(...)` |
| `appender_flush()` | `appender_flush()` | `mars_xlog_flush()` |
| `appender_flush_sync()` | `appender_flush_sync()` | `mars_xlog_flush_sync()` |
| `appender_close()` | `appender_close()` | `mars_xlog_close()` |
| `xlogger_SetLevel(level)` | `set_level(handle, level)` | `mars_xlog_set_level(level)` |
| `appender_setmode(mode)` | `appender_set_mode(mode)` | `mars_xlog_set_mode(mode)` |
| `appender_set_console_log(bool)` | `appender_set_console_log(bool)` | `mars_xlog_set_console_log(on)` |
| `appender_set_max_file_size(bytes)` | `appender_set_max_file_size(bytes)` | `mars_xlog_set_max_file_size(bytes)` |
| `appender_set_max_alive_duration(secs)` | `appender_set_max_alive_duration(secs)` | `mars_xlog_set_max_alive_duration(secs)` |
| `appender_get_current_log_path(out, len)` | `appender_get_current_log_path()` | `mars_xlog_current_log_path(out, len)` |

config 在三种写法里都是一个 struct，八个字段还是那八个：C++ 里是 `mode_`、
`logdir_`、`nameprefix_`、`pub_key_`、`compress_mode_`、`compress_level_`、
`cachedir_`、`cache_days_`，这里是 `mode`、`logdir`、`nameprefix`、`pub_key`、
`compress_mode`、`compress_level`、`cachedir`、`cache_days`。每一个都在
[配置项](/zh/configuration)那页，按每个平台的写法列着。`TAppenderMode` 是
`AppenderMode`，`TCompressMode` 是 `CompressMode`，`TLogLevel` 是 `LogLevel`。

另一半要搬走的是宏。`XLOGGER_TAG` 和 `xverbose2` / `xdebug2` / `xinfo2` / `xwarn2` /
`xerror2` / `xfatal2` 那一族，把级别、tag 和调用点塞在一行 C++ 里；取代它们的是每个
级别一个方法 —— Kotlin 的 `xlog.i(tag, message)`、Swift 的 `log.info(message:tag:)`、
Rust 的 `appender_write(None, message)` —— 文件、函数和行号从调用点取，不用调用方写。

第二个 appender 是形状差别最大的地方。C++ 项目的 Java 是用
`Log.openLogInstance(level, mode, cacheDir, logDir, nameprefix, cacheDays)` 开第二个，
然后把它返回的 handle 一路传下去；这里你拿着一个 appender 就*是*拿到一个实例，所以第
二个就是自己的另一个前缀的第二个 `Xlog`，或者 Rust 里的 `*_instance` 那一族 ——
`appender_open_instance(config)` 返回 handle，`appender_write_instance`、
`appender_flush_instance` 和 `appender_close_instance` 接它。

### 从 C++ 项目的 Java 来

Android 包是唯一还留着旧写法的地方：七个参数的 `Xlog.open`、`XLogConfig`、
`XLoggerInfo`、`logWrite` 和 `LEVEL_*` 常量都还能用，而且每一个都带着取代它的写法标记
为 deprecated。`Log.setLogImp(Xlog())` 和 `Log.d(tag, message)` 仍然写进
`Xlog.open` 装上的那个 appender，所以迁移可以一个调用点一个调用点地走：

```kotlin
// 之前
Log.setLogImp(Xlog())
Log.d("net", "…")

// 之后
val xlog = Xlog.open(XlogConfig(logDir = dir, namePrefix = "marsrs"))
xlog.d("net", "…")
```

新写法多给一样东西：`Context`。`Xlog.open(config, context)` 会在应用离开屏幕时自己
flush，那是 Android 在可以不打招呼就结束进程之前最后一个还会说话的时刻 —— 见
[Android](/zh/platforms/android#when-the-app-goes-away)。

### 从 Apple 的头文件来

没有 Objective-C 封装要搬：Apple 上的应用调的是 Objective-C++ 文件里的 C++ ——
`xlogger_SetLevel`、`appender_set_console_log`、一个字段一个字段填的 `XLogConfig`，
以及 `appender_open(config)`。那个文件变成导入模块、拿着自己打开的 appender 的文件：

::: code-group

```objc [之前]
XLogConfig config;
config.mode_ = kAppenderAsync;
config.logdir_ = [logPath UTF8String];
config.nameprefix_ = "Test";
config.pub_key_ = "";
config.compress_mode_ = kZlib;
config.compress_level_ = 0;
config.cachedir_ = "";
config.cache_days_ = 0;
appender_open(config);
```

```objc [之后]
@import MarsRSXlog;

XlogConfig *config = [[XlogConfig alloc] initWithLogDirectory:logPath.path];
config.namePrefix = @"marsrs";

NSError *error = nil;
Xlog *log = [[Xlog alloc] initWithConfig:config error:&error];
[log writeWithLevel:LogLevelInfo message:@"cold start" tag:@"startup"];
[log flushWithSync:YES];        // 读文件或上传前
```

:::

Swift 会在调用点填上文件、函数和行号；Objective-C 没有 `#file` 可填，所以这里写的记录
里文件是空的、行号是 0，除非用长形式把它们写上 ——
`[log log:message:tag:file:function:line:]`。

取代那个头文件的是 `MarsRSXlog` —— 一个 SwiftPM product 和一个同名的 pod，同一个
framework，下面是 `@objc` 的接口。[SwiftPM](/zh/platforms/swift)和
[CocoaPods](/zh/platforms/cocoapods)是它的两页。

## 迁移任务链路

任务是同一个 struct，默认值也一样 —— id、命令、路径、要走的链路、主机、超时、重试
—— 跑的过程中应用回答的还是那些问题。变的是答案怎么到 STN，以及谁来排空队列。

| C++ | Rust | Android | Swift | C |
|---|---|---|---|---|
| `mars::stn::StartTask` | `stn.start_task(task)` | `StnLogic.startTask(task)` | `MarsStn.start(task)` | `mars_stn_start_task(&task)` |
| `mars::stn::StopTask` | `stn.stop_task(id)` | `StnLogic.stopTask(id)` | `MarsStn.stop(taskID:)` | `mars_stn_stop_task(id)` |
| `mars::stn::HasTask` | `stn.has_task(id)` | `StnLogic.hasTask(id)` | `MarsStn.hasTask(id)` | `mars_stn_has_task(id)` |
| `mars::stn::SetCallback` | `stn.set_callback(app)` | `StnLogic.setCallBack(cb)` | `MarsStn.setApp { … }` | `mars_stn_set_app(ctx, ask)` |
| `mars::stn::MakesureLonglinkConnected` | `stn.make_sure_long_link_connected("default")` | `StnLogic.makesureLongLinkConnected()` | `MarsStn.makeSureLongLinkConnected()` | `mars_stn_makesure_longlink_connected()` |
| 队列的那个线程 | `stn.run_pending()` | `StnLogic.runPending()` | `MarsStn.runPending()` | `mars_stn_run_pending()` |

C++ 作为 `mars::stn::Callback` 的虚函数问的那十八个问题，在 Rust 里是一个 `App` trait，
在 Kotlin 里是一个 `ICallBack`，在 Swift 里是一个闭包；三个都为应用不回答的每个问题
准备了默认值 —— 只有 Android 的不是，它是 C++ 项目的 Java 声明的那个普通 interface，
所以那个对象要应用自己写完。名字还是 C++ 用的那些：任务要发的字节是 `req2Buf`，回来
的答案是 `buf2Resp`，结束是 `onTaskEnd`，服务器顺着长连接推下来的是 `onPush`，一个
主机的地址是 `onNewDns`。

任务怎么结束没变：一个说*在哪*失败的 error **type** —— Rust 的 `ErrCmdType`，其他
地方的 `errType`；一个说*出了什么事*的 error code；以及一份耗时 profile。Android 像
以前一样把那些 code 命名在 `StnLogic` 上。

Android 上的启动也一样：`Mars.init(context, handler)` 和 `Mars.onCreate(true)`
开局，屏幕或网络变了用 `BaseEvent.onForeground` 和 `BaseEvent.onNetworkChange`，
STN 要问的账号和设备用 `AppLogic.setCallBack` —— 还是 C++ 项目的 Java 写的那些名字，
只是放在这个移植的包里。

而队列是上面说的那个新义务：一个启动了却从来没被排空的任务会一直留在它的队列里。
[任务链路](/zh/stn)是它的全部 —— 两条链路、任务的字段、以及长连接要应用做什么。

## 迁移网络诊断

一次诊断在 C++ 里和在这里是同样的三步：给出主机和要做的检查，跑，拿报告。

| 什么 | C++ | Rust | Android、共享 Kotlin | Swift | C |
|---|---|---|---|---|---|
| 主机和 mode | `StartActiveCheck` | `start_active_check(…)` | `startActiveCheck(…)` | `startActiveCheck(…)` | `mars_sdt_start_active_check(…)` |
| 跑这个计划 | `__RunOn` 线程 | `run_checks(&mut ask, net)` | `runChecks(net, probe)` | `runChecks(networkType:probe:)` | `mars_sdt_run_checks(ctx, probe, net)` |
| 停掉它 | `CancelActiveCheck` | `cancel_active_check()` | `cancelActiveCheck()` | `cancelActiveCheck()` | `mars_sdt_cancel_active_check()` |
| net-check 的 CGI | `SetHttpNetcheckCGI(cgi)` | `set_http_netcheck_cgi(cgi)` | `setHttpNetcheckCGI(cgi)` | `setHTTPNetCheckCGI(cgi)` | `mars_sdt_set_http_netcheck_cgi(cgi)` |
| 报告 | `Callback::ReportNetCheckResult` | `report_json(&results)` | `takeReport()`，或回调 | `takeReport()` | `mars_sdt_take_report(buf, len)` |

mode 还是那几个 bit —— ping 和 DNS，然后 TCP，然后是 net-check CGI 的 HTTP 检查 ——
报告还是同一份 `{"details":[ … ]}` 文档，每项检查一个对象，里面还是同样的
`detectType`、`errorCode` 和各项耗时，所以今天读报告的东西也读得懂这一份。

归应用的是它下面的网络：C++ 把自己的 socket 链进了检查器，这里一次检查是这一轮问你的
一个问题 —— 一个要解析的域名、一个要连的 ip 和端口、一个要取的 URL、一个要 ping 的
主机 —— 你的答案就是报告里记的东西。在 Android 和共享 Kotlin 里那是你交给
`runChecks` 的 `IProbe`；在 Rust 里是 `Ask`；在 Swift 和 C 里是那个闭包。

[网络诊断](/zh/sdt)是它的全部：mode、跑之前可以先看的计划，以及报告的每个字段。

## 接下来

- [快速开始](/zh/getting-started) —— 每个平台的依赖和一段能跑的代码。
- [配置项](/zh/configuration) —— appender 的每个配置项和默认值，按每个平台的写法。
- [日志文件](/zh/log-files) —— 文件落在哪、怎么读回来，给上传路径已经认识 C++
  那个文件的应用。
