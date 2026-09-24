# mars-xlog-ffi

The C ABI seam of the Rust xlog port: a `cdylib` + `staticlib` (+ `rlib`, so the
Rust tests can call the same symbols) that lets the existing C++, JNI and ObjC
layers of Mars talk to `mars-xlog-appender` **without** a big-bang rewrite.

| C symbol (this crate)            | C++ it replaces                                                |
| -------------------------------- | -------------------------------------------------------------- |
| `mars_xlog_open`                 | `mars::xlog::appender_open(const XLogConfig&)`                 |
| `mars_xlog_write`                | `mars::xlog::XloggerWrite(...)` + `xlogger_IsEnabledFor`        |
| `mars_xlog_flush`                | `mars::xlog::appender_flush()`                                 |
| `mars_xlog_flush_sync`           | `mars::xlog::appender_flush_sync()`                            |
| `mars_xlog_close`                | `mars::xlog::appender_close()`                                 |
| `mars_xlog_set_level`            | `xlogger_SetLevel()`                                           |
| `mars_xlog_set_console_log`      | `mars::xlog::appender_set_console_log(bool)`                   |
| `mars_xlog_set_max_file_size`    | `mars::xlog::appender_set_max_file_size(uint64_t)`             |
| `mars_xlog_set_max_alive_duration` | `mars::xlog::appender_set_max_alive_duration(long)`          |
| `mars_xlog_current_log_path`     | `mars::xlog::appender_get_current_log_path(char*, unsigned)`   |

The header is checked in at **`include/mars_xlog.h`** (hand written, no cbindgen
step); `tests/header_sync.rs` fails if it drifts from `src/abi.rs`.

## Build

```sh
cd rust
export CARGO_TARGET_DIR=/tmp/mrs-ffi
cargo build -p mars-xlog-ffi --release
```

which produces

* `$CARGO_TARGET_DIR/release/libmars_xlog_ffi.a` — static
* `$CARGO_TARGET_DIR/release/libmars_xlog_ffi.{so,dylib}` — dynamic

Cross-compiling works the usual way, e.g.

```sh
cargo build -p mars-xlog-ffi --release --target aarch64-linux-android
cargo build -p mars-xlog-ffi --release --target aarch64-apple-ios
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
mars_xlog_set_level(MarsLevelInfo);
mars_xlog_write(MarsLevelInfo, "tag", __FILE__, __func__, __LINE__, "hello");
mars_xlog_flush_sync();

char path[512];
int n = mars_xlog_current_log_path(path, sizeof(path));   /* bytes, excl. NUL */

mars_xlog_close();
```

Compile and link:

```sh
# static
clang log_bridge.c -I rust/crates/mars-xlog-ffi/include \
      rust/target/release/libmars_xlog_ffi.a -lpthread -ldl -lm -o demo
# dynamic
clang log_bridge.c -I rust/crates/mars-xlog-ffi/include \
      -L rust/target/release -lmars_xlog_ffi -lpthread -ldl -lm -o demo
```

On Apple platforms the Rust run-time needs the system frameworks too:

```sh
clang log_bridge.c -I rust/crates/mars-xlog-ffi/include \
      rust/target/release/libmars_xlog_ffi.a \
      -framework CoreFoundation -framework Security -lpthread -ldl -lm -o demo
```

For an iOS app, link `libmars_xlog_ffi.a` (a `lipo` fat archive for all target
slices) in *Link Binary With Libraries* together with `CoreFoundation`,
`Security` and `libSystem`.

## Replacing a JNI call site

`mars/xlog/jni/Java2C_Xlog.cc` reads an `Xlog` config object out of Java and
calls `appender_open` + `xlogger_SetLevel`. The equivalent through this shim is
a single `mars_xlog_open` followed by `mars_xlog_set_level`; `logWrite` becomes
one `mars_xlog_write` (the level gate the JNI code performs with
`xlogger_IsEnabledFor` is built in).

## Guarantees at the boundary

* **No panic ever unwinds into C.** Every entry point wraps its body in
  `catch_unwind`; a panic is reported as `MARS_XLOG_ERR_PANIC` (or swallowed for
  the `void` symbols) and the message still reaches stderr.
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
