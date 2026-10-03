package io.github.orangeboychen.marsrs.xlog

import java.util.concurrent.ConcurrentHashMap
import kotlin.concurrent.Volatile
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * The Android `actual`: `libmarsrsxlog.so`, the JNI bridge of `crates/marsrs-jni`.
 *
 * The class is called `Xlog` and lives in this package because that is what the
 * symbols the Rust exports are called —
 * `Java_io_github_orangeboychen_marsrs_xlog_Xlog_newXlogInstance` and its
 * siblings — so neither the name nor the package can change on its own. The same
 * is true of the names of [XLogConfigJni]'s fields, which are the ones its
 * `config_from_java` reads.
 *
 * It is the `Xlog` of the Android AAR: the same constructor, the same six
 * helpers, the same handle-per-appender. The one call that differs is the write:
 * `write` is a `@JvmStatic` of the companion because `marsrs-jni` takes the
 * handle as an argument rather than as a receiver, and JNI resolves a method by
 * the name the class file carries — a `Xlog$Companion` of its own is a symbol
 * `marsrs-jni` does not export.
 */
public actual class Xlog actual constructor(config: XlogConfig) {
    public actual val namePrefix: String = config.namePrefix

    public actual var consoleLogEnabled: Boolean = false
        set(value) {
            setConsoleLogOpen(requireOpen(), value)
            field = value
        }

    public actual var maxFileSizeBytes: Long = NO_FILE_SIZE_LIMIT
        set(value) {
            val size = value.coerceAtLeast(NO_FILE_SIZE_LIMIT)
            setMaxFileSize(requireOpen(), size)
            field = size
        }

    public actual var maxAliveTimeSeconds: Long = NO_ALIVE_TIME_LIMIT
        set(value) {
            val seconds = value.coerceAtLeast(NO_ALIVE_TIME_LIMIT)
            setMaxAliveTime(requireOpen(), seconds)
            field = seconds
        }

    /**
     * The handle [close] takes away, and the one every member is forwarded
     * through. Volatile because [close] may run on another thread than the
     * writes it stops: a non-volatile `Long` is two 32-bit stores to the
     * memory model, so a writer thread can read a handle that is half of the
     * old one and half of the new.
     */
    @Volatile
    private var handle: Long = NO_HANDLE

    private var currentMode: AppenderMode = config.mode

    init {
        // `marsrsxlog`, the `crate-name` of `marsrs-jni`: loaded before the first
        // symbol of it is called, and loading it twice is nothing.
        System.loadLibrary(LIBRARY)
        val opened = newXlogInstance(config.toNative())
        require(opened != NO_HANDLE) {
            "marsrs-jni opened no appender for ${config.namePrefix} in ${config.logDir}"
        }
        handle = opened
        openHandles[namePrefix] = opened
    }

    public actual val isOpen: Boolean
        get() = openHandle() != NO_HANDLE

    public actual var level: LogLevel
        get() = LogLevel.of(getLogLevel(requireOpen()))
        set(value) = setLogLevel(requireOpen(), value.ordinal)

    public actual var mode: AppenderMode
        get() = currentMode
        set(value) {
            setAppenderMode(requireOpen(), value.ordinal)
            currentMode = value
        }

    public actual val currentLogPath: String?
        get() {
            val opened = openHandle()
            return if (opened == NO_HANDLE) null else getCurrentLogPath(opened)
        }

    public actual fun logFiles(daysAgo: Long): List<String> {
        val opened = openHandle()
        return if (opened == NO_HANDLE) emptyList() else logFiles(opened, daysAgo)?.toList().orEmpty()
    }

    public actual fun logFileNames(daysAgo: Long): List<String> {
        val opened = openHandle()
        return if (opened == NO_HANDLE) emptyList() else logFileNames(opened, daysAgo)?.toList().orEmpty()
    }

    public actual fun isLoggable(level: LogLevel): Boolean {
        val opened = openHandle()
        return opened != NO_HANDLE && LogLevel.of(getLogLevel(opened)).isEnabledFor(level)
    }

    public actual fun log(level: LogLevel, tag: String, message: String) {
        val opened = openHandle()
        if (opened != NO_HANDLE) {
            write(opened, level.ordinal, tag, message)
        }
    }

    public actual fun v(tag: String, message: String) = log(LogLevel.VERBOSE, tag, message)

    public actual fun d(tag: String, message: String) = log(LogLevel.DEBUG, tag, message)

    public actual fun i(tag: String, message: String) = log(LogLevel.INFO, tag, message)

    public actual fun w(tag: String, message: String) = log(LogLevel.WARNING, tag, message)

    public actual fun e(tag: String, message: String) = log(LogLevel.ERROR, tag, message)

    public actual fun f(tag: String, message: String) = log(LogLevel.FATAL, tag, message)

    public actual fun requestFlush() {
        val opened = openHandle()
        if (opened != NO_HANDLE) {
            appenderRequestFlush(opened)
        }
    }

    public actual fun flushNow() {
        val opened = openHandle()
        if (opened != NO_HANDLE) {
            appenderFlushNow(opened)
        }
    }

    public actual suspend fun flush() = withContext(Dispatchers.IO) {
        flushNow()
    }

    @Synchronized
    public actual fun close() {
        // Remove the exact handle from the process-local ownership table before
        // releasing it. The native call repeats that identity check under the
        // registry lock, so a reopen cannot be closed by a stale Xlog.
        if (!openHandles.remove(namePrefix, handle)) {
            handle = NO_HANDLE
            return
        }
        releaseXlogInstanceOf(namePrefix, handle)
        handle = NO_HANDLE
    }

    /**
     * The handle of this appender, or [IllegalStateException] when there is
     * none left to forward: a handle whose appender is gone is a no-op to
     * `marsrs-jni`, so a closed [Xlog] that handed it on would silently write
     * nothing, and read a level that is not its own.
     */
    private fun requireOpen(): Long {
        val opened = openHandle()
        check(opened != NO_HANDLE) {
            "no appender of this Xlog is open ('$namePrefix'): Xlog.open(XlogConfig(...)) another to log again"
        }
        return opened
    }

    /**
     * The handle of this appender, [NO_HANDLE] when it is closed, read once:
     * [handle] is what the `open` of this class was answered, and the table of
     * [openHandles] is what says the handle is still this [Xlog]'s — an [Xlog]
     * of the same [namePrefix] is closed with this one, and the table is where
     * that shows.
     *
     * One read and not two, because the two it replaces are not one answer:
     * `isOpen` and then `handle` is a window a [close] on another thread
     * lands in, and what comes out of it is the [NO_HANDLE] that [close]
     * wrote. A handle of no appender is a no-op to `marsrs-jni`, so the level
     * a wrapper read through it is the one it answers for nothing — which
     * [LogLevel.of] reads as `VERBOSE`, the level that logs everything: an
     * [Xlog] that was closed answered the opposite of what it was asked. A
     * setter handed no handle is quieter and no better: it moves nothing and
     * still answers that it took the setting. The Kotlin/Native `actual` of
     * this `expect` reads it once the same way.
     */
    private fun openHandle(): Long = handle.takeIf { it != NO_HANDLE && it == openHandles[namePrefix] } ?: NO_HANDLE

    // The names `marsrs-jni` exports, and the signatures it reads them under.
    //
    // `private` and not `internal`, and that is the whole point of these lines:
    // Kotlin mangles the name of an `internal` function — `newXlogInstance`
    // becomes `newXlogInstance$mars_xlog_release` in the bytecode, which is a
    // name no symbol of the Rust carries, and JNI resolves the method by the name
    // in the class file. `private` is not mangled, and visibility is nothing JNI
    // asks about.
    private external fun newXlogInstance(config: XLogConfigJni): Long

    private external fun releaseXlogInstanceOf(namePrefix: String, instance: Long)

    private external fun appenderRequestFlush(handle: Long)

    private external fun appenderFlushNow(handle: Long)

    private external fun getLogLevel(handle: Long): Int

    private external fun getCurrentLogPath(handle: Long): String?

    private external fun logFiles(handle: Long, timespan: Long): Array<String>?

    private external fun logFileNames(handle: Long, timespan: Long): Array<String>?

    private external fun setLogLevel(handle: Long, level: Int)

    private external fun setAppenderMode(handle: Long, mode: Int)

    private external fun setConsoleLogOpen(handle: Long, open: Boolean)

    private external fun setMaxFileSize(handle: Long, bytes: Long)

    private external fun setMaxAliveTime(handle: Long, seconds: Long)

    /** What `marsrs-jni` reads out of the config [newXlogInstance] is handed. */
    private fun XlogConfig.toNative(): XLogConfigJni = XLogConfigJni().apply {
        level = this@toNative.level.ordinal
        mode = this@toNative.mode.ordinal
        logdir = this@toNative.logDir
        nameprefix = this@toNative.namePrefix
        pubkey = this@toNative.pubKey
        compressmode = this@toNative.compressMode.ordinal
        compresslevel = this@toNative.compressLevel
        cachedir = this@toNative.cacheDir
        cachedays = this@toNative.cacheDays
    }

    public actual companion object {
        /**
         * Opens an appender of its own: the constructor of this actual, under the
         * one name every platform of the port opens one with.
         *
         * `@JvmStatic` is what puts a `static` on `Xlog` itself — the class the
         * symbols of `marsrs-jni` live beside — so a Java caller writes
         * `Xlog.open(config)` and not `Xlog.Companion.open(config)`.
         */
        @JvmStatic
        public actual fun open(config: XlogConfig): Xlog = Xlog(config)

        @JvmStatic
        private external fun write(handle: Long, level: Int, tag: String, message: String)

        /** `marsrsxlog` — the `crate-name` of `marsrs-jni`, i.e. the library [write] and the rest live in. */
        const val LIBRARY = "marsrsxlog"

        /**
         * The handle of every appender this process has open, by the prefix it was
         * opened with: what tells an [Xlog] that the appender it shares with
         * another [Xlog] of the same [namePrefix] has been closed, which the
         * handle alone cannot — `marsrs-jni` answers both with the same one.
         *
         * `internal`, because what [openHandle] trusts it to say is whether the
         * appender of this [Xlog] is still its own, and a caller that cleared
         * it or wrote a `0` into it would take every setter to an
         * `IllegalStateException`, `log` to writing nothing, and `close` to
         * skipping the release. The other `actual` keeps no table at all:
         * there, the registry behind `mars_xlog_get_instance` is the answer,
         * and it is not reachable from Kotlin.
         */
        internal val openHandles: ConcurrentHashMap<String, Long> = ConcurrentHashMap()

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
 * about the class is looked at: it is not the `XLogConfig` of
 * `platforms/android/marsrs`, whose fields the same Rust reads under the same
 * names.
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
