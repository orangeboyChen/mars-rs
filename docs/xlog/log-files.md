# Log files

## Where they are, and what they are called

An appender writes one file per day into the log directory of its config:

```text
<logDir>/<namePrefix>_YYYYMMDD.xlog
```

`XlogConfig(logDir = "/data/…/xlog/log", namePrefix = "marsrs")` gives
`marsrs_20260927.xlog`. The directory is created when it is not there, and the
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

**Flush before you read or upload** — this process reads the file, or another one
reads it while this one is still logging into it:

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

## When the app goes away

**There is nothing to call.** Nothing is lost when a process is killed mid-write,
either: a record in the cache is in a file the kernel holds and not in the
process, so it outlives the process, and the next appender of the same
`namePrefix` drains it into its log file as it opens — between the
`~~~~~ begin of mmap ~~~~~` and `~~~~~ end of mmap ~~~~~` lines of an ordinary
start. What an app that never flushes gives up is only time: the last records of
a session reach its file when the next one begins.

Two of the packages give up nothing at all, because they flush when the app
leaves the screen — the last moment Android and iOS say anything before they can
end the process without another word:

| where | what flushes it |
|---|---|
| Android | `Xlog(config, context)` — any `Context` of the app registers a `ComponentCallbacks2` that flushes from `TRIM_MEMORY_UI_HIDDEN` up |
| SwiftPM | every `Xlog`, from the moment it is built: it watches `didEnterBackground` and `willTerminate`, and `WKExtension`'s on watchOS |
| everywhere else | the next start, as above |

`close()` drains too, so an app that closes its appender on the way out is
covered by that as well. The two `flush` calls above are for the other case: a
read or an upload that happens *while the app is still running*.

Sync mode is the one exception, and it is the one place the answer is not
"nothing": there is no cache file behind its records, so the tail the process was
still holding — up to about 4 KiB of it — dies with the process. An app that logs
synchronously and wants that tail has to `close()` or `flush(sync: true)`; the
two hooks above do it, and so does an app that closes its appender.

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
cargo install marsrs-xlog          # puts `xlog` on the $PATH; a release
                                   # carries the same command as an archive
xlog decode --privkey=<hex> marsrs_20260927.xlog --out=marsrs.plain
```

```rust [Rust]
use marsrs::xlog::{get_period_logs, LogBuffer};

// the bytes of the records between two hours of the day
let (begin, end) = get_period_logs(std::path::Path::new("marsrs_20260927.xlog"), 0, 24)?;
```

```python [Upstream's tooling]
python3 decode_mars_log_file.py marsrs_20260927.xlog      # Tencent/mars
```

:::

An encrypted file needs the private key of the pair whose public key is in the
config — `xlog decode` takes it as `--privkey`, and nothing reads the records
without it. `xlog encode` is the other half: it writes a `.xlog` out of one
record per line of its input, and encrypts it when it is given the public key
of that pair with `--pubkey`.

A file that is not whole still reads. A record that cannot be read — a block a
process killed between two writes never finished, a byte that went wrong on its
way to wherever the file was copied — is skipped, and the span it took is marked
in the output where that record's text would have been, so the records behind
the damage are in what the CLI writes out and not lost with it.

The [CLI](/xlog/cli) page is the whole command line — installing it, making that pair,
reading a file back and writing one. `xlog help` prints the same thing in a
terminal, and [configuration](/xlog/configuration#compression-and-encryption) says
what the public key does to a record.
