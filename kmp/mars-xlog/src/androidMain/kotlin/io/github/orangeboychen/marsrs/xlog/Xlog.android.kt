package io.github.orangeboychen.marsrs.xlog

import android.os.Looper
import android.os.Process

/**
 * The Android `actual`: `libmarsxlog.so`, the JNI bridge of `crates/mars-jni`.
 *
 * The object is called `Xlog` and lives in this package because that is what
 * the symbols the Rust exports are called —
 * `Java_io_github_orangeboychen_marsrs_xlog_Xlog_appenderOpen` and its thirteen
 * siblings — so neither the name nor the package can change on its own. The
 * same is true of the names of [XLogConfigJni]'s fields, which are the ones its
 * `config_from_java` reads.
 *
 * A member of an `object` is a method of `Xlog` itself and not of a
 * `Xlog$Companion`, which is why none of them needs `@JvmStatic`: JNI resolves
 * a method by the name the class file carries, and the name of a member of an
 * `object` is the one the Rust wrote either way.
 */
public actual object Xlog {
    init {
        // `marsxlog`, the `crate-name` of `mars-jni`: loaded when the object is
        // first touched, which is what an `object` is for.
        System.loadLibrary("marsxlog")
    }

    public actual fun open(config: XlogConfig) {
        appenderOpen(
            XLogConfigJni().apply {
                level = config.level.ordinal
                mode = config.mode.ordinal
                logdir = config.logDir
                nameprefix = config.namePrefix
                pubkey = config.pubKey.orEmpty()
                compressmode = config.compressMode.ordinal
                compresslevel = config.compressLevel
                cachedir = config.cacheDir
                cachedays = config.cacheDays
            }
        )
    }

    public actual fun write(level: LogLevel, tag: String, message: String, file: String, function: String, line: Int) {
        logWrite2(
            LOG_INSTANCE,
            level.ordinal,
            tag,
            file,
            function,
            line,
            Process.myPid(),
            Thread.currentThread().id,
            Looper.getMainLooper().thread.id,
            message
        )
    }

    public actual fun flush(sync: Boolean) {
        appenderFlush(LOG_INSTANCE, sync)
    }

    public actual fun close() {
        appenderClose()
    }

    public actual fun setLevel(level: LogLevel) {
        setLogLevel(LOG_INSTANCE, level.ordinal)
    }

    public actual fun setConsoleLog(open: Boolean) {
        setConsoleLogOpen(LOG_INSTANCE, open)
    }

    public actual fun setMaxFileSize(bytes: Long) {
        setMaxFileSize(LOG_INSTANCE, bytes)
    }

    public actual fun setMaxAliveTime(seconds: Long) {
        setMaxAliveTime(LOG_INSTANCE, seconds)
    }

    // The names `mars-jni` exports, and the signatures it reads them under.
    //
    // `private` and not `internal`, and that is the whole point of these eight
    // lines: Kotlin mangles the name of an `internal` function — `appenderOpen`
    // becomes `appenderOpen$mars_xlog_release` in the bytecode, which is a name
    // no symbol of the Rust carries, and JNI resolves the method by the name in
    // the class file. `private` is not mangled, and visibility is nothing JNI
    // asks about: the C++ project's Java declares the same symbol `private`
    // (in a companion, under `@JvmStatic`) for the same reason.
    private external fun appenderOpen(config: XLogConfigJni)

    private external fun appenderClose()

    private external fun appenderFlush(logInstancePtr: Long, isSync: Boolean)

    private external fun logWrite2(
        logInstancePtr: Long,
        level: Int,
        tag: String,
        filename: String,
        funcname: String,
        line: Int,
        pid: Int,
        tid: Long,
        maintid: Long,
        log: String
    )

    private external fun setLogLevel(logInstancePtr: Long, level: Int)

    private external fun setConsoleLogOpen(logInstancePtr: Long, isOpen: Boolean)

    private external fun setMaxFileSize(logInstancePtr: Long, size: Long)

    private external fun setMaxAliveTime(logInstancePtr: Long, seconds: Long)

    // The instance the C++ project's `Log` passes for a process-wide write: `0`,
    // the appender `mars_xlog_open` opened, which is what the JNI bridge takes
    // it for too.
    private const val LOG_INSTANCE = 0L
}

/**
 * What `mars-jni` reads out of the object [Xlog.appenderOpen] hands it.
 *
 * The names of the fields are the ones its `config_from_java` asks for, and
 * `@JvmField` is what leaves each of them a field of that name instead of a
 * getter over a private one — without it `GetFieldID` finds nothing. Nothing
 * else about the class is looked at: it is not the `XLogConfig` of
 * `android/mars-core`, whose fields the same Rust reads under the same names.
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
