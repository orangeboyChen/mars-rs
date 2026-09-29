# marsrs-ffi

The C ABI seam of the Rust xlog port: a `cdylib` + `staticlib` (+ `rlib`, so the
Rust tests can call the same symbols) that lets the existing C++, JNI and ObjC
layers of Mars talk to `marsrs-appender` **without** a big-bang rewrite.

| C symbol (this crate)            | C++ it replaces                                                |
| -------------------------------- | -------------------------------------------------------------- |
| `mars_xlog_open`                 | `mars::xlog::appender_open(const XLogConfig&)`                 |
| `mars_xlog_write`                | `mars::xlog::XloggerWrite(...)` + `xlogger_IsEnabledFor`        |
| `mars_xlog_signal_flush_instance` | `mars::xlog::appender_flush()`                                 |
| `mars_xlog_flush_now_instance`   | `mars::xlog::appender_flush_sync()`                            |
| `mars_xlog_close`                | `mars::xlog::appender_close()`                                 |
| `mars_xlog_set_level_instance`   | `xlogger_SetLevel()` / `SetLevel` on an instance                |
| `mars_xlog_set_console_log_instance` | `mars::xlog::appender_set_console_log(bool)`                |
| `mars_xlog_set_max_file_size_instance` | `mars::xlog::appender_set_max_file_size(uint64_t)`        |
| `mars_xlog_set_max_alive_duration_instance` | `mars::xlog::appender_set_max_alive_duration(long)`  |
| `mars_xlog_current_log_path`     | `mars::xlog::appender_get_current_log_path(char*, unsigned)`   |

Handle `0` is the process-wide appender `mars_xlog_open` opened, so every
instance symbol is also how that one is asked: `mars_xlog_signal_flush_instance(0)`
tells the writer thread it may drain and returns at once, and
`mars_xlog_flush_now_instance(0)` drains on the calling thread, so the records
are on the disk when it returns. There is one spelling per operation and not
two — the C++ has a free function for the process-wide appender beside each
handle-taking one, and a second spelling here would be two ways to say one
thing, which is a thing the platforms avoid by keeping the handle in the object
an app holds.

Every drain is two calls and not one carrying a flag, and no symbol of the seam
takes a `sync`: `mars_xlog_signal_flush_all()` / `mars_xlog_flush_now_all()` are
the two that reach every appender of the process.

The header is checked in at **`include/mars_xlog.h`** (hand written, no cbindgen
step); `tests/header_sync.rs` fails if it drifts from `src/abi.rs`.

## Build

```sh
cd rust
export CARGO_TARGET_DIR=/tmp/mrs-ffi
cargo build -p marsrs-ffi --release
```

which produces

* `$CARGO_TARGET_DIR/release/libmars_ffi.a` — static
* `$CARGO_TARGET_DIR/release/libmars_ffi.{so,dylib}` — dynamic

Cross-compiling works the usual way, e.g.

```sh
cargo build -p marsrs-ffi --release --target aarch64-linux-android
cargo build -p marsrs-ffi --release --target aarch64-apple-ios
```

## Linking from C/C++

```c
#include "mars_xlog.h"

MarsXLogConfig cfg = {
    .mode          = MarsAppenderSync,      /* or MarsAppenderAsync */
    .log_dir       = "/tmp/marslog",
    .name_prefix   = "Mars",                /* may be NULL -> "Mars" */
    .pub_key       = NULL,                  /* NULL/"" -> no encryption */
    .compress_mode = MarsCompressZlib,      /* or MarsCompressZstd */
    .compress_level = 0,                    /* <= 0 -> appender default (6) */
    .cache_dir     = NULL,                  /* NULL -> use log_dir */
    .cache_days    = 0,
};

if (mars_xlog_open(&cfg) != MARS_XLOG_OK) { /* handle */ }
mars_xlog_set_level_instance(0, MarsLevelInfo);
mars_xlog_write(MarsLevelInfo, "tag", __FILE__, __func__, __LINE__, "hello");
mars_xlog_flush_now_instance(0);

char path[512];
int n = mars_xlog_current_log_path(path, sizeof(path));   /* bytes, excl. NUL */

mars_xlog_close();
```

Compile and link:

```sh
# static
clang log_bridge.c -I rust/crates/marsrs-ffi/include \
      rust/target/release/libmars_ffi.a -lpthread -ldl -lm -o demo
# dynamic
clang log_bridge.c -I rust/crates/marsrs-ffi/include \
      -L rust/target/release -lmars_ffi -lpthread -ldl -lm -o demo
```

On Apple platforms the Rust run-time needs the system frameworks too:

```sh
clang log_bridge.c -I rust/crates/marsrs-ffi/include \
      rust/target/release/libmars_ffi.a \
      -framework CoreFoundation -framework Security -lpthread -ldl -lm -o demo
```

For an iOS app, link `libmars_ffi.a` (a `lipo` fat archive for all target
slices) in *Link Binary With Libraries* together with `CoreFoundation`,
`Security` and `libSystem`.

## Replacing a JNI call site

`mars/xlog/jni/Java2C_Xlog.cc` reads an `Xlog` config object out of Java and
calls `appender_open` + `xlogger_SetLevel`. The equivalent through this shim is
a single `mars_xlog_open` followed by `mars_xlog_set_level_instance(0, level)`; `logWrite` becomes
one `mars_xlog_write` (the level gate the JNI code performs with
`xlogger_IsEnabledFor` is built in).

## Guarantees at the boundary

* **No panic ever unwinds into C.** Every entry point wraps its body in
  `catch_unwind`; a panic is reported as `MARS_XLOG_ERR_PANIC` (or swallowed for
  the `void` symbols) and the message still reaches stderr. The one thing that
  is the caller's and not an entry point is a callback it handed in — the
  console sink of `mars_xlog_set_console_fun`: a panic inside one written in
  Rust aborts at the `extern "C"` boundary, before any `catch_unwind` here can
  see it.
* **No null dereference.** Every incoming pointer is null-checked; null and
  invalid UTF-8 degrade to an empty string. `mars_xlog_open` returns
  `MARS_XLOG_ERR_NULL_CONFIG` / `MARS_XLOG_ERR_EMPTY_LOG_DIR` instead of failing
  later.
* **No truncation surprises.** `mars_xlog_current_log_path` either writes a
  NUL-terminated path and returns its byte count (excluding the NUL) or returns
  `MARS_XLOG_ERR_NO_SPACE` — it never writes a partial path.
* **Threading.** All symbols may be called from any thread; `open`/`close` and
  the setters touch process-wide state and belong in start-up / shut-down.

## Where `unsafe` lives

This is the only crate in the workspace allowed `unsafe`, and it is confined to
[`src/cstr.rs`](src/cstr.rs) (three `CStr` / raw-pointer reads) and the single
`slice::from_raw_parts_mut` in `mars_xlog_current_log_path`. Every block carries
a `// SAFETY:` note, and `#![deny(unsafe_op_in_unsafe_fn)]` is on.
