package io.github.orangeboychen.marsrs.xlog

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.widget.Toast

/**
 * The facade the C++ project's `Log.java` is: `Log.d(tag, msg)` and friends,
 * over whichever [LogImp] the app handed to [setLogImp] — [Xlog] here, and a
 * plain `android.util.Log` until it does.
 *
 * It is an `object` and not a class of statics, but every member is
 * `@JvmStatic`: an app that calls this from Java writes `Log.d(...)` and not
 * `Log.INSTANCE.d(...)`.
 */
object Log {
    private const val TAG = "mars.xlog.log"

    const val LEVEL_VERBOSE = 0
    const val LEVEL_DEBUG = 1
    const val LEVEL_INFO = 2
    const val LEVEL_WARNING = 3
    const val LEVEL_ERROR = 4
    const val LEVEL_FATAL = 5
    const val LEVEL_NONE = 6

    // defaults to LEVEL_NONE
    private var level: Int = LEVEL_NONE

    @JvmField
    var toastSupportContext: Context? = null

    interface LogImp {
        fun logV(
            logInstancePtr: Long,
            tag: String,
            filename: String,
            funcname: String,
            linuxTid: Int,
            pid: Int,
            tid: Long,
            maintid: Long,
            log: String
        )

        fun logI(
            logInstancePtr: Long,
            tag: String,
            filename: String,
            funcname: String,
            linuxTid: Int,
            pid: Int,
            tid: Long,
            maintid: Long,
            log: String
        )

        fun logD(
            logInstancePtr: Long,
            tag: String,
            filename: String,
            funcname: String,
            linuxTid: Int,
            pid: Int,
            tid: Long,
            maintid: Long,
            log: String
        )

        fun logW(
            logInstancePtr: Long,
            tag: String,
            filename: String,
            funcname: String,
            linuxTid: Int,
            pid: Int,
            tid: Long,
            maintid: Long,
            log: String
        )

        fun logE(
            logInstancePtr: Long,
            tag: String,
            filename: String,
            funcname: String,
            linuxTid: Int,
            pid: Int,
            tid: Long,
            maintid: Long,
            log: String
        )

        fun logF(
            logInstancePtr: Long,
            tag: String,
            filename: String,
            funcname: String,
            linuxTid: Int,
            pid: Int,
            tid: Long,
            maintid: Long,
            log: String
        )

        fun getLogLevel(logInstancePtr: Long): Int

        fun setAppenderMode(logInstancePtr: Long, mode: Int)

        fun openLogInstance(
            level: Int,
            mode: Int,
            cacheDir: String,
            logDir: String,
            nameprefix: String,
            cacheDays: Int
        ): Long

        fun getXlogInstance(nameprefix: String): Long

        fun releaseXlogInstance(nameprefix: String)

        fun appenderOpen(level: Int, mode: Int, cacheDir: String, logDir: String, nameprefix: String, cacheDays: Int)

        fun appenderClose()

        fun appenderFlush(logInstancePtr: Long, isSync: Boolean)

        fun setConsoleLogOpen(logInstancePtr: Long, isOpen: Boolean)

        fun setMaxFileSize(logInstancePtr: Long, aliveSeconds: Long)

        fun setMaxAliveTime(logInstancePtr: Long, aliveSeconds: Long)
    }

    private val debugLog: LogImp = object : LogImp {
        private val handler = Handler(Looper.getMainLooper())

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
            if (level <= LEVEL_VERBOSE) {
                android.util.Log.v(tag, log)
            }
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
            if (level <= LEVEL_INFO) {
                android.util.Log.i(tag, log)
            }
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
            if (level <= LEVEL_DEBUG) {
                android.util.Log.d(tag, log)
            }
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
            if (level <= LEVEL_WARNING) {
                android.util.Log.w(tag, log)
            }
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
            if (level <= LEVEL_ERROR) {
                android.util.Log.e(tag, log)
            }
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
            if (level > LEVEL_FATAL) {
                return
            }
            android.util.Log.e(tag, log)
            toastSupportContext?.let { context ->
                handler.post { Toast.makeText(context, log, Toast.LENGTH_LONG).show() }
            }
        }

        override fun getLogLevel(logInstancePtr: Long): Int = level

        override fun setAppenderMode(logInstancePtr: Long, mode: Int) {}

        override fun openLogInstance(
            level: Int,
            mode: Int,
            cacheDir: String,
            logDir: String,
            nameprefix: String,
            cacheDays: Int
        ): Long = 0

        override fun getXlogInstance(nameprefix: String): Long = 0

        override fun releaseXlogInstance(nameprefix: String) {}

        override fun appenderOpen(
            level: Int,
            mode: Int,
            cacheDir: String,
            logDir: String,
            nameprefix: String,
            cacheDays: Int
        ) {
        }

        override fun appenderClose() {}

        override fun appenderFlush(logInstancePtr: Long, isSync: Boolean) {}

        override fun setConsoleLogOpen(logInstancePtr: Long, isOpen: Boolean) {}

        override fun setMaxAliveTime(logInstancePtr: Long, aliveSeconds: Long) {}

        override fun setMaxFileSize(logInstancePtr: Long, aliveSeconds: Long) {}
    }

    private var logImp: LogImp? = debugLog

    @JvmStatic
    fun setLogImp(imp: LogImp?) {
        logImp = imp
    }

    @JvmStatic
    fun getImpl(): LogImp? = logImp

    @JvmStatic
    fun appenderOpen(level: Int, mode: Int, cacheDir: String, logDir: String, nameprefix: String, cacheDays: Int) {
        logImp?.appenderOpen(level, mode, cacheDir, logDir, nameprefix, cacheDays)
    }

    @JvmStatic
    fun appenderClose() {
        logImp?.let { imp ->
            imp.appenderClose()
            for (prefix in sLogInstanceMap.keys.toList()) {
                closeLogInstance(prefix)
            }
        }
    }

    @JvmStatic
    fun appenderFlush() {
        logImp?.let { imp ->
            imp.appenderFlush(0L, false)
            for (instance in sLogInstanceMap.values.toList()) {
                instance.appenderFlush()
            }
        }
    }

    @JvmStatic
    fun appenderFlushSync(isSync: Boolean) {
        logImp?.appenderFlush(0L, isSync)
    }

    @JvmStatic
    fun getLogLevel(): Int = logImp?.getLogLevel(0L) ?: LEVEL_NONE

    @JvmStatic
    fun setLevel(level: Int, jni: Boolean) {
        this.level = level
        android.util.Log.w(TAG, "new log level: $level")
        if (jni) {
            android.util.Log.e(TAG, "no jni log level support")
        }
    }

    @JvmStatic
    fun setConsoleLogOpen(isOpen: Boolean) {
        logImp?.setConsoleLogOpen(0L, isOpen)
    }

    /** use `f(tag, format, obj)` instead */
    @JvmStatic
    fun f(tag: String, msg: String) = f(tag, msg, *emptyArray<Any?>())

    /** use `e(tag, format, obj)` instead */
    @JvmStatic
    fun e(tag: String, msg: String) = e(tag, msg, *emptyArray<Any?>())

    /** use `w(tag, format, obj)` instead */
    @JvmStatic
    fun w(tag: String, msg: String) = w(tag, msg, *emptyArray<Any?>())

    /** use `i(tag, format, obj)` instead */
    @JvmStatic
    fun i(tag: String, msg: String) = i(tag, msg, *emptyArray<Any?>())

    /** use `d(tag, format, obj)` instead */
    @JvmStatic
    fun d(tag: String, msg: String) = d(tag, msg, *emptyArray<Any?>())

    /** use `v(tag, format, obj)` instead */
    @JvmStatic
    fun v(tag: String, msg: String) = v(tag, msg, *emptyArray<Any?>())

    @JvmStatic
    fun f(tag: String, format: String, vararg obj: Any?) {
        val imp = logImp
        if (imp != null && imp.getLogLevel(0L) <= LEVEL_FATAL) {
            val log = if (obj.isEmpty()) format else String.format(format, *obj)
            imp.logF(
                0L,
                tag,
                "",
                "",
                0,
                Process.myPid(),
                Thread.currentThread().id,
                Looper.getMainLooper().thread.id,
                log
            )
        }
    }

    @JvmStatic
    fun e(tag: String, format: String, vararg obj: Any?) {
        val imp = logImp
        if (imp != null && imp.getLogLevel(0L) <= LEVEL_ERROR) {
            val log = (if (obj.isEmpty()) format else String.format(format, *obj)) ?: ""
            imp.logE(
                0L,
                tag,
                "",
                "",
                0,
                Process.myPid(),
                Thread.currentThread().id,
                Looper.getMainLooper().thread.id,
                log
            )
        }
    }

    @JvmStatic
    fun w(tag: String, format: String, vararg obj: Any?) {
        val imp = logImp
        if (imp != null && imp.getLogLevel(0L) <= LEVEL_WARNING) {
            val log = (if (obj.isEmpty()) format else String.format(format, *obj)) ?: ""
            imp.logW(
                0L,
                tag,
                "",
                "",
                0,
                Process.myPid(),
                Thread.currentThread().id,
                Looper.getMainLooper().thread.id,
                log
            )
        }
    }

    @JvmStatic
    fun i(tag: String, format: String, vararg obj: Any?) {
        val imp = logImp
        if (imp != null && imp.getLogLevel(0L) <= LEVEL_INFO) {
            val log = (if (obj.isEmpty()) format else String.format(format, *obj)) ?: ""
            imp.logI(
                0L,
                tag,
                "",
                "",
                0,
                Process.myPid(),
                Thread.currentThread().id,
                Looper.getMainLooper().thread.id,
                log
            )
        }
    }

    @JvmStatic
    fun d(tag: String, format: String, vararg obj: Any?) {
        val imp = logImp
        if (imp != null && imp.getLogLevel(0L) <= LEVEL_DEBUG) {
            val log = (if (obj.isEmpty()) format else String.format(format, *obj)) ?: ""
            imp.logD(
                0L,
                tag,
                "",
                "",
                0,
                Process.myPid(),
                Thread.currentThread().id,
                Looper.getMainLooper().thread.id,
                log
            )
        }
    }

    @JvmStatic
    fun v(tag: String, format: String, vararg obj: Any?) {
        val imp = logImp
        if (imp != null && imp.getLogLevel(0L) <= LEVEL_VERBOSE) {
            val log = (if (obj.isEmpty()) format else String.format(format, *obj)) ?: ""
            imp.logV(
                0L,
                tag,
                "",
                "",
                0,
                Process.myPid(),
                Thread.currentThread().id,
                Looper.getMainLooper().thread.id,
                log
            )
        }
    }

    @JvmStatic
    fun printErrStackTrace(tag: String, tr: Throwable, format: String, vararg obj: Any?) {
        val imp = logImp
        if (imp != null && imp.getLogLevel(0L) <= LEVEL_ERROR) {
            var log = (if (obj.isEmpty()) format else String.format(format, *obj)) ?: ""
            log += "  " + android.util.Log.getStackTraceString(tr)
            imp.logE(
                0L,
                tag,
                "",
                "",
                0,
                Process.myPid(),
                Thread.currentThread().id,
                Looper.getMainLooper().thread.id,
                log
            )
        }
    }

    private val sLogInstanceMap: MutableMap<String, LogInstance> = HashMap()

    @JvmStatic
    fun openLogInstance(
        level: Int,
        mode: Int,
        cacheDir: String,
        logDir: String,
        nameprefix: String,
        cacheDays: Int
    ): LogInstance? = synchronized(sLogInstanceMap) {
        sLogInstanceMap[nameprefix] ?: LogInstance(level, mode, cacheDir, logDir, nameprefix, cacheDays)
            .also { sLogInstanceMap[nameprefix] = it }
    }

    @JvmStatic
    fun closeLogInstance(prefix: String) {
        synchronized(sLogInstanceMap) {
            logImp?.let { imp ->
                sLogInstanceMap.remove(prefix)?.let { instance ->
                    imp.releaseXlogInstance(prefix)
                    instance.mLogInstancePtr = 0
                }
            }
        }
    }

    @JvmStatic
    fun getLogInstance(prefix: String): LogInstance? = synchronized(sLogInstanceMap) { sLogInstanceMap[prefix] }

    class LogInstance internal constructor(
        level: Int,
        mode: Int,
        cacheDir: String,
        logDir: String,
        nameprefix: String,
        cacheDays: Int
    ) {
        internal var mLogInstancePtr: Long = 0

        private var mPrefix: String? = null

        init {
            logImp?.let { imp ->
                mLogInstancePtr = imp.openLogInstance(level, mode, cacheDir, logDir, nameprefix, cacheDays)
                mPrefix = nameprefix
            }
        }

        fun f(tag: String, format: String, vararg obj: Any?) {
            val imp = logImp
            if (imp != null && getLogLevel() <= LEVEL_FATAL && mLogInstancePtr != 0L) {
                val log = if (obj.isEmpty()) format else String.format(format, *obj)
                imp.logF(
                    mLogInstancePtr,
                    tag,
                    "",
                    "",
                    Process.myTid(),
                    Process.myPid(),
                    Thread.currentThread().id,
                    Looper.getMainLooper().thread.id,
                    log
                )
            }
        }

        fun e(tag: String, format: String, vararg obj: Any?) {
            val imp = logImp
            if (imp != null && getLogLevel() <= LEVEL_ERROR && mLogInstancePtr != 0L) {
                val log = (if (obj.isEmpty()) format else String.format(format, *obj)) ?: ""
                imp.logE(
                    mLogInstancePtr,
                    tag,
                    "",
                    "",
                    Process.myTid(),
                    Process.myPid(),
                    Thread.currentThread().id,
                    Looper.getMainLooper().thread.id,
                    log
                )
            }
        }

        fun w(tag: String, format: String, vararg obj: Any?) {
            val imp = logImp
            if (imp != null && getLogLevel() <= LEVEL_WARNING && mLogInstancePtr != 0L) {
                val log = (if (obj.isEmpty()) format else String.format(format, *obj)) ?: ""
                imp.logW(
                    mLogInstancePtr,
                    tag,
                    "",
                    "",
                    Process.myTid(),
                    Process.myPid(),
                    Thread.currentThread().id,
                    Looper.getMainLooper().thread.id,
                    log
                )
            }
        }

        fun i(tag: String, format: String, vararg obj: Any?) {
            val imp = logImp
            if (imp != null && getLogLevel() <= LEVEL_INFO && mLogInstancePtr != 0L) {
                val log = (if (obj.isEmpty()) format else String.format(format, *obj)) ?: ""
                imp.logI(
                    mLogInstancePtr,
                    tag,
                    "",
                    "",
                    Process.myTid(),
                    Process.myPid(),
                    Thread.currentThread().id,
                    Looper.getMainLooper().thread.id,
                    log
                )
            }
        }

        fun d(tag: String, format: String, vararg obj: Any?) {
            val imp = logImp
            if (imp != null && getLogLevel() <= LEVEL_DEBUG && mLogInstancePtr != 0L) {
                val log = (if (obj.isEmpty()) format else String.format(format, *obj)) ?: ""
                imp.logD(
                    mLogInstancePtr,
                    tag,
                    "",
                    "",
                    Process.myTid(),
                    Process.myPid(),
                    Thread.currentThread().id,
                    Looper.getMainLooper().thread.id,
                    log
                )
            }
        }

        fun v(tag: String, format: String, vararg obj: Any?) {
            val imp = logImp
            if (imp != null && getLogLevel() <= LEVEL_VERBOSE && mLogInstancePtr != 0L) {
                val log = (if (obj.isEmpty()) format else String.format(format, *obj)) ?: ""
                imp.logV(
                    mLogInstancePtr,
                    tag,
                    "",
                    "",
                    Process.myTid(),
                    Process.myPid(),
                    Thread.currentThread().id,
                    Looper.getMainLooper().thread.id,
                    log
                )
            }
        }

        fun printErrStackTrace(tag: String, tr: Throwable, format: String, vararg obj: Any?) {
            val imp = logImp
            if (imp != null && getLogLevel() <= LEVEL_ERROR && mLogInstancePtr != 0L) {
                var log = (if (obj.isEmpty()) format else String.format(format, *obj)) ?: ""
                log += "  " + android.util.Log.getStackTraceString(tr)
                imp.logE(
                    mLogInstancePtr,
                    tag,
                    "",
                    "",
                    Process.myTid(),
                    Process.myPid(),
                    Thread.currentThread().id,
                    Looper.getMainLooper().thread.id,
                    log
                )
            }
        }

        fun appenderFlush() {
            logImp?.let { imp ->
                if (mLogInstancePtr != 0L) {
                    imp.appenderFlush(mLogInstancePtr, false)
                }
            }
        }

        fun appenderFlushSync() {
            logImp?.let { imp ->
                if (mLogInstancePtr != 0L) {
                    imp.appenderFlush(mLogInstancePtr, true)
                }
            }
        }

        fun getLogLevel(): Int =
            logImp?.let { imp -> if (mLogInstancePtr != 0L) imp.getLogLevel(mLogInstancePtr) else LEVEL_NONE }
                ?: LEVEL_NONE

        fun setConsoleLogOpen(isOpen: Boolean) {
            logImp?.let { imp ->
                if (mLogInstancePtr != 0L) {
                    imp.setConsoleLogOpen(mLogInstancePtr, isOpen)
                }
            }
        }
    }
}
