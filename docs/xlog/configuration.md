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
| how many days a cache file is kept | `cache_days` | `cacheDays` | `cacheDays` | `cacheDays` | `cacheDays` | `cache_days` | `cacheDays` | `cacheDays` | `0` — every file is kept |
| what a closed file is compressed with | `compress_mode` | `compression` | `compressMode` | `compressMode` | `compressMode` | `compress_mode` | `compressMode` | `compressMode` | zlib |
| how hard the compressor tries | `compress_level` | `compressionLevel` | `compressLevel` | `compressLevel` | `compressLevel` | `compress_level` | `compressLevel` | `compressLevel` | `0` — the compressor's own (zlib: 6) |
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

There is no assert in the port: `mars_xlog_assert` was the process-wide
appender's, and it went with it. What an app that wants one writes is the
record itself — `xlog.f(tag, message)` — and stops the process after it,
`std::process::abort()` or a platform trap; upstream raises `SIGTRAP` on Android
and calls `__assert_rtn` on Apple once the record is written, and nothing here
does either.

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
when the file is closed. The level is the compressor's own knob: `0` keeps the
default (6 for zlib), `9` is the zlib ceiling and `22` the zstd one.

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
| where it writes | `xlog.current_log_path()` | `log.currentLogPath` | `xlog.currentLogPath` | `xlog.currentLogPath` | `await xlog.currentLogPath()` | `xlog.currentLogPath` | `mars_xlog_current_log_path_instance` | `log.currentLogPath()` | `xlog.currentLogPath` |
| a day of files | `xlog.log_files(1)` | `log.logFiles(daysAgo: 1)` | `xlog.logFiles(1L)` | `xlog.logFiles(1L)` | `await xlog.logFiles(1)` | `xlog.logFiles(1)` | `mars_xlog_getfilepath_from_timespan_instance` | `log.logFiles(1)` | `xlog.logFiles(1)` |

It answers a **directory** and not a file, which is what the C++'s `GetCurrentLogPath` hands back; the day's file is the row under it.

`0` is "no limit" for both sizes and ages: a file is never split and never
dropped — the C++ keeps its own ten days.

## Where the console copy goes

`xlog.set_console_log_enabled(true)` — `xlog.consoleLogEnabled = true` on the
platforms that spell it that way — mirrors every record to the console as well as
to the file. A record written through a Rust `Xlog` carries an `XLoggerInfo` —
the level and the tag the call named — so what the console gets is the whole
record and not only the message.

The built-in sink is standard error, on every platform and in every package —
nothing here writes to `os_log` or to logcat, and there is no sink to set: the
one the port had sat on the process-wide appender, and that appender is gone. On
Apple the system log is a call the app makes for itself — `os_log` is a macro a
library cannot reach.

An app that wants its records there writes them twice, or reads them back out
of the file: the console copy is standard error's, and it is not the port's to
hand anywhere else.
