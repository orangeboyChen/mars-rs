// The Android half of `mars-rs-react-native-xlog`: the six methods of the
// `MarsRsXlog` native module, each of them a straight call of a member of
// `Xlog` — the Kotlin face of `libmarsxlog.so` in the `mars-rs-xlog` AAR.
// `mars-rs-react-native` is the same module over the `mars-rs` AAR — the whole
// port, which today is the same xlog — and it is the one an app takes when it
// wants more than logging.
//
// `Xlog` is the same class `android/mars-xlog` publishes, so what this file is
// is a bridge over an API that already exists, and the API it bridges is the
// instance one: `Xlog.open` opens the *process-wide* appender with the
// compression and the cache the C++'s Java hard-codes, which is why
// `newXlogInstance` is what `open` below calls — a caller who asked for zstd
// would otherwise get zlib. It is the instance `MarsXlogInstance` of
// `Sources/MarsRSXlog/Xlog.swift` is, so the TypeScript surface and the Swift
// one come out the same shape.
//
// Every key is one `src/index.ts` sent, and every number is the one
// `mars_xlog.h` gives a level, a mode and a compression.

package io.github.orangeboychen.marsrs.xlog.reactnative

import android.os.Looper
import android.os.Process
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import com.facebook.react.bridge.ReadableMap
import io.github.orangeboychen.marsrs.xlog.Xlog

/** The Android half of `MarsRsXlog`. */
class MarsRsXlogModule(reactContext: ReactApplicationContext) : ReactContextBaseJavaModule(reactContext) {

    /** The instance `Xlog.newXlogInstance` gave; `0` before `open`, after `close`. */
    private var instance: Long = 0

    /** What `close` releases the instance by: the prefix it was opened with. */
    private var namePrefix: String = ""

    private val xlog = Xlog()

    override fun getName(): String = NAME

    /** `Xlog.newXlogInstance`, over the configuration `src/index.ts` sent. */
    @ReactMethod
    fun open(config: ReadableMap, promise: Promise) {
        val logDirectory = config.string("logDirectory")
        if (logDirectory.isEmpty()) {
            promise.reject(ERROR, "logDirectory is empty")
            return
        }
        val xlogConfig = Xlog.XLogConfig().apply {
            level = config.int("level", Xlog.LEVEL_INFO)
            mode = config.int("mode", Xlog.APPENDER_MODE_ASYNC)
            logdir = logDirectory
            nameprefix = config.string("namePrefix")
            pubkey = config.string("publicKey")
            compressmode = config.int("compression", Xlog.ZLIB_MODE)
            compresslevel = config.int("compressionLevel")
            cachedir = config.optionalString("cacheDirectory")
            cachedays = config.int("cacheDays")
        }
        // `0` is the answer for a configuration the appender refused, and an
        // instance the caller has no handle to is a caller who would write
        // through the process-wide appender instead.
        val handle = xlog.newXlogInstance(xlogConfig)
        if (handle == 0L) {
            promise.reject(ERROR, "the appender refused the configuration")
            return
        }
        instance = handle
        namePrefix = xlogConfig.nameprefix.orEmpty()
        promise.resolve(null)
    }

    /** `Xlog.logWrite2`. The file, the function and the line are left empty:
     * there is no JS frame to name, and the C++ writes an empty one too. */
    @ReactMethod
    fun write(level: Int, tag: String, message: String, promise: Promise) {
        Xlog.logWrite2(
            instance,
            level,
            tag,
            "",
            "",
            0,
            Process.myPid(),
            Process.myTid().toLong(),
            Looper.getMainLooper().thread.id,
            message
        )
        promise.resolve(null)
    }

    /** `Xlog.appenderFlush`. */
    @ReactMethod
    fun flush(sync: Boolean, promise: Promise) {
        xlog.appenderFlush(instance, sync)
        promise.resolve(null)
    }

    /** `Xlog.setLogLevel`. */
    @ReactMethod
    fun setLevel(level: Int, promise: Promise) {
        xlog.setLogLevel(instance, level)
        promise.resolve(null)
    }

    /** `Xlog.setConsoleLogOpen`. */
    @ReactMethod
    fun setConsoleLog(enabled: Boolean, promise: Promise) {
        xlog.setConsoleLogOpen(instance, enabled)
        promise.resolve(null)
    }

    /** `Xlog.releaseXlogInstance`: closes the appender `open` made. */
    @ReactMethod
    fun close(promise: Promise) {
        xlog.releaseXlogInstance(namePrefix)
        instance = 0
        namePrefix = ""
        promise.resolve(null)
    }

    companion object {
        /** The name `src/index.ts` reads the module out of `NativeModules` by. */
        private const val NAME = "MarsRsXlog"

        /** What every `promise.reject` answers with. */
        private const val ERROR = "mars_rs_xlog"

        // `libmarsxlog.so` is loaded here and not by `Xlog`: the AAR's class
        // loads it in `Xlog.open(isLoadLib = true)` and nowhere else, and what
        // this module bridges is the *instance* API, which never calls `open`
        // — so on a process that has opened no process-wide appender,
        // `newXlogInstance` would be the first native call of the process and
        // would throw `UnsatisfiedLinkError`. Loading a library the process has
        // already loaded is a no-op, which is what lets the module sit beside
        // an app that loads the library itself.
        init {
            System.loadLibrary("marsxlog")
        }
    }
}

/** The number sent for `key`, or [fallback] when none was sent. A field
 * `src/index.ts` left out is not one it sent as `null`. */
private fun ReadableMap.int(key: String, fallback: Int = 0): Int = if (hasKey(key)) getInt(key) else fallback

/** The string sent for `key`, or the empty one. */
private fun ReadableMap.string(key: String): String = if (hasKey(key)) getString(key).orEmpty() else ""

/** The string sent for `key`, or `null` — `cacheDirectory` is the one field a
 * caller may leave out, and `NULL` is what `mars_xlog.h` reads as "empty". */
private fun ReadableMap.optionalString(key: String): String? = if (hasKey(key)) getString(key) else null
