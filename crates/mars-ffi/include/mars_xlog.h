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
 * mars_xlog.h — C ABI of the Rust xlog implementation (`mars-ffi`).
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
 * Threading: every symbol may be called from any thread. `mars_xlog_open`,
 * `mars_xlog_close` and the setters mutate process-wide state (exactly like the
 * C++ globals) and should be called from one place during start-up / shut-down.
 *
 * Panics: no Rust panic ever crosses this boundary. Every entry point is
 * wrapped in `catch_unwind`; a panic is reported as `MARS_XLOG_ERR_PANIC` (or
 * silently swallowed for the `void` symbols).
 */

#ifndef MARS_XLOG_H_
#define MARS_XLOG_H_

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

/* --- lifecycle ---------------------------------------------------------- */

/**
 * Replaces `mars::xlog::appender_open(const XLogConfig&)`.
 *
 * @return MARS_XLOG_OK (0) on success, or a negative MARS_XLOG_ERR_* code.
 */
int mars_xlog_open(const MarsXLogConfig* config);

/**
 * Replaces `mars::xlog::XloggerWrite(...)` / `appender_write`. The record is
 * dropped (cheaply, before any formatting) when `level` is below the level set
 * by `mars_xlog_set_level`.
 *
 * `tag`, `filename`, `func_name` and `message` may be NULL; NULL and invalid
 * UTF-8 are treated as an empty string.
 */
void mars_xlog_write(int level,
                     const char* tag,
                     const char* filename,
                     const char* func_name,
                     int line,
                     const char* message);

/** Replaces `mars::xlog::appender_flush()` — signals the writer thread. */
void mars_xlog_flush(void);

/** Replaces `mars::xlog::appender_flush_sync()` — flushes and waits. */
void mars_xlog_flush_sync(void);

/** Replaces `mars::xlog::appender_close()`. */
void mars_xlog_close(void);

/** Replaces `xlogger_SetLevel()`; `level` < 0 clamps to Verbose. */
void mars_xlog_set_level(int level);

/** Replaces `appender_set_console_log(bool)`; `open` != 0 means on. */
void mars_xlog_set_console_log(int open);

/** Replaces `appender_set_max_file_size(uint64_t)`; 0 means "do not split". */
void mars_xlog_set_max_file_size(unsigned long long bytes);

/** Replaces `appender_set_max_alive_duration(long)`; negative clamps to 0. */
void mars_xlog_set_max_alive_duration(long long seconds);

/**
 * Replaces `appender_get_current_log_path(char*, unsigned int)`.
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
 * cache file. Handle 0 means "the process-wide appender opened by
 * mars_xlog_open".
 */

/* Returns the instance handle, or 0 on a null/invalid config. */
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

/* SetLevel for an instance (0 = the default logger). */
void mars_xlog_set_level_instance(long long instance, int level);

/* appender_setmode: switches the process-wide appender between async/sync. */
void mars_xlog_set_mode(int mode);

/* SetAppenderMode for an instance. */
void mars_xlog_set_mode_instance(long long instance, int mode);

/* Drains an instance; sync != 0 waits for the write to complete. */
void mars_xlog_flush_instance(long long instance, int sync);

/* FlushAll: drains the process-wide appender *and* every instance. */
void mars_xlog_flush_all(int sync);

/* SetConsoleLogOpen for an instance (0 = the default logger). */
void mars_xlog_set_console_log_instance(long long instance, int open);

/* SetMaxFileSize for an instance; 0 means "do not split". */
void mars_xlog_set_max_file_size_instance(long long instance, unsigned long long bytes);

/* SetMaxAliveTime for an instance; negative clamps to 0. */
void mars_xlog_set_max_alive_duration_instance(long long instance, long long seconds);

/**
 * Replaces `mars::xlog::appender_oneshot_flush()`: drains an `<prefix>.mmap3`
 * another process left behind, without opening an appender. It refuses to run
 * for a directory an appender of this process already owns, and then answers
 * MARS_XLOG_ACTION_UNNECESSARY.
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

/* The cache directory, or a negative MARS_XLOG_ERR_* code. */
int mars_xlog_current_log_cache_path(char* out, unsigned int len);


#endif /* MARS_XLOG_H_ */
