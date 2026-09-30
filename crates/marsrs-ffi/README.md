# marsrs-ffi

The C ABI seam of the Rust xlog port: a `cdylib` + `staticlib` (+ `rlib`, so the
Rust tests can call the same symbols) that lets the existing C++, JNI and ObjC
layers of Mars talk to `marsrs-appender` **without** a big-bang rewrite.

| C symbol (this crate)            | C++ it replaces                                                |
| -------------------------------- | -------------------------------------------------------------- |
| `mars_xlog_new_instance`         | `mars::xlog::NewXloggerInstance(_config, (TLogLevel)_level)`    |
| `mars_xlog_write_instance`       | `mars::xlog::XloggerWrite(...)` + `xlogger_IsEnabledFor`        |
| `mars_xlog_request_flush_instance` | `mars::xlog::appender_flush()`                                 |
| `mars_xlog_flush_now_instance`   | `mars::xlog::appender_flush_sync()`                            |
| `mars_xlog_release_instance`     | `mars::xlog::appender_close()`                                 |
| `mars_xlog_set_level_instance`   | `xlogger_SetLevel()` / `SetLevel` on an instance                |
| `mars_xlog_set_console_log_instance` | `mars::xlog::appender_set_console_log(bool)`                |
| `mars_xlog_set_max_file_size_instance` | `mars::xlog::appender_set_max_file_size(uint64_t)`        |
| `mars_xlog_set_max_alive_duration_instance` | `mars::xlog::appender_set_max_alive_duration(long)`  |
| `mars_xlog_current_log_path_instance` | the log directory of the appender of `instance`             |
| `mars_xlog_getfilepath_from_timespan_instance` | `mars::xlog::appender_getfilepath_from_timespan()`  |
| `mars_xlog_make_logfile_name_instance` | `mars::xlog::appender_make_logfile_name()`              |

There is one spelling per operation and not two: the C++ has a free function
for its process-wide appender beside each handle-taking one, and a second
spelling here would be two ways to say one thing — which is a thing the
platforms avoid by keeping the handle in the object an app holds. No symbol of
this ABI installs a process-wide appender, so handle `0` names no logger at
all: it is what an open that failed answers, and every symbol asked of it is a
no-op.

The header is checked in at **`include/mars_xlog.h`** (hand written, no cbindgen
step); `tests/header_sync.rs` fails if it drifts from `src/abi.rs`.
**`include/mars_xlog.hpp`** is the same seam in C++ — `marsrs::xlog::Xlog`, the
`Xlog` every platform of the port has, over these very symbols — and
`tests/cpp_surface.rs` compiles it and reads the symbols back out of the object
file.

## Build

```sh
cargo build -p marsrs-ffi --release
```

which produces

* `target/release/libmars_ffi.a` — static
* `target/release/libmars_ffi.{so,dylib}` — dynamic

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
    .name_prefix   = "Mars",                /* must not be NULL or empty */
    .pub_key       = NULL,                  /* NULL/"" -> no encryption */
    .compress_mode = MarsCompressZlib,      /* or MarsCompressZstd */
    .compress_level = 0,                    /* <= 0 -> appender default (6) */
    .cache_dir     = NULL,                  /* NULL -> use log_dir */
    .cache_days    = 0,
};

/* The level is the second argument and not a field of the config: it belongs
   to the logger and not to the file. `0` is what an instance that could not
   be opened answers, so it is the one failure to handle. */
long long xlog = mars_xlog_new_instance(&cfg, MarsLevelInfo);
if (xlog == 0) { /* handle */ }
mars_xlog_write_instance(xlog, MarsLevelInfo, "tag", __FILE__, __func__, __LINE__, "hello");
mars_xlog_flush_now_instance(xlog);

char path[512];
int n = mars_xlog_current_log_path_instance(xlog, path, sizeof(path));   /* bytes, excl. NUL */

mars_xlog_release_instance("Mars");
```

Compile and link:

```sh
# static
clang log_bridge.c -I crates/marsrs-ffi/include \
      target/release/libmars_ffi.a -lpthread -ldl -lm -o demo
# dynamic
clang log_bridge.c -I crates/marsrs-ffi/include \
      -L target/release -lmars_ffi -lpthread -ldl -lm -o demo
```

On Apple platforms the Rust run-time needs the system frameworks too:

```sh
clang log_bridge.c -I crates/marsrs-ffi/include \
      target/release/libmars_ffi.a \
      -framework CoreFoundation -framework Security -lpthread -ldl -lm -o demo
```

For an iOS app, link `libmars_ffi.a` (a `lipo` fat archive for all target
slices) in *Link Binary With Libraries* together with `CoreFoundation`,
`Security` and `libSystem`.

## Linking from C++

`include/mars_xlog.hpp` is the C ABI in C++ — one header, and nothing to link
but the library above. `Xlog::open(config)` is the `Xlog.open(config)` of
Kotlin, of Dart and of TypeScript, and the appender it answers is closed by the
destructor when it goes out of scope:

```cpp
#include "mars_xlog.hpp"

marsrs::xlog::XlogConfig config;
config.logDir = "/tmp/marslog";
config.namePrefix = "marsrs";
config.level = marsrs::xlog::LogLevel::Info;

auto log = marsrs::xlog::Xlog::open(config);   // throws marsrs::xlog::XlogError
log.i("startup", "hello");
log.flushNow();                                // on disk when it returns
log.flush().get();                             // the same drain, off this thread
```

C++17: `std::optional<std::string>` is what `currentLogPath()` answers and
`std::vector<std::string>` what `logFiles()` / `logFileNames()` answer, and
`std::future<void>` is what `flush()` answers. `Xlog` is move-only — a prefix is
one appender, and two copies of one handle are two owners of one close.

## Replacing a JNI call site

`mars/xlog/jni/Java2C_Xlog.cc` reads an `Xlog` config object out of Java and
calls the process-wide `appender_open` + `xlogger_SetLevel`. The equivalent
through this shim is a single `mars_xlog_new_instance(&cfg, level)` — the level
is its second argument, and not a second call — and `logWrite` becomes one
`mars_xlog_write_instance` (the level gate the JNI code performs with
`xlogger_IsEnabledFor` is built in).

## Guarantees at the boundary

* **No panic ever unwinds into C.** Every entry point wraps its body in
  `catch_unwind`; a panic is reported as `MARS_XLOG_ERR_PANIC` (or swallowed for
  the `void` symbols) and the message still reaches stderr. No symbol takes a
  callback, so there is no `extern "C"` frame here that is not one of these
  entry points — which is the whole of the unwind surface.
* **No null dereference.** Every incoming pointer is null-checked; null and
  invalid UTF-8 degrade to an empty string. `mars_xlog_new_instance` has no
  code to answer with, so it gives back handle `0` — which no symbol asks an
  appender through.
* **No truncation surprises.** `mars_xlog_current_log_path_instance` either
  writes a NUL-terminated path and returns its byte count (excluding the NUL)
  or returns `MARS_XLOG_ERR_NO_SPACE` — it never writes a partial path.
* **Threading.** All symbols may be called from any thread;
  `mars_xlog_new_instance` / `mars_xlog_release_instance` and the setters touch
  state every thread reads, and belong in start-up / shut-down.

## Where `unsafe` lives

This is the only crate in the workspace allowed `unsafe`, and it is confined to
[`src/cstr.rs`](src/cstr.rs) (three `CStr` / raw-pointer reads) and the
`slice::from_raw_parts_mut` of the three path-answering symbols. Every block
carries a `// SAFETY:` note, and `#![deny(unsafe_op_in_unsafe_fn)]` is on.
