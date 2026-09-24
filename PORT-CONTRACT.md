# Rust port contract (xlog)

This file is the **single source of truth** for the API surface every crate in
`rust/crates` must expose. Several agents work on different crates in parallel,
so **do not change any signature listed here** without coordination.

The C++ sources this was ported from have been removed from the repository;
only the golden `.xlog` files of `crates/mars-xlog-compat/fixtures`, which were
produced by the original encoders, still witness the on-disk format.

## Workspace layout

```
rust/
  Cargo.toml                  workspace (already created; DO NOT EDIT)
  crates/
    mars-xlog-core/           DONE  - PtrBuffer / AutoBuffer / le helpers
    mars-xlog-crypt/          AGENT 1
    mars-xlog-buffer/         AGENT 2
    mars-xlog-appender/       AGENT 3
    mars-xlog-ffi/            AGENT 4
```

Rules for every agent:

* Only create/modify files **inside your own crate directory**.
  Never touch `rust/Cargo.toml`, other crates, or anything outside `rust/`.
* Use a private target dir so concurrent cargo runs never fight over the lock:
  `export CARGO_TARGET_DIR=/tmp/mrs-<yourcrate>` (or prefix every cargo call).
* Add new third-party deps to `[workspace.dependencies]`? **No** — you cannot
  edit the workspace file. If you need a crate, add it to your crate's
  `Cargo.toml` with an explicit version. Prefer versions already listed in
  `rust/Cargo.toml`'s `[workspace.dependencies]` and wire them with
  `{ workspace = true }`.
* Every public item gets a doc comment saying which C++ symbol it replaces.
* Tests live in `src/**` `#[cfg(test)] mod tests` (unit) and/or `tests/` (integration).
* Must pass: `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test`.
* `cargo` 1.88 / edition 2021 / `#![deny(unsafe_code)]` preferred (ask if you
  genuinely need `unsafe` — only `mars-xlog-ffi` may use it).

---

## `mars-xlog-core` (DONE — read it before you code)

```rust
pub use ptrbuffer::{PtrBuffer, Seek};
pub use autobuffer::AutoBuffer;
pub mod le { pub fn read_u16(&[u8], usize) -> u16; pub fn write_u16(&mut [u8], usize, u16);
             pub fn read_u32(&[u8], usize) -> u32; pub fn write_u32(&mut [u8], usize, u32); }
pub fn local_hour() -> u8;
```

`PtrBuffer<'a>` borrows a region; `AutoBuffer` is owned + growable. Both keep
`pos` / `length` / capacity semantics identical to `mars/comm`.

---

## AGENT 1 — `mars-xlog-crypt`

Port of `mars/xlog/crypt/log_crypt.{h,cc}` and `mars/xlog/crypt/log_magic_num.h`.

On-disk header layout (73 bytes, little-endian — the C++ `memcpy`s native
`u16`/`u32`, and every Mars target is LE):

| offset | size | field                                  |
|--------|------|----------------------------------------|
| 0      | 1    | magic start                            |
| 1      | 2    | seq (`u16`, 0 for sync)                |
| 3      | 1    | begin hour (`u8`, cast from `char`)    |
| 4      | 1    | end hour                               |
| 5      | 4    | length (`u32`)                         |
| 9      | 64   | client pubkey                          |

Tailer: 1 byte, always `0x00`.

Required public API (exact):

```rust
pub const HEADER_LEN: usize;      // 73
pub const TAILER_LEN: usize;      // 1
pub const CLIENT_PUBKEY_LEN: usize; // 64
pub const TEA_BLOCK_LEN: usize;   // 8

pub mod magic {
    pub const SYNC_ZLIB_START: u8;          // 0x06
    pub const SYNC_NOCRYPT_ZLIB_START: u8;  // 0x08
    pub const ASYNC_ZLIB_START: u8;         // 0x07
    pub const ASYNC_NOCRYPT_ZLIB_START: u8; // 0x09
    pub const SYNC_ZSTD_START: u8;          // 0x0A
    pub const SYNC_NOCRYPT_ZSTD_START: u8;  // 0x0B
    pub const ASYNC_ZSTD_START: u8;         // 0x0C
    pub const ASYNC_NOCRYPT_ZSTD_START: u8; // 0x0D
    pub const END: u8;                      // 0x00
    pub fn magic_start_is_valid(m: u8) -> bool;
}

pub struct LogCrypt { /* private */ }

impl LogCrypt {
    /// `LogCrypt(const char* _pubkey)`. `None` / empty / non-hex / wrong length
    /// => `is_crypt() == false` (mirrors the C++ early returns).
    pub fn new(pubkey: Option<&str>) -> Self;
    pub fn is_crypt(&self) -> bool;
    pub fn header_len(&self) -> usize;
    pub fn tailer_len(&self) -> usize;

    // --- statics -------------------------------------------------------
    pub fn get_log_hour(data: &[u8]) -> Option<(u8, u8)>;
    pub fn update_log_hour(data: &mut [u8]);
    pub fn get_log_len(data: &[u8]) -> u32;
    pub fn update_log_len(data: &mut [u8], add_len: u32);
    pub fn set_tailer_info(data: &mut [u8], magic_end: u8);

    // --- instance ------------------------------------------------------
    /// Writes magic, seq, begin/end hour = current local hour, len = 0, pubkey.
    pub fn set_header_info(&mut self, data: &mut [u8], is_async: bool, magic_start: u8);
    /// Allocates `HEADER_LEN + TAILER_LEN + log_data.len()` in `out`, fills
    /// header + body + tailer. Returns the total byte count.
    pub fn crypt_sync_log(&mut self, log_data: &[u8], out: &mut AutoBuffer, magic_start: u8, magic_end: u8) -> usize;
    /// TEA-encrypts whole 8-byte blocks; the trailing `input % 8` bytes are
    /// appended in the clear and reported via `remain_nocrypt_len`.
    pub fn crypt_async_log(&self, log_data: &[u8], out: &mut Vec<u8>, remain_nocrypt_len: &mut usize);
    /// `Fix()`: validates the magic, returns `Some(raw_log_len)` and restores seq.
    pub fn fix(&mut self, data: &[u8]) -> Option<u32>;
}
```

Notes:
* `__GetSeq` is a process-global counter: sync => 0, async => monotonic `u16`
  that skips 0. Use an `AtomicU16` with `fetch_add(1, Relaxed)`, mapping the
  wrapped 0 to 1.
* TEA: 16 rounds, `delta = 0x9e3779b9`, key = 4 x `u32` **read little-endian
  from the first 16 bytes of the ECDH secret** (the C++ `memcpy`s the raw
  `uint8_t[32]` into `uint32_t[4]`).
* Key derivation: the C++ calls `uECC_make_key` / `uECC_shared_secret` on
  **secp256k1** with a 64-byte raw server pubkey given as 128 hex chars.
  Use the pure-Rust `k256` crate (`k256::SecretKey` / `PublicKey` +
  `elliptic_curve::ecdh::diffie_hellman`) to reproduce it. If that proves
  impossible, keep `is_crypt() == false` and clearly document the gap with a
  `TODO(port)` — do **not** silently change the API.
* Include unit tests: header layout offsets, `get_log_len` / `update_log_len`
  round trip, `get_log_hour`, TEA block handling in `crypt_async_log`
  (including the `remain_nocrypt_len` tail), `fix()` on valid/invalid magic,
  and a `decode`-style round trip of `crypt_sync_log` output.

---

## AGENT 2 — `mars-xlog-buffer`

Port of `mars/xlog/src/log_base_buffer.{h,cc}`, `log_zlib_buffer.cc`,
`log_zstd_buffer.cc`.

**Critical design constraint:** the appender owns the mmap and cannot hold a
self-referential `LogBuffer<'a>`. So `LogBuffer` stores *state only* and
receives the backing region on every call.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressMode { Zlib, Zstd }

pub struct LogBuffer { /* private: crypt, compressor, counters, magic start bytes */ }

impl LogBuffer {
    /// `is_compress`: true => run the compressor. `pubkey`: see LogCrypt.
    /// `level`: zstd level (ignored for zlib).
    pub fn new(is_compress: bool, pubkey: Option<&str>, mode: CompressMode, level: i32) -> Self;
    pub fn is_crypt(&self) -> bool;
    pub fn is_compress(&self) -> bool;

    /// `__Fix()` — call once after mapping the cache file: recover the logical
    /// length from a half-written mmap region, or reset to 0.
    pub fn attach(&mut self, region: &mut [u8]);

    /// Logical length of the buffered region.
    pub fn len(&self) -> usize;

    /// `Write(data, length)` (async path): reset when empty, compress into the
    /// region when `is_compress`, then TEA-crypt in place and update the header
    /// length. Returns false when the input is empty or the region is full.
    pub fn write(&mut self, region: &mut [u8], data: &[u8]) -> bool;

    /// `Write(data, len, out_buff)` (sync path): emits a complete
    /// header+body+tailer record into `out`.
    pub fn write_sync(&mut self, data: &[u8], out: &mut AutoBuffer) -> bool;

    /// `Flush(out)`: appends tailer + hour, copies the region into `out` and
    /// clears it. Returns the number of bytes appended to `out` (0 when empty).
    pub fn flush(&mut self, region: &mut [u8], out: &mut AutoBuffer) -> usize;
}

/// `GetPeriodLogs` — scans a finished .xlog file for the byte range covering
/// `[begin_hour, end_hour)`. Returns `(begin_pos, end_pos)`.
pub fn get_period_logs(path: &std::path::Path, begin_hour: i32, end_hour: i32)
    -> Result<(u64, u64), String>;

/// Same, but aborts after `timeout_ms` (the C++ works around an iOS deadlock
/// by running the scan on a detached thread + channel).
pub fn get_period_logs_with_timeout_ms(
    path: &std::path::Path, begin_hour: i32, end_hour: i32, timeout_ms: u32,
) -> Result<(u64, u64), String>;
```

Details that must match the C++:
* `region.len()` is the mmap size; reserve `TAILER_LEN` when compressing
  (`avail_out = max_length - length - tailer_len`).
* After compressing into the region, the *encrypted* result is written back at
  `before_len` and the logical length becomes `before_len + out.len()`.
* `flush` must call `update_log_hour`, append the tailer, then zero the whole
  region (`__Clear`).
* Compression:
  * zlib = **raw DEFLATE** (`deflateInit2(..., -MAX_WBITS, ...)`) with a
    `Z_SYNC_FLUSH` per write. In `flate2` use
    `Compress::new_with_window_bits(Compression::best(), false, 15)` and
    `flush(FlushCompress::Sync)`. Reuse the stream until `flush()`.
  * zstd = streaming `compress_stream2(..., ZSTD_e_flush)` with
    `window_log = 16`. Use the `zstd` crate's `Encoder`/`zio::Writer`, or the
    safe `zstd::stream::copy_encode` equivalent; a `flush()` at the end of each
    write must behave like `ZSTD_e_flush` (not `end`).
* `get_period_logs` must reproduce the C++ scan-and-repair loop, including the
  byte-by-byte resync on a corrupt record and the error string that reports
  `beginpos/endpos/filesize`.

Tests: a fake 4 KiB region; write N records; assert the header magic, length
and hour fields; assert `flush()` emits header+body+tailer and clears the
region; assert `attach()` recovers a length from a hand-crafted region; round
trip `get_period_logs` against a file you produced with `write_sync`.

---

## AGENT 3 — `mars-xlog-appender`

Port of `mars/xlog/src/appender.cc`, `formater.cc`, `xlogger_interface.cc`
and `appender.h` / `xlogger_interface.h`.

```rust
use mars_xlog_buffer::CompressMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppenderMode { Async, Sync }

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel { Verbose = 0, Debug = 1, Info = 2, Warn = 3, Error = 4, Fatal = 5 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIoAction { None = 0, Success = 1, Unnecessary = 2, OpenFailed = 3,
                        ReadFailed = 4, WriteFailed = 5, CloseFailed = 6, RemoveFailed = 7 }

#[derive(Debug, Clone)]
pub struct XLogConfig {
    pub mode: AppenderMode,
    pub logdir: std::path::PathBuf,
    pub nameprefix: String,
    pub pub_key: String,
    pub compress_mode: CompressMode,
    pub compress_level: i32,
    pub cachedir: Option<std::path::PathBuf>,
    pub cache_days: u32,
}
impl Default for XLogConfig { /* logdir = "./log", nameprefix = "Mars",
                                 pub_key = "", compress_mode = Zlib,
                                 compress_level = 6, cachedir = None, cache_days = 0 */ }

#[derive(Debug, Clone)]
pub struct XLoggerInfo {
    pub level: LogLevel,
    pub tag: Option<String>,
    pub filename: Option<String>,
    pub func_name: Option<String>,
    pub line: i32,
    pub pid: i64,
    pub tid: i64,
    pub maintid: i64,
    pub timeval: (i64 /*sec*/, i64 /*usec*/),
}

pub struct AppenderError(pub String);   // implement Display + std::error::Error

// --- lifecycle (process-wide singleton, like the C++ global state) --------
pub fn appender_open(config: XLogConfig) -> Result<(), AppenderError>;
pub fn appender_flush();
pub fn appender_flush_sync();
pub fn appender_close();
pub fn appender_set_mode(mode: AppenderMode);
pub fn appender_set_console_log(open: bool);
pub fn appender_set_max_file_size(bytes: u64);
pub fn appender_set_max_alive_duration(secs: u64);
pub fn appender_get_current_log_path() -> Option<std::path::PathBuf>;
pub fn appender_get_current_log_cache_path() -> Option<std::path::PathBuf>;
pub fn appender_oneshot_flush(config: &XLogConfig) -> FileIoAction;

// --- writing -------------------------------------------------------------
/// Main entry point used by the FFI layer and by `xlog!`-style macros.
pub fn appender_write(info: Option<&XLoggerInfo>, logbody: &str) -> bool;

// --- file discovery ------------------------------------------------------
/// File names for the day `timespan` seconds after the epoch.
pub fn appender_make_logfile_name(timespan: i64, prefix: &str, logdir: &std::path::Path)
    -> Vec<std::path::PathBuf>;
/// Existing files whose span covers `timespan`.
pub fn appender_getfilepath_from_timespan(timespan: i64, prefix: &str, logdir: &std::path::Path)
    -> Vec<std::path::PathBuf>;

// --- formatting ----------------------------------------------------------
/// Port of `mars::xlog::log_formater`. Writes the
/// `[L][yyyy-mm-dd +z hh:mm:ss.mmm][pid, tid*][tag][file:line, func][body\n]`
/// record into `out`.
pub fn log_formater(info: Option<&XLoggerInfo>, logbody: Option<&str>, out: &mut PtrBuffer<'_>);
```

Behaviour to reproduce (read `appender.cc` for the details):
* Cache file `<cachedir>/<prefix>.mmap3` sized to the mmap page budget; the
  `LogBuffer` is attached to it. On open, any leftover content is flushed into
  the current log file.
* Log file `<logdir>/<prefix>_<YYYYMMDD>.xlog`; a new file is created when the
  day changes or `max_file_size` is exceeded (then `_1`, `_2`, ... suffixes).
* `Async` mode spawns one writer thread fed by an unbounded channel;
  `Sync` mode writes under a lock on the calling thread. `appender_flush()`
  signals the thread, `appender_flush_sync()` also waits for it.
* `appender_set_max_alive_duration` (default 10 days) prunes old files on open.
* `appender_oneshot_flush` flushes the mmap cache into the log file without
  starting the appender.
* `log_formater` must keep the C++ guard: when less than 5 KiB of room is left
  it emits `[F]log_size <= 5*1024, err(count, size)\n`; the body is capped at
  `0xFFFF` bytes and at `max_length - length - 130`; a trailing `\n` is added
  when missing.
* The process-wide singleton must be `Send + Sync` and safe to call from
  several threads; prefer `OnceLock` + `Mutex`/`RwLock`. No `unsafe`.

Tests: open into a `tempfile` dir, write records, flush, close, then read the
`.xlog` file back and assert the record framing (magic + length + tailer) and
that the body is present; `appender_make_logfile_name` naming; `log_formater`
output shape; async + sync modes; max-file-size rotation.

---

## AGENT 4 — `mars-xlog-ffi`

A `cdylib` + `staticlib` exposing a C ABI so the existing C++ / JNI / ObjC
layers can call the Rust implementation. This is the seam that lets the port
land incrementally.

```c
/* mars_xlog.h (generated by hand, checked in next to the crate) */
typedef enum { MarsAppenderAsync = 0, MarsAppenderSync = 1 } MarsAppenderMode;
typedef enum { MarsCompressZlib = 0, MarsCompressZstd = 1 } MarsCompressMode;
typedef enum { MarsLevelVerbose = 0, ... MarsLevelFatal = 5 } MarsLogLevel;

typedef struct {
  int         mode;            /* MarsAppenderMode  */
  const char* log_dir;
  const char* name_prefix;
  const char* pub_key;
  int         compress_mode;   /* MarsCompressMode  */
  int         compress_level;
  const char* cache_dir;       /* may be NULL */
  int         cache_days;
} MarsXLogConfig;

int  mars_xlog_open(const MarsXLogConfig* config);   /* 0 = ok, <0 = error */
void mars_xlog_write(int level,
                     const char* tag,
                     const char* filename,
                     const char* func_name,
                     int line,
                     const char* message);
void mars_xlog_flush(void);
void mars_xlog_flush_sync(void);
void mars_xlog_close(void);
void mars_xlog_set_level(int level);
void mars_xlog_set_console_log(int open);
void mars_xlog_set_max_file_size(unsigned long long bytes);
void mars_xlog_set_max_alive_duration(long long seconds);
int  mars_xlog_current_log_path(char* out, unsigned int len);   /* bytes written, <0 on error */
```

Requirements:
* `#[no_mangle] pub extern "C" fn` for every symbol above, `#[repr(C)]` config
  struct, `catch_unwind` at every boundary (never unwind into C), and
  `std::ptr::null()` checks on every incoming pointer.
* All strings are NUL-terminated UTF-8; use `CStr::from_ptr` and fall back to
  an empty string rather than panicking.
* Ship `include/mars_xlog.h` in the crate and a `build.rs`? Not required —
  just check the header in next to the sources and document its path.
* This crate is the **only** one allowed `unsafe`; keep it confined to the FFI
  shims and comment every block with a safety note.
* Add a `tests/ffi_smoke.rs` integration test that drives the C ABI from Rust
  (open into a temp dir, write, flush, close, read the file back).

---

## Definition of done (whole workspace)

```
cd rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test  --workspace
cargo build --workspace --release
```
