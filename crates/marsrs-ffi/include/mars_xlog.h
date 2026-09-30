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
 * here does is install a process-wide appender: an app opens an instance with
 * `mars_xlog_new_instance` and writes through the handle it answers. Handle
 * `0` names no appender at all — it is what an open that failed answers, and
 * every symbol asked of it is a no-op.
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

/* --- return codes -------------------------------------------------------
 *
 * The five from `NULL_CONFIG` to `APPENDER` are the reasons an open may be
 * refused, and no symbol hands them out: `mars_xlog_new_instance` answers the
 * handle `0` for every one of them, there being no room in a handle for a
 * negative code. They are here for the caller that wants to name the reason in
 * its own diagnostic, not to switch on an answer.
 */
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

/* ---- logger instances (mars::xlog::NewXloggerInstance and friends) ----
 *
 * Each instance owns an appender: its own log directory, prefix, key, mode and
 * cache file. A C caller that wants a logger opens one with
 * mars_xlog_new_instance and holds the handle it answers.
 *
 * Every call that can be asked of an instance exists only in this spelling:
 * the C++ has a free function for its process-wide appender and a
 * handle-taking one beside it, and a second spelling of the same operation
 * here would be two ways to say one thing. There is no process-wide appender
 * to install, so there is no free-function spelling either — and handle `0`,
 * which is what a failed open answers, names no appender: the level, the mode,
 * the console, the two sizes and the drain asked of it do nothing, and the
 * questions answer "no" or `MARS_XLOG_ERR_NO_PATH`.
 */

/* --- writing ------------------------------------------------------------- */

/* Returns the instance handle, or 0 on a null/invalid config. */
long long mars_xlog_new_instance(const MarsXLogConfig* config, int level);

/* The handle registered for name_prefix, or 0 when there is none. */
long long mars_xlog_get_instance(const char* name_prefix);

/* Releases the instance and closes its appender. */
void mars_xlog_release_instance(const char* name_prefix);

/* The same, and only while name_prefix is still registered under `instance`.
 *
 * Releasing takes the prefix and not the handle, so asking
 * mars_xlog_get_instance and then calling mars_xlog_release_instance is two
 * answers and not one: an open of the same prefix that lands between the two is
 * handed a handle of its own, and the release closes that one. This is the
 * question and the release in one call, which is also what makes a second
 * close of a prefix a no-op however many callers hold its handle. */
void mars_xlog_release_instance_of(const char* name_prefix, long long instance);

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

/* SetLevel for an instance: the level `mars_xlog_get_level` and
 * `mars_xlog_is_enabled_for` answer, and the one a write through the same
 * handle asks. A no-op for handle `0`, which is no instance at all. */
void mars_xlog_set_level_instance(long long instance, int level);

/* appender_setmode / SetAppenderMode for an instance. */
void mars_xlog_set_mode_instance(long long instance, int mode);

/* Drains an instance, requested: the writer thread is told it may drain, and
 * this returns at once. Nothing is in the file because this returned, and
 * nothing answers when the drain is over. The C++'s name for this call is
 * `appender_flush`, and "flush" there said which mode the appender was opened
 * in and not what the call does to the thread it was called on: the port names
 * the drain by what the caller gets back, and what this one gives is no answer
 * at all — hence "request", which is the one thing the two drains that do
 * answer, `mars_xlog_flush_now_instance` and the awaited `flush()` every
 * platform carries, do give. */
void mars_xlog_request_flush_instance(long long instance);

/* Drains an instance on the calling thread, which is what
 * `appender_flush_sync` is: its records are on the disk when this returns, and
 * that is a promise the requested call above does not make. */
void mars_xlog_flush_now_instance(long long instance);

/* SetConsoleLogOpen for an instance. */
void mars_xlog_set_console_log_instance(long long instance, int open);

/* SetMaxFileSize for an instance; 0 means "do not split". */
void mars_xlog_set_max_file_size_instance(long long instance, unsigned long long bytes);

/* SetMaxAliveTime for an instance; negative clamps to 0. */
void mars_xlog_set_max_alive_duration_instance(long long instance, long long seconds);

/**
 * Replaces `mars::xlog::appender_make_logfile_name()`: the log file *name* for
 * the day `timespan` days ago (0 = today), whether or not it exists yet.
 *
 * The C++ fills a `std::vector`; a C caller walks the same list with `index`,
 * starting at 0 and stopping at MARS_XLOG_ERR_NO_PATH. `instance` is a handle
 * from `mars_xlog_new_instance`; a handle no appender is open for answers
 * MARS_XLOG_ERR_NO_PATH from the first index.
 *
 * @return the number of bytes written excluding the terminating NUL, or a
 *         negative MARS_XLOG_ERR_* code.
 */
int mars_xlog_make_logfile_name_instance(long long instance,
                                         int timespan,
                                         unsigned int index,
                                         char* out,
                                         unsigned int len);

/**
 * Replaces `mars::xlog::appender_getfilepath_from_timespan()`: the log files
 * that *exist* for the day `timespan` days ago. Same `index` protocol as
 * mars_xlog_make_logfile_name_instance.
 */
int mars_xlog_getfilepath_from_timespan_instance(long long instance,
                                                 int timespan,
                                                 unsigned int index,
                                                 char* out,
                                                 unsigned int len);

/* The directory the appender of `instance` is writing its files to, or a negative
 * MARS_XLOG_ERR_* code. A directory and not a file, which is what the C++'s
 * `XloggerAppender::GetCurrentLogPath` answers — `sg_logdir`. A handle no
 * appender is open for — `0` among them, since no symbol of this ABI installs
 * a process-wide one — answers MARS_XLOG_ERR_NO_PATH. */
int mars_xlog_current_log_path_instance(long long instance, char* out, unsigned int len);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MARS_XLOG_H_ */
