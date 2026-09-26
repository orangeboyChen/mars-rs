//! Port of `mars/xlog/src/xlogger_appender.h` / `appender.cc` — the
//! `XloggerAppender` class plus the process-wide `sg_default_appender`
//! singleton that `appender.cc` keeps in file scope.
//!
//! # Differences from the C++ (all deliberate, all documented inline)
//!
//! * The C++ keeps two locks (`mutex_buffer_async_`, `mutex_log_file_`) and a
//!   condition variable. The port keeps one `Mutex` around the whole
//!   [`AppenderInner`] plus an `mpsc` channel for the wake-ups, which is
//!   equivalent for callers and cannot deadlock.
//! * That one lock is **not** held while the record is formatted. The C++
//!   formats before it locks anything — `__WriteSync` / `__WriteAsync` fill
//!   `char temp[16 * 1024]` on the stack and only `__Log2File` takes a lock —
//!   and the port does the same (see [`Appender::write`]): formatting is the
//!   largest single part of a record, and holding one lock over it and over
//!   the file write it precedes turns `N` logging threads into one.
//! * `char temp[16 * 1024] = {0}` is zeroed on every call in the C++, 16 KiB
//!   of stores per record. The port keeps one such buffer per thread instead
//!   and never re-zeroes it: a record only ever reads the bytes the formatter
//!   just wrote (see [`format_record`]).
//! * The C++ writes through a `FILE*`, so its records sit in the `stdio`
//!   buffer until that fills. The port used to `write` each record on its own,
//!   which is one syscall — and one turn through the kernel's file lock, the
//!   thing that makes eight logging threads slower than one — per record. It
//!   buffers like the C++ now (see [`AppenderInner::pending`]) and flushes a
//!   little more eagerly: [`Appender::flush_sync`] hands the buffer to the OS
//!   as well, where the C++ leaves it in the `FILE*`.
//! * `__DelTimeoutFile` / `__MoveOldFiles` run on delayed threads in the C++.
//!   The port runs them synchronously inside [`Appender::open`].
//! * Rotation on `max_file_size` is decided when a log file is *opened* in the
//!   C++, which means a long-lived sync-mode file never splits. The port also
//!   closes the current file as soon as a write pushes it past the limit, so
//!   both modes split (see [`AppenderInner::write_file_record`]).
//! * `boost::filesystem::space()` is [`crate::sys::available_space`]
//!   (`statvfs` / `GetDiskFreeSpaceExW`), so the 1 GiB threshold and the two
//!   `space info` records of `__CacheLogs` / `Open` are applied.
//! * `LogBuffer` in this workspace is state-only, so the mmap (or the heap
//!   fallback when mmap fails) lives in [`Region`] and is handed to the buffer
//!   on every call.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mars_buffer::LogBuffer;
use mars_core::{AutoBuffer, PtrBuffer};

use crate::config::{AppenderMode, LogLevel, XLogConfig, XLoggerInfo};
use crate::console::console_log;
use crate::file_util::{
    append_file, del_timeout_file, format_local_timestamp, make_log_file_name, monotonic_millis,
    move_old_files, now_secs, same_local_day, LOG_EXT, MMAP_EXT, SECONDS_PER_DAY,
};
use crate::formater::log_formater;

/// `boost::filesystem::space(...).available >= 1 GiB` in the C++.
const MIN_FREE_SPACE: u64 = 1024 * 1024 * 1024;

/// `kBufferBlockLength` — the size of the mmap (or heap) cache region.
pub(crate) const BUFFER_BLOCK_LENGTH: usize = 150 * 1024;
/// `kMinLogAliveTime` — `SetMaxAliveDuration` ignores anything smaller.
const MIN_LOG_ALIVE_TIME: i64 = 24 * 60 * 60;
/// `max_alive_time_` initialiser in `xlogger_appender.h`.
pub(crate) const DEFAULT_MAX_ALIVE_TIME: i64 = 10 * 24 * 60 * 60;
/// `cond_buffer_async_.wait(15 * 60 * 1000)`.
const ASYNC_WAIT: Duration = Duration::from_secs(15 * 60);
/// `char temp[16 * 1024]` in `__WriteSync` / `__WriteAsync`.
const TEMP_LOG_SIZE: usize = 16 * 1024;
/// When a buffered log file is handed to the OS. The C++ never says: its
/// `fwrite` inherits `FILE*`'s buffer, which is `st_blksize` — 4 KiB on the
/// file systems this was measured on. Same size, named.
const LOG_FLUSH_THRESHOLD: usize = 4 * 1024;
/// `gettimeofday`-free recursion guard threshold (`recursion_count > 10`).
const MAX_RECURSION: u32 = 10;

/// Decrements the per-thread recursion counter when it goes out of scope.
struct RecursionGuard;

impl Drop for RecursionGuard {
    fn drop(&mut self) {
        let _ = std::panic::catch_unwind(|| {
            RECURSION_COUNT.with(|cell| cell.set(cell.get().saturating_sub(1)))
        });
    }
}

// The per-thread recursion counter undone by `RecursionGuard`.
thread_local! {
    static RECURSION_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

// `char temp[16 * 1024]` of `__WriteSync` / `__WriteAsync`, one per thread.
//
// The C++ writes it as `char temp[16 * 1024] = {0}` — 16 KiB of stores on every
// call, which is ~460 ns of the ~2.3 µs a sync record cost before this buffer
// existed. A record only ever reads the bytes the formatter just wrote, so the
// port grows the buffer once per thread and leaves the rest of it alone: the
// same bytes, 16 KiB fewer stores per record.
//
// It is a `thread_local` rather than a field of `AppenderInner` because the
// record is formatted before the lock is taken, which is the whole point — see
// `Appender::write`.
thread_local! {
    static RECORD: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Formats one record into the calling thread's buffer and answers how many
/// bytes of it are the record.
///
/// Holds no lock, which is why [`Appender::write`] calls it first: the
/// formatter is the most expensive part of a record and the C++ runs it
/// outside `mutex_buffer_async_` and `mutex_log_file_` too.
fn format_record(info: Option<&XLoggerInfo>, log: &str) -> usize {
    RECORD.with(|cell| {
        let mut record = cell.borrow_mut();
        // Grown once, on this thread's first record. `log_formater` truncates
        // against `max_length()`, so the buffer has to be the C++'s full
        // 16 KiB or a long body would be cut shorter than the C++ cuts it.
        record.resize(TEMP_LOG_SIZE, 0);
        let mut out = PtrBuffer::new(&mut record[..]);
        log_formater(info, Some(log), &mut out);
        out.len()
    })
}

/// Hands the `len` bytes [`format_record`] left in the buffer to `f`.
///
/// Separate from [`format_record`] so that no borrow of the buffer is alive
/// while the appender's lock is held: nothing under that lock formats, but
/// keeping the two apart is what makes that true by construction.
fn with_record<T>(len: usize, f: impl FnOnce(&[u8]) -> T) -> T {
    RECORD.with(|cell| f(&cell.borrow()[..len]))
}

/// Overwrites the record with `__WriteAsync`'s "the cache is nearly full"
/// marker and answers how many bytes it wrote.
fn record_buffer_full(len: usize) -> usize {
    let msg = format!("[F][ sg_buffer_async.Length() >= BUFFER_BLOCK_LENTH*4/5, len: {len}\n");
    RECORD.with(|cell| {
        let mut record = cell.borrow_mut();
        let n = msg.len().min(TEMP_LOG_SIZE);
        record[..n].copy_from_slice(&msg.as_bytes()[..n]);
        n
    })
}

/// Message sent to the async writer thread (`cond_buffer_async_.notifyAll()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Msg {
    /// Wake up and flush the cache region into the log file.
    Flush,
    /// Wake up, flush once, then exit.
    Close,
}

/// The backing store of the log buffer: the mmap'd cache file, or a heap
/// fallback when the mapping cannot be created (`use_mmap` in the C++).
enum Region {
    Mmap(memmap2::MmapMut),
    Heap(Vec<u8>),
}

impl Region {
    /// A heap region seeded from the cache file, used when the file exists but
    /// cannot be mapped. Those bytes are exactly what `attach()` would recover
    /// from a mapping, so records left by a crashed process still reach the
    /// log on the next flush instead of being resurrected days later.
    fn heap_with_cache(path: &Path) -> Self {
        let mut buffer = vec![0u8; BUFFER_BLOCK_LENGTH];
        if let Ok(bytes) = std::fs::read(path) {
            let len = bytes.len().min(BUFFER_BLOCK_LENGTH);
            buffer[..len].copy_from_slice(&bytes[..len]);
        }
        Region::Heap(buffer)
    }

    /// A zeroed heap region — the `new char[kBufferBlockLength]` fallback.
    fn heap() -> Self {
        Region::Heap(vec![0u8; BUFFER_BLOCK_LENGTH])
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            Region::Mmap(mmap) => &mut mmap[..],
            Region::Heap(vec) => &mut vec[..],
        }
    }
}

/// `OpenMmapFile(mmap_file_path, kBufferBlockLength, mmap_file_)`.
///
/// This is the only `unsafe` in the crate: `memmap2`'s mapping constructors are
/// `unsafe` because the caller must guarantee the file is not truncated or
/// mutated behind the mapping. That is exactly what the appender guarantees —
/// the cache file is only ever written through this mapping until `close()`.
///
/// The crate denies `unsafe_code` outright, so this is the one place that says
/// otherwise: the mapping cannot be made without `unsafe`, and the invariant
/// `memmap2` needs is argued below rather than assumed.
#[allow(unsafe_code)]
fn map_region(file: &File) -> std::io::Result<memmap2::MmapMut> {
    // SAFETY: the invariant memmap2 needs is that nobody truncates or resizes
    // the file while the mapping is alive. Two things hold here: the appender
    // is the only writer of `<prefix>.mmap3` and it only ever writes through
    // this mapping, and the mapping keeps the file alive on its own — the
    // `File` it was created from is dropped at the end of `open_region()`, so
    // the mapping, not the handle, is what pins the inode. What is *not*
    // guaranteed is protection against an outside process (or the C++ xlog
    // still linked into the same app during migration) truncating the file:
    // that would turn every later touch of the mapping into SIGBUS. Opening
    // the same cache file from two implementations at once is unsupported.
    unsafe {
        memmap2::MmapOptions::new()
            .len(BUFFER_BLOCK_LENGTH)
            .map_mut(file)
    }
}

/// Zeroes the cache file. Only needed when there is no mapping: with one,
/// `LogBuffer::flush` and `close()` clear the bytes in place, and the file
/// follows the mapping.
fn clear_cache_file(path: &Path) {
    // Truncate to zero and *keep* it at zero: `set_len` back to the block size
    // would build the same sparse hole that the pre-allocation below exists to
    // prevent, and a zero-length file re-arms that pre-allocation on the next
    // open.
    if let Ok(mut file) = OpenOptions::new().write(true).truncate(true).open(path) {
        let _ = file.flush();
    }
}

/// Opens (creating if needed) and maps the cache file; falls back to a heap
/// region on any error. Returns `(region, use_mmap)`.
fn open_region(path: &Path) -> (Region, bool) {
    let Ok(mut file) = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
    else {
        return (Region::heap(), false);
    };

    // `ftruncate` records a size without reserving blocks; the first store
    // into the mapping is what allocates. On a filesystem that does not
    // reserve on truncate (ext4, f2fs, FAT — the Android targets) a full disk
    // turns that store into SIGBUS, killing the host. `mars/comm/mmap_util.cc`
    // pre-allocates by writing zeros and falls back to the heap path if that
    // write fails; do the same.
    let needs_preallocation = file
        .metadata()
        .map(|meta| meta.len() < BUFFER_BLOCK_LENGTH as u64)
        .unwrap_or(true);
    if file.set_len(BUFFER_BLOCK_LENGTH as u64).is_err() {
        return (Region::heap(), false);
    }
    if needs_preallocation {
        let ok = file
            .seek(SeekFrom::Start(0))
            .and_then(|_| file.write_all(&vec![0u8; BUFFER_BLOCK_LENGTH]))
            .and_then(|()| file.flush())
            .is_ok();
        if !ok {
            return (Region::heap(), false);
        }
    }

    match map_region(&file) {
        Ok(mmap) => (Region::Mmap(mmap), true),
        // mmap is unavailable (sandbox, low memory, some OEM kernels). Fall
        // back to a heap region, but take the on-disk contents with us:
        // otherwise a cache file left by a crashed process is never drained,
        // and a later run where mmap *does* work appends those records to a
        // different day's log, behind a "begin of mmap" banner.
        Err(_) => (Region::heap_with_cache(path), false),
    }
}

/// `"<cachedir or logdir>/<nameprefix>.mmap3"`.
pub(crate) fn mmap_file_path(config: &XLogConfig) -> PathBuf {
    let dir = config
        .cachedir
        .as_deref()
        .unwrap_or(config.logdir.as_path());
    dir.join(format!("{}.{MMAP_EXT}", config.nameprefix))
}

/// The OS thread id of the calling thread (`sys::thread_id`).
///
/// The C++ stamps the real OS tid into every record, so the port does the same
/// instead of handing out a per-thread counter: logs written by C++ and Rust in
/// the same process have to be correlatable.
fn current_tid() -> i64 {
    crate::sys::thread_id()
}

/// `XloggerAppender::__GetMarkInfo` — `"[<pid>,<tid>][<YYYY-MM-DD +z HH:MM:SS>]"`.
fn mark_info() -> String {
    format!(
        "[{},{}][{}]",
        std::process::id(),
        current_tid(),
        format_local_timestamp(now_secs())
    )
}

/// Which directory `__OpenLogFile` opens: the configured log directory, or the
/// cache directory `__Log2File` falls back to when it cannot be written.
enum OpenDir {
    Log,
    Cache,
}

/// The whole mutable state of one appender (`XloggerAppender`'s members).
struct AppenderInner {
    config: XLogConfig,
    /// The mmap'd cache file (or the heap fallback).
    region: Region,
    /// `log_buff_` — state only in this workspace; the region is passed in.
    buff: LogBuffer,
    /// The `AutoBuffer` every `WriteSync` / `WriteTips2File` used to allocate
    /// for itself. Kept here and taken per record instead: it is grown once
    /// and then reused for the lifetime of the appender, so the hot path
    /// allocates nothing.
    scratch: AutoBuffer,
    /// `logfile_`
    log_file: Option<File>,
    /// What has been appended to `log_file` but not handed to the OS yet.
    ///
    /// The C++ writes through a `FILE*`, so its records sit in the `stdio`
    /// buffer until that fills and only then reach the kernel: one `write` for
    /// every ~4 KiB, not one per record. The port used to call `write` per
    /// record, which is one syscall — and, with several threads logging, one
    /// turn through the kernel's file lock — per record. This buffer is the
    /// same thing named, and it is flushed where the C++'s is: when it fills,
    /// when the file is closed or rotated, and (a little more eagerly) by
    /// [`Appender::flush_sync`].
    ///
    /// Allocated with room for one record past the threshold, so the record
    /// that crosses it copies in without reallocating — the buffer is flushed
    /// *after* the append, like the C++'s `fwrite`, which copies a record of
    /// any size into the same buffer.
    pending: Vec<u8>,
    /// How long `log_file` is, as far as this appender is concerned —
    /// [`Self::pending`] included.
    ///
    /// `XloggerAppender::__WriteFile` asks the kernel on every record
    /// (`ftell(_file)`) so that a failed write can roll the file back to where
    /// it started. The port remembers the length instead: this appender is the
    /// only writer of the file it opened — the same invariant `map_region`
    /// argues for the cache file — so the length is what the file held when it
    /// was opened plus everything written since. That is one `fstat` and one
    /// `lseek` fewer per record, on the one path where a logger cannot afford
    /// them: the caller holds the appender's lock for the whole record.
    log_file_len: u64,
    /// `openfiletime_`
    open_file_time: i64,
    /// `last_time_`
    last_time: i64,
    /// `last_tick_` (monotonic milliseconds)
    last_tick: u64,
    /// `last_file_path_`
    last_file_path: PathBuf,
    /// `max_file_size_`
    max_file_size: u64,
    /// `max_alive_time_`
    max_alive_time: i64,
    /// Channel to the async writer thread (replaces `cond_buffer_async_`).
    tx: Option<SyncSender<Msg>>,
    /// Whether the cache region is backed by the mmap file.
    /// Whether the region is the mmap'd cache file (`true`) or a heap buffer.
    use_mmap: bool,
    /// Whether this appender owns `<prefix>.mmap3`. `Appender::oneshot` works
    /// on a file left behind by another process and must never clear it.
    owns_cache: bool,
}

impl AppenderInner {
    fn is_sync(&self) -> bool {
        self.config.mode == AppenderMode::Sync
    }

    /// `cond_buffer_async_.notifyAll()`
    ///
    /// Bounded to one pending wake-up: the C++ condition variable has no
    /// backlog, and an unbounded queue would grow without bound (and memset
    /// 150 KiB per drained message) while the writer thread is stalled.
    fn notify(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(Msg::Flush);
        }
    }

    /// `Close`, which must not be dropped by the coalescing in [`Self::notify`].
    ///
    /// Callers must **not** hold the inner lock: the channel is bounded, so
    /// this can block until the writer thread takes the message, and that
    /// thread needs the lock to drain.
    fn close_sender(&self) -> Option<SyncSender<Msg>> {
        self.tx.clone()
    }

    /// `log_buff_->Flush(_out)`.
    fn flush_buffer(&mut self, out: &mut AutoBuffer) -> usize {
        let region = self.region.as_mut_slice();
        self.buff.flush(region, out)
    }

    /// `XloggerAppender::__WriteSync` — the half that needs the lock.
    ///
    /// `len` is what [`format_record`] left in the calling thread's buffer.
    fn write_sync(&mut self, len: usize) {
        // Taken out of `self` rather than allocated: the buffer is returned
        // (with its capacity) once the record is on its way to the file.
        let mut tmp_buff = std::mem::take(&mut self.scratch);
        tmp_buff.reset();
        let written = with_record(len, |data| self.buff.write_sync(data, &mut tmp_buff));
        if !written {
            self.scratch = tmp_buff;
            return;
        }

        self.log2file(tmp_buff.as_slice(), false);
        self.scratch = tmp_buff;
    }

    /// `XloggerAppender::__WriteAsync` — the half that needs the lock.
    fn write_async(&mut self, info: Option<&XLoggerInfo>, len: usize) {
        let level_fatal = info.is_some_and(|info| info.level == LogLevel::Fatal);

        let len = if self.buff.len() >= BUFFER_BLOCK_LENGTH * 4 / 5 {
            // The C++ overwrites the record it just formatted with this
            // marker; the port overwrites the same buffer it formats into.
            record_buffer_full(self.buff.len())
        } else {
            len
        };

        // The C++ `LogBaseBuffer::Write` never fails here (its `PtrBuffer`
        // clamps the copy and returns true), so a rejected write must still
        // fall through to the flush threshold: returning early would leave a
        // full region until the next fatal record or the 15 minute background
        // wake-up.
        let _written = with_record(len, |data| {
            let region = self.region.as_mut_slice();
            self.buff.write(region, data)
        });

        if self.buff.len() >= BUFFER_BLOCK_LENGTH / 3 || level_fatal {
            self.notify();
        }
    }

    /// `XloggerAppender::WriteTips2File`.
    fn write_tips2file(&mut self, tips: &str) {
        let mut tmp_buff = std::mem::take(&mut self.scratch);
        tmp_buff.reset();
        self.buff.write_sync(tips.as_bytes(), &mut tmp_buff);
        self.log2file(tmp_buff.as_slice(), false);
        self.scratch = tmp_buff;
    }

    /// `XloggerAppender::__WriteTips2Console`.
    fn write_tips2console(&self, tips: &str) {
        let info = XLoggerInfo {
            level: LogLevel::Error,
            ..Default::default()
        };
        console_log(Some(&info), tips);
    }

    /// `XloggerAppender::__Log2File`.
    ///
    /// Answers whether the records reached a file: one-shot recovery has to
    /// know, because it may only drop the mmap cache once every record in it
    /// has been persisted elsewhere.
    fn log2file(&mut self, data: &[u8], move_file: bool) -> bool {
        if data.is_empty() || self.config.logdir.as_os_str().is_empty() {
            return false;
        }

        // The paths are built from `self` where they are used instead of being
        // cloned up front: __Log2File runs once per record, and three
        // `PathBuf`/`String` clones per record were three allocations the C++
        // never makes — it passes `const char*` around.
        if self.config.cachedir.is_none() {
            if self.open_log_file(OpenDir::Log) {
                let written = self.write_file_record(data);
                if !self.is_sync() {
                    self.close_log_file();
                }
                return written;
            }
            return false;
        }

        let tv = now_secs();
        let cache_path = self.cache_file_path(tv);
        let cache_logs = self.cache_logs();

        if (cache_logs || cache_path.exists()) && self.open_log_file(OpenDir::Cache) {
            let written = self.write_file_record(data);
            if !self.is_sync() {
                self.close_log_file();
            }

            if cache_logs || !move_file {
                return written;
            }

            let log_path = self.log_file_path(tv);
            // `__AppendFile` reads the file this appender has been writing, so
            // the buffer has to reach it first.
            let flushed = self.flush_pending();
            if flushed && append_file(&cache_path, &log_path) {
                if self.is_sync() {
                    self.close_log_file();
                }
                let _ = fs::remove_file(&cache_path);
            }
            return written;
        }

        let mut write_success = false;
        let open_success = self.open_log_file(OpenDir::Log);
        if open_success {
            write_success = self.write_file_record(data);
            if !self.is_sync() {
                self.close_log_file();
            }
        }

        if !write_success {
            if open_success && self.is_sync() {
                self.close_log_file();
            }
            if self.open_log_file(OpenDir::Cache) {
                write_success = self.write_file_record(data);
                if !self.is_sync() {
                    self.close_log_file();
                }
            }
        }
        write_success
    }

    /// The log file for `tv`, in the configured log directory.
    fn log_file_path(&self, tv: i64) -> PathBuf {
        make_log_file_name(
            tv,
            &self.config.logdir,
            &self.config.logdir,
            &self.config.nameprefix,
            LOG_EXT,
            self.max_file_size,
            self.config.cachedir.as_deref(),
        )
    }

    /// The log file for `tv`, in the cache directory `__Log2File` falls back to
    /// when the log directory cannot be written.
    fn cache_file_path(&self, tv: i64) -> PathBuf {
        let cachedir = self.config.cachedir.as_deref().unwrap_or(Path::new(""));
        make_log_file_name(
            tv,
            cachedir,
            &self.config.logdir,
            &self.config.nameprefix,
            LOG_EXT,
            self.max_file_size,
            Some(cachedir),
        )
    }

    /// `XloggerAppender::__CacheLogs`.
    fn cache_logs(&self) -> bool {
        let Some(cachedir) = self.config.cachedir.as_ref() else {
            return false;
        };
        if self.config.cache_days == 0 {
            return false;
        }

        let tv = now_secs();
        if self.log_file_path(tv).exists() {
            return false;
        }

        // `boost::filesystem::space(cachedir).available >= 1 GiB`.
        match crate::sys::available_space(cachedir) {
            Some(available) => available >= MIN_FREE_SPACE,
            // The query failed: behave like the old port and do not block the
            // flush on an unknown disk.
            None => true,
        }
    }

    /// `XloggerAppender::__OpenLogFile`.
    ///
    /// The directory is named rather than passed, so the caller does not have
    /// to clone it out of `self` before it can hand `self` over mutably:
    /// `__Log2File` runs per record, and the clone was one of the allocations
    /// that made the port slower than the C++ it mirrors.
    fn open_log_file(&mut self, dir: OpenDir) -> bool {
        if self.config.logdir.as_os_str().is_empty() {
            return false;
        }

        let now_time = now_secs();

        if self.log_file.is_some() {
            if same_local_day(self.open_file_time, now_time) {
                return true;
            }
            self.close_log_file();
        }

        self.open_file_time = now_time;
        let logfilepath = match dir {
            OpenDir::Log => self.log_file_path(now_time),
            OpenDir::Cache => self.cache_file_path(now_time),
        };

        if now_time < self.last_time {
            // The clock jumped backwards: keep using the previous file.
            let last_file_path = self.last_file_path.clone();
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&last_file_path)
            {
                Ok(file) => {
                    self.log_file_len = file.metadata().map_or(0, |meta| meta.len());
                    self.log_file = Some(file);
                    true
                }
                Err(err) => {
                    self.write_tips2console(&format!(
                        "open file error:{} {}, path:{}",
                        err.raw_os_error().unwrap_or(0),
                        err,
                        last_file_path.display()
                    ));
                    false
                }
            }
        } else {
            let file = match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&logfilepath)
            {
                Ok(file) => file,
                Err(err) => {
                    self.write_tips2console(&format!(
                        "open file error:{} {}, path:{}",
                        err.raw_os_error().unwrap_or(0),
                        err,
                        logfilepath.display()
                    ));
                    return false;
                }
            };
            self.log_file_len = file.metadata().map_or(0, |meta| meta.len());
            self.log_file = Some(file);

            let now_tick = monotonic_millis();
            let tick_diff = now_tick.saturating_sub(self.last_tick);
            if self.last_time != 0 && (now_time - self.last_time) > (tick_diff / 1000) as i64 + 300
            {
                let msg = format!(
                    "[F][ last log file:{} from {} to {}, time_diff:{}, tick_diff:{}\n",
                    self.last_file_path.display(),
                    format_local_timestamp(self.last_time),
                    format_local_timestamp(now_time),
                    now_time - self.last_time,
                    tick_diff
                );
                let mut tmp_buff = AutoBuffer::new();
                self.buff.write_sync(msg.as_bytes(), &mut tmp_buff);
                self.write_file_record(tmp_buff.as_slice());
            }

            self.last_file_path = logfilepath;
            self.last_tick = now_tick;
            self.last_time = now_time;
            true
        }
    }

    /// `XloggerAppender::__CloseLogFile`.
    fn close_log_file(&mut self) {
        // `fclose` of the C++: whatever is still buffered goes out first.
        let _ = self.flush_pending();
        self.open_file_time = 0;
        self.log_file = None;
        self.log_file_len = 0;
    }

    /// Hands [`Self::pending`] to the OS — the point where the C++'s `FILE*`
    /// buffer is flushed into a `write`.
    ///
    /// Returns `false` on I/O failure, after truncating back to what the OS
    /// held and appending an error record (as `__WriteFile` does).
    fn flush_pending(&mut self) -> bool {
        if self.pending.is_empty() {
            return true;
        }
        if self.log_file.is_none() {
            self.pending.clear();
            return false;
        }

        // What the file holds, so a failure can roll back to it. The C++
        // `ftell`s before every `fwrite` to get the same number; this is the
        // length [`Self::write_file_record`] remembers.
        let before_len = self.log_file_len - self.pending.len() as u64;
        let result = {
            let file = self.log_file.as_mut().expect("checked above");
            file.write_all(&self.pending)
                .map_err(|err| err.raw_os_error().unwrap_or(-1))
        };

        match result {
            Ok(()) => {
                self.pending.clear();
                true
            }
            Err(errno) => {
                if let Some(file) = self.log_file.as_mut() {
                    let _ = file.set_len(before_len);
                    let _ = file.seek(SeekFrom::End(0));
                }
                // The batch is dropped: nothing in it reached the file, and the
                // C++ cannot roll a `stdio` buffer back either.
                self.pending.clear();
                self.log_file_len = before_len;

                self.write_tips2console(&format!("write file error:{errno}"));

                let err_log = format!("\nwrite file error:{errno}\n");
                let mut tmp_buff = AutoBuffer::new();
                self.buff.write_sync(err_log.as_bytes(), &mut tmp_buff);
                let err_len = tmp_buff.len();
                let wrote = self
                    .log_file
                    .as_mut()
                    .is_some_and(|file| file.write_all(tmp_buff.as_slice()).is_ok());
                if wrote {
                    self.log_file_len += err_len as u64;
                }
                false
            }
        }
    }

    /// `XloggerAppender::__WriteFile`.
    ///
    /// Appends `data` to the file — through [`Self::pending`], so a record
    /// costs a `memcpy` instead of a syscall, which is what the C++'s `fwrite`
    /// costs. Returns `false` on I/O failure.
    fn write_file_record(&mut self, data: &[u8]) -> bool {
        if self.log_file.is_none() {
            return false;
        }

        self.pending.extend_from_slice(data);
        self.log_file_len += data.len() as u64;

        // Rotation: the C++ only re-computes the split index when the file is
        // (re)opened, so a long-lived sync-mode file never splits. The port
        // closes the file as soon as it grows past the limit, which makes both
        // modes behave the same.
        if self.max_file_size > 0 && self.log_file_len > self.max_file_size {
            self.close_log_file();
            self.open_file_time = 0;
            return true;
        }

        if self.pending.len() >= LOG_FLUSH_THRESHOLD {
            return self.flush_pending();
        }
        true
    }
}

/// The two members `XloggerAppender::Write` reads before it touches anything
/// the lock guards (`log_close_`, `consolelog_open_`).
///
/// The C++ reads both without a lock — a data race, and one that is benign
/// only because nothing ever writes them concurrently with a read. The port
/// spends two `AtomicBool`s on the same read, so that the whole prologue of a
/// record (the two flags, the recursion guard and the formatting) runs with no
/// lock held at all.
struct Flags {
    /// `log_close_`
    log_close: AtomicBool,
    /// `consolelog_open_`
    console_log_open: AtomicBool,
}

/// What the appender and its writer thread share: the flags the write path
/// reads unlocked, and the state it reads under [`Shared::state`].
struct Shared {
    flags: Flags,
    state: Mutex<AppenderInner>,
}

/// `XloggerAppender` (the `sg_default_appender` of `appender.cc`).
///
/// Held behind a `Mutex` by the singleton in [`crate`], and shared with the
/// async writer thread through the `Arc`.
pub(crate) struct Appender {
    shared: Arc<Shared>,
    /// `thread_async_`. Behind a `Mutex` of its own so that stopping the thread
    /// needs `&self`: the process-wide slot hands out `Arc` clones, and a
    /// `close()` that needed `&mut` could never be called on one.
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for Appender {
    /// `close()` is the only path that stops the writer thread, and the thread
    /// can never exit on its own: `tx` lives inside `AppenderInner`, which the
    /// thread itself holds an `Arc` to, so the receiver is never disconnected.
    /// Dropping without closing would leak a thread that wakes up every
    /// `ASYNC_WAIT` for the lifetime of the process, pinning the mapping, the
    /// log file and the whole inner state.
    fn drop(&mut self) {
        self.close();
    }
}

impl Appender {
    fn lock(&self) -> MutexGuard<'_, AppenderInner> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// `XloggerAppender::NewInstance(_config, _max_byte_size)` + `Open()`.
    pub(crate) fn open(
        config: XLogConfig,
        max_file_size: u64,
        max_alive_time: i64,
    ) -> Result<Self, crate::config::AppenderError> {
        use crate::config::AppenderError;

        if config.logdir.as_os_str().is_empty() {
            return Err(AppenderError("appender_open: logdir is empty".to_owned()));
        }

        let cachedir = config.cachedir.clone();
        if let Some(dir) = &cachedir {
            fs::create_dir_all(dir)
                .map_err(|e| AppenderError(format!("create cache dir {}: {e}", dir.display())))?;
        }
        fs::create_dir_all(&config.logdir).map_err(|e| {
            AppenderError(format!("create log dir {}: {e}", config.logdir.display()))
        })?;

        let alive_time = if max_alive_time >= MIN_LOG_ALIVE_TIME {
            max_alive_time
        } else {
            DEFAULT_MAX_ALIVE_TIME
        };

        // The C++ runs these on 2-3 minute delayed threads; the port prunes at
        // open time so the directory is clean by the time `open` returns.
        if let Some(dir) = &cachedir {
            del_timeout_file(dir, alive_time);
            move_old_files(dir, &config.logdir, &config.nameprefix, config.cache_days);
        }
        del_timeout_file(&config.logdir, alive_time);

        let mmap_path = mmap_file_path(&config);
        let (mut region, use_mmap) = open_region(&mmap_path);

        let mut buff = LogBuffer::new(
            true,
            Some(config.pub_key.as_str()),
            config.compress_mode,
            config.compress_level,
        );
        buff.attach(region.as_mut_slice());

        let start = std::time::Instant::now();
        let inner = AppenderInner {
            config,
            region,
            buff,
            scratch: AutoBuffer::new(),
            log_file: None,
            pending: Vec::with_capacity(2 * LOG_FLUSH_THRESHOLD),
            log_file_len: 0,
            open_file_time: 0,
            last_time: 0,
            last_tick: 0,
            last_file_path: PathBuf::new(),
            max_file_size,
            max_alive_time: alive_time,
            tx: None,
            use_mmap,
            owns_cache: true,
        };

        let appender = Appender {
            shared: Arc::new(Shared {
                flags: Flags {
                    log_close: AtomicBool::new(false),
                    console_log_open: AtomicBool::new(false),
                },
                state: Mutex::new(inner),
            }),
            thread: Mutex::new(None),
        };

        // Anything a previous process left in the cache file.
        let mut leftover = AutoBuffer::new();
        appender.lock().flush_buffer(&mut leftover);

        // Without a mapping nothing ever writes the region back, so the file
        // still holds what was just drained and the next start would append it
        // again — once per start, forever. (With a mapping, `flush_buffer`
        // zeroes it in place.)
        if !use_mmap {
            clear_cache_file(&mmap_path);
        }

        if appender.lock().config.mode == AppenderMode::Async {
            // `flush_buffer` above already cleared the cache, so returning an
            // error here would lose the records that were just drained. The
            // C++ calls SetMode() without checking and writes the leftover
            // regardless; fall back to a synchronous drain instead.
            if let Err(err) = appender.start_thread() {
                appender.lock().config.mode = AppenderMode::Sync;
                appender.lock().log2file(leftover.as_slice(), false);
                return Err(err);
            }
        }

        let mark = mark_info();
        if !leftover.is_empty() {
            appender.write_tips2file("~~~~~ begin of mmap ~~~~~\n");
            appender.lock().log2file(leftover.as_slice(), false);
            appender.write_tips2file(&format!("~~~~~ end of mmap ~~~~~{mark}\n"));
        }

        // `__DATE__` / `__TIME__` of the C++ banner: which build produced this
        // log file, not what time it is now.
        let (build_date, build_time) = crate::file_util::build_stamp();
        appender.write(
            None,
            &format!("^^^^^^^^^^{build_date}^^^{build_time}^^^^^^^^^^^{mark}"),
        );
        appender.write(
            None,
            &format!("get mmap time: {}", start.elapsed().as_millis()),
        );
        // `MARS_URL` / `MARS_PATH` / `MARS_REVISION` / `MARS_BUILD_TIME` /
        // `MARS_BUILD_JOB` — the build identity the C++ splices in from its
        // build system. `build.rs` captures everything but the URL, which is
        // the crate's own `repository`.
        for line in [
            format!("MARS_URL: {}", env!("CARGO_PKG_REPOSITORY")),
            format!("MARS_PATH: {}", env!("MARS_XLOG_SOURCE_PATH")),
            format!("MARS_REVISION: {}", env!("MARS_XLOG_REVISION")),
            format!("MARS_BUILD_TIME: {build_date} {build_time}"),
            format!("MARS_BUILD_JOB: {}", env!("MARS_XLOG_BUILD_JOB")),
        ] {
            appender.write(None, &line);
        }
        // NOTE: the temporary `MutexGuard`s of inline `appender.lock()` calls
        // live until the end of the statement, so they must never be created in
        // the argument list of another `Appender` method (std::sync::Mutex is
        // not reentrant).
        let mode_line = {
            let guard = appender.lock();
            format!(
                "log appender mode:{}, use mmap:{}",
                guard.config.mode as i32, guard.use_mmap as i32
            )
        };
        appender.write(None, &mode_line);

        // `cache dir space info` / `log dir space info` — `Open` writes both,
        // the cache one only when a cache dir is configured. The C++ asks
        // `boost::filesystem::space()`, which throws when the query fails; the
        // port leaves the record out instead of failing the open.
        let (logdir, cachedir) = {
            let guard = appender.lock();
            (guard.config.logdir.clone(), guard.config.cachedir.clone())
        };
        if let Some(cachedir) = cachedir {
            if let Some((capacity, free, available)) = crate::sys::space_info(&cachedir) {
                appender.write(
                    None,
                    &format!(
                        "cache dir space info, capacity:{capacity} free:{free} available:{available}"
                    ),
                );
            }
        }
        if let Some((capacity, free, available)) = crate::sys::space_info(&logdir) {
            appender.write(
                None,
                &format!(
                    "log dir space info, capacity:{capacity} free:{free} available:{available}"
                ),
            );
        }

        Ok(appender)
    }

    /// `XloggerAppender::NewInstance(_config, _max_byte_size, true)` — the
    /// one-shot appender used by [`crate::appender_oneshot_flush`]: config
    /// only, no mmap, no thread.
    pub(crate) fn oneshot(
        config: &XLogConfig,
        max_file_size: u64,
        max_alive_time: i64,
    ) -> Result<Self, crate::config::AppenderError> {
        use crate::config::AppenderError;

        if config.logdir.as_os_str().is_empty() {
            return Err(AppenderError(
                "appender_oneshot_flush: logdir is empty".to_owned(),
            ));
        }
        if let Some(dir) = &config.cachedir {
            fs::create_dir_all(dir)
                .map_err(|e| AppenderError(format!("create cache dir {}: {e}", dir.display())))?;
        }
        fs::create_dir_all(&config.logdir).map_err(|e| {
            AppenderError(format!("create log dir {}: {e}", config.logdir.display()))
        })?;

        let alive_time = if max_alive_time >= MIN_LOG_ALIVE_TIME {
            max_alive_time
        } else {
            DEFAULT_MAX_ALIVE_TIME
        };

        let inner = AppenderInner {
            config: config.clone(),
            region: Region::heap(),
            scratch: AutoBuffer::new(),
            buff: LogBuffer::new(
                true,
                Some(config.pub_key.as_str()),
                config.compress_mode,
                config.compress_level,
            ),
            log_file: None,
            pending: Vec::with_capacity(2 * LOG_FLUSH_THRESHOLD),
            log_file_len: 0,
            open_file_time: 0,
            last_time: 0,
            last_tick: 0,
            last_file_path: PathBuf::new(),
            max_file_size,
            max_alive_time: alive_time,
            tx: None,
            use_mmap: false,
            owns_cache: false,
        };

        Ok(Appender {
            shared: Arc::new(Shared {
                // `log_close = true`: a one-shot appender writes the records it
                // recovered and nothing else.
                flags: Flags {
                    log_close: AtomicBool::new(true),
                    console_log_open: AtomicBool::new(false),
                },
                state: Mutex::new(inner),
            }),
            thread: Mutex::new(None),
        })
    }

    /// `XloggerAppender::TreatMappingAsFileAndFlush`.
    pub(crate) fn treat_mapping_as_file_and_flush(&self) -> crate::config::FileIoAction {
        use crate::config::FileIoAction;

        let config = self.lock().config.clone();
        let mmap_path = mmap_file_path(&config);

        if !mmap_path.exists() {
            return FileIoAction::Unnecessary;
        }

        // Read the whole cache file into a heap region.
        let mut data = vec![0u8; BUFFER_BLOCK_LENGTH];
        let Ok(mut file) = File::open(&mmap_path) else {
            return FileIoAction::OpenFailed;
        };
        if file.read_exact(&mut data).is_err() {
            return FileIoAction::ReadFailed;
        }
        drop(file);

        {
            let mut guard = self.lock();
            let mut buff = LogBuffer::new(
                true,
                Some(config.pub_key.as_str()),
                config.compress_mode,
                config.compress_level,
            );
            guard.region = Region::Heap(data);
            buff.attach(guard.region.as_mut_slice());
            guard.buff = buff;
        }
        // Only now may records be written through it.
        self.shared.flags.log_close.store(false, Ordering::Release);

        let mut buffer = AutoBuffer::new();
        self.lock().flush_buffer(&mut buffer);

        if buffer.is_empty() {
            return FileIoAction::Unnecessary;
        }

        let mark = mark_info();
        self.write_tips2file("~~~~~ begin of mmap from other process ~~~~~\n");
        let written = self.lock().log2file(buffer.as_slice(), false);
        self.write_tips2file(&format!(
            "~~~~~ end of mmap from other process ~~~~~{mark}\n"
        ));

        // The cache is the only copy of these records: keep it when the write
        // failed, so the next recovery can try again instead of losing them.
        // `flush_pending` is part of the write — bytes this process is still
        // holding are not "reached a file" yet.
        let flushed = self.lock().flush_pending();
        if !written || !flushed {
            return FileIoAction::WriteFailed;
        }

        match fs::remove_file(&mmap_path) {
            Ok(()) => FileIoAction::Success,
            Err(_) => FileIoAction::RemoveFailed,
        }
    }

    /// `thread_async_.start()` / `SetMode(kAppenderAsync)`.
    fn start_thread(&self) -> Result<(), crate::config::AppenderError> {
        use crate::config::AppenderError;

        if self.thread_lock().is_some() {
            return Ok(());
        }

        // Bounded at one pending wake-up: a stalled writer thread must not
        // accumulate a message per write (the C++ used a condition variable,
        // which has no backlog).
        let (tx, rx) = mpsc::sync_channel::<Msg>(1);
        self.lock().tx = Some(tx);

        let shared = Arc::clone(&self.shared);
        let handle = thread::Builder::new()
            .name("mars-xlog-async".to_owned())
            .spawn(move || async_log_thread(shared, rx))
            .map_err(|e| AppenderError(format!("start async log thread: {e}")))?;

        *self.thread_lock() = Some(handle);
        Ok(())
    }

    /// `thread_async_`.
    fn thread_lock(&self) -> MutexGuard<'_, Option<JoinHandle<()>>> {
        self.thread
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// `XloggerAppender::Write`.
    ///
    /// Everything up to the [`format_record`] call runs with no lock held,
    /// which is how the C++ does it too: `__WriteSync` / `__WriteAsync` format
    /// into `char temp[16 * 1024]` and only `__Log2File` takes
    /// `mutex_log_file_`. Holding one lock over the formatting *and* the file
    /// write it precedes is what makes `N` logging threads no faster than one —
    /// on this tree, eight threads writing 20 000 records took 2.2x the wall
    /// time one thread took before the split.
    pub(crate) fn write(&self, info: Option<&XLoggerInfo>, log: &str) {
        if self.shared.flags.log_close.load(Ordering::Acquire) {
            return;
        }

        if self.shared.flags.console_log_open.load(Ordering::Relaxed) {
            console_log(info, log);
        }

        // `thread_local uint32_t recursion_count` — protects against logging
        // from inside the logger (which would otherwise recurse forever).
        let count = RECURSION_COUNT.with(|cell| {
            let next = cell.get() + 1;
            cell.set(next);
            next
        });
        // Restored on every exit path, including a panic: leaking it would
        // leave the thread at MAX_RECURSION and silently drop every later
        // record it logs.
        let _recursion_guard = RecursionGuard;

        if count >= 2 {
            if count > MAX_RECURSION {
                return;
            }
            // The C++ also dumps the recursion stack into the log file; the port
            // only reports it on the console, which is the observable part.
            let mut recursive = info.cloned().unwrap_or_default();
            recursive.level = LogLevel::Fatal;
            console_log(
                Some(&recursive),
                &format!("ERROR!!! xlogger_appender Recursive calls!!!, count:{count}"),
            );
            return;
        }

        // The record, formatted before the lock: one buffer per thread, grown
        // once, and nothing else in the process can see it.
        let len = format_record(info, log);

        let mut guard = self.lock();
        // `write_sync` / `write_async` read the record out of that buffer; no
        // borrow of it is held across the lock, which is what keeps the two
        // apart safe.
        if guard.config.mode == AppenderMode::Sync {
            guard.write_sync(len);
        } else {
            guard.write_async(info, len);
        }
    }

    /// `XloggerAppender::WriteTips2File`.
    pub(crate) fn write_tips2file(&self, tips: &str) {
        self.lock().write_tips2file(tips);
    }

    /// `XloggerAppender::Flush` — wake the writer thread.
    pub(crate) fn flush(&self) {
        self.lock().notify();
    }

    /// `XloggerAppender::FlushSync` — flush on the caller's thread.
    pub(crate) fn flush_sync(&self) {
        if self.shared.flags.log_close.load(Ordering::Acquire) {
            return;
        }

        // A synchronous flush is only done once the bytes are the OS's: the
        // C++ leaves them in the `FILE*` here, and the caller of this cannot
        // tell the difference — except by finding them missing from the file.
        // Sync mode has nothing in the cache to drain, but it may well have
        // records in the buffer, so this happens before the mode is asked.
        self.lock().flush_pending();

        let is_sync = self.lock().config.mode == AppenderMode::Sync;
        if is_sync {
            return;
        }

        let mut buffer = AutoBuffer::new();
        let _n = self.lock().flush_buffer(&mut buffer);

        if !buffer.is_empty() {
            self.lock().log2file(buffer.as_slice(), false);
        }
        // ... and again for what that drain buffered.
        self.lock().flush_pending();
    }

    /// `XloggerAppender::Close`.
    pub(crate) fn close(&self) {
        // Mirrors the drain in `open`: without a mapping the file keeps its
        // bytes, so it has to be cleared here too or the next start appends
        // the same records again.
        let (use_mmap, owns_cache, path) = {
            let guard = self.lock();
            (
                guard.use_mmap,
                guard.owns_cache,
                mmap_file_path(&guard.config),
            )
        };
        // Only the owner of the cache file may clear it: `Appender::oneshot`
        // works on another process's file and must leave it alone when it
        // cannot drain or remove it.
        if !use_mmap && owns_cache {
            clear_cache_file(&path);
        }
        let mark = mark_info();
        // `__DATE__` / `__TIME__` again: the twin of the open banner, so the
        // same build stamp brackets the file.
        let (build_date, build_time) = crate::file_util::build_stamp();
        self.write(
            None,
            &format!("$$$$$$$$$${build_date}$$${build_time}$$$$$$$$$${mark}\n"),
        );
        // `fclose` of the C++: the banner, and everything buffered with it, is
        // handed to the OS before the appender goes away.
        self.lock().flush_pending();

        self.shared.flags.log_close.store(true, Ordering::Release);
        // `sync_channel(1)` makes `send` block, so it must not happen under the
        // inner lock: the writer thread needs that lock to drain, and the queue
        // can already be full — the two would wait for each other forever.
        let close_tx = self.lock().close_sender();
        if let Some(tx) = close_tx {
            let _ = tx.send(Msg::Close);
        }

        if let Some(handle) = self.thread_lock().take() {
            let _ = handle.join();
        }

        let mut guard = self.lock();
        guard.tx = None;
        // C++: `memset(mmap_file_.data(), 0, kBufferBlockLength)` before closing
        // the mapping, so a later `open` starts from an empty cache.
        guard.region.as_mut_slice().fill(0);
    }

    /// `XloggerAppender::SetMode`.
    pub(crate) fn set_mode(&self, mode: AppenderMode) -> Result<(), crate::config::AppenderError> {
        let previous = self.lock().config.mode;
        self.lock().config.mode = mode;
        if mode == AppenderMode::Async {
            if let Err(err) = self.start_thread() {
                // The thread could not be created: leaving the appender in
                // async mode would buffer every record into a channel whose
                // receiver is dropped, and neither `flush` nor `close` could
                // recover them. Roll back to the mode that still works.
                self.lock().config.mode = previous;
                self.lock().tx = None;
                return Err(err);
            }
        }
        self.lock().notify();
        Ok(())
    }

    /// `XloggerAppender::SetConsoleLog`.
    pub(crate) fn set_console_log(&self, open: bool) {
        self.shared
            .flags
            .console_log_open
            .store(open, Ordering::Relaxed);
    }

    /// `XloggerAppender::SetMaxFileSize`.
    pub(crate) fn set_max_file_size(&self, bytes: u64) {
        self.lock().max_file_size = bytes;
    }

    /// `XloggerAppender::SetMaxAliveDuration`.
    pub(crate) fn set_max_alive_duration(&self, secs: u64) {
        if secs as i64 >= MIN_LOG_ALIVE_TIME {
            self.lock().max_alive_time = secs as i64;
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.shared.flags.log_close.load(Ordering::Acquire)
    }

    /// `XloggerAppender::GetCurrentLogPath`.
    pub(crate) fn current_log_path(&self) -> Option<PathBuf> {
        let logdir = self.lock().config.logdir.clone();
        if logdir.as_os_str().is_empty() {
            None
        } else {
            Some(logdir)
        }
    }

    /// `XloggerAppender::GetCurrentLogCachePath`.
    pub(crate) fn current_log_cache_path(&self) -> Option<PathBuf> {
        self.lock().config.cachedir.clone()
    }

    /// `Some(self)` when this appender writes to `_logdir` — used by the free
    /// discovery helpers in [`crate`].
    pub(crate) fn for_logdir(&self, logdir: &Path) -> Option<&Appender> {
        (self.lock().config.logdir == logdir).then_some(self)
    }

    /// `XloggerAppender::MakeLogfileName`.
    pub(crate) fn make_logfile_name(&self, timespan: i64, prefix: &str) -> Vec<PathBuf> {
        let guard = self.lock();
        let logdir = guard.config.logdir.clone();
        let cachedir = guard.config.cachedir.clone();
        let max_file_size = guard.max_file_size;
        if logdir.as_os_str().is_empty() {
            return Vec::new();
        }

        let tv = now_secs().saturating_sub(timespan.saturating_mul(SECONDS_PER_DAY));
        let log_path = make_log_file_name(
            tv,
            &logdir,
            &logdir,
            prefix,
            LOG_EXT,
            max_file_size,
            cachedir.as_deref(),
        );

        let Some(cachedir) = cachedir else {
            return vec![log_path];
        };

        let cache_path = make_log_file_name(
            tv,
            &cachedir,
            &logdir,
            prefix,
            LOG_EXT,
            max_file_size,
            Some(&cachedir),
        );

        let mut paths = Vec::new();
        if log_path.exists() {
            paths.push(log_path.clone());
        }
        if cache_path.exists() {
            paths.push(cache_path);
        }
        if paths.is_empty() {
            paths.push(log_path);
        }
        paths
    }

    /// `XloggerAppender::GetfilepathFromTimespan`.
    pub(crate) fn getfilepath_from_timespan(&self, timespan: i64, prefix: &str) -> Vec<PathBuf> {
        use crate::file_util::get_file_paths_from_timeval;

        let guard = self.lock();
        let logdir = guard.config.logdir.clone();
        let cachedir = guard.config.cachedir.clone();
        if logdir.as_os_str().is_empty() {
            return Vec::new();
        }

        let tv = now_secs().saturating_sub(timespan.saturating_mul(SECONDS_PER_DAY));
        let mut paths = get_file_paths_from_timeval(tv, &logdir, prefix, LOG_EXT);
        if let Some(cachedir) = cachedir {
            paths.extend(get_file_paths_from_timeval(tv, &cachedir, prefix, LOG_EXT));
        }
        paths
    }
}

/// `XloggerAppender::__AsyncLogThread`.
fn async_log_thread(shared: Arc<Shared>, rx: Receiver<Msg>) {
    loop {
        let mut disconnected = false;
        let msg = match rx.recv_timeout(ASYNC_WAIT) {
            Ok(msg) => Some(msg),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                disconnected = true;
                None
            }
        };

        let mut buffer = AutoBuffer::new();
        let closed = {
            let mut guard = shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.flush_buffer(&mut buffer);
            shared.flags.log_close.load(Ordering::Acquire)
        };

        if !buffer.is_empty() {
            let mut guard = shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.log2file(buffer.as_slice(), true);
        }

        if closed || disconnected || msg == Some(Msg::Close) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppenderMode, FileIoAction, XLogConfig};
    use mars_buffer::CompressMode;
    use mars_crypt::{magic, LogCrypt, HEADER_LEN, TAILER_LEN};

    /// Inflates a raw-DEFLATE body; returns `None` when the sibling crate's
    /// buffer did not compress the payload.
    fn inflate(data: &[u8]) -> Option<Vec<u8>> {
        use std::io::Read;
        // Raw DEFLATE (window bits = -MAX_WBITS). `read_to_end` is used instead
        // of `finish()` because a `Z_SYNC_FLUSH` stream is not terminated; any
        // bytes decoded before the stream runs out are kept.
        let mut out = Vec::new();
        let mut decoder = flate2::bufread::DeflateDecoder::new(std::io::Cursor::new(data));
        let _ = decoder.read_to_end(&mut out);
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    /// Splits a `.xlog` file into `[header_len + len + tailer_len]` records.
    fn records(bytes: &[u8]) -> Vec<&[u8]> {
        let mut out = Vec::new();
        let mut pos = 0usize;
        while pos + HEADER_LEN + TAILER_LEN <= bytes.len() {
            assert!(
                magic::magic_start_is_valid(bytes[pos]),
                "bad magic {:#04x} at {pos}",
                bytes[pos]
            );
            let len = LogCrypt::get_log_len(&bytes[pos..]) as usize;
            let end = pos + HEADER_LEN + len;
            assert!(end + TAILER_LEN <= bytes.len(), "truncated record at {pos}");
            assert_eq!(bytes[end], magic::END, "bad tailer at {end}");
            out.push(&bytes[pos + HEADER_LEN..end]);
            pos = end + TAILER_LEN;
        }
        assert_eq!(
            pos,
            bytes.len(),
            "trailing garbage: {} bytes left",
            bytes.len() - pos
        );
        out
    }

    /// The text of every record.
    ///
    /// Two readings are concatenated on purpose: the C++ sync path stores the
    /// payload in the clear (`CryptSyncLog` only frames it), while the async
    /// mmap path stores a raw-DEFLATE stream, so the same file can hold both.
    fn decoded_text(bytes: &[u8]) -> String {
        let mut text = String::new();
        for body in records(bytes) {
            text.push_str(&String::from_utf8_lossy(body));
            if let Some(plain) = inflate(body) {
                text.push('\n');
                text.push_str(&String::from_utf8_lossy(&plain));
            }
        }
        text
    }

    fn config(dir: &Path, mode: AppenderMode) -> XLogConfig {
        XLogConfig {
            mode,
            logdir: dir.to_path_buf(),
            nameprefix: "Mars".to_owned(),
            pub_key: String::new(),
            compress_mode: CompressMode::Zlib,
            compress_level: 6,
            cachedir: None,
            cache_days: 0,
        }
    }

    fn today_name(dir: &Path) -> PathBuf {
        let prefix = crate::file_util::make_log_file_name_prefix(now_secs(), "Mars");
        dir.join(format!("{prefix}.xlog"))
    }

    fn info(level: LogLevel) -> XLoggerInfo<'static> {
        XLoggerInfo {
            level,
            pid: std::process::id() as i64,
            tid: current_tid(),
            maintid: current_tid(),
            ..Default::default()
        }
    }

    #[test]
    fn sync_mode_writes_a_framed_record() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Sync), 0, 0).unwrap();

        let info = info(LogLevel::Info);
        appender.write(Some(&info), "hello from mars");

        appender.close();

        let path = today_name(tmp.path());
        assert!(path.exists(), "{path:?} missing");
        let bytes = fs::read(&path).unwrap();
        assert!(!bytes.is_empty());

        let text = decoded_text(&bytes);
        assert!(text.contains("hello from mars"), "{text}");
    }

    /// The C++ writes through a `FILE*`, so its records are not on disk the
    /// moment `write` returns either — they sit in the `stdio` buffer until it
    /// fills. [`Appender::flush_sync`] is what hands them over, in both modes.
    #[test]
    fn a_record_sits_in_the_buffer_until_it_is_flushed() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Sync), 0, 0).unwrap();

        appender.write(Some(&info(LogLevel::Info)), "buffered, not written yet");
        let path = today_name(tmp.path());
        assert_eq!(
            fs::metadata(&path).map_or(0, |meta| meta.len()),
            0,
            "the record reached the file before anything was flushed"
        );

        appender.flush_sync();
        let text = decoded_text(&fs::read(&path).unwrap());
        assert!(text.contains("buffered, not written yet"), "{text}");

        appender.close();
    }

    #[test]
    fn async_mode_flushes_through_the_writer_thread() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Async), 0, 0).unwrap();

        let info = info(LogLevel::Debug);
        appender.write(Some(&info), "async payload");
        // No flush yet: nothing has reached the log file.
        let path = today_name(tmp.path());
        let mut bytes = Vec::new();
        // `flush` wakes the writer thread and `flush_sync` drains whatever is
        // still in the buffer — and the C++ lets the two race for it. When
        // the writer thread got there first the bytes are on their way to the
        // file rather than in it, so the file is given a moment to catch up.
        for _ in 0..100 {
            appender.flush();
            appender.flush_sync();
            bytes = fs::read(&path).unwrap_or_default();
            if !bytes.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!bytes.is_empty(), "flush_sync must drain the cache");

        appender.close();

        let bytes = fs::read(&path).unwrap();
        let text = decoded_text(&bytes);
        assert!(text.contains("async payload"), "{text}");
    }

    #[test]
    fn async_close_flushes_pending_records() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Async), 0, 0).unwrap();

        let info = info(LogLevel::Warn);
        appender.write(Some(&info), "pending payload");
        appender.close();

        let bytes = fs::read(today_name(tmp.path())).unwrap();
        let text = decoded_text(&bytes);
        assert!(text.contains("pending payload"), "{text}");
    }

    #[test]
    fn many_writes_from_several_threads_are_framed() {
        let tmp = tempfile::tempdir().unwrap();
        let appender =
            Arc::new(Appender::open(config(tmp.path(), AppenderMode::Sync), 0, 0).unwrap());

        let handles: Vec<_> = (0..4)
            .map(|t| {
                let appender = Arc::clone(&appender);
                std::thread::spawn(move || {
                    let info = info(LogLevel::Info);
                    for i in 0..25 {
                        appender.write(Some(&info), &format!("t{t}-i{i}"));
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        let appender = Arc::try_unwrap(appender)
            .ok()
            .expect("all worker clones are dropped");
        appender.close();

        let bytes = fs::read(today_name(tmp.path())).unwrap();
        // Framing must be intact over the whole file.
        let text = decoded_text(&bytes);
        assert!(text.contains("t0-i0"), "{text}");
        assert!(text.contains("t3-i24"), "{text}");
    }

    #[test]
    fn cachedir_without_cache_days_writes_to_the_log_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let mut cfg = config(tmp.path(), AppenderMode::Sync);
        cfg.cachedir = Some(cache.clone());

        let appender = Appender::open(cfg, 0, 0).unwrap();
        appender.write(Some(&info(LogLevel::Info)), "with a cache dir");
        appender.close();

        assert!(cache.is_dir(), "the cache dir must be created");
        let bytes = fs::read(today_name(tmp.path())).unwrap();
        assert!(decoded_text(&bytes).contains("with a cache dir"));

        // `cache_days == 0` -> `__CacheLogs()` is false and no cache file is
        // produced, so nothing is staged in the cache dir.
        let staged: Vec<_> = fs::read_dir(&cache)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".xlog"))
            .collect();
        assert!(staged.is_empty(), "{staged:?}");
    }

    #[test]
    fn max_file_size_rotation_creates_split_files() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Sync), 256, 0).unwrap();

        for i in 0..200 {
            appender.write(
                Some(&info(LogLevel::Info)),
                &format!("rotation record {i} padding padding padding"),
            );
        }
        appender.close();

        let prefix = crate::file_util::make_log_file_name_prefix(now_secs(), "Mars");
        let names: Vec<String> = fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(&prefix) && n.ends_with(".xlog"))
            .collect();
        assert!(
            names.iter().any(|n| n == &format!("{prefix}_1.xlog")),
            "{names:?}"
        );
        assert!(names.len() > 1, "{names:?}");
    }

    /// Leaves a cache file with one record in it behind, the way a process that
    /// died does: opens an async appender, writes, then drops it without
    /// `close()`. Answers the `<prefix>.mmap3` path.
    fn leave_cache_behind(dir: &Path) -> PathBuf {
        let appender = Appender::open(config(dir, AppenderMode::Async), 0, 0).unwrap();
        appender.write(Some(&info(LogLevel::Error)), "cached in mmap");
        // Close the appender's view of the world and take the writer thread
        // back, rather than dropping the appender: a real `close()` would drain
        // the cache into the log file, and the record has to stay in the cache.
        appender
            .shared
            .flags
            .log_close
            .store(true, Ordering::Release);
        let close_tx = appender.lock().close_sender();
        if let Some(tx) = close_tx {
            let _ = tx.send(Msg::Close);
        }
        if let Some(handle) = appender.thread_lock().take() {
            let _ = handle.join();
        }
        appender.lock().tx = None;
        dir.join("Mars.mmap3")
    }

    /// Writes `<prefix>.mmap3` with one record in it, the way a process that
    /// died does — straight into the file, without an appender around it.
    fn records_in_cache(dir: &Path) -> PathBuf {
        let mut region = vec![0u8; BUFFER_BLOCK_LENGTH];
        let mut buff = LogBuffer::new(true, Some(""), CompressMode::Zlib, 6);
        buff.attach(&mut region);
        assert!(buff.write(&mut region, b"cached in mmap"));
        let path = dir.join("Mars.mmap3");
        fs::write(&path, &region).unwrap();
        path
    }

    #[test]
    fn mmap_cache_file_is_created_and_drained_on_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let mmap_path = leave_cache_behind(tmp.path());

        assert!(mmap_path.exists(), "the cache file must survive");
        assert!(fs::metadata(&mmap_path).unwrap().len() >= BUFFER_BLOCK_LENGTH as u64);

        // Re-opening must drain the leftover into the log file.
        let appender = Appender::open(config(tmp.path(), AppenderMode::Sync), 0, 0).unwrap();
        appender.close();

        let bytes = fs::read(today_name(tmp.path())).unwrap();
        let text = decoded_text(&bytes);
        assert!(text.contains("cached in mmap"), "{text}");
    }

    #[test]
    fn one_shot_recovery_keeps_the_cache_when_the_write_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let mmap_path = records_in_cache(tmp.path());

        // Today's log file is a directory, so every write into it fails — what
        // a full or read-only file system looks like from here.
        let log_file = today_name(tmp.path());
        let _ = fs::remove_file(&log_file);
        fs::create_dir(&log_file).unwrap();

        let appender = Appender::oneshot(&config(tmp.path(), AppenderMode::Sync), 0, 0).unwrap();
        let action = appender.treat_mapping_as_file_and_flush();
        appender.close();

        assert_eq!(action, FileIoAction::WriteFailed);
        assert!(mmap_path.exists(), "the cache is the only copy left");
    }

    #[test]
    fn current_paths_reflect_the_config() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let mut cfg = config(tmp.path(), AppenderMode::Sync);
        cfg.cachedir = Some(cache.clone());
        let appender = Appender::open(cfg, 0, 0).unwrap();

        assert_eq!(appender.current_log_path().as_deref(), Some(tmp.path()));
        assert_eq!(
            appender.current_log_cache_path().as_deref(),
            Some(cache.as_path())
        );
    }

    /// What a record costs the allocator.
    ///
    /// The C++ formats into stack arrays (`char temp[16*1024]`,
    /// `char temp_time[64]`, the `snprintf` buffer) and reuses its
    /// `AutoBuffer`s, so one record costs it no allocation at all. The port
    /// used to spend five or six: two `String`s in the formatter, an
    /// `AutoBuffer` per `WriteSync`, a `Vec` per `LogBuffer::Write`, and the
    /// `PathBuf`/`String` clones `Log2File` made before it knew it needed
    /// them. This pins the port at the C++'s zero: once the first record has
    /// grown every buffer, writing must not touch the allocator — a logger is
    /// a thing that has to keep working when the allocator is what is
    /// struggling.
    #[test]
    fn a_record_costs_no_allocation_once_the_appender_is_warm() {
        let _guard = crate::test_lock::serial();
        for mode in [AppenderMode::Sync, AppenderMode::Async] {
            let tmp = tempfile::tempdir().unwrap();
            let appender = Appender::open(config(tmp.path(), mode), 0, 0).unwrap();
            let record_info = info(LogLevel::Info);

            // Warm-up: opens the file and grows the buffers a real logger
            // grows in its first record.
            appender.write(Some(&record_info), "warm up");

            crate::test_alloc::watch();
            for _ in 0..16 {
                appender.write(Some(&record_info), "a record, long enough to be a record");
            }
            let count = crate::test_alloc::stop();

            // And the console path: `ConsoleLog.cc` also formats into a stack
            // array (`char strFuncName[128]`, then `printf`), so it must not
            // allocate either — including the trim of `__FUNCTION__`, which
            // the port used to do into a fresh `String`.
            let mut console_info = info(LogLevel::Info);
            console_info.func_name = Some("void Foo::bar(int)".into());
            appender.set_console_log(true);
            appender.write(Some(&console_info), "console warm up");
            crate::test_alloc::watch();
            for _ in 0..3 {
                appender.write(Some(&console_info), "and to the console");
            }
            appender.set_console_log(false);
            let console_count = crate::test_alloc::stop();

            appender.close();
            assert_eq!(count, 0, "{mode:?}: 16 records allocated {count} times");
            assert_eq!(
                console_count, 0,
                "{mode:?}: 3 console records allocated {console_count} times"
            );
        }
    }

    #[test]
    fn make_logfile_name_and_timespan_lookup() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Sync), 0, 0).unwrap();

        let paths = appender.make_logfile_name(0, "Mars");
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], today_name(tmp.path()));

        // Nothing exists yet for `timespan = 1` (yesterday)...
        assert!(appender.getfilepath_from_timespan(1, "Mars").is_empty());
        // ...but something does for today once the file is there.
        appender.write(Some(&info(LogLevel::Info)), "now it exists");
        let today = appender.getfilepath_from_timespan(0, "Mars");
        assert_eq!(today.len(), 1, "{today:?}");
    }
}
