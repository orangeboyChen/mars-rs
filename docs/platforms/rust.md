# Rust

```bash
cargo add marsrs          # the whole port: xlog, stn and sdt
cargo add marsrs-xlog     # xlog alone — the logger and nothing else
```

`marsrs` is one module per component — `xlog`, `stn`, `sdt`, `comm`, `bytes` —
and each of the first four is behind a feature of its own, all four on by
default. An app that only logs takes `marsrs-xlog`, or `marsrs` with the rest
turned off:

```toml
[dependencies]
marsrs = { version = "0.1", default-features = false, features = ["xlog"] }
```

## Open, write, flush, close

```rust
use marsrs::xlog::{appender_close, appender_flush_sync, appender_open, appender_write, XLogConfig};

let mut config = XLogConfig::default();
config.logdir = std::path::PathBuf::from("/tmp/mars-log");
config.nameprefix = "Ham".to_owned();
appender_open(config)?;

appender_write(None, "hello from mars");

appender_flush_sync();
appender_close();
```

`appender_open` installs one process-wide appender, which is what `appender_write`
writes through. `XLogConfig::default()` is `./log`, the prefix `Mars`, async and
zlib; every option is on [the configuration page](/configuration).

A record can carry more than a message — `appender_write` takes an `XLoggerInfo`
with the level, the tag and the file, function and line of the call site, and
`None` is the default one:

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

## One appender per prefix

An app that reads one part of its logs apart from the rest gives that part an
appender of its own — its own directory, prefix and options:

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

## Level, console, rotation

```rust
use marsrs::xlog::{
    appender_get_current_log_path, appender_set_console_log, appender_set_max_alive_duration,
    appender_set_max_file_size, appender_set_mode, is_enabled_for, set_level, AppenderMode,
    LogLevel, DEFAULT_HANDLE,
};

set_level(DEFAULT_HANDLE, LogLevel::Info);      // a record below this is dropped
appender_set_mode(AppenderMode::Async);
appender_set_console_log(true);                 // mirror records to stdout
appender_set_max_file_size(8 * 1024 * 1024);    // close a file at 8 MiB
appender_set_max_alive_duration(10 * 24 * 3600); // drop a file older than ten days

if is_enabled_for(DEFAULT_HANDLE, LogLevel::Debug) {
    appender_write(None, &expensive_description());
}

let path = appender_get_current_log_path();
```

## Reading a file back

```rust
use marsrs::xlog::{get_period_logs, LogBuffer};

let (begin, end) = get_period_logs(std::path::Path::new("Ham_20260927.xlog"), 0, 24)?;
```

`LogBuffer` is the decoder; see [log files](/log-files).
