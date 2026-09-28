# The C ABI

`marsrs-<version>-<host>.tar.gz` (Linux, macOS) and `.zip` (Windows) hold the
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
    .name_prefix = "marsrs",
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

## The task pipeline

`mars_stn.h` is the half of the port that talks to a server, and it is the same
library: nothing else to link, nothing else to open.

```c
#include <mars_stn.h>

mars_stn_set_app(NULL, ask);          /* one callback answers the eighteen questions */

MarsStnTask task;
memset(&task, 0, sizeof task);
task.taskid = mars_stn_gen_task_id();
task.channel_select = 0x3;            /* both links */
task.cgi = "/cgi-bin/hello";
mars_stn_start_task(&task);

/* no threads in the port: the queues are drained by whoever calls this —
   and the answer is how many milliseconds the pass may wait, with
   MARS_STN_ERR_NO_DUE for "nothing to wait for" */
long long due = mars_stn_due_time();
while (due >= 0) {
    usleep(due * 1000);
    mars_stn_run_pending();
    due = mars_stn_due_time();
}
```

[The task pipeline](/stn) is the whole of it — the two links, the fields of a
task, how a task ends, and what a long link asks of an app.

## The network diagnosis

```c
#include <mars_sdt.h>

MarsSdtIpPort port = { "1.2.3.4", 80 };
MarsSdtHosts longlink[] = { { "default", &port, 1 } };
mars_sdt_set_http_netcheck_cgi("http://example.com/netcheck");
mars_sdt_start_active_check(longlink, 1, NULL, 0, 0, 10000);
mars_sdt_run_checks(NULL, probe, 1);   /* one probe at a time, on the calling thread */

char buffer[4096];
if (mars_sdt_take_report(buffer, sizeof buffer) >= 0) { send(buffer); }
```

The four probes — ping, dns, tcp and http — are the caller's: this port owns no
sockets, so `mars_sdt_run_checks` asks them of the `MarsSdtProbe` you hand it.

[The network diagnosis](/sdt) is the whole of it: the mode, the plan, and the
JSON of the report.

## Errors, of both halves

Every `int` of the two headers answers `MARS_STN_OK` / `MARS_SDT_OK` (0) or a
negative `MARS_STN_ERR_*` / `MARS_SDT_ERR_*` — a `NULL` argument, a buffer too
small, a task the queues refused, a report that does not fit — and
`MARS_STN_ERR_PANIC` / `MARS_SDT_ERR_PANIC` is a Rust panic caught at the
boundary. Nothing in the C ABI unwinds into C, and a diagnosis whose report did
not fit its buffer keeps its results, so a caller who asks again with a bigger
one gets the diagnosis rather than an empty one.
