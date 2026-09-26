// The constants below carry the name the C++ project's Java gives them, spelled
// the way Kotlin spells a constant: `K_PING_CHECK` there is `K_PING_CHECK` here.
// The JNI reaches a constant by the number it carries and not by its name, so
// nothing on the Rust side had to change with them.

package io.github.orangeboychen.marsrs.xlog

/**
 * The Kotlin face of `libmarsxlog.so` (crate `mars-jni`).
 *
 * Every [external] here is one of the
 * `Java_io_github_orangeboychen_marsrs_xlog_Xlog_*` symbols of that crate, and
 * the field names of [XLogConfig] are the ones its `config_from_java` reads, so
 * the two must be changed together. Two things about the Kotlin are load
 * bearing and easy to break:
 *
 * * `@JvmStatic` on a companion function is what puts a `static` on [Xlog]
 *   itself — the class JNI looks the symbol up under. Without it the native
 *   lands on `Xlog$Companion` and the name JNI wants is
 *   `..._Xlog_00024Companion_*`, which nobody exports.
 * * `@JvmField` on a property is what leaves it a field of the name the Rust
 *   asks for, instead of a getter over a private one.
 *
 * The library is loaded by [open]: it is `marsxlog`, the `crate-name` of
 * `mars-jni`.
 *
 * The API is the C++ project's `Xlog`, minus what that one needed to load
 * (`c++_shared`) and plus what the port added (`setLogLevel`);
 * [COMPRESS_LEVEL1]..[COMPRESS_LEVEL9] are the C++'s own names for the levels a
 * compressed appender can be opened with.
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

        /**
         * Loads `libmarsxlog.so` and opens the process-wide appender.
         *
         * @param isLoadLib whether to load the library here; pass `false` when
         *                  the app has loaded it already
         */
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
