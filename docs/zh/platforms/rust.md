# Rust

```bash
cargo add marsrs          # 整个端口：xlog、stn、sdt
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
