package io.github.orangeboychen.marsrs.xlog

/**
 * The Kotlin face of xlog, on every platform of a Kotlin Multiplatform project:
 * the same calls in `commonMain`, whether the module is linked into an Android
 * app over the JNI bridge of `crates/mars-jni` or into an iOS, watchOS, tvOS,
 * macOS, Linux or Windows one over the C ABI of `crates/mars-ffi`.
 *
 * The surface is the intersection of the two bridges, which is what a `common`
 * declaration can only be: everything here is something both of them answer.
 * `mars_xlog_current_log_path` and the named instances of `mars_xlog.h` are not
 * in it, because the JNI bridge exports neither — a caller who wants them is a
 * caller of one platform, and the C ABI archive of the release is theirs to
 * link.
 *
 * An app that wants the C++ project's facade rather than the appender writes
 * [Log].
 */
public expect object Xlog {
    /**
     * Opens the process-wide appender: `appender_open` of the C++ project,
     * `mars_xlog_open` of the C ABI.
     *
     * Calling it twice closes the first appender, as it does in the C++, so an
     * app opens once — at start-up, from one place — and [close]s on the way
     * out.
     */
    public fun open(config: XlogConfig)

    /**
     * Writes one record: `XloggerWrite` of the C++ project, `mars_xlog_write`
     * of the C ABI.
     *
     * A record below the level the appender was opened at is dropped before it
     * is formatted, which is why a call here costs nothing a log call should
     * not. [file], [function] and [line] are what the C++ takes from its macros
     * and what a Kotlin caller has to name itself; all three are empty unless
     * they are given.
     */
    public fun write(
        level: LogLevel,
        tag: String,
        message: String,
        file: String = "",
        function: String = "",
        line: Int = 0
    )

    /**
     * Drains the appender: `appender_flush`, or `appender_flush_sync` when
     * [sync] — which waits for the write instead of only signalling the writer
     * thread, and is the one to call before a process goes away.
     */
    public fun flush(sync: Boolean)

    /**
     * Closes the appender and flushes what is left: `appender_close`.
     */
    public fun close()

    /**
     * The level the appender drops records below: `xlogger_SetLevel`.
     */
    public fun setLevel(level: LogLevel)

    /**
     * Whether a record is written to the console as well: `appender_set_console_log`.
     */
    public fun setConsoleLog(open: Boolean)

    /**
     * The size a log file is split at: `appender_set_max_file_size`. `0` never
     * splits, which is the C++'s own default.
     */
    public fun setMaxFileSize(bytes: Long)

    /**
     * How long a log file is kept: `appender_set_max_alive_duration`. A
     * negative value is clamped to `0`.
     */
    public fun setMaxAliveTime(seconds: Long)
}
