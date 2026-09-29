# The C demo

A binary over the C ABI — `mars_xlog.h`, the 23 `mars_xlog_*` symbols
`crates/marsrs-ffi` exports. No Rust, and no Kotlin, Swift or Dart anywhere in
the build: one `demo.c` linked against a static library.

```bash
make run
```

The `Makefile` builds `libmars_ffi.a` out of the checkout first
(`cargo build --release -p marsrs-ffi`), which is what a release carries as
`marsrs-android-native.zip` and as the three `libmars_ffi.so` of the HarmonyOS
package. An app that has one already points `LIB` at it and skips the build.

On macOS the link needs `-framework CoreFoundation`, and the `Makefile` adds it:
the static library's time-zone lookup reaches it, and a link without it fails on
`CFRelease` and friends. `-lpthread` and `-ldl` are what the Rust core wants on
Linux.

## What to look at

- `MarsXLogConfig`, zeroed with `memset`: every default of the C++'s own struct
  is `0`, so a config is filled field by field and not built by a constructor.
- `mars_xlog_new_instance(&config, level)` — the appender this program holds,
  and the handle every call below is asked of. There is no process-wide
  appender to open from C: an app gets an instance, the way it gets an `Xlog`
  in Rust and in Kotlin.
- `mars_xlog_write_instance(handle, level, tag, __FILE__, __func__, __LINE__,
  message)` — where a record was written goes *in* the record, and C is the one
  language that has to spell all three out.
- `mars_xlog_current_log_path_instance` — the directory that instance writes
  into. It takes the handle, unlike `mars_xlog_current_log_path`, which can
  only answer for a process-wide appender a C caller never opened.
- `mars_xlog_getfilepath_from_timespan` — the file of one day.
