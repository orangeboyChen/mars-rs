package io.github.orangeboychen.marsrs.xlog

/**
 * The Kotlin face of xlog, on every platform of a Kotlin Multiplatform project:
 * one appender of its own, written through the same calls in `commonMain`,
 * whether the module is linked into an Android app over the JNI bridge of
 * `crates/marsrs-jni` or into an iOS, watchOS, tvOS, macOS, Linux or Windows one
 * over the C ABI of `crates/marsrs-ffi`.
 *
 * It is the same class the Android AAR publishes — `Xlog.open(XlogConfig(...))`,
 * `xlog.i(tag, message)` — because the two of them are one API: what a shared
 * module writes compiles unchanged against `xlog-kmp` and against `xlog`, and
 * the only thing that changes is which bridge answers. The `androidMain` and
 * `nativeMain` actuals are the two halves of the same declaration, and neither
 * of them adds a member the other does not have.
 *
 * An appender is the instance: [close] on this [Xlog] closes what another
 * [Xlog] of the same [namePrefix] writes through, which is why a part of an app
 * whose logs are read apart from the rest is given a prefix of its own.
 */
public expect class Xlog(config: XlogConfig) {
    /** What every file of this appender starts with, and what it is known by. */
    public val namePrefix: String

    /**
     * Whether this appender is still open: `false` after [close] — on this
     * [Xlog] and on every other one of this [namePrefix], which is the same
     * appender and is closed with this one.
     */
    public val isOpen: Boolean

    /**
     * The level of this appender: a record less severe than this is dropped.
     *
     * Read from the bridge and not mirrored in Kotlin, so a level another part
     * of the app set is what this answers with.
     */
    public var level: LogLevel

    /** Whether a write reaches the file before it returns: what [XlogConfig] gave, until this says otherwise. */
    public var mode: AppenderMode

    /** Whether the console prints the record too — off until an app turns it on. */
    public var consoleLogEnabled: Boolean

    /**
     * How many bytes a log file of this appender may reach before it is closed
     * and a new one opened; `0`, the start, is "never split".
     */
    public var maxFileSizeBytes: Long

    /**
     * How many seconds a log file of this appender is kept; `0`, the start, is
     * the C++'s own ten days. Anything under a day asks for the same ten.
     */
    public var maxAliveTimeSeconds: Long

    /**
     * Whether a record of [level] would be written: what an app asks before it
     * builds a message that is expensive to build. `false` once [close] ran,
     * which is the one honest answer of an appender that writes nothing.
     */
    /**
     * The file this appender is writing to, or `null` when it has none open yet
     * — the first record of a day is what opens one.
     *
     * A day is one file, so this is the path an app hands to something that
     * reads the log while it is being written.
     */
    public val currentLogPath: String?

    /**
     * The log files of the day [daysAgo] days ago that are *there* — what an app
     * that uploads yesterday's asks for. Empty when the directory holds none of
     * that day's. `0` is today, `1` is yesterday, and so on.
     *
     * This is a day of files and not the file being written: what
     * [currentLogPath] answers is one, and this is this appender's own prefix
     * and directory.
     */
    public fun logFiles(daysAgo: Long): List<String>

    /**
     * The names of the log files of the day [daysAgo] days ago, whether or not
     * they are *there yet* — the name an app that is about to write, or that is
     * naming a file to someone else, asks for.
     *
     * A day's answer is the log-dir file and, when a cache dir is configured and
     * the file exists, its cache-dir twin, so this can answer two where
     * [logFiles] answers one.
     */
    public fun logFileNames(daysAgo: Long): List<String>

    public fun isLoggable(level: LogLevel): Boolean

    /** Writes one record of [level]. */
    public fun log(level: LogLevel, tag: String, message: String)

    /** Writes one record of [LogLevel.VERBOSE]. */
    public fun v(tag: String, message: String)

    /** Writes one record of [LogLevel.DEBUG]. */
    public fun d(tag: String, message: String)

    /** Writes one record of [LogLevel.INFO]. */
    public fun i(tag: String, message: String)

    /** Writes one record of [LogLevel.WARNING]. */
    public fun w(tag: String, message: String)

    /** Writes one record of [LogLevel.ERROR]. */
    public fun e(tag: String, message: String)

    /** Writes one record of [LogLevel.FATAL]. */
    public fun f(tag: String, message: String)

    /**
     * Tells the writer thread to take what is in the cache to the log file, and
     * returns at once: the drain is the writer's, and nothing here says when it
     * is over. What it is for is a drain an app wants soon and does not want to
     * wait for — a record still in the cache sits in a file the kernel holds, so
     * nothing is lost by a drain that has not happened yet.
     */
    public fun requestFlush()

    /**
     * Takes what is in the cache to the log file on the calling thread, and
     * hands the file's own buffer to the OS — the last few KiB of a log file sit
     * in a `FILE*` until this runs, so a reader in another process cannot see
     * them yet. The records are on disk when it returns, and what it costs is
     * the time the drain takes, on that thread.
     */
    public fun flushNow()

    /**
     * [flushNow] for a caller that can suspend and would rather not block the
     * thread it is on: the records are on disk when this resumes, and what
     * waited for them is a thread of the I/O pool. A drain blocks whatever
     * thread it runs on, which is why this one is handed to another one.
     */
    public suspend fun flush()

    /**
     * Closes this appender: drains what is left and drops it. Writing through
     * this [Xlog] afterwards writes nothing, and asking the bridge for this
     * [namePrefix] answers no handle. Safe to call twice.
     *
     * [level], [mode], [consoleLogEnabled], [maxFileSizeBytes] and
     * [maxAliveTimeSeconds] throw [IllegalStateException] afterwards: no handle
     * is left to forward, and a closed [Xlog] that went on forwarding would
     * reach the appender every other part of the app writes through rather than
     * its own.
     */
    public fun close()

    public companion object {
        /**
         * Opens an appender of its own: its own log directory, file name prefix,
         * key, mode and cache file, all of them [config]'s.
         *
         * The one call every platform of the port opens one with, under the one
         * name — `Xlog.open(config)` in Swift, in TypeScript and in Dart, and in
         * the Kotlin of the Android AAR. Kotlin has a constructor that does the
         * same thing, and this is still the name an app wants: an appender is
         * opened for the process, and not for an expression.
         *
         * @param config what to open it with
         * @throws IllegalArgumentException when the bridge answers no appender,
         *                                  which is what a directory it cannot
         *                                  create comes to
         */
        public fun open(config: XlogConfig): Xlog
    }
}
