package io.github.orangeboychen.marsrs.xlog

/**
 * An appender of its own — its own log directory, file name prefix, key, mode
 * and cache file — for the part of an app whose logs are read apart from the
 * rest: a module that ships them, a feature that is debugged on its own.
 *
 * [Xlog.openInstance] opens one and [Xlog.instance] finds one that is already
 * open. Both are keyed by [namePrefix], which is also what every file of this
 * appender starts with.
 *
 * An instance is **not** given what [Xlog.open] set on the process-wide
 * appender: the port builds it from its [XlogConfig] alone, so it starts with
 * console logging off, no file size limit and the C++'s own ten day expiry.
 * [level], [consoleLogEnabled], [maxFileSizeBytes] and [maxAliveTimeSeconds]
 * are set here, on the instance, and not on [Xlog].
 *
 * [Xlog.flush] drains these along with the process-wide appender and
 * [Xlog.close] closes them; an app that opens one on purpose usually closes it
 * on purpose too, with [close].
 */
class XlogInstance internal constructor(
    /**
     * What every file of this appender starts with, and what [Xlog.instance]
     * asks for to find it again.
     */
    val namePrefix: String,
    /** The handle `Xlog.newXlogInstance` answered with; `0` once [close] ran. */
    internal var handle: Long
) {
    /** Whether this instance is still open: `false` after [close]. */
    val isOpen: Boolean
        get() = handle != NO_INSTANCE

    /**
     * The level of this instance: a record less severe than this is dropped.
     *
     * Read from `mars-jni`, not mirrored in Kotlin, so a level another part of
     * the app set through `Xlog.setLogLevel` is the one this answers with.
     */
    var level: LogLevel
        get() = LogLevel.of(natives.getLogLevel(handle))
        set(value) {
            natives.setLogLevel(handle, value.native)
        }

    /** [AppenderMode] of this instance's own appender. */
    var mode: AppenderMode = AppenderMode.ASYNC
        set(value) {
            field = value
            natives.setAppenderMode(handle, value.native)
        }

    /** Whether this instance prints to the console as well; off until set. */
    var consoleLogEnabled: Boolean = false
        set(value) {
            field = value
            natives.setConsoleLogOpen(handle, value)
        }

    /**
     * How many bytes a log file of this instance may reach before it is closed
     * and a new one opened; `0`, the start, is "never split".
     */
    var maxFileSizeBytes: Long = NO_FILE_SIZE_LIMIT
        set(value) {
            field = value.coerceAtLeast(NO_FILE_SIZE_LIMIT)
            natives.setMaxFileSize(handle, field)
        }

    /**
     * How many seconds a log file of this instance is kept; `0`, the start, is
     * the C++'s own ten days. Anything under a day asks for the same ten.
     */
    var maxAliveTimeSeconds: Long = NO_ALIVE_TIME_LIMIT
        set(value) {
            field = value.coerceAtLeast(NO_ALIVE_TIME_LIMIT)
            natives.setMaxAliveTime(handle, field)
        }

    /** Writes a record of [level]; the lambda is only called when it is written. */
    fun log(level: LogLevel, tag: String, message: () -> String) {
        Xlog.writeRecord(handle, level, tag, message)
    }

    /**
     * Writes a record of [level].
     *
     * [message] is built by the caller either way; [log] with a lambda is the
     * one that costs nothing when the level drops the record.
     */
    fun log(level: LogLevel, tag: String, message: String) {
        Xlog.writeRecord(handle, level, tag) { message }
    }

    /** [LogLevel.VERBOSE] — [log] with a lambda. */
    fun v(tag: String, message: () -> String) = log(LogLevel.VERBOSE, tag, message)

    /** [LogLevel.VERBOSE]. */
    fun v(tag: String, message: String) = log(LogLevel.VERBOSE, tag, message)

    /** [LogLevel.DEBUG] — [log] with a lambda. */
    fun d(tag: String, message: () -> String) = log(LogLevel.DEBUG, tag, message)

    /** [LogLevel.DEBUG]. */
    fun d(tag: String, message: String) = log(LogLevel.DEBUG, tag, message)

    /** [LogLevel.INFO] — [log] with a lambda. */
    fun i(tag: String, message: () -> String) = log(LogLevel.INFO, tag, message)

    /** [LogLevel.INFO]. */
    fun i(tag: String, message: String) = log(LogLevel.INFO, tag, message)

    /** [LogLevel.WARNING] — [log] with a lambda. */
    fun w(tag: String, message: () -> String) = log(LogLevel.WARNING, tag, message)

    /** [LogLevel.WARNING]. */
    fun w(tag: String, message: String) = log(LogLevel.WARNING, tag, message)

    /** [LogLevel.ERROR] — [log] with a lambda. */
    fun e(tag: String, message: () -> String) = log(LogLevel.ERROR, tag, message)

    /** [LogLevel.ERROR]. */
    fun e(tag: String, message: String) = log(LogLevel.ERROR, tag, message)

    /** [LogLevel.FATAL] — [log] with a lambda. */
    fun f(tag: String, message: () -> String) = log(LogLevel.FATAL, tag, message)

    /** [LogLevel.FATAL]. */
    fun f(tag: String, message: String) = log(LogLevel.FATAL, tag, message)

    /**
     * Drains this instance's cache, and hands its log file's buffer to the OS.
     *
     * @param sync `true` drains on the calling thread — what an app wants
     *             before it reads the files; `false` asks the writer thread to
     *             do it and returns.
     */
    @JvmOverloads
    fun flush(sync: Boolean = false) {
        if (isOpen) {
            natives.appenderFlush(handle, sync)
        }
    }

    /**
     * Closes this appender: drains what is left and drops it. `Xlog.instance`
     * answers `null` for [namePrefix] afterwards. Safe to call twice.
     */
    fun close() {
        if (!isOpen) {
            return
        }
        natives.releaseXlogInstance(namePrefix)
        handle = NO_INSTANCE
    }

    // The natives of `Xlog` are `external` on the class `mars-jni` exports its
    // symbols under, so an instance of it is what any of them is called on —
    // the same reason `Log` holds a `LogImp`.
    private val natives: Xlog = Xlog()

    private companion object {
        const val NO_INSTANCE = 0L
        const val NO_FILE_SIZE_LIMIT = 0L
        const val NO_ALIVE_TIME_LIMIT = 0L
    }
}
