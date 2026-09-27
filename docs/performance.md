# What a record costs

xlog writes a record on the thread that logs it, so what one record costs is
what the app pays on its hot path. The table below is one measurement of it: one
harness per implementation, the same scenarios and the same constant payload on
both, a discarded warm-up run per scenario and then five timed ones, the fastest
of the five reported. Neither harness is in this tree — the C++ half needs a
clone of upstream `Tencent/mars` to build at all, and what they compare is two
code paths, not a property of this repository. Both hand the logger a payload
that is already formatted, because what is being compared is the logging path —
header, compression, crypt, file — and not `snprintf` against `format!`.

The scenarios are one record through `log_formater` into a 16 KiB buffer, and
`appender_open` → *N* threads × 20 000 `appender_write`s →
`appender_flush_sync` → `appender_close` for every combination of sync/async,
zlib/zstd and 1/8 threads. Nanoseconds per record, macOS/arm64, one run of
each harness (run-to-run spread is about 10%):

| scenario | Tencent/mars (C++) | mars-rs | rust is |
|---|---|---|---|
| format one record | 519 ns | 104 ns | 4.96x faster |
| append sync/zlib/1t | 2038 ns | 830 ns | 2.45x faster |
| append sync/zlib/8t | 3708 ns | 2111 ns | 1.76x faster |
| append sync/zstd/1t | 2180 ns | 1052 ns | 2.07x faster |
| append sync/zstd/8t | 3929 ns | 2198 ns | 1.79x faster |
| append async/zlib/1t | 2793 ns | 1654 ns | 1.69x faster |
| append async/zlib/8t | 4818 ns | 2979 ns | 1.62x faster |
| append async/zstd/1t | 1700 ns | 848 ns | 2.01x faster |
| append async/zstd/8t | 3295 ns | 2570 ns | 1.28x faster |

Async is the *slower* of the two modes per record, in both implementations: a
record that goes to the cache is compressed into the mapping, copied back out of
it and handed to another thread, where the sync path writes it straight through.
What async buys is that the `write` syscall is not on the logging thread, which
is worth it when the file is the bottleneck and not when the record is.

## What the C++ spends a record on that the port does not

| | upstream `Tencent/mars` | this port |
|---|---|---|
| local time | **four `localtime` calls per record** — `formater.cc:90`, both halves of the day check in `__OpenLogFile` (`appender.cc:750-751`, one of them on `openfiletime_`, which never changes) and, with crypt on, `log_crypt.cc:191` | one per **second**, cached per thread (`marsrs_core::localtime`) |
| the scratch buffer | `char temp[16 * 1024] = {0}` per call — 16 KiB of stores — in `__WriteSync` / `__WriteAsync` (`appender.cc:960`, `972`) | one `Vec` per thread, grown once (`RECORD`) |
| the thread id | `syscall(SYS_gettid)` per record (`comm/unix/xlogger_threadinfo.cc:39`); only the pid next to it is a cached static | cached per thread, keyed on the pid so that a `fork` re-reads it (`sys.rs`) |
| the open appender | `sg_default_appender`, read with no lock at all | a per-thread `Arc` tagged with a generation counter: no lock per record, and no shared atomic write either |
| locking | `mutex_buffer_async_` and `mutex_log_file_`, taken several times per flush | one `Mutex` plus one bounded channel to the writer thread |
| file writes | stdio `FILE*` buffering: a failed `fwrite` has already consumed the batch | an explicit `pending` batch, which a refused write keeps, so it can be retried |

The one thing both do the same way — and the reason the port's numbers are not
better still — is that neither formats under a lock: the record is built on the
calling thread and only its bytes cross into the shared state.

## The optimizations behind the table

Measured the same way, against the port as it stood before them:
format 360 → 104 ns, `append sync/zlib/1t` 1150 → 830 ns, at the other end
`append async/zlib/8t` 3259 → 2979 ns. In the order they pay:

1. **`marsrs_core::localtime`** — a per-thread snapshot of the second (offset,
   hour, date) that every caller of `localtime` in a record shares. This is the
   one that moves the needle: it is what turns four conversions per record into
   one per second.
2. **The timestamp text is rendered once per second** — `"yyyy-mm-dd +8.0
   hh:mm:ss."` is decided by `tv_sec` alone, so a second of logging renders it
   once instead of paying six integer conversions and a float per record
   (`LocalStamp` in `formater.rs`).
3. **Integers are written without `write!`** — `push_decimal` / `push_signed`
   put digits into the buffer directly, which is what a `Formatter` cannot do
   without a `Display` impl per integer width.
4. **Local fields by integer arithmetic** — `DateTime::from_timestamp(secs +
   gmtoff, 0)` *is* the local time, and costs ~2 ns where `Local` is a time-zone
   lookup.
5. **The day the file was opened on is stamped once** — `open_file_day`, so the
   per-record roll-over check compares against a cached `local_time` of today
   instead of converting two timestamps, one of which is a constant.
6. **The open appender is cached per thread** — a generation counter says
   whether the cached `Arc` is still current, so a record costs one shared
   atomic *load* instead of a turn through a mutex every logging thread in the
   process contends for.
7. **`flush_sync` takes the lock once** — it took it five times for one flush,
   and nothing in between was observable by anyone else.
8. **`RecursionGuard::drop` uses `try_with`** — no `catch_unwind` around a
   decrement that cannot panic, on the way out of every single record.

Three things were measured and left alone on purpose: `LOG_FLUSH_THRESHOLD`
stays at 4 KiB (it is `st_blksize`, and a bigger batch trades durability for
throughput — a `write` of 4 KiB is ~5 µs, half a sync record); the crypt path's
`localtime` is not threaded through (`set_header_info` would have to take the
second, for ~8%); and `LogBuffer::clear` keeps its 150 KiB `memset`, which is
what the C++ does before unmapping.
