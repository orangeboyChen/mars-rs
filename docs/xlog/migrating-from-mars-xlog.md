# Migrating from mars-xlog

An app that took the C++ project's logger — and nothing else — has one thing to
move: the appender. What it writes is the file the C++ one writes — same framing,
same compression, same encryption — so the `.xlog` files you have already
collected are read by the tooling you already have, and a migration costs you no
history.

## What you take

| the piece you use | what you take here | where it is written down |
|---|---|---|
| `mars/xlog` | `xlog`: `marsrs-xlog` on crates.io, `xlog` on JitPack, `xlog-kmp` for a shared Kotlin module, `MarsRSXlog` on Apple | [Getting started](/xlog/getting-started) |

The split is the one the Rust crates, JitPack and the shared Kotlin module
make: the logger is one package and the whole port is the other, and the whole
port carries the same logger — only the name differs. Apple makes it three ways
instead — `MarsRSXlog` is the logger, `MarsRSNet` is the pipeline and the
diagnosis and no logger at all, and `MarsRS` is both — and Flutter and React
Native carry the logger and nothing else in both of their packages: see
[Flutter](/xlog/getting-started#flutter) and
[React Native](/xlog/getting-started#react-native).

An app that only logs takes the logger's package, an app that also runs a task
or a diagnosis takes the one that carries them, and an app that is not sure
starts with the logger: `marsrs` is the logger plus the other two pieces, so
moving from one package to the other renames nothing about the file.

## The appender

The appender is the same object: one process-wide writer per prefix, opened once
when the app starts, written through, and flushed before its file is read or
uploaded. What changes is who holds it — the C++ installs one process-wide
appender behind free functions, and here you hold the `Xlog` you opened and write
through it.

### From the C++ headers

The appender is one struct of options and a handful of free calls in
`mars/xlog/appender.h`, and it is one struct and a handful of calls here:

| the C++ | Rust | the C ABI |
|---|---|---|
| `appender_open(const XLogConfig&)` | `appender_open(XLogConfig)` | `mars_xlog_open(&config)` |
| `xlogger_Write(info, log)`, or the `xinfo2` family | `appender_write(info, message)` | `mars_xlog_write(...)` |
| `appender_flush()` | `appender_signal_flush()` | `mars_xlog_flush()` |
| `appender_flush_sync()` | `appender_flush_now()` | `mars_xlog_flush_sync()` |
| `appender_close()` | `appender_close()` | `mars_xlog_close()` |
| `xlogger_SetLevel(level)` | `set_level(handle, level)` | `mars_xlog_set_level(level)` |
| `appender_setmode(mode)` | `appender_set_mode(mode)` | `mars_xlog_set_mode(mode)` |
| `appender_set_console_log(bool)` | `appender_set_console_log(bool)` | `mars_xlog_set_console_log(on)` |
| `appender_set_max_file_size(bytes)` | `appender_set_max_file_size(bytes)` | `mars_xlog_set_max_file_size(bytes)` |
| `appender_set_max_alive_duration(secs)` | `appender_set_max_alive_duration(secs)` | `mars_xlog_set_max_alive_duration(secs)` |
| `appender_get_current_log_path(out, len)` | `appender_get_current_log_path()` | `mars_xlog_current_log_path(out, len)` |

The two flush rows are the one place the Rust column is not the C++'s name:
what the C++ calls `appender_flush` is `appender_signal_flush` here, because
`appender_flush` in Rust is the drain a caller `await`s — see
[log files](/xlog/log-files). `appender_flush_sync` still compiles and does what
`appender_flush_now` does, deprecated, so that code written against the C++'s
name keeps building.

The config is one struct in all three, and the eight fields are the same eight:
`mode_`, `logdir_`, `nameprefix_`, `pub_key_`, `compress_mode_`,
`compress_level_`, `cachedir_` and `cache_days_` in the C++, which are `mode`,
`logdir`, `nameprefix`, `pub_key`, `compress_mode`, `compress_level`, `cachedir`
and `cache_days` here. Every one of them is on
[the configuration page](/xlog/configuration), in the spelling of every platform.
`TAppenderMode` is `AppenderMode`, `TCompressMode` is `CompressMode` and
`TLogLevel` is `LogLevel`.

The macros are the other half of what goes. `XLOGGER_TAG` and the
`xverbose2` / `xdebug2` / `xinfo2` / `xwarn2` / `xerror2` / `xfatal2` family
carried the level, the tag and the call site in one line of C++; what replaces
them is a method per level — `xlog.i(tag, message)` in Kotlin,
`log.info(message:tag:)` in Swift, `appender_write(None, message)` in Rust.

The call site is the one of the three that does not come along everywhere.
Swift fills the file, the function and the line in from `#file`, `#function`
and `#line`, so a record written through `log.info(message:tag:)` says where
it was written. Kotlin's write takes a handle, a level, a tag and a message and
nothing else, so a record written through `xlog.i(tag, message)` carries an
empty file and the line 0 — what the C++ project's own `Log` always passed. An
app that needs them in the record has two ways: the `XLoggerInfo` the old
`logWrite` takes, or, in Rust, the one `appender_write` is handed instead of
`None`:

```rust
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

A second appender is where the shapes differ most. The C++ project's Java opened
one with `Log.openLogInstance(level, mode, cacheDir, logDir, nameprefix,
cacheDays)` and threaded the handle it answered through every call after it;
here an appender you hold *is* the instance, so a second one is a second `Xlog`
of a prefix of its own, or the `*_instance` family in Rust —
`appender_open_instance(config)` answers a handle, and `appender_write_instance`,
`appender_flush_instance` and `appender_close_instance` take it.

### From the C++ project's Java

The Android package is the one place the old spelling is still there:
`Xlog.open` with its seven arguments, `XLogConfig`, `XLoggerInfo`, `logWrite`
and the `LEVEL_*` constants all work. What carries a `@Deprecated` — and the
spelling that replaces it — is the seven-argument `Xlog.open`, `logWrite`, the
`Xlog()` with no argument and the `Log` facade around them; `XLogConfig`,
`XLoggerInfo` and the `LEVEL_*` constants carry no such pointer, so a call site
that uses them compiles with nothing pointing it at the new spelling, and
finding it is the app's own grep. `Log.setLogImp(Xlog())` and
`Log.d(tag, message)` still write through the same appender `Xlog.open`
installs, so the rest of a migration can go one call site at a time:

```kotlin
// before
Log.setLogImp(Xlog())
Log.d("net", "…")

// after
val xlog = Xlog.open(XlogConfig(logDir = dir, namePrefix = "marsrs"))
xlog.d("net", "…")
```

What the new spelling buys is a `Context`: `Xlog.open(config, context)` flushes
itself when the app leaves the screen, which is the last moment Android says
anything before it can end the process without another word — see
[Android](/xlog/log-files#when-the-app-goes-away).

### From the Apple headers

There is no Objective-C wrapper to move: what an app on Apple called was the C++
out of an Objective-C++ file — `xlogger_SetLevel`, `appender_set_console_log`,
an `XLogConfig` filled in field by field and `appender_open(config)`. That file
becomes one that imports the module and holds the appender it opened:

::: code-group

```objc [before]
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

```objc [after]
@import MarsRSXlog;

XlogConfig *config = [[XlogConfig alloc] initWithLogDirectory:logPath.path];
config.namePrefix = @"marsrs";

NSError *error = nil;
Xlog *log = [[Xlog alloc] initWithConfig:config error:&error];
[log writeWithLevel:LogLevelInfo message:@"cold start" tag:@"startup"];
[log flushNow];        // before the app reads or uploads the files
```

:::

Swift fills the file, the function and the line in at the call site;
Objective-C has no `#file` to fill one with, so a record written here carries an
empty file and the line 0 unless the long form names them —
`[log log:message:tag:file:function:line:]`.

What replaces the header is `MarsRSXlog` — a SwiftPM product and a pod of the
same name, over the same framework, with an `@objc` surface under it.
[SwiftPM](/xlog/getting-started#swiftpm) and [CocoaPods](/xlog/getting-started#cocoapods) are the two
pages of it.

## Where to go next

- [Configuration](/xlog/configuration) — every option of the appender and its default,
  in the spelling of every platform.
- [Log files](/xlog/log-files) — where the file lands and how it is read back, for an
  app whose upload path already knows the C++ one.
- [Migrating from mars-stn](/stn/migrating-from-mars-stn) and
  [Migrating from mars-sdt](/sdt/migrating-from-mars-sdt) — for an app whose
  logger was not the only piece it took: the task pipeline and the network
  diagnosis.
