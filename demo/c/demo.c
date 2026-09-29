/*
 * The C demo of mars-rs: one program that writes a `.xlog` through the C ABI.
 *
 * It is the C twin of `demo/rust` — the same six records, the same order, the
 * same file at the end — so that a reader who knows one of the two can read the
 * other by the shape of it. What it does, in the order an app does it:
 *
 *   1. fill in a `MarsXLogConfig` and open the appender — `mars_xlog_open`;
 *   2. write one record at every level — `mars_xlog_write`;
 *   3. drain it to disk — `mars_xlog_flush_sync`;
 *   4. ask where the file went, and close — `mars_xlog_close`.
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

    /* The one call with a return value to answer. Every other entry point is
     * `void`: there is nothing left for them to refuse, and no Rust panic ever
     * crosses this boundary either — each one is wrapped in `catch_unwind`, and
     * a panic becomes `MARS_XLOG_ERR_PANIC` here and a no-op there. */
    int rc = mars_xlog_open(&config);
    if (rc != MARS_XLOG_OK) {
        fprintf(stderr, "mars_xlog_open failed: %d\n", rc);
        return 1;
    }

    /* The level a record has to reach to be written at all. Verbose, so that
     * all six records below survive; an app in the field sets `MarsLevelInfo`.
     *
     * The level is not part of the config — there is no level field in it —
     * because the level belongs to the logger and not to the file. */
    mars_xlog_set_level(MarsLevelVerbose);
    /* Mirror every record to stderr as well: off in an app that ships, on here
     * so a run shows what went into the file. */
    mars_xlog_set_console_log(1);
    /* Close a file at 8 MiB, drop one at ten days. Both are 0 by default, which
     * is not the same 0 twice: a maximum size of 0 never splits a file, and a
     * lifetime of 0 is the C++'s own ten days. */
    mars_xlog_set_max_file_size(8ULL * 1024 * 1024);
    mars_xlog_set_max_alive_duration(10LL * 24 * 3600);

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
        mars_xlog_write(records[i].level, records[i].tag, __FILE__, __func__, __LINE__,
                        records[i].message);
        printf("wrote %d [%s] %s\n", records[i].level, records[i].tag, records[i].message);
    }

    /* A record that is expensive to build is worth asking about first: the
     * level gate is the only thing standing between a dropped record and the
     * work its caller did to build it. Handle 0 is the process-wide appender
     * `mars_xlog_open` opened. */
    if (mars_xlog_is_enabled_for(0, MarsLevelDebug)) {
        char summary[64];
        snprintf(summary, sizeof summary, "%zu records written", count);
        mars_xlog_write(MarsLevelDebug, "trace", __FILE__, __func__, __LINE__, summary);
    }

    /* -- 3. flush ----------------------------------------------------------
     *
     * `mars_xlog_flush` only signals the writer thread and returns; this one
     * waits, so every record above is on disk before the next line runs. An app
     * calls it before it reads the files, uploads them, or exits. */
    mars_xlog_flush_sync();

    /* -- 4. where it went, and close ---------------------------------------
     *
     * A directory and not a file, because that is what the C++ function this
     * ports answers — `XloggerAppender::GetCurrentLogPath` hands back
     * `sg_logdir`. */
    char dir[1024];
    if (mars_xlog_current_log_path(dir, sizeof dir) > 0) {
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
     * last records it wrote. */
    mars_xlog_close();

    return 0;
}
