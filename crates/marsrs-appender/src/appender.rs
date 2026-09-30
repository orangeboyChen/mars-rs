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
//! * A batch the file refuses is **kept**, not dropped (see
//!   [`AppenderInner::pending`]), so `__Log2File` can still put it in the cache
//!   directory and the next flush can try again. The C++ cannot: `fwrite` has
//!   already consumed its `stdio` buffer by the time it fails, so it can only
//!   retry with the one record it was handed — one out of every ~4 KiB batch.
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
//! * The C++ mmaps `<prefix>.mmap3` whatever else is doing, so two processes
//!   — or two copies of the C++ linked into one — write through the same 150
//!   KiB region with their own idea of its length, their own compressor and
//!   their own flush: records are lost, and each flush either writes what the
//!   other buffered or clears it before the other gets there. The port gives
//!   every writer a cache file of its own (`CacheSlot`, claimed by `O_EXCL`
//!   and held for the appender's lifetime) and takes the log's own
//!   `<logdir>/<prefix>.lock` around the steps that move more than one file,
//!   so a log two writers share is still complete. Where the filesystem's
//!   advisory locking excludes nobody the C++ behaviour — one shared cache
//!   file, unprotected — is all there is, and that is what the port falls back
//!   to rather than trusting a lock that locks nobody.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use marsrs_buffer::LogBuffer;
use marsrs_core::{local_time, AutoBuffer, PtrBuffer};

use crate::config::{AppenderMode, LogLevel, XLogConfig, XLoggerInfo};
use crate::console::{console_log, console_log_stderr};
use crate::file_util::{
    append_file, create_private_dir, del_timeout_file, format_local_timestamp, make_log_file_name,
    monotonic_millis, move_old_files, now_secs, private_file, LOG_EXT, MMAP_EXT, SECONDS_PER_DAY,
};
use crate::formater::log_formater;
use crate::sys;

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
/// What [`AppenderInner::pending`] is allocated with: [`LOG_FLUSH_THRESHOLD`]
/// plus room for the record that crosses it, so that record copies in without
/// reallocating (the buffer is flushed *after* the append, like the C++'s
/// `fwrite`, which copies a record of any size into the same buffer).
///
/// It is only where the buffer starts. One drain of a full cache region is far
/// bigger than this, and a batch is handed to the OS whole, so the buffer grows
/// to whatever a drain needs — once, and not again.
const PENDING_CAPACITY: usize = 2 * LOG_FLUSH_THRESHOLD;
/// The recursion guard of `XloggerAppender::Write` (`recursion_count > 10`).
const MAX_RECURSION: u32 = 10;
/// A local day no log file can be opened on: [`AppenderInner::open_file_day`]
/// holds it whenever no file is open.
const NO_DAY: (i32, u32, u32) = (0, 0, 0);

/// Decrements the per-thread recursion counter when it goes out of scope.
struct RecursionGuard;

impl Drop for RecursionGuard {
    fn drop(&mut self) {
        // `catch_unwind` is not needed here: nothing in the decrement panics,
        // and `try_with` is what a thread-local whose destructor may already
        // have run wants anyway — a panic on the way *out* of a logger is the
        // one thing this guard exists to prevent.
        let _ = RECURSION_COUNT.try_with(|cell| cell.set(cell.get().saturating_sub(1)));
    }
}

// The per-thread recursion counter undone by `RecursionGuard`.
thread_local! {
    static RECURSION_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

// `thread_local std::string recursion_str` — the dump of a write that was
// logged from inside the logger, which the next write that is not recursive
// hands to `WriteTips2File`.
//
// The C++ keeps it because the console is not a copy: a process started from
// an icon has no terminal behind it, and the log file is the only trace of a
// logging loop an app that uploads its logs ever has.
thread_local! {
    static RECURSION_DUMP: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
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

/// Formats one record into `record` and answers how many bytes of it are the
/// record.
///
/// The buffer is grown to [`TEMP_LOG_SIZE`] because `log_formater` truncates
/// against `max_length()`, so a shorter one would cut a long body sooner than
/// the C++ cuts it.
fn format_record_into(info: Option<&XLoggerInfo>, log: &str, record: &mut Vec<u8>) -> usize {
    record.resize(TEMP_LOG_SIZE, 0);
    let mut out = PtrBuffer::new(&mut record[..]);
    log_formater(info, Some(log), &mut out);
    out.len()
}

/// Formats one record into the calling thread's buffer and answers how many
/// bytes of it are the record.
///
/// Holds no lock, which is why [`Appender::write`] calls it first: the
/// formatter is the most expensive part of a record and the C++ runs it
/// outside `mutex_buffer_async_` and `mutex_log_file_` too.
///
/// Callers that are themselves inside a write — the recursive dump — must not
/// come through here: [`format_record_into`] into a buffer of their own is
/// what they want, and why it exists.
fn format_record(info: Option<&XLoggerInfo>, log: &str) -> usize {
    RECORD.with(|cell| format_record_into(info, log, &mut cell.borrow_mut()))
}

/// Hands the `len` bytes [`format_record`] left in the buffer to `f`.
///
/// Separate from [`format_record`] so that no borrow of the buffer is alive
/// while the appender's lock is held: nothing under that lock formats, but
/// keeping the two apart is what makes that true by construction.
fn with_record<T>(len: usize, f: impl FnOnce(&[u8]) -> T) -> T {
    RECORD.with(|cell| f(&cell.borrow()[..len]))
}

/// The `recursion_str` of a write that was logged from inside the logger: the
/// record that names the episode, formatted because the file gets the string
/// raw — `WriteTips2File` hands its argument to the buffer as it stands, so
/// the line a decoder of the log shows is the one the formatter produced.
///
/// Formatted into a buffer of its own, and never into [`RECORD`]. A recursive
/// write is one that runs while an outer write is formatting or consuming that
/// buffer, so borrowing it again is a panic, and formatting into it would
/// leave the outer write to file the dump's bytes under the outer record's
/// length. The dump is built once per episode, so the buffer costs one
/// allocation on the one write that recurses.
fn recursion_dump(info: Option<&XLoggerInfo>, count: u32) -> String {
    let mut recursive = info.cloned().unwrap_or_default();
    recursive.level = LogLevel::Fatal;
    let body = format!("ERROR!!! xlogger_appender Recursive calls!!!, count:{count}");

    let mut record = Vec::new();
    let len = format_record_into(Some(&recursive), &body, &mut record);
    let dump = String::from_utf8_lossy(&record[..len]).into_owned();
    // The C++ hands `ConsoleLog` that same string, which is why its console
    // shows the prefix twice; the port consoles the body.
    console_log(Some(&recursive), &body);
    dump
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
/// the cache file is the claimed slot's, only ever written through this
/// mapping, and nobody else truncates it: see `claim_cache_slot`.
///
/// The crate denies `unsafe_code` outright, so this is the one place that says
/// otherwise: the mapping cannot be made without `unsafe`, and the invariant
/// `memmap2` needs is argued below rather than assumed.
#[allow(unsafe_code)]
fn map_region(file: &File) -> std::io::Result<memmap2::MmapMut> {
    // SAFETY: the invariant memmap2 needs is that nobody truncates or resizes
    // the file while the mapping is alive. The slot the file belongs to was
    // claimed for this appender alone (`claim_cache_slot`), and within it the
    // region is only ever written through this mapping; the mapping keeps the
    // inode alive on its own, so it is not the `File` that pins it. What is
    // *not* guaranteed is protection against an outside process — or the C++
    // xlog still linked into the same app during migration — truncating the
    // file: that would turn every later touch of the mapping into SIGBUS.
    // Opening the same cache file from two implementations at once is
    // unsupported.
    //
    // The other half of what a mapping needs is not `memmap2`'s to ask for but
    // the kernel's: every page of it must have blocks behind it. A file that
    // is `BUFFER_BLOCK_LENGTH` long because `set_len` made it so — and nothing
    // ever wrote the bytes — is a hole, and the first store into a hole on a
    // filesystem that is out of space is SIGBUS too, which is what
    // `open_region` pre-allocates for. This is only reached once that
    // pre-allocation has succeeded; a file it failed for never gets here.
    unsafe {
        memmap2::MmapOptions::new()
            .len(BUFFER_BLOCK_LENGTH)
            .map_mut(file)
    }
}

/// Zeroes the cache file of a slot this appender owns.
///
/// Only needed when there is no mapping: with one, `LogBuffer::flush` and
/// `close()` clear the bytes in place, and the file follows the mapping.
fn clear_cache_file(path: &Path) {
    // Truncate to zero and *keep* it at zero: `set_len` back to the block size
    // would build the same sparse hole that the pre-allocation below exists to
    // prevent, and a zero-length file re-arms that pre-allocation on the next
    // open.
    if let Ok(mut file) = OpenOptions::new().write(true).truncate(true).open(path) {
        let _ = file.flush();
    }
}

/// Whether the whole region reads as zeros: a hole `set_len` made, with no
/// record stored through the mapping yet.
///
/// `st_blocks` cannot answer this, though it looks like it should. It counts
/// what the disk has given the file, and a file written the only way this one
/// is written to — through a mapping — is not promised blocks before
/// writeback: a cache file holding a crashed run's records measures as sparse
/// on a filesystem that allocates late (ext4's delayed allocation) or
/// compresses (f2fs, btrfs), and accounts nothing at all on one that does not
/// implement it (FUSE, a card's sdcardfs). Taking that at its word and
/// pre-allocating over such a file writes a block of zeros over records no
/// log holds a copy of, and the "begin of mmap" recovery never fires. What is
/// asked instead is the one thing a hole cannot be: a byte that is not zero.
///
/// A read that fails is answered as `false`: a file this process cannot read
/// is one the mapping is not going to take either, and zeroing bytes nobody
/// could inspect first is the wrong way round to be careful.
fn is_unwritten(file: &mut File) -> bool {
    let mut buffer = vec![0u8; BUFFER_BLOCK_LENGTH];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut buffer))
        .map(|()| buffer.iter().all(|byte| *byte == 0))
        .unwrap_or(false)
}

/// Opens (creating if needed) and maps the claimed cache file; falls back to a
/// heap region on any error. Returns `(region, use_mmap)`.
///
/// The file is the caller's own slot, so nothing else resizes it while this
/// runs — which is what lets the mapping be pre-allocated here at all.
fn open_region(file: &mut File, path: &Path) -> (Region, bool) {
    // `ftruncate` records a size without reserving blocks; the first store
    // into the mapping is what allocates. On a filesystem that does not
    // reserve on truncate (ext4, f2fs, FAT — the Android targets) a full disk
    // turns that store into SIGBUS, killing the host. `mars/comm/mmap_util.cc`
    // pre-allocates by writing zeros and falls back to the heap path if that
    // write fails; do the same.
    // What the file measured on entry: both whether it has to be
    // pre-allocated, and the length a failed pre-allocation puts it back to.
    //
    // A file shorter than a block has never been written to, and length alone
    // says so. A file that measures a block is asked what its bytes are: a
    // build before this one — and the C++ the port sits beside during a
    // migration — made this file `BUFFER_BLOCK_LENGTH` long with `set_len` and
    // wrote nothing into it, and that is a hole of exactly the length a later
    // open would take for "already allocated". A hole reads as zeros, so a
    // file that is all zeros is pre-allocated over whatever it measures, and
    // a file with a single byte in it that is not is left as it is: it holds
    // records, and the bytes of a record are the one thing this may not write
    // over. See [`is_unwritten`] for why the block count is not what is asked.
    let entry_len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let needs_preallocation = entry_len < BUFFER_BLOCK_LENGTH as u64 || is_unwritten(file);
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
            // A failed pre-allocation is put back the way the file was found:
            // what it leaves behind is a length the file does not hold, and
            // the length of a cache file is what a reader of the cache
            // directory — the heap fallback below, a decoder, or the C++
            // still linked into the same app — takes for how much of it is
            // real. A `set_len` that fails too is ignored: nothing is written
            // through the file either way, the heap region below does not read
            // it, and the length is the only thing at stake.
            let _ = file.set_len(entry_len);
            return (Region::heap(), false);
        }
    }

    match map_region(file) {
        Ok(mmap) => (Region::Mmap(mmap), true),
        // mmap is unavailable (sandbox, low memory, some OEM kernels). Fall
        // back to a heap region, but take the on-disk contents with us:
        // otherwise a cache file left by a crashed process is never drained,
        // and a later run where mmap *does* work appends those records to a
        // different day's log, behind a "begin of mmap" banner.
        Err(_) => (Region::heap_with_cache(path), false),
    }
}

/// The directory `<prefix>.mmap3` (and `<prefix>.lock`) live in: the configured
/// cache directory, or the log directory when there is none.
pub(crate) fn cache_dir(config: &XLogConfig) -> &Path {
    config
        .cachedir
        .as_deref()
        .unwrap_or(config.logdir.as_path())
}

/// `<dir>/<prefix>.mmap3` — what `appender.cc` calls `mmap_file_path`, i.e.
/// the first of the slots `claim_cache_slot` hands out.
pub(crate) fn mmap_file_path(config: &XLogConfig) -> PathBuf {
    cache_slot_path(cache_dir(config), &config.nameprefix, 0)
}

/// `<dir>/<prefix>.lock`.
///
/// Two different locks of that shape are taken, each in the directory of the
/// thing it is about: the log's own (see [`output_lock_path`]) and, only to
/// find out whether locking excludes anybody there at all, the cache
/// directory's (see [`claim_cache_slot`]).
///
/// Neither is inside anything the sweep or the log-file discovery look at:
/// `del_timeout_file` only removes `<prefix>*.xlog` files and `YYYYMMDD`
/// directories, and [`file_util::get_file_names_by_prefix`] only matches
/// `.xlog`.
pub(crate) fn dir_lock_path(dir: &Path, prefix: &str) -> PathBuf {
    dir.join(format!("{prefix}.lock"))
}

/// `<logdir>/<prefix>.lock` — the lock every writer of one log file takes: see
/// [`AppenderInner::with_dir_lock`].
///
/// The log file is what writers that name this `logdir` and prefix share,
/// wherever each of them keeps its cache — a cache directory of its own,
/// another one's, or none at all — so a lock named after the cache directory
/// would be a different file for every one of them and would exclude nobody:
/// two writers with two cache directories would interleave their batches and
/// their cache-file moves inside one `.xlog`.
pub(crate) fn output_lock_path(config: &XLogConfig) -> PathBuf {
    dir_lock_path(&config.logdir, &config.nameprefix)
}

/// `<dir>/<prefix>.mmap3` for slot `0`, `<dir>/<prefix>_<n>.mmap3` after that.
pub(crate) fn cache_slot_path(dir: &Path, prefix: &str, slot: usize) -> PathBuf {
    if slot == 0 {
        dir.join(format!("{prefix}.{MMAP_EXT}"))
    } else {
        dir.join(format!("{prefix}_{slot}.{MMAP_EXT}"))
    }
}

/// How many cache files one prefix may have in one directory at once.
///
/// Eight live writers of one prefix is already far past anything real — an
/// Android app with a `:push` process and a React Native module that linked
/// its own copy of the crate uses two — and a bounded number is what keeps a
/// directory that cannot be locked from filling with them.
pub(crate) const MAX_CACHE_SLOTS: usize = 8;

/// The cache file this appender owns: `<prefix>[_<n>].mmap3`.
///
/// The C++ mmaps `<prefix>.mmap3` whatever else is doing, so a second process
/// (or a second copy of the C++ in one process) writes through the *same* 150
/// KiB region with its own idea of the length, its own compressor and its own
/// flush: records are lost, and each flush either writes what the other
/// buffered or clears it before the other gets there. The cache is not a file
/// to be shared, it is the buffer itself, so the port gives every writer its
/// own and claims it:
///
/// * `O_EXCL` decides who owns a slot — atomic across processes and across
///   copies of this crate in one process, which is the case the C++ cannot
///   tell apart either;
/// * the lock held on it for the appender's lifetime is what a later
///   [`crate::appender_oneshot_flush`] reads to tell a slot a *dead* process
///   left behind from one a live writer is still using. Nothing else can: a
///   process that was killed leaves exactly the file a running one has.
struct CacheSlot {
    path: PathBuf,
    /// Held open for as long as the appender lives. Dropping it — including
    /// when the process dies — is what releases the lock.
    file: File,
}

/// Opens the log's lock — [`output_lock_path`] — or `None` when it cannot be
/// opened.
///
/// One handle for the whole appender, locked and unlocked around each section
/// by [`AppenderInner::with_dir_lock`] rather than reopened: on macOS an `open`
/// and a `close` measured ~16 µs against ~0.5 µs for the `flock` pair, which is
/// more than a whole record costs. The lock is still released when the file is
/// dropped, so a process that dies releases it too.
///
/// Opened whatever [`sys::lock_excludes`] answered: on a filesystem whose
/// advisory locking excludes nobody the sections below run unprotected, which
/// is the fallback either way, but a directory that *does* exclude while the
/// cache directory does not — or the other way round — is still protected.
fn open_dir_lock(path: Option<&Path>) -> Option<File> {
    let path = path?;
    private_file(
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false),
    )
    .open(path)
    .ok()
}

/// Claims the cache file no other writer holds, or `None` when every slot is
/// taken.
///
/// `locking` is what `sys::lock_excludes` answered for this directory.
fn claim_cache_slot(dir: &Path, prefix: &str, locking: bool) -> Option<CacheSlot> {
    if !locking {
        // Without a lock a live slot cannot be told from a dead one, so the
        // only safe thing left is the C++'s single fixed name, shared exactly
        // as the C++ shares it.
        let path = cache_slot_path(dir, prefix, 0);
        return private_file(
            File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false),
        )
        .open(&path)
        .ok()
        .map(|file| CacheSlot { path, file });
    }

    for slot in 0..MAX_CACHE_SLOTS {
        let path = cache_slot_path(dir, prefix, slot);
        let file = match private_file(File::options().read(true).write(true).create_new(true))
            .open(&path)
        {
            // Whoever creates a slot owns it.
            Ok(file) => Some(file),
            // Already there: either a writer that is gone left it — and an
            // unheld lock is what says so — or a live one owns it, in which
            // case the lock is denied and the next slot is tried.
            Err(_) => File::options().read(true).write(true).open(&path).ok(),
        };
        let Some(file) = file else { continue };
        if sys::try_lock_exclusive(&file) {
            return Some(CacheSlot { path, file });
        }
    }
    None
}

/// Opens `path` and takes its lock, which is what proves the slot belongs to a
/// writer that is gone.
///
/// `None` when a live writer still holds it — and, just as much, when this
/// filesystem's locking excludes nobody, where the answer would be a guess.
pub(crate) fn claim_dead_cache_slot(path: &Path) -> Option<File> {
    let file = File::options().read(true).write(true).open(path).ok()?;
    sys::try_lock_exclusive(&file).then_some(file)
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
    /// Allocated with room for one record past the threshold and grown to
    /// whatever one drain needs: see [`PENDING_CAPACITY`].
    ///
    /// A batch the file refuses is **kept** here, not dropped — the C++ cannot
    /// keep it (`fwrite` has already consumed its buffer, and `ftruncate`
    /// throws away whatever the kernel took), but the port can, so
    /// [`Self::log2file`] still has the whole batch to put in the cache
    /// directory and the next flush gets to try again.
    pending: Vec<u8>,
    /// How much of `log_file` the OS holds — `Self::pending` **not**
    /// included.
    ///
    /// `XloggerAppender::__WriteFile` asks the kernel on every record
    /// (`ftell(_file)`) so that a failed write can roll the file back to where
    /// it started. The port remembers the length instead: this appender is the
    /// only writer of the file it opened — the same invariant `map_region`
    /// argues for the cache file — so the length is what the file held when it
    /// was opened plus everything handed to the OS since. That is one `fstat`
    /// and one `lseek` fewer per record, on the one path where a logger cannot
    /// afford them: the caller holds the appender's lock for the whole record.
    ///
    /// Kept apart from `Self::pending` so that the two can disagree: a batch
    /// that failed belongs to no file yet, and the next file that opens — the
    /// cache directory's, say — takes it from zero.
    flushed_len: u64,
    /// Whether `Self::pending` has already been refused once.
    ///
    /// A batch is held back so that [`Self::log2file`] can put it somewhere
    /// else and the next flush can try again — but a file that keeps refusing
    /// writes must not let the logger grow without bound, so a batch that is
    /// refused a second time is given up on. The C++'s `FILE*` cannot hold a
    /// refused batch at all.
    pending_refused: bool,
    /// `openfiletime_`
    open_file_time: i64,
    /// The local day [`Self::open_file_time`] falls on — the `filetm.tm_year ==
    /// tcur.tm_year && ...` of `XloggerAppender::__OpenLogFile`.
    ///
    /// Stamped once, when the file is opened, so that the per-record
    /// roll-over check is one [`local_time`] of *today* — which the record's own
    /// header has already asked for this second, and which therefore costs no
    /// `localtime` at all. Comparing two `tv_sec`s the way the C++ does is two
    /// conversions per record: one for the file's, which never changes, and one
    /// for now's.
    ///
    /// `(0, 0, 0)` is a day no file can be opened on, so it is also what "no
    /// file is open" looks like.
    open_file_day: (i32, u32, u32),
    /// `last_time_`
    last_time: i64,
    /// The second the records about to be written carry — the newest record's
    /// own `timeval`, and not the second the write happens in.
    ///
    /// It is what dates the file they land in: see [`Self::write_time`]. `None`
    /// when nothing has been logged yet through this appender, or when the
    /// record carried no time of its own.
    write_sec: Option<i64>,
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
    /// Whether the region is the mmap'd cache file (`true`) or a heap buffer.
    use_mmap: bool,
    /// The cache file this appender owns: see `CacheSlot`.
    ///
    /// `None` for `Appender::oneshot`, which works on a file left behind by
    /// another process and must never clear it, and for an appender that could
    /// not claim a slot of its own at all.
    cache: Option<CacheSlot>,
    /// `<logdir>/<prefix>.lock`, taken around the operations that move more
    /// than one file: see `Self::with_dir_lock`. `None` when the file cannot be
    /// opened, in which case those operations run unprotected — the way the
    /// C++ runs them.
    ///
    /// Held open rather than reopened per section: see [`open_dir_lock`].
    dir_lock: Option<File>,
    /// Whether `Self::dir_lock` is locked right now — i.e. whether this
    /// appender is inside a [`Self::with_dir_lock`] section.
    ///
    /// `flock` is per open file description, so a section nested in another
    /// would take the lock it already holds (a no-op) and then release it
    /// while the section it is nested in is still running. Remembering that it
    /// is held is what lets one section call another — a `flush_pending` inside
    /// a locked drain, say — without losing the lock half way through.
    dir_lock_held: bool,
}

impl AppenderInner {
    fn is_sync(&self) -> bool {
        self.config.mode == AppenderMode::Sync
    }

    /// The cache file this appender owns, if it claimed one.
    fn cache_path(&self) -> Option<PathBuf> {
        self.cache.as_ref().map(|slot| slot.path.clone())
    }

    /// The second a write should date the file it lands in.
    ///
    /// The records', and not the clock's: in async mode a block waits in the
    /// cache until it is big enough or the writer thread next wakes, so the two
    /// can be minutes — or hours — apart, and a block produced on one day and
    /// drained on the next used to be written into the next day's file, where
    /// the file's date and the timestamps of the records inside it disagree.
    ///
    /// The C++ has nowhere to get the record's time from at this point — it
    /// closed the file after the previous write, so `openfiletime_` is 0 and
    /// `__MakeLogFileName` has to ask `gettimeofday` — but the port is handed
    /// the record's own `timeval` and can answer it.
    fn write_time(&self) -> i64 {
        self.write_sec.unwrap_or_else(now_secs)
    }

    /// Runs `f` with the log's lock held.
    ///
    /// Every operation below moves more than one file of one prefix: a batch
    /// must reach the log file whole, a cache file must be read, appended to
    /// the log and then removed without another writer doing the same in
    /// between, and the expiry sweep must not race a write. With two processes
    /// (or two copies of this crate in one) writing the same prefix, each of
    /// those is a race that loses or duplicates records.
    ///
    /// Nesting is allowed, and does what the caller means: the inner section
    /// runs under the lock the outer one already holds, and only the outer one
    /// releases it.
    ///
    /// Not holding it is not an error worth failing a write over — a log is
    /// best-effort, and the sections below still run, unprotected, the way the
    /// C++ runs them.
    fn with_dir_lock<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        if self.dir_lock_held {
            return f(self);
        }
        let held = self.dir_lock.as_ref().is_some_and(sys::lock_exclusive);
        self.dir_lock_held = held;
        let out = f(self);
        if held {
            self.dir_lock_held = false;
            if let Some(file) = self.dir_lock.as_ref() {
                sys::unlock(file);
            }
        }
        out
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
    ///
    /// Copies the block out of the region but leaves the region alone: see
    /// [`LogBuffer::flush`].
    fn flush_buffer(&mut self, out: &mut AutoBuffer) -> usize {
        let region = self.region.as_mut_slice();
        self.buff.flush(region, out)
    }

    /// `log_buff_->__Clear()` — only for a block that has reached a file.
    fn buffer_drained(&mut self) {
        let region = self.region.as_mut_slice();
        self.buff.drained(region);
        // The block is gone, so the second that dated it goes with it: the next
        // file is named after the next record that carries one. Without this, a
        // write that carries no time of its own — the tips, and the banner
        // `close` writes — is filed under the day of a block that is already in
        // a file, which is the very disagreement [`Self::write_time`] exists to
        // prevent.
        self.write_sec = None;
    }

    /// Drains the cache region into the log and gives it up — in that order.
    ///
    /// `true` when the block reached a file (or when the region was empty, in
    /// which case it is cleared anyway). `false` when it did not, and then the
    /// region is left exactly as it was so that the next drain writes the same
    /// block again.
    ///
    /// That order is what upstream does differently: `XloggerAppender` flushes
    /// the mmap buffer into an `AutoBuffer`, clears the buffer, and only then
    /// writes the `AutoBuffer` to the log file, so a process killed in between
    /// loses every record of the block. The region *is* the durable copy
    /// — it is the mmap'd cache file — so the port writes first and gives it up
    /// after. A crash between the two now costs a duplicate block, which the
    /// next start recovers and writes again, and not a lost one.
    fn drain_buffer(&mut self, move_file: bool) -> bool {
        let mut buffer = std::mem::take(&mut self.scratch);
        buffer.reset();
        self.flush_buffer(&mut buffer);
        if buffer.is_empty() {
            self.scratch = buffer;
            return true;
        }

        // `log2file` answers whether the records reached a file, but in sync
        // mode it leaves what it buffered behind for the next record, so the
        // flush that hands it to the OS is part of "reached".
        let reached = self.log2file(buffer.as_slice(), move_file) && self.flush_pending();
        if reached {
            self.buffer_drained();
        }
        self.scratch = buffer;
        reached
    }

    /// [`Self::drain_buffer`] for a block that was already copied out — the one
    /// [`Appender::open`] recovers from the cache file before the writer thread
    /// exists.
    fn drain_leftover(&mut self, leftover: &[u8]) -> bool {
        if leftover.is_empty() {
            return true;
        }
        let reached = self.log2file(leftover, false) && self.flush_pending();
        if reached {
            self.buffer_drained();
        }
        reached
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
        // wake-up. What it must not do is fall through *silently*: a `false`
        // from `write` leaves no byte of the record and no marker anywhere, so
        // the block goes out now and the record is written into the region the
        // drain emptied. (`LogBuffer::write` answers `false` for a region that
        // cannot hold it — a compressor that emitted nothing included — and a
        // record that cannot be written even then is one the C++ would have
        // clamped away too.)
        let written = with_record(len, |data| {
            let region = self.region.as_mut_slice();
            self.buff.write(region, data)
        });
        if !written && self.drain_buffer(false) {
            let _written = with_record(len, |data| {
                let region = self.region.as_mut_slice();
                self.buff.write(region, data)
            });
        }

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
        console_log_stderr(Some(&info), tips);
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

        // One clock reading for the whole of `__Log2File`: the cache file's
        // name, the log file's name and the day the roll-over check asks about
        // all come from this one second. Reading it again per path is how a
        // record could be dated by one second and filed under another.
        let tv = self.write_time();

        // The paths are built from `self` where they are used instead of being
        // cloned up front: __Log2File runs once per record, and three
        // `PathBuf`/`String` clones per record were three allocations the C++
        // never makes — it passes `const char*` around.
        if self.config.cachedir.is_none() {
            if self.open_log_file(OpenDir::Log, tv) {
                let mut written = self.write_file_record(data);
                if !self.is_sync() {
                    written |= self.closed_after_a_failed_write();
                }
                return written;
            }
            return false;
        }
        let cache_path = self.cache_file_path(tv);
        let cache_logs = self.cache_logs(tv);

        if (cache_logs || cache_path.exists()) && self.open_log_file(OpenDir::Cache, tv) {
            let mut written = self.write_file_record(data);
            if !self.is_sync() {
                written |= self.closed_after_a_failed_write();
            }

            if cache_logs || !move_file {
                return written;
            }

            let log_path = self.log_file_path(tv);
            // The cache file is the only copy of these records, so reading it,
            // appending it to the log and removing it is one step: a writer
            // that found it still there would append the same records a second
            // time. The handle is closed first because Windows cannot unlink a
            // file somebody still holds open.
            let moved = self.with_dir_lock(|me| {
                // `__AppendFile` reads the file this appender has been writing,
                // so the buffer has to reach it first — `flush_pending`, which
                // runs under the lock this section is already holding.
                if !me.flush_pending() {
                    return false;
                }
                me.close_log_file();
                if !append_file(&cache_path, &log_path) {
                    return false;
                }
                let _ = fs::remove_file(&cache_path);
                true
            });
            // `written` is the answer, not `moved`: the record reached the
            // cache file, and a move that failed leaves it there for the next
            // one rather than losing it.
            let _ = moved;
            return written;
        }

        let open_success = self.open_log_file(OpenDir::Log, tv);
        // Buffered whether or not the log directory's file opened: with no
        // file to hand it to, [`Self::write_file_record`] answers `false` and
        // the record goes to the cache directory below instead of waiting in
        // the buffer for a file that is not there.
        let mut write_success = self.write_file_record(data);
        if open_success && !self.is_sync() {
            // A cache directory is configured — nothing else reaches this far
            // down `__Log2File` — so the batch has somewhere to go when the
            // log directory will not take it, and the close must not spend it
            // on its way there.
            write_success |= self.closed_before_the_cache_directory();
        }

        if !write_success {
            // The log directory's file is being left for the cache
            // directory's, but the batch is not: it is what the cache
            // directory is for. Only the handle goes, because
            // [`Self::close_log_file`] flushes, and a flush is one more
            // attempt at the file that has just refused the batch — and a
            // second refusal gives the batch up, which would leave the
            // cache directory nothing to stage.
            if open_success && self.is_sync() {
                self.forget_log_file();
            }
            if self.open_log_file(OpenDir::Cache, tv) {
                // The batch the log directory would not take — `data` included,
                // because [`Self::write_file_record`] left it in
                // `Self::pending` for exactly this. The C++ can only retry
                // with `data` here: `fwrite` has already consumed the rest.
                let had_batch = !self.pending.is_empty();
                write_success = self.flush_pending() && had_batch;
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
    ///
    /// `tv` is the second the records being written carry, which is the one
    /// every path around this answer is built from: what is asked is whether
    /// *their* log file is there, and a block that waited in the cache
    /// overnight is not answered about by tomorrow's. The C++ asks about now
    /// because it has nowhere else to read the time from — see
    /// [`Self::write_time`].
    fn cache_logs(&self, tv: i64) -> bool {
        let Some(cachedir) = self.config.cachedir.as_ref() else {
            return false;
        };
        if self.config.cache_days == 0 {
            return false;
        }

        if self.log_file_path(tv).exists() {
            return false;
        }

        // `boost::filesystem::space(cachedir).available >= 1 GiB`.
        match crate::sys::available_space(cachedir) {
            Some(available) => available >= MIN_FREE_SPACE,
            // A failed query leaves the space unknown, and unknown is read
            // as roomy rather than full: the value boost's own non-throwing
            // overload reports, `uintmax_t(-1)` for `available`, clears the
            // threshold, and the C++ has no arm to mirror — its `space()`
            // throws and the exception escapes `__Log2File`. An unreadable
            // disk therefore caches, and [`Self::log2file`] holds a cached
            // record back: it writes the cache file and returns without
            // moving it into the log directory, which a later call that can
            // read the disk does.
            None => true,
        }
    }

    /// `XloggerAppender::__OpenLogFile`.
    ///
    /// The directory is named rather than passed, so the caller does not have
    /// to clone it out of `self` before it can hand `self` over mutably:
    /// `__Log2File` runs per record, and the clone was one of the allocations
    /// that made the port slower than the C++ it mirrors.
    fn open_log_file(&mut self, dir: OpenDir, tv: i64) -> bool {
        if self.config.logdir.as_os_str().is_empty() {
            return false;
        }

        // `tv` is the second the records being written carry, and not the clock:
        // see [`Self::write_time`]. The file's name and the day the roll-over
        // check asks about both come from it, so a block that crossed midnight
        // is filed under the day it was produced and the timestamps inside it
        // agree with the file's date.
        let now_time = tv;
        // ... except here, where what is asked is whether the *clock* jumped
        // backwards. A record's own second may legitimately be behind the one
        // this file was opened with — the tips written around a drain are newer
        // than the block — and that is not a clock jump.
        let wall_time = now_secs();

        if self.log_file.is_some() {
            if self.open_file_day == local_time(now_time).date {
                return true;
            }
            self.close_log_file();
        }

        self.open_file_time = now_time;
        self.open_file_day = local_time(now_time).date;
        let logfilepath = match dir {
            OpenDir::Log => self.log_file_path(now_time),
            OpenDir::Cache => self.cache_file_path(now_time),
        };

        if wall_time < self.last_time {
            // The clock jumped backwards: keep using the previous file.
            let last_file_path = self.last_file_path.clone();
            match private_file(OpenOptions::new().create(true).truncate(false).append(true))
                .open(&last_file_path)
            {
                Ok(file) => {
                    self.flushed_len = file.metadata().map_or(0, |meta| meta.len());
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
            // The day's log file, and the cache-directory copy of it: see
            // [`private_file`] — a log holds everything an app wrote since it
            // was installed, and `std` would create it readable by every uid
            // on the device.
            let file =
                match private_file(OpenOptions::new().create(true).truncate(false).append(true))
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
            self.flushed_len = file.metadata().map_or(0, |meta| meta.len());
            self.log_file = Some(file);

            let now_tick = monotonic_millis();
            let tick_diff = now_tick.saturating_sub(self.last_tick);
            if self.last_time != 0 && (wall_time - self.last_time) > (tick_diff / 1000) as i64 + 300
            {
                let msg = format!(
                    "[F][ last log file:{} from {} to {}, time_diff:{}, tick_diff:{}\n",
                    self.last_file_path.display(),
                    format_local_timestamp(self.last_time),
                    format_local_timestamp(wall_time),
                    wall_time - self.last_time,
                    tick_diff
                );
                let mut tmp_buff = AutoBuffer::new();
                self.buff.write_sync(msg.as_bytes(), &mut tmp_buff);
                self.write_file_record(tmp_buff.as_slice());
            }

            self.last_file_path = logfilepath;
            self.last_tick = now_tick;
            self.last_time = wall_time;
            true
        }
    }

    /// `XloggerAppender::__CloseLogFile`.
    /// `fclose` of the C++: whatever is still buffered goes out first.
    ///
    /// Answers whether it did. Async closes the file after every block, and that
    /// flush is one more attempt at a batch [`Self::write_file_record`] could not
    /// get out: when it lands, the block *did* reach the file, and a `false` from
    /// the write would keep the block in the region for the next drain to write
    /// a second time (see [`Self::drain_buffer`]).
    fn close_log_file(&mut self) -> bool {
        let flushed = self.flush_pending();
        self.forget_log_file();
        flushed
    }

    /// [`Self::close_log_file`] of an async write that did not reach the file:
    /// `true` only when the batch that failed is the one the close got out.
    ///
    /// A batch is held for exactly one more attempt, so a flush that finds
    /// nothing to write is not a flush that wrote the block — it is
    /// [`Self::refuse_pending`] having given the batch up — and saying "reached
    /// the file" for that is what would drop a block no file ever saw.
    fn closed_after_a_failed_write(&mut self) -> bool {
        let held = !self.pending.is_empty();
        held && self.close_log_file()
    }

    /// [`Self::closed_after_a_failed_write`] for a write that still has the
    /// cache directory to fall back on.
    ///
    /// The close cannot be left out the way [`Self::forget_log_file`] stands
    /// in for it in the synchronous half: [`Self::open_log_file`] answers
    /// `true` for a file this appender still holds without moving to the
    /// directory it was asked for, so the handle has to go before the cache
    /// directory can be opened at all. And the flush the close does is one
    /// more attempt at a batch the write could not get out, which is what
    /// makes an async write succeed when the refusal was transient.
    ///
    /// What it must not do is be the attempt that *spends* the batch: a batch
    /// the file refuses twice is given up ([`Self::pending_refused`]), and the
    /// batch is the cache directory's to stage, not the log directory's. So
    /// the retry it carries is held back across this one attempt, and the
    /// fallback finds the batch whole.
    fn closed_before_the_cache_directory(&mut self) -> bool {
        if self.pending.is_empty() {
            return false;
        }
        let refused = self.pending_refused;
        self.pending_refused = false;
        let closed = self.close_log_file();
        if !closed {
            self.pending_refused = refused;
        }
        closed
    }

    fn forget_log_file(&mut self) {
        self.open_file_time = 0;
        self.open_file_day = NO_DAY;
        self.log_file = None;
        self.flushed_len = 0;
    }

    /// Hands `Self::pending` to the OS — the point where the C++'s `FILE*`
    /// buffer is flushed into a `write`.
    ///
    /// Returns `false` on I/O failure, after truncating the file back to where
    /// this write started and appending an error record (as `__WriteFile`
    /// does) — and **keeping** the batch, so it is neither lost nor written
    /// twice: see `Self::pending`.
    fn flush_pending(&mut self) -> bool {
        if self.pending.is_empty() {
            return true;
        }
        self.with_dir_lock(Self::flush_pending_locked)
    }

    /// `Self::flush_pending` with the log's lock held — by this caller's
    /// section or by one it is nested in.
    fn flush_pending_locked(&mut self) -> bool {
        if self.log_file.is_none() {
            return self.refuse_pending();
        }

        // Where the file ends, asked here rather than remembered in
        // `Self::flushed_len`: another writer may have appended since this
        // appender last looked, and rolling back to a length that is not the
        // file's would cut off what they wrote. The C++ `ftell`s before every
        // `fwrite` for the same number; this is one `lseek` per batch.
        let start = self
            .log_file
            .as_mut()
            .and_then(|file| file.seek(SeekFrom::End(0)).ok())
            .unwrap_or(self.flushed_len);

        let result = {
            let file = self.log_file.as_mut().expect("checked above");
            file.write_all(&self.pending)
                .map_err(|err| err.raw_os_error().unwrap_or(-1))
        };

        match result {
            Ok(()) => {
                self.flushed_len = start + self.pending.len() as u64;
                self.pending.clear();
                self.pending_refused = false;
                true
            }
            Err(errno) => {
                // What the OS held, so the file can be rolled back to it. The
                // C++ `ftell`s before every `fwrite` for the same number.
                if let Some(file) = self.log_file.as_mut() {
                    let _ = file.set_len(start);
                    let _ = file.seek(SeekFrom::End(0));
                }

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
                    // The file was rolled back to `start` and the error
                    // record is what follows it, so that is how long it is —
                    // and not [`Self::flushed_len`] plus the record, which is
                    // the same number only while this appender is the file's
                    // only writer. Another one that appended since the last
                    // flush leaves `flushed_len` behind the file's end, and
                    // the length the port remembers is what the rotation
                    // check and the next rollback both measure.
                    self.flushed_len = start + err_len as u64;
                }

                // Kept for one more attempt — but only one: see
                // [`Self::pending_refused`].
                self.refuse_pending()
            }
        }
    }

    /// Writes the records of one dead writer's cache file into the log and
    /// unlinks it.
    ///
    /// `data` is the file's bytes, read by
    /// [`Appender::treat_mapping_as_file_and_flush`], which calls this with the
    /// log's lock held: the drain, the append and the removal are one step as
    /// far as every other writer of the log is concerned.
    fn drain_dead_cache_slot(&mut self, path: &Path, data: Vec<u8>) -> crate::config::FileIoAction {
        use crate::config::FileIoAction;

        let mut buff = LogBuffer::new(
            true,
            Some(self.config.pub_key.as_str()),
            self.config.compress_mode,
            self.config.compress_level,
        );
        self.region = Region::Heap(data);
        buff.attach(self.region.as_mut_slice());
        self.buff = buff;

        let mut buffer = AutoBuffer::new();
        self.flush_buffer(&mut buffer);

        if buffer.is_empty() {
            return FileIoAction::Unnecessary;
        }

        let mark = mark_info();
        self.write_tips2file("~~~~~ begin of mmap from other process ~~~~~\n");
        let written = self.log2file(buffer.as_slice(), false);
        self.write_tips2file(&format!(
            "~~~~~ end of mmap from other process ~~~~~{mark}\n"
        ));

        // The cache is the only copy of these records: keep it when the write
        // failed, so the next recovery can try again instead of losing them.
        // `flush_pending` is part of the write — bytes this process is still
        // holding are not "reached a file" yet.
        if !written || !self.flush_pending() {
            return FileIoAction::WriteFailed;
        }
        // The heap region this drain read is a copy of a file that is about to
        // be unlinked, so giving it up here changes nothing on disk — but it is
        // what leaves `self.buff` able to start a block of its own.
        self.buffer_drained();

        match fs::remove_file(path) {
            Ok(()) => FileIoAction::Success,
            Err(_) => FileIoAction::RemoveFailed,
        }
    }

    /// Gives `Self::pending` up, or one more attempt first: see
    /// [`Self::pending_refused`]. Answers `false`, whatever it decided.
    fn refuse_pending(&mut self) -> bool {
        if self.pending_refused {
            self.pending.clear();
            self.pending_refused = false;
        } else {
            self.pending_refused = true;
        }
        false
    }

    /// `XloggerAppender::__WriteFile`.
    ///
    /// Appends `data` to the file — through `Self::pending`, so a record
    /// costs a `memcpy` instead of a syscall, which is what the C++'s `fwrite`
    /// costs. Returns `false` when the batch behind it could not reach the file,
    /// in which case `data` is still in `Self::pending` and no byte of it is
    /// in any file.
    fn write_file_record(&mut self, data: &[u8]) -> bool {
        self.pending.extend_from_slice(data);

        // Rotation: the C++ only re-computes the split index when the file is
        // (re)opened, so a long-lived sync-mode file never splits. The port
        // closes the file as soon as it grows past the limit, which makes both
        // modes behave the same.
        let len = self.flushed_len + self.pending.len() as u64;
        if self.max_file_size > 0 && len > self.max_file_size {
            // Answered by the close, and not with a `true` of its own: a
            // batch the flush could not get out is still the caller's, and a
            // rotation that claimed it had been written would let that
            // caller give up the block it came from.
            let flushed = self.close_log_file();
            self.open_file_time = 0;
            self.open_file_day = NO_DAY;
            return flushed;
        }

        if self.pending.len() >= LOG_FLUSH_THRESHOLD {
            return self.flush_pending();
        }

        // Buffered, but not accepted by anything yet when no file is open: the
        // C++'s `fwrite` cannot say "yes" for bytes nothing has taken either,
        // and `__Log2File` needs the `no` to try the cache directory.
        self.log_file.is_some()
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
            create_private_dir(dir)
                .map_err(|e| AppenderError(format!("create cache dir {}: {e}", dir.display())))?;
        }
        create_private_dir(&config.logdir).map_err(|e| {
            AppenderError(format!("create log dir {}: {e}", config.logdir.display()))
        })?;

        let alive_time = if max_alive_time >= MIN_LOG_ALIVE_TIME {
            max_alive_time
        } else {
            DEFAULT_MAX_ALIVE_TIME
        };

        // The C++ runs these on 2-3 minute delayed threads; the port prunes at
        // open time so the directory is clean by the time `open` returns. It is
        // held under the log's lock because it reads, moves and removes whole
        // files: two processes opening at once would otherwise both move the
        // same cache file into the log.
        //
        // That lock is the log's, not the cache's: what the sweep and the
        // sections below protect is one `<prefix>_<YYYYMMDD>.xlog`, which every
        // writer of this `logdir` appends to whatever its `cachedir` is — so a
        // lock named after the cache directory would be a different file for
        // each of them and would exclude nobody.
        let dir = cache_dir(&config).to_path_buf();
        let dir_lock = open_dir_lock(Some(&output_lock_path(&config)));
        // Whether advisory locking excludes anybody *where the cache files
        // live* decides the other thing `open` needs it for: whether a slot a
        // dead writer left behind can be told from one a live writer is using.
        //
        // Asked of the directory and not of a lock file in it: `<prefix>.lock`
        // is the file every writer of this log takes and gives up again around
        // each section it protects, so a probe of it asks who is holding it at
        // this instant — and the answer a writer that opens while another is
        // inside a section gets is "locking excludes nobody here", which is the
        // answer that turns the slots off and hands both writers one cache
        // file.
        let locking = sys::lock_excludes(&dir);
        {
            if let Some(file) = dir_lock.as_ref() {
                sys::lock_exclusive(file);
            }
            if let Some(cache) = &cachedir {
                del_timeout_file(cache, alive_time, &config.nameprefix);
                move_old_files(cache, &config.logdir, &config.nameprefix, config.cache_days);
            }
            del_timeout_file(&config.logdir, alive_time, &config.nameprefix);
            if let Some(file) = dir_lock.as_ref() {
                sys::unlock(file);
            }
        }

        // A cache file of this appender's own: see `CacheSlot`.
        let mut cache = claim_cache_slot(&dir, &config.nameprefix, locking);
        let (mut region, use_mmap) = match cache.as_mut() {
            Some(slot) => open_region(&mut slot.file, &slot.path),
            // Every slot is taken by a live writer. Nothing is corrupted by
            // logging without a cache — only the records a crash would have
            // left in one are lost.
            None => (Region::heap(), false),
        };

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
            pending: Vec::with_capacity(PENDING_CAPACITY),
            flushed_len: 0,
            pending_refused: false,
            open_file_time: 0,
            open_file_day: NO_DAY,
            last_time: 0,
            write_sec: None,
            last_tick: 0,
            last_file_path: PathBuf::new(),
            max_file_size,
            max_alive_time: alive_time,
            tx: None,
            use_mmap,
            cache,
            dir_lock,
            dir_lock_held: false,
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

        // Anything a previous process left in the cache file. `flush_buffer`
        // copies it out but leaves it in the region: the region is the only
        // durable copy of those records until they are in a log file, and
        // `__Clear()`ing it before the write is what loses them when the process
        // dies in between (see [`AppenderInner::drain_buffer`]).
        let mut leftover = AutoBuffer::new();
        appender.lock().flush_buffer(&mut leftover);

        if appender.lock().config.mode == AppenderMode::Async {
            // Nothing has been written yet, so returning an error here cannot
            // lose the drained records: they are still in the cache file. The
            // C++ calls SetMode() without checking and writes the leftover
            // regardless; fall back to a synchronous drain instead.
            if let Err(err) = appender.start_thread() {
                let mut guard = appender.lock();
                guard.config.mode = AppenderMode::Sync;
                let reached = guard.drain_leftover(leftover.as_slice());
                drop(guard);
                if reached {
                    appender.clear_cache_file_if_heap();
                }
                return Err(err);
            }
        }

        let mark = mark_info();
        if !leftover.is_empty() {
            appender.write_tips2file("~~~~~ begin of mmap ~~~~~\n");
            let reached = appender.lock().drain_leftover(leftover.as_slice());
            appender.write_tips2file(&format!("~~~~~ end of mmap ~~~~~{mark}\n"));
            if reached {
                // A slot with no mapping keeps its bytes, so once the records
                // above are in a log the file has to be emptied or the next
                // start appends them a second time — once per start, forever.
                // (With a mapping, `buffer_drained` zeroes it in place.)
                appender.clear_cache_file_if_heap();
            } else {
                // A block this process did not compress itself cannot stay in
                // the region: [`LogBuffer::attach`] recovered its length, but the
                // compressor that produced it is gone, so the next record written
                // through this appender would start a *second* compressed stream
                // in the middle of it — under the one header the block has — and
                // neither stream is readable after that. A block this process
                // compressed itself is a different matter ([`AppenderInner::
                // drain_buffer`] keeps it), because there the stream is still
                // alive and appending to it is what the next drain writes out.
                //
                // The records are not given up with it: they were copied out
                // above, and a batch [`AppenderInner::log2file`] could not write
                // stays in `AppenderInner::pending` for one more attempt — and
                // what the region stops holding the *file* still does, so a
                // run that dies before that attempt lands leaves them where the
                // next start finds them. Emptying the file here is what would
                // take the only copy left from a process that is about to need
                // it: see [`Appender::close`], which keeps it for the same
                // reason when its own drain is the one that failed.
                appender.lock().buffer_drained();
            }
        }

        // What a configured `pub_key` does *not* buy, said in the file ahead of
        // this run's own banner and its records — and not ahead of everything
        // in the file: a reopened log carries records of the runs before it,
        // and the block above has just put the ones recovered out of a cache
        // slot behind them. See [`Appender::crypt_caveat`].
        if let Some(caveat) = appender.crypt_caveat() {
            appender.write_tips2file(caveat);
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
            create_private_dir(dir)
                .map_err(|e| AppenderError(format!("create cache dir {}: {e}", dir.display())))?;
        }
        create_private_dir(&config.logdir).map_err(|e| {
            AppenderError(format!("create log dir {}: {e}", config.logdir.display()))
        })?;

        // The same lock a live writer of this log takes, opened here for the
        // same reason: what recovery persists goes into a log file another
        // writer may be appending to at that very moment, and the sections that
        // move more than one file are the ones that lock is for. Without it a
        // live writer's multi-write cache-file move and this drain — or this
        // drain and that writer's I/O-error rollback — interleave.
        let dir_lock = open_dir_lock(Some(&output_lock_path(config)));

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
            pending: Vec::with_capacity(PENDING_CAPACITY),
            flushed_len: 0,
            pending_refused: false,
            open_file_time: 0,
            open_file_day: NO_DAY,
            last_time: 0,
            write_sec: None,
            last_tick: 0,
            last_file_path: PathBuf::new(),
            max_file_size,
            max_alive_time: alive_time,
            tx: None,
            use_mmap: false,
            cache: None,
            dir_lock,
            dir_lock_held: false,
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

    /// `XloggerAppender::TreatMappingAsFileAndFlush` of one cache file.
    ///
    /// `path` is a slot a dead writer left behind: [`crate::appender_oneshot_flush`]
    /// only calls this for one whose lock nobody holds. The slot's lock says
    /// the file is a dead writer's; the log's own lock, taken here for the
    /// whole drain, says nobody else is writing to the log while it is appended
    /// to.
    ///
    /// `slot` is that very lock's handle, and it is the one the records are
    /// read through — not a second `open` of `path`. A range locked with
    /// `LockFileEx` is not the advisory `flock` it is on unix: Windows denies
    /// every *other* handle access to it, including handles opened afterwards
    /// in the process that took it, so a drain that reopened the slot to read
    /// it got `ERROR_LOCK_VIOLATION` and reported `ReadFailed` on every cache
    /// file it was given. The handle that holds a lock is the only one the
    /// bytes are readable through.
    pub(crate) fn treat_mapping_as_file_and_flush(
        &self,
        path: &Path,
        slot: &mut File,
    ) -> crate::config::FileIoAction {
        use crate::config::FileIoAction;

        if !path.exists() {
            return FileIoAction::Unnecessary;
        }

        // A slot with no bytes in it holds no records, and it is a state the
        // C++ has no file for: a writer whose region is a heap one keeps no
        // cache file at all, so `appender.cc` never gets as far as reading
        // one. This port empties the file instead (`clear_cache_file`) so that
        // the next start re-arms the pre-allocation, and what the C++ answers
        // for a buffer that flushes to nothing is what this answers for it:
        // `kActionUnnecessary`. `read_exact` would call it a read that failed,
        // and a caller that flushes until the code says there is nothing left
        // would never stop.
        if slot.metadata().is_ok_and(|meta| meta.len() == 0) {
            return FileIoAction::Unnecessary;
        }

        // Read the whole cache file into a heap region. A file that is there
        // and is shorter than a region is what a truncated or half-written one
        // looks like: the C++'s `kActionReadFailed`.
        let mut data = vec![0u8; BUFFER_BLOCK_LENGTH];
        if slot.read_exact(&mut data).is_err() {
            return FileIoAction::ReadFailed;
        }

        // One guard for the whole drain, write and unlink, and the log's lock
        // held across it: the records go into a log file another writer may be
        // appending to, and every step between them — the buffer, the file, the
        // cache file's removal — has to look like one step to that writer.
        //
        // `log_close` is left alone, and it is `true` for every appender that
        // gets here: this is the one-shot appender [`crate::appender_oneshot_flush`]
        // makes, whose records go out through the inner appender —
        // `flush_buffer`, `log2file`, `flush_pending` — and not through
        // [`Appender::write`], which is the only thing that asks the flag. So
        // there is no window to open: what `appender_oneshot_flush` calls when
        // this returns is `close`, and a closed appender answers that with
        // nothing at all — no `$$$$$` banner of its own behind the block this
        // just wrote, which is what a second banner there would be.
        let mut guard = self.lock();
        guard.with_dir_lock(|me| me.drain_dead_cache_slot(path, data))
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

    /// `consolelog_open_`, and the one other way a record reaches the console
    /// with the switch off.
    ///
    /// `XloggerAppender::Write` reads `_info->traceLog == 1` under
    /// `#ifdef ANDROID` — `XLogger::ForwardToSysTrace` is what an app sets it
    /// with — so a trace record is echoed where every other check of the
    /// switch is bypassed. No other platform of the C++ does it, and neither
    /// does this port: the console sink here is stderr everywhere (there is no
    /// portable `os_log`), and echoing a trace record to it on iOS or a
    /// desktop is a behaviour the C++ has nowhere.
    fn console_echoes(console_log_open: bool, info: Option<&XLoggerInfo>) -> bool {
        console_log_open
            || (cfg!(target_os = "android") && info.is_some_and(|info| info.trace_log == 1))
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

        if Self::console_echoes(
            self.shared.flags.console_log_open.load(Ordering::Relaxed),
            info,
        ) {
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

        // `if (2 <= recursion_count && recursion_str.empty())` — one dump per
        // episode: a deeper frame takes the one that is already there rather
        // than overwriting it, so what reaches the file is the first record
        // that recursed.
        if count >= 2 && RECURSION_DUMP.with(|cell| cell.borrow().is_none()) {
            if count > MAX_RECURSION {
                return;
            }
            let dump = recursion_dump(info, count);
            RECURSION_DUMP.with(|cell| *cell.borrow_mut() = Some(dump));
            return;
        }

        // `else`: the dump the last recursive write left, filed with the next
        // record. This is the half the port used to drop, so a logger that logs
        // from inside itself left nothing behind but a line on stderr.
        if let Some(dump) = RECURSION_DUMP.with(|cell| cell.borrow_mut().take()) {
            self.write_tips2file(&dump);
        }

        // The record, formatted before the lock: one buffer per thread, grown
        // once, and nothing else in the process can see it.
        let len = format_record(info, log);

        let mut guard = self.lock();
        // Read again, under the lock: `close` may have drained the cache and
        // stopped the writer thread while this record was being formatted. A
        // record added after that would sit in the cache with nothing left to
        // take it to a file, and be dropped when the appender is.
        if self.shared.flags.log_close.load(Ordering::Acquire) {
            return;
        }
        // The record's own second, which is what dates the file it lands in:
        // see [`AppenderInner::write_time`]. `None` when the record carries none
        // of its own — the tips, and the banner `close` writes — and then the
        // file is named after the clock.
        let sec = info.map(|info| info.timeval.0).filter(|sec| *sec > 0);
        // `write_sync` / `write_async` read the record out of that buffer; no
        // borrow of it is held across the lock, which is what keeps the two
        // apart safe.
        if guard.config.mode == AppenderMode::Sync {
            // Every record is filed as it is logged, so this one's own second is
            // the whole story — including the `None` of a record that carries
            // none: an undated record is dated by the clock, and the second of
            // the record before it must not stay behind and file it under that
            // record's day, which is another day's file as soon as the two are
            // written either side of midnight.
            guard.write_sec = sec;
            guard.write_sync(len);
        } else {
            // A block reaches a file **whole**, so it must not hold records of
            // two days: what is in the cache is yesterday's, and the file the
            // block would be filed under is the one this record's day names, so
            // yesterday's last records would land in today's file and today's
            // file would hold records that are not today's. Handing the block
            // over before the new day's record joins it costs one drain per
            // midnight and keeps each day's records in that day's file.
            //
            // `write_sec` is the block's: it carries the second of the last
            // record that joined it and a drain clears it (see
            // [`AppenderInner::buffer_drained`]), so it is `None` until the
            // block has a record of its own.
            let rolled = match sec.map(|sec| local_time(sec).date) {
                Some(day)
                    if guard
                        .write_sec
                        .is_some_and(|sec| local_time(sec).date != day) =>
                {
                    // A drain that did not reach a file leaves the block in the
                    // region, for the next drain to write again (see
                    // [`AppenderInner::drain_buffer`]) — yesterday's records and
                    // today's both, once this record has joined it. Dating the
                    // block by this record anyway files the whole of it under
                    // today's name, which is the disagreement the drain is here
                    // to prevent; so the day stays until a drain has actually
                    // taken the block away.
                    guard.drain_buffer(false)
                }
                _ => true,
            };
            // A record with no day of its own leaves the block's alone: it is the
            // day of the records the block already holds.
            if rolled && sec.is_some() {
                guard.write_sec = sec;
            }
            guard.write_async(info, len);
        }
    }

    /// `XloggerAppender::WriteTips2File`.
    pub(crate) fn write_tips2file(&self, tips: &str) {
        self.lock().write_tips2file(tips);
    }

    /// `XloggerAppender::Flush` — wake the writer thread, and nothing else: no
    /// record is on the disk because this returned, and the caller is given no
    /// way to find out when it is. `request_flush` is this, and `flush_sync`
    /// is the drain that answers when it is over.
    pub(crate) fn wake_writer(&self) {
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
        //
        // One lock for the whole flush: taking it five times was five turns
        // through the same mutex for one flush, and the five were not needed —
        // nothing in between can be seen by anyone else.
        let mut guard = self.lock();
        guard.flush_pending();

        if guard.config.mode == AppenderMode::Sync {
            return;
        }

        guard.drain_buffer(false);
    }

    /// `XloggerAppender::Close`.
    ///
    /// Idempotent: a second `close` — the one [`Appender::drop`] runs on an
    /// appender [`crate::appender_close`] already closed — does nothing at all.
    /// Re-running it used to be harmless too, except for the last line: zeroing
    /// the cache region again would wipe the region of *another* appender that
    /// has since mapped the same cache file, along with every record in it.
    pub(crate) fn close(&self) {
        if self.shared.flags.log_close.load(Ordering::Acquire) {
            return;
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

        // The writer thread's last drain may have left a batch in
        // [`AppenderInner::pending`]: it appends one the way any write does, and
        // nothing flushes it once the appender is in sync mode, where
        // `__Log2File` leaves the file open. Handed to the OS here — the file
        // is about to be dropped, and a batch that outlived its appender is a
        // batch no later flush can recover. So is a block the thread could not
        // write at all, which [`AppenderInner::drain_buffer`] writes now and
        // gives up only if it got there.
        let drained = {
            let mut guard = self.lock();
            guard.tx = None;
            guard.drain_buffer(false)
        };
        // C++: `memset(mmap_file_.data(), 0, kBufferBlockLength)` before closing
        // the mapping, so a later `open` starts from an empty cache — which is
        // what `drain_buffer` did on its way through. What is *not* done is
        // giving the region up when the write failed: those records are still
        // only in the cache file, and the next start recovers them from it.
        if drained {
            self.clear_cache_file_if_heap();
        }
    }

    /// Empties this appender's own cache file when there is no mapping behind
    /// it, which is when the file and the region are two copies of the same
    /// records rather than one.
    ///
    /// A mapping's bytes *are* the file's, so zeroing the region empties it
    /// too; without one, the file keeps whatever the region held and the next
    /// start would append it a second time — once per start, forever.
    ///
    /// Only for the writer that owns the file: [`Appender::oneshot`] works on
    /// another process's cache file and must leave it alone when it cannot
    /// drain or remove it.
    fn clear_cache_file_if_heap(&self) {
        let (use_mmap, path) = {
            let guard = self.lock();
            (guard.use_mmap, guard.cache_path())
        };
        if !use_mmap {
            if let Some(path) = path {
                clear_cache_file(&path);
            }
        }
    }

    /// What a configured [`XLogConfig::pub_key`] does **not** give the caller:
    /// the line the log says it in, or `None` when there is nothing to say.
    ///
    /// Two ways a `pub_key` encrypts nothing, and until now neither was said
    /// anywhere:
    ///
    /// * `LogCrypt::new` leaves `is_crypt` false for a key that is not the 128
    ///   hex characters of a valid secp256k1 point — it early-returns, the way
    ///   the C++ does — and every record of the log then goes out with the
    ///   NOCRYPT magics and a plaintext body;
    /// * `LogCrypt::CryptSyncLog` stores a sync record's body in the clear
    ///   whatever the key is: the C++ has the TEA loop commented out, the port
    ///   kept that, and the record still carries the "crypt" magic.
    ///
    /// Both are written into the file and not answered to the caller, which is
    /// what `xlog encode` does for the first (`--pubkey` that is not a key is an
    /// error there): an app on a device cannot be rebuilt the moment its key
    /// turns out to be bad, and an appender that refused to open would cost it
    /// every record of the run, encryption or none. What nobody may believe is
    /// that the log is encrypted when it is not, and the file is where whoever
    /// reads that log — or ships it off the device — looks first.
    fn crypt_caveat(&self) -> Option<&'static str> {
        let guard = self.lock();
        if guard.config.pub_key.is_empty() {
            return None;
        }
        if !guard.buff.is_crypt() {
            return Some(
                "[F][ the configured pub_key is not the 128 hex characters of a secp256k1 \
                 public key, so no record of this log is encrypted\n",
            );
        }
        if guard.config.mode == AppenderMode::Sync {
            return Some(
                "[F][ appender mode is sync: `LogCrypt::CryptSyncLog` stores a record's body \
                 in the clear, so the configured pub_key encrypts nothing in this mode\n",
            );
        }
        None
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
        // A mode the configured `pub_key` does nothing in is worth a line in
        // the log as much as an open in that mode is: see
        // [`Appender::crypt_caveat`].
        if let Some(caveat) = self.crypt_caveat() {
            self.write_tips2file(caveat);
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

    /// The cache file this appender claimed — see `CacheSlot`. `None` when it
    /// writes without one, which is what every slot being taken leaves.
    ///
    /// Not to be confused with `AppenderInner::cache_file_path`, which is the
    /// day's log file *inside the cache directory*.
    pub(crate) fn claimed_cache_path(&self) -> Option<PathBuf> {
        self.lock().cache_path()
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

        // One drain under one lock, and not two halves with the lock dropped in
        // between: the block is copied out of the region, written, and only then
        // given up, so nothing can observe the region emptied while the records
        // it held are still in memory.
        let closed = {
            let mut guard = shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.drain_buffer(true);
            shared.flags.log_close.load(Ordering::Acquire)
        };

        if closed || disconnected || msg == Some(Msg::Close) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppenderMode, FileIoAction, XLogConfig};
    use marsrs_buffer::CompressMode;
    use marsrs_crypt::{magic, LogCrypt, HEADER_LEN, TAILER_LEN};
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicU64, AtomicUsize};

    /// A trace record — one an app marked with
    /// `XLogger::ForwardToSysTrace` — is echoed only where the C++ echoes
    /// it, which is Android and nowhere else.
    #[cfg(target_os = "android")]
    #[test]
    fn a_trace_record_is_echoed_with_console_logging_off() {
        let info = XLoggerInfo {
            trace_log: 1,
            ..XLoggerInfo::default()
        };
        assert!(Appender::console_echoes(false, Some(&info)));
        assert!(!Appender::console_echoes(
            false,
            Some(&XLoggerInfo::default())
        ));
        assert!(Appender::console_echoes(true, Some(&info)));
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn a_trace_record_is_not_echoed_off_android() {
        let info = XLoggerInfo {
            trace_log: 1,
            ..XLoggerInfo::default()
        };
        assert!(!Appender::console_echoes(false, Some(&info)));
        assert!(Appender::console_echoes(true, Some(&info)));
    }

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
            appender.wake_writer();
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

    /// A block that waits in the cache across midnight lands in the day it was
    /// produced, and not in the day it is drained, so that the file's date and
    /// the timestamps of the records inside it agree.
    #[test]
    fn a_block_is_filed_under_the_day_its_records_carry() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Async), 0, 0).unwrap();

        // Three days back, so no hour of a day boundary is needed for the two
        // names to differ.
        let day = now_secs() - 3 * SECONDS_PER_DAY;
        let name = |dir: &Path, tv: i64| {
            let prefix = crate::file_util::make_log_file_name_prefix(tv, "Mars");
            dir.join(format!("{prefix}.xlog"))
        };

        let info = XLoggerInfo {
            timeval: (day, 0),
            ..info(LogLevel::Info)
        };
        appender.write(Some(&info), "written before the drain");
        appender.close();

        let filed = name(tmp.path(), day);
        assert!(filed.exists(), "{filed:?} missing");
        let text = decoded_text(&fs::read(&filed).unwrap());
        assert!(text.contains("written before the drain"), "{text}");
        // The drain itself happened today, and the file is not named after it.
        assert!(
            !name(tmp.path(), now_secs()).exists(),
            "the block went to today's file instead of its own"
        );
    }

    /// The same second dates every path of one `__Log2File`, so a record cannot
    /// be named by one day and filed under another.
    #[test]
    fn a_record_with_no_time_of_its_own_is_filed_under_the_clock() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Sync), 0, 0).unwrap();

        // `XLoggerInfo::default()` carries `timeval: (0, 0)`, which is what an
        // appender that is only handed tips sees.
        appender.write(None, "no timeval on this one");
        appender.close();

        let path = today_name(tmp.path());
        assert!(path.exists(), "{path:?} missing");
        let text = decoded_text(&fs::read(&path).unwrap());
        assert!(text.contains("no timeval on this one"), "{text}");
    }

    /// A block reaches a file whole, so it must not hold records of two days.
    ///
    /// The block is filed under the day of the last record that joined it, so
    /// one that was started before midnight and drained after it puts the
    /// earlier day's records in the later day's file — the file's date and the
    /// timestamps of the records inside it disagree, which is what an app that
    /// uploads one file per day sees as a day's logs being in the wrong file.
    #[test]
    fn a_block_does_not_hold_records_of_two_days() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Async), 0, 0).unwrap();

        let first_day = now_secs() - 3 * SECONDS_PER_DAY;
        let next_day = first_day + SECONDS_PER_DAY;
        assert_ne!(
            local_time(first_day).date,
            local_time(next_day).date,
            "the two records have to fall on different days"
        );
        let name = |dir: &Path, tv: i64| {
            let prefix = crate::file_util::make_log_file_name_prefix(tv, "Mars");
            dir.join(format!("{prefix}.xlog"))
        };

        let earlier = XLoggerInfo {
            timeval: (first_day, 0),
            ..info(LogLevel::Info)
        };
        appender.write(Some(&earlier), "written on the earlier day");
        let later = XLoggerInfo {
            timeval: (next_day, 0),
            ..info(LogLevel::Info)
        };
        appender.write(Some(&later), "written on the later day");
        appender.flush_sync();
        appender.close();

        let earlier_file = name(tmp.path(), first_day);
        let later_file = name(tmp.path(), next_day);
        assert!(earlier_file.exists(), "{earlier_file:?} missing");
        assert!(later_file.exists(), "{later_file:?} missing");

        let earlier_text = decoded_text(&fs::read(&earlier_file).unwrap());
        let later_text = decoded_text(&fs::read(&later_file).unwrap());
        assert!(
            earlier_text.contains("written on the earlier day"),
            "{earlier_text}"
        );
        assert!(
            !earlier_text.contains("written on the later day"),
            "{earlier_text}"
        );
        assert!(
            later_text.contains("written on the later day"),
            "{later_text}"
        );
        assert!(
            !later_text.contains("written on the earlier day"),
            "the whole block went to the later day's file: {later_text}"
        );
    }

    /// A roll-over drain the log directory refuses leaves the block in the region
    /// — with the day it carries, which is the day of the records it holds. The
    /// record that crossed midnight must not re-date it: the block would then be
    /// filed, whole, under the later day, which is the disagreement the drain is
    /// there to prevent.
    #[test]
    fn a_block_the_drain_could_not_hand_over_keeps_its_day() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Async), 0, 0).unwrap();

        let first_day = now_secs() - 3 * SECONDS_PER_DAY;
        let name = |dir: &Path, tv: i64| {
            let prefix = crate::file_util::make_log_file_name_prefix(tv, "Mars");
            dir.join(format!("{prefix}.xlog"))
        };

        let earlier = XLoggerInfo {
            timeval: (first_day, 0),
            ..info(LogLevel::Info)
        };
        appender.write(Some(&earlier), "written on the earlier day");

        // The file the block would be handed over to cannot be opened, so the
        // roll-over drain fails — what a full or read-only file system looks
        // like from here.
        let blocked = name(tmp.path(), first_day);
        fs::create_dir(&blocked).unwrap();
        let later = XLoggerInfo {
            timeval: (now_secs(), 0),
            ..info(LogLevel::Info)
        };
        appender.write(Some(&later), "written on the later day");

        // It can be written again, and the block is handed over with the day it
        // carries.
        fs::remove_dir(&blocked).unwrap();
        appender.flush_sync();
        appender.close();

        let text = decoded_text(&fs::read(&blocked).unwrap());
        assert!(text.contains("written on the earlier day"), "{text}");
        assert!(text.contains("written on the later day"), "{text}");

        let today = name(tmp.path(), now_secs());
        if today.exists() {
            let today_text = decoded_text(&fs::read(&today).unwrap());
            assert!(
                !today_text.contains("written on the earlier day"),
                "the block was filed under the later day: {today_text}"
            );
        }
    }

    /// A block's second dates that block and nothing after it.
    ///
    /// Once the block has been written out the buffer is empty again, so the
    /// next file is named after the next record that carries a time of its own.
    /// A write that carries none — the tips, and the banner `close` writes — is
    /// filed under the clock, and not under the day of a block that is already
    /// in a file.
    #[test]
    fn a_write_after_the_drain_is_filed_under_the_clock() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Async), 0, 0).unwrap();

        let day = now_secs() - 3 * SECONDS_PER_DAY;
        let name = |dir: &Path, tv: i64| {
            let prefix = crate::file_util::make_log_file_name_prefix(tv, "Mars");
            dir.join(format!("{prefix}.xlog"))
        };

        let info = XLoggerInfo {
            timeval: (day, 0),
            ..info(LogLevel::Info)
        };
        appender.write(Some(&info), "written before the drain");
        // The block reaches its own day's file and the buffer is empty again.
        appender.flush_sync();
        // `write_tips2file` is a write with no time of its own.
        appender.write_tips2file("a tip with no time of its own");
        appender.close();

        let filed = name(tmp.path(), day);
        assert!(filed.exists(), "{filed:?} missing");
        let text = decoded_text(&fs::read(&filed).unwrap());
        assert!(text.contains("written before the drain"), "{text}");
        assert!(!text.contains("a tip with no time of its own"), "{text}");

        let today = name(tmp.path(), now_secs());
        assert!(
            today.exists(),
            "{today:?} missing: the tip was filed under the drained block's day"
        );
        let text = decoded_text(&fs::read(&today).unwrap());
        assert!(text.contains("a tip with no time of its own"), "{text}");
    }

    /// `WriteTips2File(recursion_str)` — the dump of a write that was logged
    /// from inside the logger is filed with the next record, and not only
    /// printed on the console.
    #[test]
    fn the_recursion_dump_reaches_the_log_file() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Sync), 0, 0).unwrap();

        // The counter is per thread, so raising it is what a write that is
        // itself inside a write looks like from here.
        RECURSION_COUNT.with(|cell| cell.set(1));
        appender.write(Some(&info(LogLevel::Info)), "logged from inside the logger");
        RECURSION_COUNT.with(|cell| cell.set(0));

        appender.write(Some(&info(LogLevel::Info)), "the record after it");
        appender.close();

        let text = decoded_text(&fs::read(today_name(tmp.path())).unwrap());
        // The record that recursed is the one that is *not* filed: the dump
        // stands in for it.
        assert!(!text.contains("logged from inside the logger"), "{text}");
        assert!(text.contains("Recursive calls!!!, count:2"), "{text}");
        assert!(text.contains("the record after it"), "{text}");
    }

    /// A recursive write is one that runs while an outer write is consuming the
    /// record it formatted: [`with_record`] holds a borrow of the buffer for as
    /// long as its callback does, so a dump that formatted into that buffer
    /// borrowed it a second time and panicked.
    #[test]
    fn the_recursion_dump_does_not_borrow_the_in_flight_record() {
        let len = format_record(Some(&info(LogLevel::Info)), "the record being written");
        let before = with_record(len, |data| data.to_vec());

        // What re-entry from inside that callback looks like.
        let dump = with_record(len, |_| recursion_dump(Some(&info(LogLevel::Info)), 2));

        assert!(dump.contains("Recursive calls!!!, count:2"), "{dump}");
        assert_eq!(
            with_record(len, |data| data.to_vec()),
            before,
            "the record the outer write is reading was overwritten"
        );
    }

    /// The dump is built beside the record the outer write has formatted and
    /// not yet consumed, so that record reaches the file as it was formatted,
    /// and not as the dump's bytes under the outer record's length.
    #[test]
    fn the_recursion_dump_leaves_the_outer_record_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Sync), 0, 0).unwrap();

        // The record an outer write holds at the point a write recurses.
        let len = format_record(Some(&info(LogLevel::Info)), "the record being written");
        let before = with_record(len, |data| data.to_vec());

        // The counter is per thread, so raising it is what a write that is
        // itself inside a write looks like from here.
        RECURSION_COUNT.with(|cell| cell.set(1));
        appender.write(Some(&info(LogLevel::Info)), "logged from inside the logger");
        RECURSION_COUNT.with(|cell| cell.set(0));

        assert_eq!(
            with_record(len, |data| data.to_vec()),
            before,
            "the dump was built in the buffer the outer record is read out of"
        );

        // The outer write goes on, and the next write files the dump with it.
        appender.lock().write_sync(len);
        appender.write(Some(&info(LogLevel::Info)), "the record after it");
        appender.close();

        let text = decoded_text(&fs::read(today_name(tmp.path())).unwrap());
        assert!(text.contains("the record being written"), "{text}");
        assert!(text.contains("Recursive calls!!!, count:2"), "{text}");
        assert!(text.contains("the record after it"), "{text}");
    }

    /// Sync mode writes every record to its own file, so a record that carries no
    /// time of its own is filed under the clock: the second of the record before
    /// it has no business dating a record that is not that record.
    #[test]
    fn an_undated_record_is_filed_under_the_clock() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Sync), 0, 0).unwrap();

        let day = now_secs() - 3 * SECONDS_PER_DAY;
        let name = |dir: &Path, tv: i64| {
            let prefix = crate::file_util::make_log_file_name_prefix(tv, "Mars");
            dir.join(format!("{prefix}.xlog"))
        };

        let info = XLoggerInfo {
            timeval: (day, 0),
            ..info(LogLevel::Info)
        };
        appender.write(Some(&info), "written on that day");
        appender.write(None, "a record with no time of its own");
        appender.close();

        let filed = name(tmp.path(), day);
        assert!(filed.exists(), "{filed:?} missing");
        let text = decoded_text(&fs::read(&filed).unwrap());
        assert!(text.contains("written on that day"), "{text}");
        assert!(
            !text.contains("a record with no time of its own"),
            "the undated record was filed under the record before it: {text}"
        );

        let today = name(tmp.path(), now_secs());
        assert!(
            today.exists(),
            "{today:?} missing: the undated record went to the day before"
        );
        let text = decoded_text(&fs::read(&today).unwrap());
        assert!(text.contains("a record with no time of its own"), "{text}");
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

    /// One drain of the cache region is tens of KiB — far more than
    /// [`PENDING_CAPACITY`] — and it is handed to the OS whole, whatever the
    /// buffer was allocated with.
    #[test]
    fn a_drain_bigger_than_the_buffer_reaches_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Async), 0, 0).unwrap();

        for i in 0..3_000 {
            appender.write(
                Some(&info(LogLevel::Info)),
                &format!("async record {i} padding padding padding"),
            );
        }
        appender.close();

        let bytes = fs::read(today_name(tmp.path())).unwrap();
        assert!(
            bytes.len() > PENDING_CAPACITY,
            "the drain never reached the file: {} bytes",
            bytes.len()
        );
        let text = decoded_text(&bytes);
        assert!(text.contains("async record 0 "), "{text}");
    }

    /// The contract above, end to end: the batch is still there after the
    /// failure, so it reaches the file when there is one again.
    #[test]
    fn a_batch_that_fails_to_reach_the_file_is_written_later() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Sync), 0, 0).unwrap();

        appender.write(Some(&info(LogLevel::Info)), "the very first record");
        let mut guard = appender.lock();
        // The file goes away under the buffer, which is what a failed write
        // leaves behind as well.
        guard.log_file = None;
        assert!(!guard.flush_pending(), "nothing reached the file");
        drop(guard);

        appender.write(Some(&info(LogLevel::Info)), "one after the failure");
        appender.close();

        let text = decoded_text(&fs::read(today_name(tmp.path())).unwrap());
        assert!(
            text.contains("the very first record"),
            "the batch was dropped instead of kept: {text}"
        );
    }

    /// The other end of that: the close's flush is one more attempt at the
    /// batch, but not the attempt that gives it up. What it is for is the file
    /// *handle*, which has to go before the cache directory can be opened — so
    /// an async write whose batch the log directory refused still finds the
    /// batch whole when it gets there.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_batch_the_closing_flush_could_not_get_out_is_staged_in_the_cache_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let mut cfg = config(tmp.path(), AppenderMode::Async);
        cfg.cachedir = Some(cache.clone());
        // A file that opens and then refuses every write: what a full disk
        // looks like to `__Log2File`, which cannot tell the two apart.
        std::os::unix::fs::symlink("/dev/full", today_name(tmp.path())).unwrap();

        let appender = Appender::open(cfg, 0, 0).unwrap();
        let mut guard = appender.lock();
        guard.close_log_file();
        guard.pending.clear();
        guard.pending_refused = false;
        assert!(guard.open_log_file(OpenDir::Log, now_secs()));

        // Over the threshold, so `write_file_record` asks the file for the
        // batch there and then, and fails.
        let mut batch = vec![b'.'; LOG_FLUSH_THRESHOLD];
        batch.extend_from_slice(b"a batch only the cache directory will take");
        assert!(
            guard.log2file(&batch, false),
            "the batch reached no file at all"
        );
        drop(guard);
        appender.close();

        let name = crate::file_util::make_log_file_name_prefix(now_secs(), "Mars");
        let text = String::from_utf8_lossy(&fs::read(cache.join(format!("{name}.xlog"))).unwrap())
            .to_string();
        assert!(
            text.contains("a batch only the cache directory will take"),
            "the batch the cache directory was given is not in it: {text}"
        );
    }

    /// Async closes the file after every block, and `fclose` flushes first — one
    /// more attempt at a batch the write could not get out. When it lands, the
    /// block *did* reach the file, and answering `false` would keep it in the
    /// region for the next drain to write a second time.
    #[test]
    fn a_block_the_closing_flush_got_out_has_reached_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Async), 0, 0).unwrap();
        let mut guard = appender.lock();
        assert!(guard.open_log_file(OpenDir::Log, now_secs()));
        // The file goes away under the buffer, which is what a failed write
        // leaves behind as well: the batch stays, for one more attempt.
        let file = guard.log_file.take().expect("opened above");
        assert!(
            !guard.write_file_record(b"a block only the closing flush gets out"),
            "the batch reached a file that is not there"
        );
        guard.log_file = Some(file);
        assert!(
            guard.closed_after_a_failed_write(),
            "the batch the close got out is not what answered"
        );
        drop(guard);
        appender.close();

        let text = String::from_utf8_lossy(&fs::read(today_name(tmp.path())).unwrap()).to_string();
        assert!(
            text.contains("a block only the closing flush gets out"),
            "the batch the close got out is not in the file: {text}"
        );
    }

    /// The other half of that: a batch is kept for one attempt and no more, so an
    /// empty `pending` at the close is a batch that was given up — not one that
    /// got out.
    #[test]
    fn a_batch_that_was_given_up_on_has_not_reached_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Async), 0, 0).unwrap();
        let mut guard = appender.lock();
        assert!(guard.open_log_file(OpenDir::Log, now_secs()));
        let file = guard.log_file.take().expect("opened above");
        // The attempt the batch is kept for has already been spent, so this one
        // gives it up rather than writing it.
        guard.pending.extend_from_slice(b"a block no file will see");
        guard.pending_refused = true;
        assert!(!guard.flush_pending(), "the batch reached a file");
        assert!(guard.pending.is_empty(), "the batch was not given up");

        // A file to write to again: a close that found nothing to flush is not a
        // close that got the batch out, whatever `fclose` answers.
        guard.log_file = Some(file);
        assert!(
            !guard.closed_after_a_failed_write(),
            "a batch that was given up answered that it reached the file"
        );
        drop(guard);
        appender.close();

        let text = String::from_utf8_lossy(&fs::read(today_name(tmp.path())).unwrap()).to_string();
        assert!(
            !text.contains("a block no file will see"),
            "the batch is in the file after all: {text}"
        );
    }

    /// The same, for one record: with a cache directory configured but not
    /// active (`cache_days == 0`), the log directory's file is what is missing,
    /// and the record has to be staged in the cache directory rather than wait
    /// in the buffer for a file that never opens.
    #[test]
    fn a_record_the_log_directory_cannot_open_is_staged_in_the_cache_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let mut cfg = config(tmp.path(), AppenderMode::Sync);
        cfg.cachedir = Some(cache.clone());
        fs::create_dir(today_name(tmp.path())).unwrap();

        let appender = Appender::open(cfg, 0, 0).unwrap();
        // The banner `open` writes has already fallen back to the cache
        // directory: it left a file behind, and a record that finds one takes
        // the cache branch of `__Log2File` instead of the fallback — which
        // finds nothing to flush when the buffer is empty. Both taken away, so
        // that this record is the one that has to open the cache copy.
        appender.lock().close_log_file();
        for entry in fs::read_dir(&cache).unwrap().flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "xlog") {
                fs::remove_file(entry.path()).unwrap();
            }
        }
        appender.write(Some(&info(LogLevel::Info)), "staged, not buffered");
        appender.close();

        let mut text = String::new();
        for entry in fs::read_dir(&cache).unwrap().flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "xlog") {
                text.push_str(&decoded_text(&fs::read(entry.path()).unwrap()));
            }
        }
        assert!(
            text.contains("staged, not buffered"),
            "the record waited in the buffer instead: {text}"
        );
    }

    /// What `__Log2File` does with a batch the log directory refuses: it is
    /// staged in the cache directory, whole — which is what
    /// `Self::flush_pending` keeping it is for.
    #[test]
    fn a_log_directory_that_refuses_a_batch_stages_it_in_the_cache_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let mut cfg = config(tmp.path(), AppenderMode::Sync);
        cfg.cachedir = Some(cache.clone());

        // `<logdir>/Mars_<today>.xlog` as a directory: it cannot be opened, so
        // nothing this appender accepts can reach a file until `__Log2File`
        // opens the cache directory's copy instead.
        fs::create_dir(today_name(tmp.path())).unwrap();

        let appender = Appender::open(cfg, 0, 0).unwrap();
        appender.write(Some(&info(LogLevel::Info)), "the very first record");
        for i in 0..500 {
            appender.write(
                Some(&info(LogLevel::Info)),
                &format!("record {i} {i:x} {i:o} the quick brown fox"),
            );
        }
        appender.close();

        let mut text = String::new();
        for entry in fs::read_dir(&cache).unwrap().flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "xlog") {
                text.push_str(&decoded_text(&fs::read(entry.path()).unwrap()));
            }
        }
        assert!(
            text.contains("the very first record"),
            "the batch was dropped instead of cached: {text}"
        );
    }

    /// The other way a batch misses the file: the log directory's file opens
    /// and then refuses the write, which is what a full disk does. An `open`
    /// that fails is the easy half — nothing has been written, so the batch is
    /// still whole — and a `write` that fails is the half the cache directory
    /// is really there for.
    ///
    /// `/dev/full` is the one file a unix answers every write to with ENOSPC,
    /// so the day's log file is a link to it: `open` succeeds, `write` cannot.
    #[cfg(unix)]
    #[test]
    fn a_log_file_that_refuses_a_write_stages_the_batch_in_the_cache_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let mut cfg = config(tmp.path(), AppenderMode::Sync);
        cfg.cachedir = Some(cache.clone());
        std::os::unix::fs::symlink("/dev/full", today_name(tmp.path())).unwrap();

        let appender = Appender::open(cfg, 0, 0).unwrap();
        appender.write(Some(&info(LogLevel::Info)), "the very first record");
        for i in 0..500 {
            appender.write(
                Some(&info(LogLevel::Info)),
                &format!("record {i} {i:x} {i:o} the quick brown fox"),
            );
        }
        appender.close();

        let mut text = String::new();
        for entry in fs::read_dir(&cache).unwrap().flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "xlog") {
                text.push_str(&decoded_text(&fs::read(entry.path()).unwrap()));
            }
        }
        assert!(
            text.contains("the very first record"),
            "the batch was dropped instead of cached: {text}"
        );
    }

    /// The contract behind the test above, pinned where an ENOSPC cannot be
    /// arranged: a flush that cannot reach the file leaves the batch in the
    /// buffer, so the caller still has it.
    #[test]
    fn a_batch_that_fails_to_reach_the_file_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let appender = Appender::open(config(tmp.path(), AppenderMode::Sync), 0, 0).unwrap();

        appender.write(Some(&info(LogLevel::Info)), "kept, not dropped");
        let mut guard = appender.lock();
        assert!(!guard.pending.is_empty(), "the record was not buffered");
        // The file goes away under the buffer, which is what a failed write
        // leaves behind as well.
        guard.log_file = None;
        assert!(!guard.flush_pending(), "nothing reached a file");
        assert!(
            !guard.pending.is_empty(),
            "the batch was dropped with the file"
        );
        drop(guard);
        appender.close();
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

    /// A file whose blocks could not be reserved must be left **short**.
    ///
    /// The length `set_len` records and the blocks the zero-fill reserves are
    /// two different things, and only the second is what a mapping needs: a
    /// file that is `BUFFER_BLOCK_LENGTH` long with a hole where the zeros
    /// should be is a file the next `open` skips the pre-allocation for — it
    /// measures the length, not the blocks — and maps, and the first record
    /// stored into the hole raises SIGBUS on a disk that is still full. That is
    /// the crash the pre-allocation exists to prevent, handed to the next
    /// start.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_cache_file_whose_blocks_were_not_reserved_is_left_short() {
        // The one file a test can hold that takes the `set_len` and refuses
        // the write: see `sys::unwritable_file`.
        let Some(mut file) = crate::sys::unwritable_file() else {
            return;
        };
        assert_eq!(file.metadata().unwrap().len(), 0);

        let (region, use_mmap) = open_region(&mut file, Path::new("marsrs-unwritable"));
        assert!(!use_mmap);
        assert!(matches!(region, Region::Heap(_)), "heap, and not a mapping");
        assert_eq!(
            file.metadata().unwrap().len(),
            0,
            "the file is a {BUFFER_BLOCK_LENGTH} byte hole otherwise, and the next open \
             maps it instead of pre-allocating it again"
        );
    }

    /// The other half of the same hole: a file that already measures the
    /// block, which is what `set_len` of a build before this one left behind.
    /// The length is what the file claims and the zeros are what say nothing
    /// was stored, so one that measures the whole block and reads as nothing
    /// is pre-allocated like any other and not taken at its word.
    #[test]
    fn a_cache_file_that_measures_the_block_but_reads_as_zeros_is_filled() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Mars.mmap3");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        file.set_len(BUFFER_BLOCK_LENGTH as u64).unwrap();
        assert!(
            is_unwritten(&mut file),
            "a hole the length of the block reads as zeros"
        );

        let (region, _) = open_region(&mut file, &path);
        drop(region);
        // The file's length, and not the region's kind: a filesystem that
        // allocates late or compresses is free to answer the block count
        // either way for a file of zeros, and that count is what stopped
        // being asked. What every way out leaves behind is a file that
        // measures the block — one the pre-allocation filled, one it put
        // back the way it found it, and one it filled before the mapping
        // declined it are all the same length.
        assert_eq!(
            file.metadata().unwrap().len(),
            BUFFER_BLOCK_LENGTH as u64,
            "the pre-allocation wrote the block and left the file at the block"
        );
    }

    /// The way round that costs records: a file that measures the block *and*
    /// holds a crashed run's undrained ones. That is exactly the file a block
    /// count calls sparse on a filesystem that allocates late or compresses,
    /// and pre-allocating over it would write a block of zeros over the only
    /// copy of those records there is — the "begin of mmap" recovery of the
    /// next open would find nothing to recover. What decides is what is in
    /// the file, and not what the disk says it gave it.
    #[test]
    fn a_cache_file_that_holds_records_is_not_written_over() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Mars.mmap3");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        file.set_len(BUFFER_BLOCK_LENGTH as u64).unwrap();
        // One byte of a record: `st_blocks` cannot tell this file from a hole
        // of the same length, and the length cannot either.
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(&[1u8]).unwrap();
        file.flush().unwrap();
        assert!(!is_unwritten(&mut file), "the file holds a record");

        let (region, _use_mmap) = open_region(&mut file, &path);
        drop(region);

        let mut first = [0u8; 1];
        file.seek(SeekFrom::Start(0)).unwrap();
        file.read_exact(&mut first).unwrap();
        assert_eq!(
            first,
            [1u8],
            "the record the appender has not drained yet is still in the file"
        );
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

    /// A block a previous process left in the cache cannot be appended to: the
    /// compressor that produced it died with that process, and a second
    /// compressed stream under the one header the block has is a block no
    /// decoder reads. So when such a block cannot be written either, it gives the
    /// region up and the next record starts a block of its own.
    #[test]
    fn a_recovered_block_that_could_not_be_written_leaves_the_region() {
        let tmp = tempfile::tempdir().unwrap();
        let _mmap_path = records_in_cache(tmp.path());

        // Today's log file is a directory, so the block `open` recovers cannot
        // reach it — what a full or read-only file system looks like from here.
        let log_file = today_name(tmp.path());
        fs::create_dir(&log_file).unwrap();

        let appender = Appender::open(config(tmp.path(), AppenderMode::Async), 0, 0).unwrap();

        // The log file can be written again: what this process logs now has to
        // come out as a block of its own.
        fs::remove_dir(&log_file).unwrap();
        appender.write(Some(&info(LogLevel::Info)), "a record of this process");
        appender.flush_sync();
        appender.close();

        let text = decoded_text(&fs::read(&log_file).unwrap());
        assert!(
            !text.contains("cached in mmap"),
            "the block that could not be written is still in the region: {text}"
        );
        assert!(
            text.contains("a record of this process"),
            "the record joined a block it cannot be read from: {text}"
        );
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
        // The handle the drain reads the records through, which is the one a
        // real recovery claims the slot with.
        let mut mmap = fs::File::open(&mmap_path).unwrap();
        let action = appender.treat_mapping_as_file_and_flush(&mmap_path, &mut mmap);
        appender.close();

        assert_eq!(action, FileIoAction::WriteFailed);
        assert!(mmap_path.exists(), "the cache is the only copy left");
    }

    /// The lock the sections are taken under is the log's: writers that name
    /// one `logdir` and prefix append to one `.xlog` wherever each of them
    /// keeps its cache, so a lock named after the cache directory would be a
    /// different file for every one of them and would exclude nobody.
    #[test]
    fn the_shared_log_lock_comes_from_the_log_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("log");
        let mut with_cache = config(&log, AppenderMode::Sync);
        with_cache.cachedir = Some(tmp.path().join("cache"));
        let without_cache = config(&log, AppenderMode::Sync);

        assert_eq!(output_lock_path(&with_cache), log.join("Mars.lock"));
        assert_eq!(
            output_lock_path(&without_cache),
            output_lock_path(&with_cache),
            "one cache dir and none must not mean two locks"
        );
    }

    /// Recovery writes into a log file a live writer may be appending to, so it
    /// has to wait for the same lock: this holds the lock the way a live
    /// writer's flush section does and watches recovery wait for it, then lets
    /// it through and checks the records arrived.
    #[test]
    fn one_shot_recovery_waits_for_the_log_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("log");
        let cache = tmp.path().join("cache");
        let mut cfg = config(&log, AppenderMode::Sync);
        cfg.cachedir = Some(cache.clone());

        // The log's lock, held here — which is where a live writer is while it
        // moves a cache file into the log.
        fs::create_dir_all(&log).unwrap();
        let lock_path = output_lock_path(&cfg);
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        assert!(sys::lock_exclusive(&held));

        // A dead writer's slot: one record in it, and a lock nobody holds.
        fs::create_dir_all(&cache).unwrap();
        let dead_slot = cache_slot_path(&cache, &cfg.nameprefix, 1);
        let mut region = vec![0u8; BUFFER_BLOCK_LENGTH];
        let mut buffer = LogBuffer::new(true, Some(""), CompressMode::Zlib, 6);
        buffer.attach(&mut region);
        assert!(buffer.write(&mut region, b"recovered while the log is locked"));
        fs::write(&dead_slot, &region).unwrap();

        // Recovery runs on another thread: it is the only way to see it wait.
        let marker = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&marker);
        let recovery = {
            let cfg = cfg.clone();
            thread::spawn(move || {
                let action = crate::appender_oneshot_flush(&cfg);
                done.store(true, Ordering::Release);
                action
            })
        };

        thread::sleep(Duration::from_millis(200));
        assert!(
            !marker.load(Ordering::Acquire),
            "recovery appended to the log while another writer held its lock"
        );
        assert!(dead_slot.exists(), "the slot must not be drained yet");

        // Hand the lock over: recovery may now finish.
        assert!(sys::unlock(&held));
        drop(held);

        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while !marker.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        let action = recovery.join().expect("recovery panicked");
        assert_eq!(action, FileIoAction::Success);
        assert!(!dead_slot.exists(), "the recovered slot must be gone");
        let text = decoded_text(&fs::read(today_name(&log)).unwrap());
        assert!(text.contains("recovered while the log is locked"), "{text}");
    }

    /// Two writers of one prefix in one directory: what two processes — or two
    /// copies of this crate linked into one, which is the case `flock` per open
    /// file description covers and a process-wide singleton cannot — look like.
    ///
    /// Each must get a cache file of its own, and every record of both must
    /// reach the log.
    #[test]
    fn two_writers_of_one_prefix_claim_two_cache_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // The claim is built on advisory locking; where it excludes nobody the
        // C++ behaviour (one shared cache file) is all there is.
        if !sys::lock_excludes(dir) {
            return;
        }

        let first = Appender::open(config(dir, AppenderMode::Sync), 0, 0).unwrap();
        let second = Appender::open(config(dir, AppenderMode::Sync), 0, 0).unwrap();
        let first_path = first.claimed_cache_path().expect("no slot for the first");
        let second_path = second.claimed_cache_path().expect("no slot for the second");
        assert_ne!(first_path, second_path, "both mmapped the same cache file");
        assert!(first_path.exists() && second_path.exists());

        for i in 0..64 {
            first.write(Some(&info(LogLevel::Info)), &format!("first {i:03}"));
            second.write(Some(&info(LogLevel::Info)), &format!("second {i:03}"));
        }
        first.close();
        second.close();

        let text = decoded_text(&fs::read(today_name(dir)).unwrap());
        for i in 0..64 {
            assert!(text.contains(&format!("first {i:03}")), "{text}");
            assert!(text.contains(&format!("second {i:03}")), "{text}");
        }
    }

    /// Past the last slot a writer still logs: it just has nowhere to keep the
    /// records a crash would have left in a cache file.
    #[test]
    fn every_slot_taken_leaves_the_next_writer_without_a_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        if !sys::lock_excludes(dir) {
            return;
        }

        let holders: Vec<Appender> = (0..MAX_CACHE_SLOTS)
            .map(|_| Appender::open(config(dir, AppenderMode::Sync), 0, 0).unwrap())
            .collect();
        let claimed: HashSet<PathBuf> = holders
            .iter()
            .map(|a| a.claimed_cache_path().expect("a slot each"))
            .collect();
        assert_eq!(claimed.len(), MAX_CACHE_SLOTS, "{claimed:?}");

        let next = Appender::open(config(dir, AppenderMode::Sync), 0, 0).unwrap();
        assert_eq!(next.claimed_cache_path(), None);
        next.write(Some(&info(LogLevel::Info)), "no cache of my own");
        next.close();

        let text = decoded_text(&fs::read(today_name(dir)).unwrap());
        assert!(text.contains("no cache of my own"), "{text}");
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

    /// A sink of the app's own is not handed a record the sink itself wrote.
    ///
    /// An adapter that routes everything back through xlog is the ordinary
    /// shape of one, and the path it takes is short: `Appender::write` echoes
    /// a record to the console *before* it puts its own recursion counter up,
    /// so a sink that logs is running again before there is anything to stop
    /// it, and the two of them call one another until the stack goes. What a
    /// record from inside the sink comes to now is the built-in line, and
    /// then the one recursive-call diagnostic.
    #[test]
    fn a_sink_that_logs_from_inside_itself_is_not_asked_again() {
        let _guard = crate::test_lock::serial();
        let tmp = tempfile::tempdir().unwrap();
        // An appender of this test's own, and not the process-wide one. The
        // sink is one process-wide static, so what any test of this binary
        // echoes to a console reaches it: a record another test is writing
        // through the default appender is counted here as well, and the
        // count this test asserts is then not the count of its own records.
        // Only this test knows the id of this instance, so only this test
        // puts a record in front of the sink.
        static ID: AtomicU64 = AtomicU64::new(0);
        let id = crate::appender_open_instance(config(tmp.path(), AppenderMode::Sync)).unwrap();
        ID.store(id, Ordering::Relaxed);
        crate::appender_set_console_log_instance(id, true);

        struct NoSink;
        impl Drop for NoSink {
            fn drop(&mut self) {
                crate::set_console_fun(None);
                crate::appender_close_instance(ID.load(Ordering::Relaxed));
            }
        }
        let _no_sink = NoSink;

        static ASKED: AtomicUsize = AtomicUsize::new(0);
        fn log_from_inside(_info: &XLoggerInfo, log: &str) {
            // The sink is installed process-wide, and not every record that
            // reaches it was written through an open console switch: a
            // diagnostic of the appender's own — an `open file error` of a
            // file another test pointed at a directory — is put to the
            // console whatever the switch says. Only the two records this
            // test writes are what it counts.
            let ours = matches!(log, "from the app" | "from the sink");
            if !ours {
                return;
            }
            ASKED.fetch_add(1, Ordering::Relaxed);
            if log == "from the app" {
                crate::appender_write_instance(
                    ID.load(Ordering::Relaxed),
                    Some(&info(LogLevel::Info)),
                    "from the sink",
                );
            }
        }
        crate::set_console_fun(Some(log_from_inside));

        crate::appender_write_instance(
            ID.load(Ordering::Relaxed),
            Some(&info(LogLevel::Info)),
            "from the app",
        );

        // Asked once and not twice: the record the sink wrote reached the
        // console on its way through and was not handed back to the sink.
        assert_eq!(ASKED.load(Ordering::Relaxed), 1);
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
