# Rust

```bash
cargo add marsrs          # 整个移植：xlog、stn、sdt
cargo add marsrs-xlog     # 只有 xlog —— 日志，别的都没有
```

`marsrs` 一个组件一个模块 —— `xlog`、`stn`、`sdt`、`comm`、`bytes` —— 前四个各有一个
feature，默认全开。只打日志的应用直接用 `marsrs-xlog`，或者把 `marsrs` 其余的关掉：

```toml
[dependencies]
marsrs = { version = "0.1", default-features = false, features = ["xlog"] }
```

## 打开、写、flush、关闭

```rust
use marsrs::xlog::{appender_close, appender_flush_sync, appender_open, appender_write, XLogConfig};

let mut config = XLogConfig::default();
config.logdir = std::path::PathBuf::from("/tmp/mars-log");
config.nameprefix = "marsrs".to_owned();
appender_open(config)?;

appender_write(None, "hello from mars");

appender_flush_sync();
appender_close();
```

`appender_open` 装一个进程级别的 appender，`appender_write` 就往它写。
`XLogConfig::default()` 是 `./log`、前缀 `Mars`、异步、zlib；每个选项都在
[配置项](/zh/configuration)那页。

一条记录可以带的比一条消息更多 —— `appender_write` 接一个 `XLoggerInfo`，
里面有级别、tag，以及调用处的文件、函数、行号；`None` 就是默认那份：

```rust
use marsrs::xlog::{appender_write, LogLevel, XLoggerInfo};

appender_write(
    Some(&XLoggerInfo {
        level: LogLevel::Info,
        tag: Some("startup".into()),
        filename: Some(file!().into()),
        func_name: Some("main".into()),
        line: line!() as i32,
        ..Default::default()
    }),
    "cold start in 412 ms",
);
```

## 一个前缀一个 appender

哪一块日志要单独读取，就给它一个自己的 appender —— 自己的目录、前缀和选项：

```rust
use marsrs::xlog::{
    appender_close_instance, appender_flush_instance, appender_open_instance,
    appender_write_instance,
};

let id = appender_open_instance(config)?;
appender_write_instance(id, None, "hello from the other appender");
appender_flush_instance(id, true);
appender_close_instance(id);
```

## 级别、控制台、轮转

```rust
use marsrs::xlog::{
    appender_get_current_log_path, appender_set_console_log, appender_set_max_alive_duration,
    appender_set_max_file_size, appender_set_mode, is_enabled_for, set_level, AppenderMode,
    LogLevel, DEFAULT_HANDLE,
};

set_level(DEFAULT_HANDLE, LogLevel::Info);      // 低于这个级别的记录会被丢掉
appender_set_mode(AppenderMode::Async);
appender_set_console_log(true);                 // 同时打到 stdout
appender_set_max_file_size(8 * 1024 * 1024);    // 到 8 MiB 就换文件
appender_set_max_alive_duration(10 * 24 * 3600); // 超过十天就删

if is_enabled_for(DEFAULT_HANDLE, LogLevel::Debug) {
    appender_write(None, &expensive_description());
}

let path = appender_get_current_log_path();
```

## 把文件读回来

```rust
use marsrs::xlog::{get_period_logs, LogBuffer};

let (begin, end) = get_period_logs(std::path::Path::new("marsrs_20260927.xlog"), 0, 24)?;
```

`LogBuffer` 就是解码器；见[日志文件](/zh/log-files)。

## 任务链路

```rust
use marsrs::stn::{gen_task_id, App, StnLogic, Task};

let mut stn = StnLogic::new();
stn.set_callback(MyApp);   // 一个 App 回答那十八个问题
stn.create();              // 建出 net core；在这之前别的都用不了

let mut task = Task::new(gen_task_id(), 100);
task.cgi = "/cgi-bin/hello".to_owned();
stn.start_task(task);

// 这个移植没有线程：队列由谁调用谁来排空
while let Some(wait) = stn.due_delay() {   // 这一趟还能等多久，毫秒
    std::thread::sleep(std::time::Duration::from_millis(wait));
    stn.run_pending();
}
```

`marsrs::stn` 是这个移植里跟服务器说话的那半：一个任务走短连接或长连接出去，会被重试、
超时、上报，而 App 回答 STN 跑的过程中问的那十八个问题 —— 每个都有默认实现，所以 App
只写它关心的那几个。

[任务链路](/zh/stn)是它的全部 —— 两条连接、一个任务的各个字段、任务怎么结束、长连接
要 App 做什么。

## 网络诊断

```rust
use marsrs::sdt::{report_json, Ask, CheckIPPort, CheckIPPorts, SdtLogic, NET_CHECK_BASIC, NET_CHECK_LONG};

let mut sdt = SdtLogic::new();
sdt.set_http_netcheck_cgi("http://example.com/netcheck");
let mut longlink = CheckIPPorts::new();
longlink.insert("default".to_owned(), vec![CheckIPPort::new("1.2.3.4", 80)]);
sdt.start_active_check(&longlink, &CheckIPPorts::new(), NET_CHECK_BASIC | NET_CHECK_LONG, 10_000);

let results = sdt.run_checks(&mut Ask::new(probe), 1 /* comm::getNetInfo() */);
println!("{}", report_json(&results));
```

四个探针 —— ping、dns、tcp、http —— 是 App 的：这个移植不持有任何 socket，所以
`run_checks` 是向交给它的那个 `Ask` 一个一个地问，都在调用的线程上。

[网络诊断](/zh/sdt)是它的全部：那个模式、那份计划、以及报告的那份 JSON。
