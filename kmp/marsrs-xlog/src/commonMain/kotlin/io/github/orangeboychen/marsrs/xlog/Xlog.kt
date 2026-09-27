package io.github.orangeboychen.marsrs.xlog

/**
 * The Kotlin face of xlog, on every platform of a Kotlin Multiplatform project:
 * one appender of its own, written through the same calls in `commonMain`,
 * whether the module is linked into an Android app over the JNI bridge of
 * `crates/marsrs-jni` or into an iOS, watchOS, tvOS, macOS, Linux or Windows one
 * over the C ABI of `crates/marsrs-ffi`.
 *
 * It is the same class the Android AAR publishes — `Xlog(XlogConfig(...))`,
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
     * Takes what is in the cache to the log file, and hands the file's own
     * buffer to the OS — the last few KiB of a log file sit in a `FILE*` until
     * this runs, so a reader in another process cannot see them yet.
     *
     * @param sync `true` drains on the calling thread, which is what an app
     *             wants before it reads or uploads the files; `false` asks the
     *             writer thread to do it and returns
     */
    public fun flush(sync: Boolean = false)

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
}
