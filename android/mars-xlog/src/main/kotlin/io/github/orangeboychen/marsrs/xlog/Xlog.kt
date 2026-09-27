// The constants below carry the name the C++ project's Java gives them, spelled
// the way Kotlin spells a constant: `K_PING_CHECK` there is `K_PING_CHECK` here.
// The JNI reaches a constant by the number it carries and not by its name, so
// nothing on the Rust side had to change with them.

package io.github.orangeboychen.marsrs.xlog

/**
 * The Kotlin face of `libmarsxlog.so` (crate `mars-jni`), and the whole xlog
 * API: [open] the appender once when the app starts, then [i], [w], [e] and
 * the rest from wherever there is something to say.
 *
 * ```kotlin
 * Xlog.open(
 *     XlogConfig(
 *         logDir = File(context.filesDir, "xlog/log").path,
 *         cacheDir = File(context.filesDir, "xlog/cache").path,
 *         namePrefix = "Ham",
 *         level = LogLevel.INFO,
 *     )
 * )
 * Xlog.consoleLogEnabled = BuildConfig.DEBUG
 *
 * Xlog.i("startup") { "cold start in $elapsedMillis ms" }
 * Xlog.e("login") { "login failed\n${cause.stackTraceToString()}" }
 * ```
 *
 * ## The older spelling
 *
 * `Log` — `Log.d(tag, msg)` over a `LogImp` the app hands to
 * `Log.setLogImp` — and this class's own seven-argument `open`,
 * `XLogConfig` and `LEVEL_*` constants are the API the C++ project's Java
 * spelled, kept so that an app that already calls it keeps working. Both
 * families write through one appender: [open] installs the `Xlog`
 * implementation into [Log] as well, so a migration can go one call site at a
 * time and the lines of the two land in the same file. What the port changed
 * about that Java is what it dropped — `c++_shared`, which a Rust `.so` needs
 * no loader for — and what it added: `setLogLevel`, and
 * [COMPRESS_LEVEL1]..[COMPRESS_LEVEL9], the C++'s own names for the levels a
 * compressed appender is opened with.
 *
 * ## What a write costs
 *
 * A message is a lambda and not a [String]: [i] asks the appender for its
 * level first and calls the lambda only when the record is going to be
 * written, so building a line nobody reads is free. The pid, the tid and the
 * main tid are not gathered here either — `mars-jni` fills a `-1` in from the
 * OS, which is a truer tid than `Thread.currentThread().id` and needs no
 * `Looper`.
 *
 * ## Threads, and a process that dies
 *
 * Every member is safe to call from any thread; [open], [close] and the three
 * instance calls serialize. A record of [AppenderMode.ASYNC] — the default —
 * sits in a memory-mapped cache file until a writer thread takes it to the log
 * file, so the last lines of a process that is killed reach the disk only
 * after [flush]: call it before the app reads or uploads its logs.
 *
 * The library is loaded by [open]: it is `marsxlog`, the `crate-name` of
 * `mars-jni`. Two things about the Kotlin below are load bearing and easy to
 * break:
 *
 * * `@JvmStatic` on a companion function is what puts a `static` on [Xlog]
 *   itself — the class JNI looks the symbol up under. Without it the native
 *   lands on `Xlog$Companion` and the name JNI wants is
 *   `..._Xlog_00024Companion_*`, which nobody exports.
 * * `@JvmField` on a property is what leaves it a field of the name the Rust
 *   asks for, instead of a getter over a private one.
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

    /** `XLogConfig` — the fields `config_from_java` reads by name. */
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

        // #################### the API ####################
        //
        // `Xlog.open(...)` once and `Xlog.i("tag") { ... }` from anywhere — the
        // half an app calls. The natives it runs on are below, with the seven-
        // argument `open`, `XLogConfig` and the `LEVEL_*` constants of the C++
        // project's Java.

        private val lock = Any()

        private val natives: Xlog = Xlog()

        /** What [openInstance] opened and [instance] has not closed yet. */
        private val instances: MutableMap<String, XlogInstance> = LinkedHashMap()

        private var opened = false

        private var consoleLog = false

        private var appenderMode = AppenderMode.ASYNC

        private var fileSizeLimit = NO_FILE_SIZE_LIMIT

        private var aliveTimeLimit = NO_ALIVE_TIME_LIMIT

        /**
         * Whether [open] has run. A second [open] is refused by `mars-jni`,
         * which keeps the appender it already has; so is one whose directory
         * cannot be created, and neither is reported back — [XlogConfig]
         * refusing a config it cannot honour is what an app hears about a
         * directory that will not do.
         */
        @JvmStatic
        val isOpen: Boolean
            get() = synchronized(lock) { opened }

        /**
         * Loads `libmarsxlog.so` and opens the process-wide appender — the one
         * [i], [w], [e] and the rest write through, and the one `Log` writes
         * through once this has run.
         *
         * @param config what to open it with; [XlogConfig]
         * @param loadLibrary whether to load the library here; pass `false`
         *                    when the app has loaded it already
         */
        @JvmStatic
        @JvmOverloads
        fun open(config: XlogConfig, loadLibrary: Boolean = true) {
            synchronized(lock) {
                if (loadLibrary) {
                    System.loadLibrary("marsxlog")
                }
                appenderOpenFromConfig(config.toNative())
                // `Log` is the facade the older call sites write through, and
                // an app that migrates one class at a time needs both to land
                // in the same file.
                Log.setLogImp(Xlog())
                appenderMode = config.mode
                opened = true
            }
        }

        /**
         * Closes the process-wide appender — and every instance [openInstance]
         * opened — after draining what is left in their caches.
         *
         * For the end of the process, not for `Activity.onDestroy`, which a
         * turning screen reaches. Nothing is written after this until [open]
         * runs again.
         */
        @JvmStatic
        fun close() {
            synchronized(lock) {
                val open = instances.values.toList()
                instances.clear()
                open.forEach { it.close() }
                natives.appenderClose()
                opened = false
            }
        }

        /**
         * Takes what is in the caches to the log files, and hands the files'
         * own buffers to the OS — the last few KiB of a log file are in a
         * `FILE*` until this runs, so a reader in another process cannot see
         * them yet.
         *
         * @param sync `true` drains on the calling thread, which is what an
         *             app wants before it reads or uploads the files; `false`
         *             asks the writer thread to do it and returns
         */
        @JvmStatic
        @JvmOverloads
        fun flush(sync: Boolean = false) {
            synchronized(lock) {
                natives.appenderFlush(PROCESS_WIDE, sync)
                instances.values.toList().forEach { it.flush(sync) }
            }
        }

        /**
         * The level of the process-wide appender: a record less severe than
         * this is dropped.
         *
         * Read from `mars-jni` and not mirrored in Kotlin, so a level another
         * part of the app set through `setLogLevel` is what this answers with.
         */
        @JvmStatic
        var level: LogLevel
            get() = LogLevel.of(natives.getLogLevel(PROCESS_WIDE))
            set(value) = natives.setLogLevel(PROCESS_WIDE, value.native)

        /** [AppenderMode] of the process-wide appender: what [XlogConfig] gave, until this says otherwise. */
        @JvmStatic
        var mode: AppenderMode
            get() = synchronized(lock) { appenderMode }
            set(value) =
                synchronized(lock) {
                    appenderMode = value
                    natives.setAppenderMode(PROCESS_WIDE, value.native)
                }

        /** Whether the console prints the log too — off until an app turns it on. */
        @JvmStatic
        var consoleLogEnabled: Boolean
            get() = synchronized(lock) { consoleLog }
            set(value) =
                synchronized(lock) {
                    consoleLog = value
                    natives.setConsoleLogOpen(PROCESS_WIDE, value)
                }

        /**
         * How many bytes a log file may reach before it is closed and a new one
         * opened; `0`, the start, is "never split".
         */
        @JvmStatic
        var maxFileSizeBytes: Long
            get() = synchronized(lock) { fileSizeLimit }
            set(value) =
                synchronized(lock) {
                    fileSizeLimit = value.coerceAtLeast(NO_FILE_SIZE_LIMIT)
                    natives.setMaxFileSize(PROCESS_WIDE, fileSizeLimit)
                }

        /**
         * How many seconds a log file is kept; `0`, the start, is the C++'s own
         * ten days. Anything under a day asks for the same ten.
         */
        @JvmStatic
        var maxAliveTimeSeconds: Long
            get() = synchronized(lock) { aliveTimeLimit }
            set(value) =
                synchronized(lock) {
                    aliveTimeLimit = value.coerceAtLeast(NO_ALIVE_TIME_LIMIT)
                    natives.setMaxAliveTime(PROCESS_WIDE, aliveTimeLimit)
                }

        /**
         * Opens an appender of its own — its own directory, prefix, key, mode
         * and cache file — and answers it; `null` when `mars-jni` refused the
         * config, which is what an empty `logDir` or `namePrefix` comes to.
         *
         * Asking for a prefix that is already open answers the instance that is
         * open, not a second one.
         */
        @JvmStatic
        fun openInstance(config: XlogConfig): XlogInstance? = synchronized(lock) {
            val handle = natives.newXlogInstance(config.toNative())
            when (handle) {
                NO_INSTANCE -> null
                else -> XlogInstance(config.namePrefix, handle).also { instances[config.namePrefix] = it }
            }
        }

        /** The instance [openInstance] opened for [namePrefix], or `null`. */
        @JvmStatic
        fun instance(namePrefix: String): XlogInstance? = synchronized(lock) {
            val handle = natives.getXlogInstance(namePrefix)
            when {
                // A prefix `mars-jni` has no appender for — one that was never
                // opened, or one whose [XlogInstance.close] ran — answers
                // `null`, and stops being remembered here.
                handle == NO_INSTANCE -> {
                    instances.remove(namePrefix)
                    null
                }

                else -> instances[namePrefix] ?: XlogInstance(namePrefix, handle).also {
                    instances[namePrefix] = it
                }
            }
        }

        /** Closes the instance [openInstance] opened for [namePrefix]. */
        @JvmStatic
        fun closeInstance(namePrefix: String) {
            synchronized(lock) {
                val instance = instances.remove(namePrefix)
                if (instance != null) {
                    instance.close()
                } else {
                    natives.releaseXlogInstance(namePrefix)
                }
            }
        }

        /** Writes a record of [level]; the lambda is called only when it is written. */
        @JvmStatic
        fun log(level: LogLevel, tag: String, message: () -> String) {
            writeRecord(PROCESS_WIDE, level, tag, message)
        }

        /**
         * Writes a record of [level].
         *
         * [message] is built by the caller either way; [log] with a lambda is
         * the one that costs nothing when the level drops the record.
         */
        @JvmStatic
        fun log(level: LogLevel, tag: String, message: String) {
            writeRecord(PROCESS_WIDE, level, tag) { message }
        }

        /** [LogLevel.VERBOSE] — [log] with a lambda. */
        @JvmStatic
        fun v(tag: String, message: () -> String) = log(LogLevel.VERBOSE, tag, message)

        /** [LogLevel.VERBOSE]. */
        @JvmStatic
        fun v(tag: String, message: String) = log(LogLevel.VERBOSE, tag, message)

        /** [LogLevel.DEBUG] — [log] with a lambda. */
        @JvmStatic
        fun d(tag: String, message: () -> String) = log(LogLevel.DEBUG, tag, message)

        /** [LogLevel.DEBUG]. */
        @JvmStatic
        fun d(tag: String, message: String) = log(LogLevel.DEBUG, tag, message)

        /** [LogLevel.INFO] — [log] with a lambda. */
        @JvmStatic
        fun i(tag: String, message: () -> String) = log(LogLevel.INFO, tag, message)

        /** [LogLevel.INFO]. */
        @JvmStatic
        fun i(tag: String, message: String) = log(LogLevel.INFO, tag, message)

        /** [LogLevel.WARNING] — [log] with a lambda. */
        @JvmStatic
        fun w(tag: String, message: () -> String) = log(LogLevel.WARNING, tag, message)

        /** [LogLevel.WARNING]. */
        @JvmStatic
        fun w(tag: String, message: String) = log(LogLevel.WARNING, tag, message)

        /** [LogLevel.ERROR] — [log] with a lambda. */
        @JvmStatic
        fun e(tag: String, message: () -> String) = log(LogLevel.ERROR, tag, message)

        /** [LogLevel.ERROR]. */
        @JvmStatic
        fun e(tag: String, message: String) = log(LogLevel.ERROR, tag, message)

        /** [LogLevel.FATAL] — [log] with a lambda. */
        @JvmStatic
        fun f(tag: String, message: () -> String) = log(LogLevel.FATAL, tag, message)

        /** [LogLevel.FATAL]. */
        @JvmStatic
        fun f(tag: String, message: String) = log(LogLevel.FATAL, tag, message)

        // #################### the natives ####################
        //
        // Every `external` here is one of the
        // `Java_io_github_orangeboychen_marsrs_xlog_Xlog_*` symbols of
        // `mars-jni`, and the field names of [XLogConfig] are the ones its
        // `config_from_java` reads, so the two must be changed together.

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
                System.loadLibrary("marsxlog")
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
        }

        /** The process-wide appender; [logInstancePtr] `0` is what [Log] passes. */
        @JvmStatic
        fun logWrite2(
            level: Int,
            tag: String?,
            filename: String?,
            funcname: String?,
            line: Int,
            pid: Int,
            tid: Long,
            maintid: Long,
            log: String?
        ) {
            logWrite2(0L, level, tag, filename, funcname, line, pid, tid, maintid, log)
        }

        @JvmStatic
        external fun logWrite(logInfo: XLoggerInfo, log: String?)

        @JvmStatic
        external fun logWrite2(
            logInstancePtr: Long,
            level: Int,
            tag: String?,
            filename: String?,
            funcname: String?,
            line: Int,
            pid: Int,
            tid: Long,
            maintid: Long,
            log: String?
        )

        /**
         * The one place a record is written from: [XlogInstance] calls this with
         * the handle of its own appender.
         *
         * The level is asked of `mars-jni` first — a record that goes through
         * handle `0` is not filtered on the Rust side at all, so this is where
         * a dropped record is dropped — and [message] runs only once the record
         * is going out. The pid, the tid and the main tid are `-1`, which is
         * what asks `mars-jni` to fill them in from the OS.
         */
        internal fun writeRecord(handle: Long, level: LogLevel, tag: String, message: () -> String) {
            if (!LogLevel.of(natives.getLogLevel(handle)).isEnabledFor(level)) {
                return
            }
            logWrite2(
                logInstancePtr = handle,
                level = level.native,
                tag = tag,
                filename = NO_SOURCE_FILE,
                funcname = NO_FUNCTION,
                line = NO_LINE,
                pid = PID_FROM_OS,
                tid = TID_FROM_OS,
                maintid = TID_FROM_OS,
                log = message()
            )
        }

        /**
         * Opens the process-wide appender with a config the caller filled in.
         *
         * Not public: an app opens the appender through [open], and a second
         * door to the same `private` native is one more thing an app could
         * come to depend on.
         */
        internal fun appenderOpenFromConfig(config: XLogConfig) {
            appenderOpen(config)
        }

        // `private` in the C++ project's Java too — an app opens the appender
        // through `open` or through `Log.appenderOpen`, not through this. It
        // still has to be `@JvmStatic`, or the symbol JNI wants is not the one
        // `mars-jni` exports.
        @JvmStatic
        private external fun appenderOpen(logConfig: XLogConfig)

        /**
         * Where the C++ project decrypts the tag it was handed. The port has no
         * encrypted tags, so this is the identity — kept because it is the one
         * place every write goes through.
         */
        private fun decryptTag(tag: String): String = tag

        /** The handle of the process-wide appender: what [Log] writes through. */
        private const val PROCESS_WIDE = 0L

        /** The handle `mars-jni` answers for an instance it did not open. */
        private const val NO_INSTANCE = 0L

        /** `mars-jni` reads a max file size of `0` as "never split". */
        private const val NO_FILE_SIZE_LIMIT = 0L

        /** `mars-jni` reads a max alive time of `0` as the C++'s own ten days. */
        private const val NO_ALIVE_TIME_LIMIT = 0L

        /** What [writeRecord] passes for the source file of a record. */
        private const val NO_SOURCE_FILE = ""

        /** What [writeRecord] passes for the function of a record. */
        private const val NO_FUNCTION = ""

        /** What [writeRecord] passes for the source line of a record. */
        private const val NO_LINE = 0

        /** A pid of `-1` is one `mars-jni` fills in from the OS. */
        private const val PID_FROM_OS = -1

        /** A tid — or a main tid — of `-1` is one `mars-jni` fills in from the OS. */
        private const val TID_FROM_OS = -1L
    }

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
    // a native above.

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
        logWrite2(logInstancePtr, LEVEL_VERBOSE, decryptTag(tag), filename, funcname, line, pid, tid, maintid, log)
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
        logWrite2(logInstancePtr, LEVEL_DEBUG, decryptTag(tag), filename, funcname, line, pid, tid, maintid, log)
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
        logWrite2(logInstancePtr, LEVEL_INFO, decryptTag(tag), filename, funcname, line, pid, tid, maintid, log)
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
        logWrite2(logInstancePtr, LEVEL_WARNING, decryptTag(tag), filename, funcname, line, pid, tid, maintid, log)
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
        logWrite2(logInstancePtr, LEVEL_ERROR, decryptTag(tag), filename, funcname, line, pid, tid, maintid, log)
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
        logWrite2(logInstancePtr, LEVEL_FATAL, decryptTag(tag), filename, funcname, line, pid, tid, maintid, log)
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
