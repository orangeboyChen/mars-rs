# The C ABI

`mars-rs-<version>-<host>.tar.gz` (Linux, macOS) and `.zip` (Windows) hold the
headers and the libraries of `marsrs-ffi`:

```text
include/mars_xlog.h     the logger
include/mars_sdt.h      the network diagnosis
include/mars_stn.h      the task pipeline
libmars_ffi.a / libmars_ffi.so (.dylib, .dll)
```

built for `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin` and
`x86_64-pc-windows-msvc`, with the task pipeline and the diagnosis included.

## Open, write, flush, close

```c
#include <mars_xlog.h>

MarsXLogConfig config = {
    .mode = MarsAppenderAsync,
    .log_dir = "/tmp/mars-log",
    .name_prefix = "Ham",
    .compress_mode = MarsCompressZlib,
    /* .pub_key, .compress_level, .cache_dir, .cache_days — see the header */
};

int rc = mars_xlog_open(&config);
if (rc != MARS_XLOG_OK) { /* MARS_XLOG_ERR_* — see the header */ }

mars_xlog_set_level(MarsLevelInfo);

mars_xlog_write(MarsLevelInfo, "startup", __FILE__, __func__, __LINE__, "hello from mars");

mars_xlog_flush_sync();   /* the records are on disk when this returns */
mars_xlog_close();
```

Every pointer in `MarsXLogConfig` has to be NUL-terminated UTF-8 or `NULL`;
`NULL` is "empty", except for `log_dir`, which is mandatory. `tag`, `filename`,
`func_name` and `message` of a write may be `NULL`.

`mars_xlog_open` installs one process-wide appender, which `mars_xlog_write`
writes through. `mars_xlog_flush()` only signals the writer thread;
`mars_xlog_flush_sync()` waits.

## More than one appender

```c
long long id = mars_xlog_new_instance(&config, MarsLevelInfo);
if (id != 0) {
    mars_xlog_write_instance(id, MarsLevelDebug, "net", __FILE__, __func__, __LINE__, "…");
    mars_xlog_flush_instance(id, 1 /* sync */);
    mars_xlog_get_level(id);
    mars_xlog_set_level_instance(id, MarsLevelWarning);
    mars_xlog_set_mode_instance(id, MarsAppenderSync);
    mars_xlog_release_instance(config.name_prefix);
}
```

`mars_xlog_new_instance` answers `0` when it refuses the config. An instance is
known by its `name_prefix`, and `mars_xlog_get_instance(name_prefix)` answers the
one that is already open.

## Linking

```bash
cc -I include -o app app.c libmars_ffi.a -lpthread -ldl     # static
cc -I include -o app app.c -L. -lmars_ffi                  # shared
```

The headers are checked in next to the crate, at `crates/marsrs-ffi/include`, so
a build can point at the repository instead of at a copy.

## The current file, and the errors

```c
char path[1024];
if (mars_xlog_current_log_path(path, sizeof path) == MARS_XLOG_OK) {
    /* path is where the appender is writing */
}
```

Every call that returns an `int` answers `MARS_XLOG_OK` (0) or a negative
`MARS_XLOG_ERR_*` — a `NULL` config, a mode or a compressor that is not one, an
empty `log_dir`, an appender that refused the config, an output buffer too small,
no file open yet. `MARS_XLOG_ERR_PANIC` is a Rust panic caught at the boundary;
nothing in the C ABI unwinds into C.
