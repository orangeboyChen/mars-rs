package io.github.orangeboychen.marsrs.xlog

import android.content.ComponentCallbacks2
import android.content.Context
import android.content.res.Configuration
import java.lang.ref.WeakReference
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * An appender of an app's own — build one when the app starts, then write
 * through it from wherever there is something to say.
 *
 * ```kotlin
 * val xlog = Xlog.open(
 *     XlogConfig(
 *         logDir = File(context.filesDir, "xlog/log").path,
 *         cacheDir = File(context.filesDir, "xlog/cache").path,
 *         namePrefix = "marsrs",
 *         level = LogLevel.INFO,
 *     )
 * )
 * xlog.consoleLogEnabled = BuildConfig.DEBUG
 *
 * xlog.i("startup", "cold start in $elapsedMillis ms")
 * xlog.e("login", "login failed\n${cause.stackTraceToString()}")
 * ```
 *
 * This is the `Xlog` of the Kotlin Multiplatform module, member for member:
 * [open] is the only call that is not an [Xlog]'s own, and every other member
 * writes, drains or closes the appender it was answered with. The AAR once
 * carried a second, process-wide spelling of all of it — `Log`, a seven-argument
 * `open`, and an `Xlog` with no argument that wrote through the process-wide
 * appender — because that is the API the C++ project's Java spelled. An app that
 * migrates writes [Xlog.open] with an [XlogConfig], the way it does on every
 * other platform of the port.
 *
 * Two things about the Kotlin below are load bearing and easy to break:
 *
 * * `@JvmStatic` on a companion function is what puts a `static` on [Xlog]
 *   itself — the class JNI looks the symbol up under. Without it the native
 *   lands on `Xlog$Companion` and the name JNI wants is
 *   `..._Xlog_00024Companion_*`, which nobody exports.
 * * `private` and not `internal` on every `external`: Kotlin mangles the name of
 *   an `internal` function — `newXlogInstance` becomes
 *   `newXlogInstance$mars_xlog_release` in the bytecode — and JNI resolves the
 *   method by the name in the class file.
 *
 * ## What a record costs
 *
 * One JNI call: [log] hands `marsrs-jni` the handle, the level, the tag and the
 * message, and `marsrs-jni` decides whether the level lets the record through
 * before it formats anything. The pid, the tid and the main tid are not
 * gathered here — `marsrs-jni` fills them in from the OS, which is a truer tid
 * than `Thread.currentThread().id` and needs no `Looper`. A message that is
 * expensive to build is worth asking [isLoggable] about first; a record the
 * level drops costs the caller the `String` either way.
 *
 * ## Threads, and a process that dies
 *
 * Every member is safe to call from any thread. A record written through
 * [AppenderMode.ASYNC] — the default — sits in a memory-mapped cache file until
 * a writer thread takes it to the log file, so the last lines of a process that
 * is killed reach the disk only after [flushNow]: call it before the app reads
 * or uploads its logs. Nothing is *lost* without it — the cache file is the
 * kernel's, and the next [Xlog] of this [namePrefix] drains it when it opens —
 * but the file of the session that is ending is complete only once [flushNow]
 * ran.
 *
 * Which is why an [Xlog] built with a [Context] needs no [flushNow] on its way
 * out: it flushes itself when Android says the app's UI is no longer on screen,
 * the last moment Android says anything at all before it can end the process.
 */
class Xlog(config: XlogConfig, context: Context? = null) {
    /** What every file of this appender starts with, and what it is known by. */
    val namePrefix: String = config.namePrefix

    /**
     * Whether this appender is still open: `false` after [close] — on this
     * [Xlog] and on every other one of this [namePrefix], which is the same
     * appender and is closed with this one.
     */
    val isOpen: Boolean
        get() = handle != NO_HANDLE && handle == openHandles[namePrefix]

    /**
     * The level of this appender: a record less severe than this is dropped.
     *
     * Read from `marsrs-jni` and not mirrored in Kotlin, so a level another part
     * of the app set is what this answers with.
     */
    var level: LogLevel
        get() = LogLevel.of(getLogLevel(requireOpen()))
        set(value) = setLogLevel(requireOpen(), value.native)

    /**
     * Whether a write reaches the file before it returns: what the [XlogConfig]
     * gave, until this says otherwise. `marsrs-jni` has no getter for it, so this
     * is the last value this side wrote.
     */
    var mode: AppenderMode
        get() = currentMode
        set(value) {
            setAppenderMode(requireOpen(), value.native)
            currentMode = value
        }

    /**
     * Whether the console prints the log too — off until an app turns it on.
     *
     * What it turns on is `stderr`, and not the system log: `marsrs` writes to
     * the console of the process, and an Android app that wants its records in
     * logcat takes them out of the file.
     */
    var consoleLogEnabled: Boolean = false
        set(value) {
            setConsoleLogOpen(requireOpen(), value)
            field = value
        }

    /** How many bytes a log file may reach before it is closed and a new one
     * opened; `0` is "never split". */
    var maxFileSizeBytes: Long = NO_FILE_SIZE_LIMIT
        set(value) {
            val size = value.coerceAtLeast(NO_FILE_SIZE_LIMIT)
            setMaxFileSize(requireOpen(), size)
            field = size
        }

    /** How many seconds a log file is kept; `0` is the C++'s own ten days. */
    var maxAliveTimeSeconds: Long = NO_ALIVE_TIME_LIMIT
        set(value) {
            val seconds = value.coerceAtLeast(NO_ALIVE_TIME_LIMIT)
            setMaxAliveTime(requireOpen(), seconds)
            field = seconds
        }

    private var handle: Long = NO_HANDLE

    private var currentMode: AppenderMode = config.mode

    /**
     * What an [Xlog] built with a [Context] registers: [flushNow] on the moment
     * the app's UI is no longer on screen — and not [requestFlush], which only
     * wakes the writer: the process can be ended the moment this returns, so a
     * drain nobody has waited for is a drain that may not have happened.
     *
     * Android has no "the app is quitting" — `Application.onTerminate` is never
     * called on a device, and a process the system ends is told nothing at all.
     * What is left is `onTrimMemory`, and the levels from
     * `TRIM_MEMORY_UI_HIDDEN` up: every activity of the app is behind something
     * else now, which is where a backgrounded app lives until it is killed.
     *
     * The appender is held weakly: a callback the app `Context` keeps would hold
     * the appender open with it, and a cache slot a dropped [Xlog] never closed
     * is a slot no other one can claim.
     */
    private class BackgroundFlush(xlog: Xlog) : ComponentCallbacks2 {
        private val log = WeakReference(xlog)

        override fun onTrimMemory(level: Int) {
            if (level >= ComponentCallbacks2.TRIM_MEMORY_UI_HIDDEN) {
                log.get()?.flushNow()
            }
        }

        override fun onLowMemory() {
            log.get()?.flushNow()
        }

        override fun onConfigurationChanged(newConfig: Configuration) = Unit
    }

    /**
     * The callback that flushes this [Xlog] when the app's UI is no longer on
     * screen; `null` when it was built without a [Context].
     */
    private var backgroundFlush: BackgroundFlush? = null

    /** The [Context] [backgroundFlush] was registered with, and what [close] unregisters it from. */
    private var registeredWith: Context? = null

    init {
        // `marsrsxlog` is the `crate-name` of `marsrs-jni`; loading it twice is
        // nothing, so an app that loaded it already needs no way to say so.
        System.loadLibrary(LIBRARY)
        val opened = newXlogInstance(config.toNative())
        require(opened != NO_HANDLE) {
            "marsrs-jni opened no appender for ${config.namePrefix} in ${config.logDir}"
        }
        handle = opened
        openHandles[namePrefix] = opened
        if (context != null) {
            val app = context.applicationContext
            val callback = BackgroundFlush(this)
            app.registerComponentCallbacks(callback)
            backgroundFlush = callback
            registeredWith = app
        }
    }

    /**
     * Whether a record of [level] would be written: what an app asks before it
     * builds a message that is expensive to build. `false` once [close] ran,
     * which is the one honest answer of an appender that writes nothing.
     */
    fun isLoggable(level: LogLevel): Boolean = isOpen && LogLevel.of(getLogLevel(handle)).isEnabledFor(level)

    /**
     * The file this appender is writing to, or `null` before the day's first
     * record opens one.
     */
    val currentLogPath: String?
        get() = if (isOpen) getCurrentLogPath(handle) else null

    /**
     * The log files of the day `daysAgo` days ago that are *there* — what an app
     * that uploads yesterday's opens. Empty when the directory holds none of that
     * day's. `0` is today, `1` is yesterday, and so on.
     *
     * This is a day of files and not the file being written: what
     * [currentLogPath] answers is the directory, and this names the day's file in
     * it.
     */
    fun logFiles(daysAgo: Long): List<String> = if (isOpen) logFiles(handle, daysAgo).toList() else emptyList()

    /**
     * The names of the log files of the day `daysAgo` days ago, whether or not
     * they are there yet — the name an app that is about to write, or that is
     * naming a file to someone else, asks for.
     */
    fun logFileNames(daysAgo: Long): List<String> = if (isOpen) logFileNames(handle, daysAgo).toList() else emptyList()

    /** Writes a record of [level]. */
    fun log(level: LogLevel, tag: String, message: String) {
        if (isOpen) {
            write(handle, level.native, tag, message)
        }
    }

    /** [LogLevel.VERBOSE]. */
    fun v(tag: String, message: String) = log(LogLevel.VERBOSE, tag, message)

    /** [LogLevel.DEBUG]. */
    fun d(tag: String, message: String) = log(LogLevel.DEBUG, tag, message)

    /** [LogLevel.INFO]. */
    fun i(tag: String, message: String) = log(LogLevel.INFO, tag, message)

    /** [LogLevel.WARNING]. */
    fun w(tag: String, message: String) = log(LogLevel.WARNING, tag, message)

    /** [LogLevel.ERROR]. */
    fun e(tag: String, message: String) = log(LogLevel.ERROR, tag, message)

    /** [LogLevel.FATAL]. */
    fun f(tag: String, message: String) = log(LogLevel.FATAL, tag, message)

    /**
     * Tells the writer thread to take what is in the cache to the log file, and
     * returns at once: the drain is the writer's, and nothing here says when it
     * is over. What it is for is a drain an app wants soon and does not want to
     * wait for — a record still in the cache sits in a file the kernel holds, so
     * nothing is lost by a drain that has not happened yet.
     */
    fun requestFlush() {
        if (isOpen) {
            appenderRequestFlush(handle)
        }
    }

    /**
     * Takes what is in the cache to the log file on the calling thread, and
     * hands the file's own buffer to the OS — the last few KiB of a log file are
     * in a `FILE*` until this runs, so a reader in another process cannot see
     * them yet. The records are on disk when it returns, and what it costs is
     * the time the drain takes, on that thread.
     */
    fun flushNow() {
        if (isOpen) {
            appenderFlushNow(handle)
        }
    }

    /**
     * [flushNow] for a caller that can suspend and would rather not block the
     * thread it is on: the records are on disk when this resumes, and what
     * waited for them is a thread of the I/O pool. A drain blocks whatever
     * thread it runs on, which is why this one is handed to another one.
     */
    suspend fun flush() = withContext(Dispatchers.IO) {
        flushNow()
    }

    /**
     * Closes this appender: drains what is left and drops it. Writing through
     * this [Xlog] afterwards writes nothing, and asking `marsrs-jni` for this
     * [namePrefix] answers `0`. Safe to call twice.
     *
     * [level], [mode], [consoleLogEnabled], [maxFileSizeBytes] and
     * [maxAliveTimeSeconds] throw [IllegalStateException] afterwards: handle `0`
     * is the process-wide appender to `marsrs-jni`, and a closed [Xlog] that
     * went on forwarding it would read and move the appender every other part
     * of the app writes through, rather than its own.
     */
    fun close() {
        if (!isOpen) {
            return
        }
        // Before the handle goes: a callback left registered would be handed a
        // closed [Xlog] by Android, and would find nothing to flush.
        backgroundFlush?.let { registeredWith?.unregisterComponentCallbacks(it) }
        backgroundFlush = null
        registeredWith = null
        releaseXlogInstance(namePrefix)
        // The appender is the prefix's and not this wrapper's: `marsrs-jni`
        // answers an [Xlog] of the same prefix with the same handle, so every one
        // of them is closed with this one.
        openHandles.remove(namePrefix, handle)
        handle = NO_HANDLE
    }

    /**
     * The handle of this appender, or [IllegalStateException] when there is
     * none left to forward: handle `0` names the process-wide appender in this
     * JNI API, so a closed [Xlog] that handed it on would read and move the
     * appender every other part of the app writes through, and read a level that
     * is not its own.
     */
    private fun requireOpen(): Long {
        check(isOpen) {
            "no appender of this Xlog is open ('$namePrefix'): Xlog.open(XlogConfig(...)) another to log again"
        }
        return handle
    }

    // The names `marsrs-jni` exports, and the signatures it reads them under.
    //
    // `private` and not `internal`, and that is the whole point of these lines:
    // Kotlin mangles the name of an `internal` function — `newXlogInstance`
    // becomes `newXlogInstance$mars_xlog_release` in the bytecode, which is a
    // name no symbol of the Rust carries, and JNI resolves the method by the name
    // in the class file. `private` is not mangled, and visibility is nothing JNI
    // asks about.
    private external fun newXlogInstance(config: XLogConfigJni): Long

    private external fun releaseXlogInstance(namePrefix: String)

    private external fun appenderRequestFlush(handle: Long)

    private external fun appenderFlushNow(handle: Long)

    private external fun getLogLevel(handle: Long): Int

    private external fun getCurrentLogPath(handle: Long): String?

    private external fun logFiles(handle: Long, timespan: Long): Array<String>

    private external fun logFileNames(handle: Long, timespan: Long): Array<String>

    private external fun setLogLevel(handle: Long, level: Int)

    private external fun setAppenderMode(handle: Long, mode: Int)

    private external fun setConsoleLogOpen(handle: Long, open: Boolean)

    private external fun setMaxFileSize(handle: Long, bytes: Long)

    private external fun setMaxAliveTime(handle: Long, seconds: Long)

    companion object {
        /**
         * Opens an appender of its own: its own log directory, file name prefix,
         * key, mode and cache file, all of them [config]'s.
         *
         * The one call every platform of the port opens one with, under the one
         * name — `Xlog.open(config)` in the Kotlin Multiplatform module, in
         * Swift, in TypeScript and in Dart. Kotlin has a constructor that does
         * the same thing, and this is still the name an app wants: an appender is
         * opened for the process, and not for an expression.
         *
         * `@JvmStatic` is what puts a `static` on [Xlog] itself — the class JNI
         * looks the symbols up under — so a Java caller writes `Xlog.open(config)`
         * and not `Xlog.Companion.open(config)`.
         *
         * @param config what to open it with
         * @param context any `Context` of the app, when this appender is to flush
         *                itself when the app's UI goes away — the last thing
         *                Android says before it can end the process without
         *                another word. `null`, the start, registers nothing; see
         *                [flushNow]
         * @throws IllegalArgumentException when `marsrs-jni` opens nothing, which
         *                                  is what a directory it cannot create
         *                                  comes to
         */
        @JvmStatic
        @JvmOverloads
        fun open(config: XlogConfig, context: Context? = null): Xlog = Xlog(config, context)

        @JvmStatic
        private external fun write(handle: Long, level: Int, tag: String, message: String)

        /** `marsrsxlog` — the `crate-name` of `marsrs-jni`, i.e. the library [write] and the rest live in. */
        const val LIBRARY = "marsrsxlog"

        /**
         * The handle of every appender this process has open, by the prefix it
         * was opened with: what tells an [Xlog] that the appender it shares with
         * another [Xlog] of the same [namePrefix] has been closed, which the
         * handle alone cannot — `marsrs-jni` answers both with the same one.
         */
        val openHandles: ConcurrentHashMap<String, Long> = ConcurrentHashMap()

        /** The handle `marsrs-jni` answers for an appender it did not open. */
        const val NO_HANDLE = 0L

        /** `marsrs-jni` reads a max file size of `0` as "never split". */
        const val NO_FILE_SIZE_LIMIT = 0L

        /** `marsrs-jni` reads a max alive time of `0` as the C++'s own ten days. */
        const val NO_ALIVE_TIME_LIMIT = 0L
    }
}

/**
 * What `marsrs-jni` reads out of the object [Xlog.newXlogInstance] hands it.
 *
 * The names of the fields are the ones its `config_from_java` asks for, and
 * `@JvmField` is what leaves each of them a field of that name instead of a
 * getter over a private one — without it `GetFieldID` finds nothing. Nothing else
 * about the class is looked at: it is not the [XlogConfig] an app writes, whose
 * properties are the ones the Kotlin of this module names.
 */
internal class XLogConfigJni {
    @JvmField var level: Int = 0

    @JvmField var mode: Int = 0

    @JvmField var logdir: String? = null

    @JvmField var nameprefix: String? = null

    @JvmField var pubkey: String = ""

    @JvmField var compressmode: Int = 0

    @JvmField var compresslevel: Int = 0

    @JvmField var cachedir: String? = null

    @JvmField var cachedays: Int = 0
}
