// The constants below carry the name the C++ project's Java gives them, spelled
// the way Kotlin spells a constant: `K_PING_CHECK` there is `K_PING_CHECK` here.
// The JNI reaches a constant by the number it carries and not by its name, so
// nothing on the Rust side had to change with them. They are what the older
// calls — `Xlog.open(...)` of seven arguments, and `Log` — still hand over;
// what new code writes is [LogLevel], [AppenderMode] and [CompressMode], which
// carry the same numbers.

package io.github.orangeboychen.marsrs.xlog

import android.content.ComponentCallbacks2
import android.content.Context
import android.content.res.Configuration
import java.lang.ref.WeakReference
import java.util.concurrent.ConcurrentHashMap

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
 * Two things about the Kotlin below are load bearing and easy to break:
 *
 * * `@JvmStatic` on a companion function is what puts a `static` on [Xlog]
 *   itself — the class JNI looks the symbol up under. Without it the native
 *   lands on `Xlog$Companion` and the name JNI wants is
 *   `..._Xlog_00024Companion_*`, which nobody exports.
 * * `@JvmField` on a property is what leaves it a field of the name the Rust
 *   asks for, instead of a getter over a private one.
 *
 * ## What a record costs
 *
 * One JNI call: [write] hands `marsrs-jni` the handle, the level, the tag and the
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
 * is killed reach the disk only after [flush]: call it before the app reads or
 * uploads its logs. Nothing is *lost* without it — the cache file is the
 * kernel's, and the next [Xlog] of this [namePrefix] drains it when it opens —
 * but the file of the session that is ending is complete only once [flush] ran.
 *
 * Which is why an [Xlog] built with a `Context` needs no [flush] on its way out:
 * it flushes itself when Android says the app's UI is no longer on screen, the
 * last moment Android says anything at all before it can end the process.
 *
 * ## The older spelling
 *
 * `Log` — `Log.d(tag, msg)` over a `LogImp` the app hands to `Log.setLogImp` —
 * and this class's own seven-argument [open], `XLogConfig`, `XLoggerInfo`,
 * `logWrite` and the `LEVEL_*` constants are the API the C++ project's Java
 * spelled. They are deprecated, and they work: an app that calls them gets the
 * process-wide appender `open` opens, and the `Xlog` it hands `Log.setLogImp` is
 * the one [Xlog] with no argument builds. An app that migrates calls
 * [Xlog.open] with an [XlogConfig] and writes through what it answers.
 */
class Xlog : Log.LogImp {

    /** `XLoggerInfo`: what [logWrite] reads out of its argument. */
    class XLoggerInfo {
        @JvmField var level: Int = 0

        @JvmField var tag: String? = null

        @JvmField var filename: String? = null

        @JvmField var funcname: String? = null

        @JvmField var line: Int = 0

        @JvmField var pid: Long = 0

        @JvmField var tid: Long = 0

        @JvmField var maintid: Long = 0
    }

    /**
     * `XLogConfig` — the fields `config_from_java` reads by name: what an
     * [XlogConfig] becomes on its way to the `.so`.
     */
    class XLogConfig {
        @JvmField var level: Int = LEVEL_INFO

        @JvmField var mode: Int = APPENDER_MODE_ASYNC

        @JvmField var logdir: String? = null

        @JvmField var nameprefix: String? = null

        @JvmField var pubkey: String = ""

        @JvmField var compressmode: Int = ZLIB_MODE

        @JvmField var compresslevel: Int = 0

        @JvmField var cachedir: String? = null

        @JvmField var cachedays: Int = 0
    }

    /**
     * Opens an appender of this [Xlog]'s own: its own log directory, file name
     * prefix, key, mode and cache file, all of them [config]'s.
     *
     * The prefix is what the appender is known by — and what every one of its
     * files starts with — so an app that wants two gives them two. Asking for a
     * prefix that is already open answers the appender that is open and not a
     * second one, which is why [close] on one `Xlog` closes what another `Xlog`
     * of the same prefix writes through.
     *
     * @param config what to open it with; [XlogConfig]
     * @param context any `Context` of the app, when this appender is to flush
     *                itself when the app's UI goes away — the last thing
     *                Android says before it can end the process without another
     *                word. `null`, the start, registers nothing; see [flush].
     * @throws IllegalArgumentException when `marsrs-jni` opens nothing, which is
     *                                  what a directory it cannot create comes
     *                                  to. [XlogConfig] refuses a config it
     *                                  cannot honour before this is reached.
     */
    @JvmOverloads
    constructor(config: XlogConfig, context: Context? = null) {
        // `marsrsxlog` is the `crate-name` of `marsrs-jni`; loading it twice is
        // nothing, so an app that loaded it already needs no way to say so.
        System.loadLibrary(LIBRARY)
        namePrefix = config.namePrefix
        currentMode = config.mode
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
     * The `Log.LogImp` of the process-wide appender: what an app hands to
     * `Log.setLogImp`, and what [open] opens that appender with.
     *
     * @deprecated build the appender an app writes through: `Xlog.open(XlogConfig(...))`.
     */
    @Deprecated("Build the appender you write through: Xlog.open(XlogConfig(...))")
    constructor() {
        namePrefix = ""
        handle = PROCESS_WIDE
        currentMode = AppenderMode.ASYNC
    }

    /** What every file of this appender starts with, and what it is known by. */
    val namePrefix: String

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
     * of the app set through `setLogLevel` is what this answers with.
     */
    var level: LogLevel
        get() = LogLevel.of(getLogLevel(requireOpen()))
        set(value) = setLogLevel(requireOpen(), value.native)

    /** [AppenderMode] of this appender: what the [XlogConfig] gave, until this says otherwise. */
    var mode: AppenderMode
        get() = currentMode
        set(value) {
            setAppenderMode(requireOpen(), value.native)
            currentMode = value
        }

    /** Whether the console prints the log too — off until an app turns it on. */
    var consoleLogEnabled: Boolean = false
        set(value) {
            setConsoleLogOpen(requireOpen(), value)
            field = value
        }

    /**
     * How many bytes a log file of this appender may reach before it is closed
     * and a new one opened; `0`, the start, is "never split".
     */
    var maxFileSizeBytes: Long = NO_FILE_SIZE_LIMIT
        set(value) {
            val size = value.coerceAtLeast(NO_FILE_SIZE_LIMIT)
            setMaxFileSize(requireOpen(), size)
            field = size
        }

    /**
     * How many seconds a log file of this appender is kept; `0`, the start, is
     * the C++'s own ten days. Anything under a day asks for the same ten.
     */
    var maxAliveTimeSeconds: Long = NO_ALIVE_TIME_LIMIT
        set(value) {
            val seconds = value.coerceAtLeast(NO_ALIVE_TIME_LIMIT)
            setMaxAliveTime(requireOpen(), seconds)
            field = seconds
        }

    /**
     * Whether a record of [level] would be written: what an app asks before it
     * builds a message that is expensive to build. `false` once [close] ran,
     * which is the one honest answer of an appender that writes nothing.
     */
    fun isLoggable(level: LogLevel): Boolean = isOpen && LogLevel.of(getLogLevel(handle)).isEnabledFor(level)

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
     * Takes what is in the cache to the log file, and hands the file's own
     * buffer to the OS — the last few KiB of a log file are in a `FILE*` until
     * this runs, so a reader in another process cannot see them yet.
     *
     * An [Xlog] built with a `Context` runs this itself when the app's UI is no
     * longer on screen, so an app that reads its logs in a later session needs
     * no call of its own.
     *
     * @param sync `true` drains on the calling thread, which is what an app
     *             wants before it reads or uploads the files; `false` asks the
     *             writer thread to do it and returns
     */
    @JvmOverloads
    fun flush(sync: Boolean = false) {
        if (isOpen) {
            appenderFlush(handle, sync)
        }
    }

    /**
     * Closes this appender: drains what is left and drops it. Writing through
     * this [Xlog] afterwards writes nothing, and asking `marsrs-jni` for this
     * [namePrefix] answers `0`. Safe to call twice.
     *
     * [level], [mode], [consoleLogEnabled], [maxFileSizeBytes] and
     * [maxAliveTimeSeconds] throw [IllegalStateException] afterwards: handle
     * `0` is the process-wide appender to `marsrs-jni`, and a closed [Xlog] that
     * went on forwarding it would read and move the appender every other part
     * of the app writes through, rather than its own.
     */
    fun close() {
        if (!isOpen) {
            return
        }
        // Before the handle goes: a callback left registered would be handed a
        // closed [Xlog] by Android and would find nothing to flush.
        backgroundFlush?.let { registeredWith?.unregisterComponentCallbacks(it) }
        backgroundFlush = null
        registeredWith = null
        releaseXlogInstance(namePrefix)
        // The appender is the prefix's and not this wrapper's: `marsrs-jni`
        // answers an [Xlog] of the same prefix with the same handle, so every
        // one of them is closed with this one.
        openHandles.remove(namePrefix, handle)
        handle = NO_HANDLE
    }

    /**
     * The handle of this appender, or [IllegalStateException] when there is
     * none left to forward: handle `0` names the process-wide appender in this
     * JNI API, so a closed [Xlog] that handed it on would read and move the
     * appender `Log` writes through — and read a level that is not its own.
     */
    private fun requireOpen(): Long {
        check(isOpen) {
            "no appender of this Xlog is open ('$namePrefix'): Xlog.open(XlogConfig(...)) another to log again"
        }
        return handle
    }

    private var handle: Long

    private var currentMode: AppenderMode

    /**
     * The callback that flushes this [Xlog] when the app's UI is no longer on
     * screen; `null` when it was built without a `Context`.
     */
    private var backgroundFlush: BackgroundFlush? = null

    /** The `Context` [backgroundFlush] was registered with, and what [close] unregisters it from. */
    private var registeredWith: Context? = null

    /**
     * What an [Xlog] built with a `Context` registers: [flush] on the moment the
     * app's UI is no longer on screen.
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
                log.get()?.flush(sync = true)
            }
        }

        override fun onLowMemory() {
            log.get()?.flush(sync = true)
        }

        override fun onConfigurationChanged(newConfig: Configuration) = Unit
    }

    companion object {
        const val LEVEL_ALL = 0
        const val LEVEL_VERBOSE = 0
        const val LEVEL_DEBUG = 1
        const val LEVEL_INFO = 2
        const val LEVEL_WARNING = 3
        const val LEVEL_ERROR = 4
        const val LEVEL_FATAL = 5
        const val LEVEL_NONE = 6

        const val COMPRESS_LEVEL1 = 1
        const val COMPRESS_LEVEL2 = 2
        const val COMPRESS_LEVEL3 = 3
        const val COMPRESS_LEVEL4 = 4
        const val COMPRESS_LEVEL5 = 5
        const val COMPRESS_LEVEL6 = 6
        const val COMPRESS_LEVEL7 = 7
        const val COMPRESS_LEVEL8 = 8
        const val COMPRESS_LEVEL9 = 9

        const val APPENDER_MODE_ASYNC = 0
        const val APPENDER_MODE_SYNC = 1

        const val ZLIB_MODE = 0
        const val ZSTD_MODE = 1

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
         *                another word. `null`, the start, registers nothing;
         *                see [flush]
         * @throws IllegalArgumentException when `marsrs-jni` opens nothing, which
         *                                  is what a directory it cannot create
         *                                  comes to
         */
        @JvmStatic
        @JvmOverloads
        fun open(config: XlogConfig, context: Context? = null): Xlog = Xlog(config, context)

        /**
         * Loads `libmarsrsxlog.so` and opens the process-wide appender — the one
         * `Log` writes through, and the one [Xlog] with no argument writes
         * through.
         *
         * @deprecated open the appender an app writes through: [Xlog.open].
         */
        @Deprecated("Open the appender you write through: Xlog.open(XlogConfig(...))")
        @Suppress("DEPRECATION") // `Xlog()` is what `Log` is handed, and this is the call that opens it
        @JvmStatic
        fun open(
            isLoadLib: Boolean,
            level: Int,
            mode: Int,
            cacheDir: String?,
            logDir: String?,
            nameprefix: String?,
            pubkey: String?
        ) {
            if (isLoadLib) {
                System.loadLibrary(LIBRARY)
            }

            val logConfig = XLogConfig().apply {
                this.level = level
                this.mode = mode
                this.logdir = logDir
                this.nameprefix = nameprefix
                this.pubkey = pubkey ?: ""
                this.compressmode = ZLIB_MODE
                this.compresslevel = 0
                this.cachedir = cacheDir
                this.cachedays = 0
            }
            appenderOpen(logConfig)
            Log.setLogImp(Xlog())
        }

        /**
         * Writes through the process-wide appender with a whole `XLoggerInfo`.
         *
         * @deprecated write through an `Xlog`: `xlog.i(tag, message)`.
         */
        @Deprecated("Write through an Xlog: xlog.i(tag, message)")
        @JvmStatic
        external fun logWrite(logInfo: XLoggerInfo, log: String?)

        /**
         * The write of this whole API: the handle, the level, the tag and the
         * message, and nothing else.
         *
         * `marsrs-jni` is what drops a record whose level the appender is above —
         * asking it from Kotlin would be a second JNI call per line — and what
         * fills the pid, the tid and the main tid in from the OS.
         */
        @JvmStatic
        private external fun write(logInstancePtr: Long, level: Int, tag: String, log: String)

        // `private` in the C++ project's Java too — an app opens the appender
        // through `open` or through `Log.appenderOpen`, not through this. It
        // still has to be `@JvmStatic`, or the symbol JNI wants is not the one
        // `marsrs-jni` exports.
        @JvmStatic
        private external fun appenderOpen(logConfig: XLogConfig)

        /** `marsrsxlog` — the `crate-name` of `marsrs-jni`, i.e. the library [write] and the rest live in. */
        private const val LIBRARY = "marsrsxlog"

        /**
         * The handle of every appender this process has open, by the prefix it
         * was opened with: what tells an [Xlog] that the appender it shares
         * with another [Xlog] of the same [namePrefix] has been closed, which
         * the handle alone cannot — `marsrs-jni` answers both with the same one,
         * so a [close] through either is a [close] of both.
         */
        private val openHandles: ConcurrentHashMap<String, Long> = ConcurrentHashMap()

        /** The handle of the process-wide appender: what `Log` writes through. */
        private const val PROCESS_WIDE = 0L

        /** The handle `marsrs-jni` answers for an appender it did not open. */
        private const val NO_HANDLE = 0L

        /** `marsrs-jni` reads a max file size of `0` as "never split". */
        private const val NO_FILE_SIZE_LIMIT = 0L

        /** `marsrs-jni` reads a max alive time of `0` as the C++'s own ten days. */
        private const val NO_ALIVE_TIME_LIMIT = 0L
    }

    // #################### the natives ####################
    //
    // Every `external` below is one of the
    // `Java_io_github_orangeboychen_marsrs_xlog_Xlog_*` symbols of `marsrs-jni`,
    // and the field names of [XLogConfig] are the ones its `config_from_java`
    // reads, so the two must be changed together.

    external override fun getLogLevel(logInstancePtr: Long): Int

    /** The port exports this one; the C++ project's Java did not declare it. */
    external fun setLogLevel(logInstancePtr: Long, level: Int)

    external override fun setAppenderMode(logInstancePtr: Long, mode: Int)

    external override fun getXlogInstance(nameprefix: String): Long

    external override fun releaseXlogInstance(nameprefix: String)

    external fun newXlogInstance(logConfig: XLogConfig): Long

    /** Whether the console prints the log too. */
    external override fun setConsoleLogOpen(logInstancePtr: Long, isOpen: Boolean)

    external override fun appenderClose()

    external override fun appenderFlush(logInstancePtr: Long, isSync: Boolean)

    external override fun setMaxFileSize(logInstancePtr: Long, size: Long)

    external override fun setMaxAliveTime(logInstancePtr: Long, seconds: Long)

    // #################### Log.LogImp ####################
    //
    // `Log` is the facade the C++ project's `Log.java` is: `Log.d(tag, msg)`
    // and friends, over whichever `LogImp` the app handed to
    // `Log.setLogImp` — `Xlog()`, here. Everything below is a straight call of
    // a native above; the filename, the function and the line the C++ project's
    // Java passes are `""`, `""` and `0` (`Log` has no `__FILE__`), and the
    // pid, the tid and the main tid `marsrs-jni` fills in from the OS are truer
    // than the `Thread.id` Java hands over.

    override fun logV(
        logInstancePtr: Long,
        tag: String,
        filename: String,
        funcname: String,
        line: Int,
        pid: Int,
        tid: Long,
        maintid: Long,
        log: String
    ) {
        write(logInstancePtr, LEVEL_VERBOSE, tag, log)
    }

    override fun logD(
        logInstancePtr: Long,
        tag: String,
        filename: String,
        funcname: String,
        line: Int,
        pid: Int,
        tid: Long,
        maintid: Long,
        log: String
    ) {
        write(logInstancePtr, LEVEL_DEBUG, tag, log)
    }

    override fun logI(
        logInstancePtr: Long,
        tag: String,
        filename: String,
        funcname: String,
        line: Int,
        pid: Int,
        tid: Long,
        maintid: Long,
        log: String
    ) {
        write(logInstancePtr, LEVEL_INFO, tag, log)
    }

    override fun logW(
        logInstancePtr: Long,
        tag: String,
        filename: String,
        funcname: String,
        line: Int,
        pid: Int,
        tid: Long,
        maintid: Long,
        log: String
    ) {
        write(logInstancePtr, LEVEL_WARNING, tag, log)
    }

    override fun logE(
        logInstancePtr: Long,
        tag: String,
        filename: String,
        funcname: String,
        line: Int,
        pid: Int,
        tid: Long,
        maintid: Long,
        log: String
    ) {
        write(logInstancePtr, LEVEL_ERROR, tag, log)
    }

    override fun logF(
        logInstancePtr: Long,
        tag: String,
        filename: String,
        funcname: String,
        line: Int,
        pid: Int,
        tid: Long,
        maintid: Long,
        log: String
    ) {
        write(logInstancePtr, LEVEL_FATAL, tag, log)
    }

    override fun openLogInstance(
        level: Int,
        mode: Int,
        cacheDir: String,
        logDir: String,
        nameprefix: String,
        cacheDays: Int
    ): Long {
        val logConfig = XLogConfig().apply {
            this.level = level
            this.mode = mode
            this.logdir = logDir
            this.nameprefix = nameprefix
            this.cachedir = cacheDir
            this.cachedays = cacheDays
        }
        return newXlogInstance(logConfig)
    }

    override fun appenderOpen(
        level: Int,
        mode: Int,
        cacheDir: String,
        logDir: String,
        nameprefix: String,
        cacheDays: Int
    ) {
        val logConfig = XLogConfig().apply {
            this.level = level
            this.mode = mode
            this.logdir = logDir
            this.nameprefix = nameprefix
            this.cachedir = cacheDir
            this.cachedays = cacheDays
        }
        appenderOpen(logConfig)
    }
}
