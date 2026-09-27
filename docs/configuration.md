# Configuration

An appender is opened with one config, and every platform carries the same
options under its own spelling. `logdir` / `logDir` / `logDirectory` is the only
one with no default — the appender is not opened without somewhere to write.

| what it does | Rust | Swift | Android | Kotlin Multiplatform | Flutter / React Native | C | default |
|---|---|---|---|---|---|---|---|
| where the `.xlog` files go; created when it is not there | `logdir` | `logDirectory` | `logDir` | `logDir` | `logDir` | `log_dir` | **required** (`./log` in Rust) |
| what every file starts with, and the name the appender is known by | `nameprefix` | `namePrefix` | `namePrefix` | `namePrefix` | `namePrefix` | `name_prefix` | `xlog` (`Mars` in Rust) |
| the level a record has to reach | see [levels](#levels) | `level` | `level` | `level` | `level` | `mars_xlog_set_level` | `info` |
| whether a write waits for the file | `mode` | `mode` | `mode` | `mode` | `mode` | `mode` | async |
| where the async cache file goes | `cachedir` | `cacheDirectory` | `cacheDir` | `cacheDir` | `cacheDir` | `cache_dir` | next to the log files |
| how many days a cache file is kept | `cache_days` | `cacheDays` | `cacheDays` | `cacheDays` | `cacheDays` | `cache_days` | `0` — every file is kept |
| what a closed file is compressed with | `compress_mode` | `compression` | `compressMode` | `compressMode` | `compressMode` | `compress_mode` | zlib |
| how hard the compressor tries | `compress_level` | `compressionLevel` | `compressLevel` | `compressLevel` | `compressLevel` | `compress_level` | `0` — the compressor's own (zlib: 6) |
| the public key a record is encrypted with | `pub_key` | `publicKey` | `pubKey` | `pubKey` | `pubKey` | `pub_key` | empty — no encryption |

A config the appender cannot honour is refused where you build it and not
silently by the library: Swift throws an `XlogError`, Kotlin raises
`IllegalArgumentException`, Rust returns an `Err`, C answers a negative
`MARS_XLOG_ERR_*`, and on the two bridging platforms the call that opened it
fails — a `PlatformException` in Dart, a rejected `Promise` in TypeScript.

## Levels

A record is written when the level of the appender is at most the record's own:
an appender opened at `info` keeps `warning` and drops `debug`.

| level | Rust | Swift | Android | KMP | Flutter / React Native | C |
|---|---|---|---|---|---|---|
| 0 | `LogLevel::Verbose` | `.verbose` | `LogLevel.VERBOSE` | `LogLevel.VERBOSE` | `LogLevel.verbose` | `MarsLevelVerbose` |
| 1 | `LogLevel::Debug` | `.debug` | `LogLevel.DEBUG` | `LogLevel.DEBUG` | `LogLevel.debug` | `MarsLevelDebug` |
| 2 | `LogLevel::Info` | `.info` | `LogLevel.INFO` | `LogLevel.INFO` | `LogLevel.info` | `MarsLevelInfo` |
| 3 | `LogLevel::Warn` | `.warning` | `LogLevel.WARNING` | `LogLevel.WARNING` | `LogLevel.warning` | `MarsLevelWarn` |
| 4 | `LogLevel::Error` | `.error` | `LogLevel.ERROR` | `LogLevel.ERROR` | `LogLevel.error` | `MarsLevelError` |
| 5 | `LogLevel::Fatal` | `.fatal` | `LogLevel.FATAL` | `LogLevel.FATAL` | `LogLevel.fatal` | `MarsLevelFatal` |
| 6 | `LogLevel::None` | `.none` | `LogLevel.NONE` | `LogLevel.NONE` | `LogLevel.none` | `MARS_LEVEL_NONE` |

The two Kotlin columns are the same on purpose: `xlog-kmp` and `xlog` publish one
API, so a shared module that moves between them renames nothing. So are the two
columns that follow them — `marsrs_flutter_xlog` and `marsrs-react-native-xlog`
carry the same spelling of the same options, and it is the Kotlin one.

`none` writes nothing, not even `fatal` — it is how an appender is quieted
without being closed.

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
  await xlog.d("net", expensiveDescription());
}
```

```ts [React Native]
if (await xlog.isLoggable(LogLevel.debug)) {
  await xlog.d("net", expensiveDescription());
}
```

:::

## Async or sync

| | async (the default) | sync |
|---|---|---|
| what a write does | compresses the record into a memory-mapped cache and hands it to a writer thread | writes it through to the file |
| what it costs | the `write` syscall is not on the logging thread | the thread that logs waits for the file |
| what you must do | `flush(sync: true)` before the file is read or uploaded, and before the process goes away | nothing |

Async is the default because the file is slower than the record; sync is what you
want when a record has to be on disk before the next line runs — a crash log, or
the last lines before an exit.

## Compression and encryption

`zlib` is the default and `zstd` compresses harder; both are per file, applied
when the file is closed. The level is the compressor's own knob: `0` keeps the
default (6 for zlib), `9` is the zlib ceiling and `22` the zstd one.

A `pubKey` encrypts each record's body with ECDH + AES-GCM. What you put there is
the *public* key of the pair whose private key reads the file back; leaving it
empty writes a file any mars log reader can open.

## After it is open

These are not in the config — they are setters on the appender, and every one of
them takes effect from the next record:

| what it does | Rust | Swift | Android | Kotlin Multiplatform | Flutter / React Native | C |
|---|---|---|---|---|---|---|
| move the level | `set_level` | `log.level` | `xlog.level` | `xlog.level` | `await xlog.setLevel(…)` | `mars_xlog_set_level` |
| switch async / sync | `appender_set_mode` | `log.mode` | `xlog.mode` | `xlog.mode` | `await xlog.setMode(…)` | `mars_xlog_set_mode` |
| mirror records to the console | `appender_set_console_log` | `log.isConsoleLogEnabled` | `xlog.consoleLogEnabled` | `xlog.consoleLogEnabled` | `await xlog.setConsoleLogEnabled(…)` | `mars_xlog_set_console_log` |
| close a file after N bytes | `appender_set_max_file_size` | `log.maxFileSizeBytes` | `xlog.maxFileSizeBytes` | `xlog.maxFileSizeBytes` | `await xlog.setMaxFileSize(…)` | `mars_xlog_set_max_file_size` |
| drop a file older than N seconds | `appender_set_max_alive_duration` | `log.maxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` | `await xlog.setMaxAliveTime(…)` | `mars_xlog_set_max_alive_duration` |
| where the current file is | `appender_get_current_log_path` | `Xlog.currentLogPath` | — | — | — | `mars_xlog_current_log_path` |

`0` is "no limit" for both sizes and ages: a file is never split and never
dropped — the C++ keeps its own ten days.
