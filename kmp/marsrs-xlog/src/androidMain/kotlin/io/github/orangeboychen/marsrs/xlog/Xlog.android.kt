package io.github.orangeboychen.marsrs.xlog

import java.util.concurrent.ConcurrentHashMap

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
        get() = handle != NO_HANDLE && handle == openHandles[namePrefix]

    public actual var level: LogLevel
        get() = LogLevel.of(getLogLevel(requireOpen()))
        set(value) = setLogLevel(requireOpen(), value.ordinal)

    public actual var mode: AppenderMode
        get() = currentMode
        set(value) {
            setAppenderMode(requireOpen(), value.ordinal)
            currentMode = value
        }

    public actual fun isLoggable(level: LogLevel): Boolean =
        isOpen && LogLevel.of(getLogLevel(handle)).isEnabledFor(level)

    public actual fun log(level: LogLevel, tag: String, message: String) {
        if (isOpen) {
            write(handle, level.ordinal, tag, message)
        }
    }

    public actual fun v(tag: String, message: String) = log(LogLevel.VERBOSE, tag, message)

    public actual fun d(tag: String, message: String) = log(LogLevel.DEBUG, tag, message)

    public actual fun i(tag: String, message: String) = log(LogLevel.INFO, tag, message)

    public actual fun w(tag: String, message: String) = log(LogLevel.WARNING, tag, message)

    public actual fun e(tag: String, message: String) = log(LogLevel.ERROR, tag, message)

    public actual fun f(tag: String, message: String) = log(LogLevel.FATAL, tag, message)

    public actual fun flush(sync: Boolean) {
        if (isOpen) {
            appenderFlush(handle, sync)
        }
    }

    public actual fun close() {
        if (!isOpen) {
            return
        }
        releaseXlogInstance(namePrefix)
        // The appender is the prefix's and not this wrapper's: `marsrs-jni`
        // answers an [Xlog] of the same prefix with the same handle, so every one
        // of them is closed with this one.
        openHandles.remove(namePrefix, handle)
        handle = NO_HANDLE
    }

    /**
     * The handle of this appender, or [IllegalStateException] when there is none
     * left to forward: no handle is the process-wide appender to `marsrs-jni`, so
     * a closed [Xlog] that handed it on would read and move the appender every
     * other part of the app writes through, and read a level that is not its own.
     */
    private fun requireOpen(): Long {
        check(isOpen) {
            "no appender of this Xlog is open ('$namePrefix'): build another Xlog(XlogConfig(...)) to log again"
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

    private external fun appenderFlush(handle: Long, sync: Boolean)

    private external fun getLogLevel(handle: Long): Int

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

    private companion object {
        @JvmStatic
        private external fun write(handle: Long, level: Int, tag: String, message: String)

        /** `marsrsxlog` — the `crate-name` of `marsrs-jni`, i.e. the library [write] and the rest live in. */
        const val LIBRARY = "marsrsxlog"

        /**
         * The handle of every appender this process has open, by the prefix it was
         * opened with: what tells an [Xlog] that the appender it shares with
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
 * about the class is looked at: it is not the `XLogConfig` of `android/marsrs`,
 * whose fields the same Rust reads under the same names.
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
