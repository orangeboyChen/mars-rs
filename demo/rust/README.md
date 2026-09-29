# The Rust demo

A binary over `marsrs-xlog`: opens an appender in `log/`, writes six records,
flushes, and decodes today's file back to text.

```bash
cargo run
```

The crate takes the port by path and not from crates.io: `marsrs-xlog` is not
published there, so `Cargo.toml` says

```toml
marsrs-xlog = { path = "../../crates/marsrs-xlog" }
```

and the empty `[workspace]` table above it is what takes this package out of the
repository's own workspace — without it, `cargo` answers "current package
believes it's in a workspace when it's not". An app outside this repository
writes `marsrs-xlog = "0.1.0"` instead, once the crate is up.

## What to look at

- `XlogConfig` — every option has the default the C++ gives it, so the demo
  names four and leaves the rest.
- `appender_write` with an `XLoggerInfo` — the file, the function and the line
  of a record are given by the caller here, where Swift fills them in at the
  call site.
- `is_enabled_for` — the check that goes before a message that is expensive to
  build.
- `decode_log_file` — the appender's own file read back, which is what the CLI
  does.

`appender_get_current_log_path` is printed as "writing into", and it answers a
directory and not a file: it is the C++'s `GetCurrentLogPath`, whose answer is
`sg_logdir`. The file of a day is
`appender_getfilepath_from_timespan(0, PREFIX, &logdir)`.
