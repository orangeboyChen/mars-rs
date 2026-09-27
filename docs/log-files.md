# Log files

## Where they are, and what they are called

An appender writes one file per day into the log directory of its config:

```text
<logDir>/<namePrefix>_YYYYMMDD.xlog
```

`XlogConfig(logDir = "/data/…/xlog/log", namePrefix = "Ham")` gives
`Ham_20260927.xlog`. The directory is created when it is not there, and the
prefix is what the appender is known by: two appenders that share one share the
file, and closing one of them closes what the other writes through. Give a part
of an app whose logs are read apart from the rest a prefix of its own.

Where the file that is being written right now is:

::: code-group

```rust [Rust]
appender_get_current_log_path()          // Option<PathBuf>
appender_get_current_log_path_instance(id)
```

```swift [Swift]
Xlog.currentLogPath
```

```c [C]
char path[512];
mars_xlog_current_log_path(path, sizeof path);   // MARS_XLOG_OK, or a negative code
```

:::

## Async: the record may still be in the cache

The default mode hands a record to a writer thread through a memory-mapped cache
file, so a write returns before the bytes reach the log file. The cache lives in
`cacheDir`, or next to the log files when you give none.

**Flush before you read or upload, and before the process goes away** — the last
records of a process that is killed reach the disk only when something drains
them:

::: code-group

```rust [Rust]
appender_flush_sync()                    // waits for the file
appender_flush_instance(id, true)
```

```swift [Swift]
log.flush(sync: true)
```

```kotlin [Android]
xlog.flush(sync = true)
```

```kotlin [Kotlin Multiplatform]
Xlog.flush(sync = true)
```

```c [C]
mars_xlog_flush_sync();
```

:::

`flush(sync = false)` — `appender_flush()`, `mars_xlog_flush()` — only signals
the writer thread and returns; it is the cheap one to call on a timer, and not
the one to call before you upload.

## Rotation and retention

| knob | what it does | default |
|---|---|---|
| `maxFileSizeBytes` | a file is closed and a new one opened once it reaches this many bytes | `0` — never split |
| `maxAliveTimeSeconds` | a file older than this many seconds is dropped | `0` — keep it (the C++ keeps its own ten days) |
| `cacheDays` | an async cache file older than this many days is dropped | `0` — keep every one |

## Reading a file back

A `.xlog` written here is the `.xlog` the C++ implementation writes — same
framing, same compression, same encryption — so the tooling that already reads
mars logs reads these.

::: code-group

```bash [The CLI]
xlog-compat decode --privkey=<hex> --in=Ham_20260927.xlog --out=Ham.plain
```

```rust [Rust]
use marsrs::xlog::{get_period_logs, LogBuffer};

// the bytes of the records between two hours of the day
let (begin, end) = get_period_logs(std::path::Path::new("Ham_20260927.xlog"), 0, 24)?;
```

```python [Upstream's tooling]
python3 decode_mars_log_file.py Ham_20260927.xlog      # Tencent/mars
```

:::

An encrypted file needs the private key of the pair whose public key is in the
config — `xlog-compat decode` takes it as `--privkey`, and nothing reads the
records without it.
