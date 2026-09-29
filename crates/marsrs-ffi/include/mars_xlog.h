/* Tencent is pleased to support the open source community by making Mars available.
 * Copyright (C) 2016 THL A29 Limited, a Tencent company. All rights reserved.
 *
 * Licensed under the MIT License (the "License"); you may not use this file except in
 * compliance with the License. You may obtain a copy of the License at
 * http://opensource.org/licenses/MIT
 *
 * Unless required by applicable law or agreed to in writing, software distributed under the License is
 * distributed on an "AS IS" basis, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND,
 * either express or implied. See the License for the specific language governing permissions and
 * limitations under the License.
 */

/*
 * mars_xlog.h — C ABI of the Rust xlog implementation (`marsrs-ffi`).
 *
 * This header is the drop-in seam for the existing C++ / JNI / ObjC layers of
 * Mars. It mirrors the C++ surface of
 *
 *     mars/xlog/appender.h            (mars::xlog::appender_open / flush / close ...)
 *     mars/xlog/xlogger_interface.h   (mars::xlog::XloggerWrite / SetLevel ...)
 *     mars/comm/xlogger/xloggerbase.h (TLogLevel)
 *
 * and is the C equivalent of the JNI bridge in mars/xlog/jni/Java2C_Xlog.cc.
 *
 * The header is hand-written and checked in next to the crate so that a C/C++
 * caller can include it without running cbindgen. It is kept in sync with
 * `src/abi.rs` by `tests/header_sync.rs`.
 *
 * Threading: every symbol may be called from any thread. The setters and the
 * instance lifecycle mutate shared state (exactly like the C++ globals) and
 * should be called from one place during start-up / shut-down. What no symbol
 * here does is install the process-wide appender: an app opens an instance
 * with `mars_xlog_new_instance`, and the process-wide one is the plumbing the
 * JNI bridge sets up from Rust.
 *
 * Panics: no Rust panic ever crosses this boundary. Every entry point is
 * wrapped in `catch_unwind`; a panic is reported as `MARS_XLOG_ERR_PANIC` (or
 * silently swallowed for the `void` symbols).
 */

#ifndef MARS_XLOG_H_
#define MARS_XLOG_H_

/* The symbols this header declares are the ones `#[no_mangle]` exports: plain
 * C names, and a C++ translation unit that includes this and calls
 * `mars_xlog_new_instance()` asks its linker for a mangled one that does not
 * exist.
 * `mars_stn.h` and `mars_sdt.h` carry the same guard, and `mars_xlog.hpp` —
 * the C++ face of this seam — includes this one, so a C++ caller asks for the
 * same names whichever of the two it takes. */
#ifdef __cplusplus
extern "C" {
#endif

/* --- enums ------------------------------------------------------------- */

/** Mirrors `mars::xlog::TAppenderMode` (appender.h). */
typedef enum { MarsAppenderAsync = 0, MarsAppenderSync = 1 } MarsAppenderMode;

/** Mirrors `mars::xlog::TCompressMode` (appender.h). */
typedef enum { MarsCompressZlib = 0, MarsCompressZstd = 1 } MarsCompressMode;

/** Mirrors `TLogLevel` (xloggerbase.h); `kLevelAll == kLevelVerbose == 0`. */
typedef enum {
    MarsLevelVerbose = 0,
    MarsLevelDebug = 1,
    MarsLevelInfo = 2,
    MarsLevelWarn = 3,
    MarsLevelError = 4,
    MarsLevelFatal = 5
} MarsLogLevel;

/** Additional level understood only by the filter: disables every record. */
#define MARS_LEVEL_NONE 6

/* --- return codes ------------------------------------------------------- */

#define MARS_XLOG_OK 0
#define MARS_XLOG_ERR_NULL_CONFIG (-1)   /* `config` was NULL                     */
#define MARS_XLOG_ERR_BAD_MODE (-2)      /* `mode` outside MarsAppenderMode       */
#define MARS_XLOG_ERR_BAD_COMPRESS (-3)  /* `compress_mode` outside MarsCompressMode */
#define MARS_XLOG_ERR_EMPTY_LOG_DIR (-4) /* `log_dir` was NULL or ""              */
#define MARS_XLOG_ERR_APPENDER (-5)      /* the Rust appender refused the config  */
#define MARS_XLOG_ERR_NULL_OUT (-6)      /* `out` was NULL                        */
#define MARS_XLOG_ERR_NO_SPACE (-7)      /* output buffer too small (0 or < need) */
#define MARS_XLOG_ERR_NO_PATH (-8)       /* no current log file is open           */
#define MARS_XLOG_ERR_PANIC (-99)        /* a Rust panic was caught at the boundary */

/* --- `mars_xlog_oneshot_flush` result: mars::xlog::TFileIOAction --------- */

#define MARS_XLOG_ACTION_NONE 0
#define MARS_XLOG_ACTION_SUCCESS 1
#define MARS_XLOG_ACTION_UNNECESSARY 2  /* there was nothing to flush        */
#define MARS_XLOG_ACTION_OPEN_FAILED 3
#define MARS_XLOG_ACTION_READ_FAILED 4
#define MARS_XLOG_ACTION_WRITE_FAILED 5
#define MARS_XLOG_ACTION_CLOSE_FAILED 6
#define MARS_XLOG_ACTION_REMOVE_FAILED 7

/* --- config ------------------------------------------------------------- */

/**
 * Mirrors `mars::xlog::XLogConfig` (appender.h) with C strings instead of
 * `std::string`. Every pointer must be NUL-terminated UTF-8 or NULL; NULL is
 * treated as "empty" (see the field docs) except for `log_dir`, which is
 * mandatory.
 */
typedef struct {
    int mode;              /* MarsAppenderMode; default when unset: Async   */
    const char* log_dir;   /* mandatory, directory is created if missing     */
    const char* name_prefix; /* used verbatim; no default, as in C++      */
    const char* pub_key;   /* NULL / "" => logs are written unencrypted      */
    int compress_mode;     /* MarsCompressMode; default when unset: Zlib     */
    int compress_level;    /* <= 0 => keep the appender default (6)          */
    const char* cache_dir; /* NULL / "" => the mmap cache lives in log_dir   */
    int cache_days;        /* < 0 => 0; 0 => keep every file                 */
} MarsXLogConfig;

/* --- writing ------------------------------------------------------------- */

/**
 * Replaces `xlogger_Assert(...)` of `mars/comm/xlogger/xloggerbase.h` — the
 * record an assert writes, with the same flattened fields
 * `mars_xlog_write_instance` takes plus the expression that failed.
 *
 * The record is written at `MarsLevelFatal` and its body is
 * `[ASSERT(<expression>)]` followed by `message`. `xloggerbase.h` annotates
 * `xlogger_Assert` "no level filter", so no level an app set is asked here:
 * the gate the C++ has lives in its own `xassert2` macro, and a caller that
 * wants it has `mars_xlog_is_enabled_for`.
 *
 * `tag`, `filename`, `func_name`, `expression` and `message` may be NULL;
 * NULL and invalid UTF-8 are treated as an empty string.
 */
void mars_xlog_assert(const char* tag,
                      const char* filename,
                      const char* func_name,
                      int line,
                      const char* expression,
                      const char* message);

/**
 * Replaces `appender_set_console_fun(TConsoleFun)` of `mars/xlog/appender.h`
 * — where a console record goes instead of the built-in sink, which is stderr
 * on every platform here.
 *
 * The C++ `TConsoleFun` is an Apple-only enum of three sinks of its own
 * (`kConsolePrintf` / `kConsoleNSLog` / `kConsoleOSLog`), and
 * `os_log_with_type` is a macro with no symbol to link against, so the port
 * takes the sink instead: a callback handed the same fields
 * `mars_xlog_write_instance` takes, unformatted, so what a console record looks like is the caller's
 * decision and not the port's.
 *
 * Pass NULL to take the callback away again. The callback may be called from
 * any thread, the writer thread of an async appender included, and it must not
 * unwind: one written in Rust that panics aborts at the `extern "C"`
 * boundary, before the `catch_unwind` every entry point of the port is wrapped
 * in can see the panic at all, so what it ends is the process and not the
 * record. A NULL string is never handed to it.
 */
typedef void (*MarsXLogConsoleFun)(int level,
                                   const char* tag,
                                   const char* filename,
                                   const char* func_name,
                                   int line,
                                   const char* log);

/* The sink is process-wide, as it is in the C++ (`sg_console_fun`): there is no
 * per-instance one to set it on. */
void mars_xlog_set_console_fun(MarsXLogConsoleFun fun);

/**
 * Replaces `appender_get_current_log_path(char*, unsigned int)` — the file the
 * process-wide appender is appending to.
 *
 * Writes the NUL-terminated path of the log file currently being appended to
 * into `out` (which must hold at least `len` bytes).
 *
 * @return the number of bytes written excluding the terminating NUL, or a
 *         negative MARS_XLOG_ERR_* code (`MARS_XLOG_ERR_NULL_OUT`,
 *         `MARS_XLOG_ERR_NO_SPACE`, `MARS_XLOG_ERR_NO_PATH`).
 */
int mars_xlog_current_log_path(char* out, unsigned int len);

/* ---- logger instances (mars::xlog::NewXloggerInstance and friends) ----
 *
 * Each instance owns an appender: its own log directory, prefix, key, mode and
 * cache file. Handle 0 means "the process-wide appender", which no symbol here
 * installs: it is the one the JNI bridge sets up from Rust, and a C caller
 * that wants a logger of its own opens an instance with
 * mars_xlog_new_instance.
 *
 * Every call that can be asked of an instance exists only in this spelling: the
 * C++ has a free function for the process-wide appender and a handle-taking one
 * beside it, and a second spelling of the same operation here would be two ways
 * to say one thing. So the level, the mode, the console, the two sizes and the
 * drain of the process-wide appender are all asked for with a handle of `0`.
 */

/* The handle of the instance, or a negative MARS_XLOG_ERR_* code when no
 * instance could be opened: MARS_XLOG_ERR_NULL_CONFIG for a NULL `config`,
 * MARS_XLOG_ERR_BAD_MODE / MARS_XLOG_ERR_BAD_COMPRESS /
 * MARS_XLOG_ERR_EMPTY_LOG_DIR for one the appender cannot use, and
 * MARS_XLOG_ERR_APPENDER when the open itself failed.
 *
 * `0` is never the answer to a failure: it is the process-wide appender above,
 * so an app that read it as "no instance" and went on would log through handle
 * `0` — a logger it never opened, whose records go wherever the JNI bridge put
 * them — and never learn that its own config was refused. */
long long mars_xlog_new_instance(const MarsXLogConfig* config, int level);

/* The handle registered for name_prefix, or 0 when there is none. */
long long mars_xlog_get_instance(const char* name_prefix);

/* Releases the instance and closes its appender. */
void mars_xlog_release_instance(const char* name_prefix);

/* Writes through an instance; honours the instance's level. */
void mars_xlog_write_instance(long long instance,
                              int level,
                              const char* tag,
                              const char* filename,
                              const char* func_name,
                              int line,
                              const char* log);

/* 1 when the instance would write this level, 0 otherwise. */
int mars_xlog_is_enabled_for(long long instance, int level);

/* The instance's level, or -1 for an unknown handle. */
int mars_xlog_get_level(long long instance);

/* SetLevel for an instance; `0` is the process-wide appender, so this is also
 * `xlogger_SetLevel` — the level `mars_xlog_get_level` and
 * `mars_xlog_is_enabled_for` answer, and the one a write through handle `0`
 * asks. */
void mars_xlog_set_level_instance(long long instance, int level);

/* appender_setmode / SetAppenderMode; `0` is the process-wide appender. */
void mars_xlog_set_mode_instance(long long instance, int mode);

/* Drains an instance (0 = the process-wide appender), requested: the writer
 * thread is told it may drain, and this returns at once. Nothing is in the file
 * because this returned, and nothing answers when the drain is over. The C++'s
 * name for this call is `appender_flush`, and "flush" there said which mode the
 * appender was opened in and not what the call does to the thread it was called
 * on: the port names the drain by what the caller gets back, and what this one
 * gives is no answer at all — hence "request", which is the one thing the two
 * drains that do answer, `mars_xlog_flush_now_instance` and the awaited
 * `flush()` every platform carries, do give. */
void mars_xlog_request_flush_instance(long long instance);

/* Drains an instance on the calling thread, which is what
 * `appender_flush_sync` is: its records are on the disk when this returns, and
 * that is a promise the requested call above does not make. */
void mars_xlog_flush_now_instance(long long instance);

/* FlushAll, requested: every appender is told its writer thread may drain. */
void mars_xlog_request_flush_all(void);

/* FlushAll on the calling thread: drains the process-wide appender *and* every
 * instance — each of which owns an appender of its own, so a caller that leaves
 * them out misses their records. */
void mars_xlog_flush_now_all(void);

/* SetConsoleLogOpen for an instance (0 = the default logger). */
void mars_xlog_set_console_log_instance(long long instance, int open);

/* SetMaxFileSize for an instance; 0 means "do not split". */
void mars_xlog_set_max_file_size_instance(long long instance, unsigned long long bytes);

/* SetMaxAliveTime for an instance; negative clamps to 0. */
void mars_xlog_set_max_alive_duration_instance(long long instance, long long seconds);

/**
 * Replaces `mars::xlog::appender_oneshot_flush()`: drains the
 * `<prefix>[_<n>].mmap3` cache files another process left behind, without
 * opening an appender. The ones a live writer of this process — or of any
 * other — still holds are left alone, so it answers
 * MARS_XLOG_ACTION_UNNECESSARY when there is nothing of a dead process's to
 * recover.
 *
 * @return one of the MARS_XLOG_ACTION_* values (0..7), or a negative
 *         MARS_XLOG_ERR_* code when `config` is unusable.
 */
int mars_xlog_oneshot_flush(const MarsXLogConfig* config);

/**
 * Replaces `mars::xlog::appender_make_logfile_name()`: the log file *name* for
 * the day `timespan` days ago (0 = today), whether or not it exists yet.
 *
 * The C++ fills a `std::vector`; a C caller walks the same list with `index`,
 * starting at 0 and stopping at MARS_XLOG_ERR_NO_PATH. `prefix` and `log_dir`
 * may be NULL (an empty `log_dir` yields no name at all).
 *
 * @return the number of bytes written excluding the terminating NUL, or a
 *         negative MARS_XLOG_ERR_* code.
 */
int mars_xlog_make_logfile_name(int timespan,
                                const char* prefix,
                                const char* log_dir,
                                unsigned int index,
                                char* out,
                                unsigned int len);

/**
 * Replaces `mars::xlog::appender_getfilepath_from_timespan()`: the log files
 * that *exist* for the day `timespan` days ago. Same `index` protocol as
 * mars_xlog_make_logfile_name.
 */
int mars_xlog_getfilepath_from_timespan(int timespan,
                                        const char* prefix,
                                        const char* log_dir,
                                        unsigned int index,
                                        char* out,
                                        unsigned int len);

/* The directory an instance is writing its files to, or a negative
 * MARS_XLOG_ERR_* code. It is the same question mars_xlog_current_log_path
 * asks of the process-wide appender, and the only spelling an app that opened
 * its logger with mars_xlog_new_instance has: no symbol of this ABI installs
 * the process-wide one — the JNI bridge does, from Rust — so the process-wide
 * question answers MARS_XLOG_ERR_NO_PATH for it. */
int mars_xlog_current_log_path_instance(long long instance, char* out, unsigned int len);

/* The cache directory, or a negative MARS_XLOG_ERR_* code. */
int mars_xlog_current_log_cache_path(char* out, unsigned int len);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MARS_XLOG_H_ */
