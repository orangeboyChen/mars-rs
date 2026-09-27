# Configuration

An appender is opened with one config, and every platform carries the same
options under its own spelling. `logdir` / `logDir` / `logDirectory` is the only
one with no default — the appender is not opened without somewhere to write.

| what it does | Rust | Swift | Android | Kotlin Multiplatform | C | default |
|---|---|---|---|---|---|---|
| where the `.xlog` files go; created when it is not there | `logdir` | `logDirectory` | `logDir` | `logDir` | `log_dir` | **required** (`./log` in Rust) |
| what every file starts with, and the name the appender is known by | `nameprefix` | `namePrefix` | `namePrefix` | `namePrefix` | `name_prefix` | `xlog` (`Mars` in Rust) |
| the level a record has to reach | see [levels](#levels) | `level` | `level` | `level` | `mars_xlog_set_level` | `info` |
| whether a write waits for the file | `mode` | `mode` | `mode` | `mode` | `mode` | async |
| where the async cache file goes | `cachedir` | `cacheDirectory` | `cacheDir` | `cacheDir` | `cache_dir` | next to the log files |
| how many days a cache file is kept | `cache_days` | `cacheDays` | `cacheDays` | `cacheDays` | `cache_days` | `0` — every file is kept |
| what a closed file is compressed with | `compress_mode` | `compression` | `compressMode` | `compressMode` | `compress_mode` | zlib |
| how hard the compressor tries | `compress_level` | `compressionLevel` | `compressLevel` | `compressLevel` | `compress_level` | `0` — the compressor's own (zlib: 6) |
| the public key a record is encrypted with | `pub_key` | `publicKey` | `pubKey` | `pubKey` | `pub_key` | empty — no encryption |

A config the appender cannot honour is refused where you build it and not
silently by the library: Swift throws an `XlogError`, Kotlin raises
`IllegalArgumentException`, Rust returns an `Err` and C answers a negative
`MARS_XLOG_ERR_*`.

## Levels

A record is written when the level of the appender is at most the record's own:
an appender opened at `info` keeps `warning` and drops `debug`.

| level | Rust | Swift | Android | KMP | C |
|---|---|---|---|---|---|
| 0 | `LogLevel::Verbose` | `.verbose` | `LogLevel.VERBOSE` | `LogLevel.Verbose` | `MarsLevelVerbose` |
| 1 | `LogLevel::Debug` | `.debug` | `LogLevel.DEBUG` | `LogLevel.Debug` | `MarsLevelDebug` |
| 2 | `LogLevel::Info` | `.info` | `LogLevel.INFO` | `LogLevel.Info` | `MarsLevelInfo` |
| 3 | `LogLevel::Warn` | `.warning` | `LogLevel.WARNING` | `LogLevel.Warning` | `MarsLevelWarn` |
| 4 | `LogLevel::Error` | `.error` | `LogLevel.ERROR` | `LogLevel.Error` | `MarsLevelError` |
| 5 | `LogLevel::Fatal` | `.fatal` | `LogLevel.FATAL` | `LogLevel.Fatal` | `MarsLevelFatal` |
| 6 | `LogLevel::None` | `.none` | `LogLevel.NONE` | — | `MARS_LEVEL_NONE` |

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

| what it does | Rust | Swift | Android | C |
|---|---|---|---|---|
| move the level | `set_level` | `log.level` | `xlog.level` | `mars_xlog_set_level` |
| switch async / sync | `appender_set_mode` | `log.mode` | `xlog.mode` | `mars_xlog_set_mode` |
| mirror records to the console | `appender_set_console_log` | `log.isConsoleLogEnabled` | `xlog.consoleLogEnabled` | `mars_xlog_set_console_log` |
| close a file after N bytes | `appender_set_max_file_size` | `log.maxFileSizeBytes` | `xlog.maxFileSizeBytes` | `mars_xlog_set_max_file_size` |
| drop a file older than N seconds | `appender_set_max_alive_duration` | `log.maxAliveTimeSeconds` | `xlog.maxAliveTimeSeconds` | `mars_xlog_set_max_alive_duration` |
| where the current file is | `appender_get_current_log_path` | `Xlog.currentLogPath` | — | `mars_xlog_current_log_path` |

`0` is "no limit" for both sizes and ages: a file is never split and never
dropped — the C++ keeps its own ten days.
