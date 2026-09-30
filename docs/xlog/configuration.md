# Configuration

An appender is opened with one config, and every platform carries the same
options under its own spelling. `logdir` / `logDir` / `logDirectory` is the only
one with no default — the appender is not opened without somewhere to write.

| what it does | Rust | Swift | Android | Kotlin Multiplatform | Flutter / React Native | C | C++ | HarmonyOS | default |
|---|---|---|---|---|---|---|---|---|---|
| where the `.xlog` files go; created when it is not there | `logdir` | `logDirectory` | `logDir` | `logDir` | `logDir` | `log_dir` | `logDir` | `logDir` | **required** (`./log` in Rust) |
| what every file starts with, and the name the appender is known by | `nameprefix` | `namePrefix` | `namePrefix` | `namePrefix` | `namePrefix` | `name_prefix` | `namePrefix` | `namePrefix` | `xlog` |
| the level a record has to reach | see [levels](#levels) | `level` | `level` | `level` | `level` | `mars_xlog_set_level_instance(0, level)` | `level` | `level` | `info` |
| whether a write waits for the file | `mode` | `mode` | `mode` | `mode` | `mode` | `mode` | `mode` | `mode` | async |
| where the async cache file goes | `cachedir` | `cacheDirectory` | `cacheDir` | `cacheDir` | `cacheDir` | `cache_dir` | `cacheDir` | `cacheDir` | next to the log files |
| how many days a cache file is kept | `cache_days` | `cacheDays` | `cacheDays` | `cacheDays` | `cacheDays` | `cache_days` | `cacheDays` | `cacheDays` | `0` — no cache file is written at all |
| what a closed file is compressed with | `compress_mode` | `compression` | `compressMode` | `compressMode` | `compressMode` | `compress_mode` | `compressMode` | `compressMode` | zlib |
| how hard the compressor tries | `compress_level` | `compressionLevel` | `compressLevel` | `compressLevel` | `compressLevel` | `compress_level` | `compressLevel` | `compressLevel` | `0` — the appender's own, which is `6`; Rust names it and starts at `6` |
| the public key a record is encrypted with | `pub_key` | `publicKey` | `pubKey` | `pubKey` | `pubKey` | `pub_key` | `pubKey` | `pubKey` | empty — no encryption |

A config the appender cannot honour is refused where you build it and not
silently by the library: Swift and C++ throw an `XlogError`, Kotlin raises
`IllegalArgumentException`, Rust returns an `Err`, C answers a negative
`MARS_XLOG_ERR_*`, ArkTS throws an `Error`, and on the two bridging platforms the
call that opened it fails — a `PlatformException` in Dart, a thrown `Error` in
TypeScript.

## Levels

A record is written when the level of the appender is at most the record's own:
an appender opened at `info` keeps `warning` and drops `debug`.

| level | Rust | Swift | Android | KMP | Flutter / React Native | C | C++ | HarmonyOS |
|---|---|---|---|---|---|---|---|---|
| 0 | `LogLevel::Verbose` | `.verbose` | `LogLevel.VERBOSE` | `LogLevel.VERBOSE` | `LogLevel.verbose` | `MarsLevelVerbose` | `LogLevel::Verbose` | `LogLevel.Verbose` |
| 1 | `LogLevel::Debug` | `.debug` | `LogLevel.DEBUG` | `LogLevel.DEBUG` | `LogLevel.debug` | `MarsLevelDebug` | `LogLevel::Debug` | `LogLevel.Debug` |
| 2 | `LogLevel::Info` | `.info` | `LogLevel.INFO` | `LogLevel.INFO` | `LogLevel.info` | `MarsLevelInfo` | `LogLevel::Info` | `LogLevel.Info` |
| 3 | `LogLevel::Warn` | `.warning` | `LogLevel.WARNING` | `LogLevel.WARNING` | `LogLevel.warning` | `MarsLevelWarn` | `LogLevel::Warning` | `LogLevel.Warning` |
| 4 | `LogLevel::Error` | `.error` | `LogLevel.ERROR` | `LogLevel.ERROR` | `LogLevel.error` | `MarsLevelError` | `LogLevel::Error` | `LogLevel.Error` |
| 5 | `LogLevel::Fatal` | `.fatal` | `LogLevel.FATAL` | `LogLevel.FATAL` | `LogLevel.fatal` | `MarsLevelFatal` | `LogLevel::Fatal` | `LogLevel.Fatal` |
| 6 | `LogLevel::None` | `.none` | `LogLevel.NONE` | `LogLevel.NONE` | `LogLevel.none` | `MARS_LEVEL_NONE` | `LogLevel::None` | `LogLevel.None` |

The two Kotlin columns are the same on purpose: `xlog-kmp` and `xlog` publish one
API, so a shared module that moves between them renames nothing. So are the two
columns that follow them — `marsrs_xlog` and `marsrs-react-native-xlog` carry
the same spelling of the same options, and it is the Kotlin one.

HarmonyOS is the Kotlin spelling in a different case: `LogLevel.Verbose` and not
`LogLevel.VERBOSE`, because an ArkTS enum sits in an app full of HarmonyOS enums.

`none` writes nothing, not even `fatal` — it is how an appender is quieted
without being closed.

An assert is the one record the level does not gate: it is written at `fatal`
whatever the appender's level is, because what an assert names is a condition
that is not supposed to be possible.

Neither call ends the process. An app that wants its process stopped on an
assert stops it itself — `std::process::abort()`, or a platform trap — after the
write.

::: code-group

```rust [Rust]
marsrs::xlog::xlogger_assert(None, "fd >= 0", "the socket was already closed");
```

```c [C]
mars_xlog_assert("net", __FILE__, __func__, __LINE__, "fd >= 0", "the socket was already closed");
```

:::

A message that is expensive to build is worth asking about first, because a
record the level drops still costs the caller the string:

::: code-group

```swift [Swift]
if log.isEnabled(for: .debug) {
    log.debug(message: "\(expensiveDescription())", tag: "net")
}
```

```kotlin [Android]
if (xlog.isLoggable(LogLevel.DEBUG)) {
    xlog.d("net", expensiveDescription())
}
```

```dart [Flutter]
if (await xlog.isLoggable(LogLevel.debug)) {
  xlog.d("net", expensiveDescription());
}
```

```ts [React Native]
if (xlog.isLoggable(LogLevel.debug)) {
  xlog.d("net", expensiveDescription());
}
```

```typescript [HarmonyOS]
if (xlog.isLoggable(LogLevel.Debug)) {
  xlog.d('net', expensiveDescription());
}
```

:::

## Async or sync

| | async (the default) | sync |
|---|---|---|
| what a write does | compresses the record into a memory-mapped cache and hands it to a writer thread | writes it through to the file |
| what it costs | the `write` syscall is not on the logging thread | the thread that logs waits for the file |
| what you must do | `flushNow()` before the file is read or uploaded — and nothing at all when the app goes away ([log files](/xlog/log-files)) | `close()` or `flushNow()` before the process can be killed: what is still buffered goes with it |

Async is the default because the file is slower than the record; sync is what you
want when a record cannot wait for a writer thread — a crash log, or the last
lines before an exit.

What neither mode does is reach the kernel once per record: both hold what they
have until roughly 4 KiB of it has piled up, or until `close()` or `flushNow()`
runs. Async loses nothing by waiting — the record is in the
cache file the kernel holds as well — but sync has nothing behind it, so a
process that is killed loses the tail it was still holding. That tail is the one
thing an exit needs a call for.

## Compression and encryption

`zlib` is the default and `zstd` compresses harder; both are per file, applied
when the file is closed. The level is a `zstd` knob and nothing else: zlib
compresses at the one setting the C++ uses whatever the level says, and `0` asks
for the appender's own, `6` — which is what Rust's config carries, so an app that
names no level gets the same file on every platform. The ceiling is the
compressor's: `9` for zlib, `22` for zstd.

A `pubKey` encrypts each record's body with ECDH + TEA — the cipher the C++
implementation uses, over a key the writer and the reader agree on. What you put there is
the *public* key of the pair whose private key reads the file back; leaving it
empty writes a file any mars log reader can open.

## After it is open

These are not in the config — they are setters or properties on the appender, and
every one of them takes effect from the next record:

| what it does | Rust | Swift | Android | Kotlin Multiplatform | Flutter | React Native | C | C++ | HarmonyOS |
|---|---|---|---|---|---|---|---|---|---|
| move the level | `xlog.set_level` | `log.level` | `xlog.level` | `xlog.level` | `xlog.level` | `xlog.level` | `mars_xlog_set_level_instance(0, level)` | `log.setLevel` | `xlog.level` |
| switch async / sync | `xlog.set_mode` | `log.mode` | `xlog.mode` | `xlog.mode` | `xlog.mode` | `xlog.mode` | `mars_xlog_set_mode_instance(0, mode)` | `log.setMode` | `xlog.mode` |
| mirror records to the console | `xlog.set_console_log_enabled` | `log.isConsoleLogEnabled` | `xlog.consoleLogEnabled` | `xlog.consoleLogEnabled` | `xlog.consoleLogEnabled` | `xlog.consoleLogEnabled` | `mars_xlog_set_console_log_instance(0, on)` | `log.setConsoleLogEnabled` | `xlog.consoleLogEnabled` |
| close a file after N bytes | `xlog.set_max_file_size_bytes` | `log.maxFileSizeBytes` | `xlog.maxFileSizeBytes` | `xlog.maxFileSizeBytes` | `xlog.maxFileSizeBytes` | `xlog.maxFileSizeBytes` | `mars_xlog_set_max_file_size_instance(0, bytes)` | `log.setMaxFileSizeBytes` | `xlog.maxFileSizeBytes` |
| drop a file older than N seconds | `xlog.set_max_alive_time_seconds` | `log.maxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` | `mars_xlog_set_max_alive_duration_instance(0, secs)` | `log.setMaxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` |
| where the current file is | `xlog.current_log_path` | `Xlog.currentLogPath` | — | — | — | — | `mars_xlog_current_log_path` | — | — |
| a day of files | `appender_getfilepath_from_timespan` | `Xlog.logFiles(…)` | — | — | — | — | `mars_xlog_getfilepath_from_timespan` | — | — |

`0` is "no limit" on the size: a file is never split. On the age it is ten
days, because anything under a day is raised to ten — the ten days the C++
keeps too.

## Where the console copy goes

`xlog.set_console_log_enabled(true)` — `xlog.consoleLogEnabled = true` on the
platforms that spell it that way — mirrors every record to the console as well as
to the file. A record written through a Rust `Xlog` carries an `XLoggerInfo` —
the level and the tag the call named — so what the console gets is the whole
record and not only the message.

The built-in sink is standard error, on every platform and in every package —
nothing here writes to `os_log` or to logcat. On Apple the system log is one
`Xlog.setConsoleSink` away, and it is the app's own code that calls `os_log` in
it.

An app that wants it somewhere else hands the logger a sink of its own, and
what was going to the console goes to that instead:

::: code-group

```rust [Rust]
use marsrs::xlog::set_console_fun;

set_console_fun(Some(|info, log| println!("{:?}: {log}", info.level)));
set_console_fun(None);   // the console has it again
```

```swift [Swift]
Xlog.setConsoleSink { level, tag, file, function, line, log in
    os_log(.default, "%{public}@", String(cString: log))
}
Xlog.setConsoleSink(nil)   // the console has it again
```

```c [C]
static void to_my_console(int level, const char* tag, const char* filename,
                          const char* func_name, int line, const char* log) {
    my_console_write(level, log);
}

mars_xlog_set_console_fun(to_my_console);
mars_xlog_set_console_fun(NULL);   /* the console has it again */
```

:::

There is one sink and it is the appender's, not a config: setting it again
replaces it. What it is handed is the record unformatted — the level, the tag,
where the call site is, and the message — which is the whole point: on Apple
that is where `os_log` goes, and only the app's own code can call it.
