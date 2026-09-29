/*
 * The C demo of mars-rs: one program that writes a `.xlog` through the C ABI.
 *
 * It is the C twin of `demo/rust` — the same six records, the same order, the
 * same file at the end — so that a reader who knows one of the two can read the
 * other by the shape of it. What it does, in the order an app does it:
 *
 *   1. fill in a `MarsXLogConfig` and open an instance —
 *      `mars_xlog_new_instance`;
 *   2. write one record at every level — `mars_xlog_write_instance`;
 *   3. drain it to disk — `mars_xlog_flush_now_instance`;
 *   4. ask where the file went, and close — `mars_xlog_release_instance`.
 *
 * Build it with `make` and run it with `make run`; the Makefile builds the
 * `libmars_ffi.a` of this checkout first.
 *
 * There is no reader in C. The decoder is Rust's to write — `decode_log_file`
 * of `marsrs-xlog`, or the CLI over it:
 *
 *     xlog decode log/marsrs_20260929.xlog --out=plain.txt
 *
 * which is the same command an app would run on a file a phone uploaded.
 *
 * Two things this program deliberately does *not* show:
 *
 * - **An appender per component.** `mars_xlog_new_instance` opens a second one
 *   with a directory, a prefix and a level of its own; `mars_xlog_write_instance`
 *   writes through the handle it answers. One appender is what an app that only
 *   logs needs, so one is what this writes.
 * - **Encryption.** `pub_key` is the 128 hex characters of a public key, and an
 *   appender opened with one writes records only the matching private key
 *   decrypts — `xlog keygen` makes the pair.
 */

#include <mars_xlog.h>

#include <stdio.h>
#include <string.h>

/* The prefix every file of this demo is named after: a file is
 * `<prefix>_<YYYYMMDD>.xlog`, so the prefix is what an app uses to tell its own
 * files from another component's in a shared directory. */
static const char *PREFIX = "marsrs";
static const char *LOGDIR = "log";

int main(void) {
    /* -- 1. open -----------------------------------------------------------
     *
     * Every default of `MarsXLogConfig` is a zero — async mode, zlib, no cache
     * directory, no expiry — so zeroing the struct is the whole of "the
     * defaults", and what follows names only what this demo chooses. */
    MarsXLogConfig config;
    memset(&config, 0, sizeof config);
    config.mode = MarsAppenderAsync;
    config.log_dir = LOGDIR;
    config.name_prefix = PREFIX;
    config.compress_mode = MarsCompressZlib;

    /* The appender this program writes through, and the one call with a
     * return value to answer: `mars_xlog_new_instance` opens an instance —
     * a log directory, a prefix, a key and a cache file of its own, which is
     * what `Xlog::open` gives Rust and `Xlog.open(config)` gives Kotlin —
     * and hands back the handle it is known by. The level is the second
     * argument and not a field of the config, because the level belongs to
     * the logger and not to the file.
     *
     * Not handle `0`: that one names the *process-wide* appender, which is
     * the plumbing the JNI bridge installs from Rust and which no symbol of
     * this ABI opens, and not the thing an app holds. What this call answers
     * when it refuses a config is a negative `MARS_XLOG_ERR_*` code, one per
     * cause, so `0` is not the number that means "no instance" — anything
     * that is not a positive handle is.
     *
     * No Rust panic crosses this boundary either: every entry point is
     * wrapped in `catch_unwind`, and a panic becomes `MARS_XLOG_ERR_PANIC`
     * here and a no-op there. */
    long long xlog = mars_xlog_new_instance(&config, MarsLevelVerbose);
    if (xlog <= 0) {
        fprintf(stderr, "mars_xlog_new_instance failed for '%s' in %s\n", PREFIX, LOGDIR);
        return 1;
    }

    /* Mirror every record to stderr as well: off in an app that ships, on here
     * so a run shows what went into the file. */
    mars_xlog_set_console_log_instance(xlog, 1);
    /* Close a file at 8 MiB, drop one at ten days. Both are 0 by default, which
     * is not the same 0 twice: a maximum size of 0 never splits a file, and a
     * lifetime of 0 is the C++'s own ten days. */
    mars_xlog_set_max_file_size_instance(xlog, 8ULL * 1024 * 1024);
    mars_xlog_set_max_alive_duration_instance(xlog, 10LL * 24 * 3600);

    /* -- 2. write ---------------------------------------------------------- */
    struct {
        int level;
        const char *tag;
        const char *message;
    } records[] = {
        {MarsLevelVerbose, "trace", "the finest record there is"},
        {MarsLevelDebug, "net", "resolved 3 addresses for example.com"},
        {MarsLevelInfo, "startup", "cold start in 412 ms"},
        {MarsLevelWarn, "net", "retrying after 1204 ms"},
        {MarsLevelError, "login", "login failed: token expired"},
        {MarsLevelFatal, "login", "giving up after 3 attempts"},
    };
    const size_t count = sizeof records / sizeof records[0];

    for (size_t i = 0; i < count; ++i) {
        /* `__LINE__` is the line this call sits on, so every record carries the
         * same one — a real caller writes from inside the macro it wraps the
         * ABI in, where `__LINE__` expands at the caller's line instead. */
        mars_xlog_write_instance(xlog, records[i].level, records[i].tag, __FILE__, __func__,
                                 __LINE__, records[i].message);
        printf("wrote %d [%s] %s\n", records[i].level, records[i].tag, records[i].message);
    }

    /* A record that is expensive to build is worth asking about first: the
     * level gate is the only thing standing between a dropped record and the
     * work its caller did to build it. */
    if (mars_xlog_is_enabled_for(xlog, MarsLevelDebug)) {
        char summary[64];
        snprintf(summary, sizeof summary, "%zu records written", count);
        mars_xlog_write_instance(xlog, MarsLevelDebug, "trace", __FILE__, __func__, __LINE__,
                                 summary);
    }

    /* -- 3. flush ----------------------------------------------------------
     *
     * `mars_xlog_request_flush_instance` asks the writer thread for the drain
     * and returns at once, and nothing ever answers when it is over. This one
     * drains on the calling thread, so every record above is on disk before the
     * next line runs. An app calls it before it reads the files, uploads them,
     * or exits. */
    mars_xlog_flush_now_instance(xlog);

    /* -- 4. where it went, and close ---------------------------------------
     *
     * A directory and not a file, because that is what the C++ function this
     * ports answers — `XloggerAppender::GetCurrentLogPath` hands back
     * `sg_logdir`. */
    char dir[1024];
    if (mars_xlog_current_log_path_instance(xlog, dir, sizeof dir) > 0) {
        printf("writing into:  %s\n", dir);
    }

    /* Today's file, asked of the port rather than rebuilt from the date. The
     * `0` before `path` is the index: a day split for size has a second file
     * beside the first, and the call answers `MARS_XLOG_ERR_NO_PATH` once the
     * caller has walked past the last one. */
    char path[1024];
    if (mars_xlog_getfilepath_from_timespan(0, PREFIX, LOGDIR, 0, path, sizeof path) > 0) {
        printf("log file:      %s\n", path);
    }

    /* Drains what is left and closes the appender. An app that skips it loses
     * whatever the writer thread still held, which with an async appender is the
     * last records it wrote. It takes the prefix and not the handle, because a
     * prefix is one appender: two modules that opened the same one share it. */
    mars_xlog_release_instance(PREFIX);

    return 0;
}
