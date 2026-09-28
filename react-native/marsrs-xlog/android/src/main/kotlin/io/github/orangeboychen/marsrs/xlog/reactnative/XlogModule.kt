// The Android half of `marsrs-react-native-xlog`: the eleven methods of the
// `Xlog` native module, each of them a straight call of a member of `Xlog` — the
// Kotlin face of `libmarsrsxlog.so` in the `marsrs-xlog` AAR, and the same class
// `kmp/marsrs-xlog` publishes to a Kotlin Multiplatform app and
// `Sources/MarsRSXlog/Xlog.swift` to a Swift one. So what this file is is a
// bridge over an API that already exists, and nothing of the API is invented
// here: `Xlog(XlogConfig(...))`, `log`, `isLoggable`, `flush`, `close`, and the
// five settings, under the names every other platform of the port gives them.
//
// `marsrs-react-native` is the same module over the `marsrs` AAR — the whole
// port, which today is the same xlog — and this file is its Kotlin with the two
// names changed: xlog is the whole C ABI today, so the two are one package under
// two names, and they diverge the day STN and SDT land — there, and not here.
//
// Every key below is a field `src/index.ts` put there, and every number is the
// one `mars_xlog.h` gives a level, a mode and a compression. The appenders are
// kept by the prefix they were opened with because that is what an appender is
// known by in the C ABI and in `marsrs-jni`: two `Xlog`s of two prefixes are
// two appenders, and one of a prefix the module already has is the appender it
// already has.

package io.github.orangeboychen.marsrs.xlog.reactnative

import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import com.facebook.react.bridge.ReadableMap
import io.github.orangeboychen.marsrs.xlog.AppenderMode
import io.github.orangeboychen.marsrs.xlog.CompressMode
import io.github.orangeboychen.marsrs.xlog.LogLevel
import io.github.orangeboychen.marsrs.xlog.Xlog
import io.github.orangeboychen.marsrs.xlog.XlogConfig
import java.util.concurrent.ConcurrentHashMap

/** The Android half of `Xlog`. */
class XlogModule(reactContext: ReactApplicationContext) : ReactContextBaseJavaModule(reactContext) {

    /**
     * The appender of every prefix `open` has opened, by the prefix: what keeps
     * two `Xlog`s apart, which a single field cannot — the C ABI and
     * `marsrs-jni` both answer an appender by the name it was opened with.
     */
    private val appenders = ConcurrentHashMap<String, Xlog>()

    override fun getName(): String = NAME

    /**
     * `Xlog(XlogConfig(...))`: opens the appender of the configuration
     * `src/index.ts` sent.
     *
     * The constructor is what refuses a configuration the appender cannot honour
     * — a blank `logDir` or `namePrefix`, a compression level out of range — and
     * its `IllegalArgumentException` is what the caller is answered with, rather
     * than a handle it would write through the process-wide appender with.
     */
    @ReactMethod
    fun open(config: ReadableMap, promise: Promise) {
        rejectIfFailed(promise) {
            val xlogConfig = XlogConfig(
                logDir = config.string("logDir"),
                namePrefix = config.string("namePrefix").ifBlank { DEFAULT_NAME_PREFIX },
                level = LogLevel.of(config.int("level", LogLevel.INFO.ordinal)),
                mode = appenderModeOf(config.int("mode", AppenderMode.ASYNC.ordinal)),
                pubKey = config.string("pubKey"),
                compressMode = compressModeOf(config.int("compressMode", CompressMode.ZLIB.ordinal)),
                compressLevel = config.int("compressLevel", DEFAULT_COMPRESS_LEVEL),
                cacheDir = config.optionalString("cacheDir"),
                cacheDays = config.int("cacheDays", NO_CACHE_DAYS)
            )
            appenders[xlogConfig.namePrefix] = Xlog(xlogConfig)
            promise.resolve(null)
        }
    }

    /**
     * `Xlog.log`. The file, the function and the line are the C++'s own
     * defaults: there is no JS frame to name, and the C++ writes an empty one
     * too.
     */
    @ReactMethod
    fun log(namePrefix: String, level: Int, tag: String, message: String, promise: Promise) {
        rejectIfFailed(promise) {
            appender(namePrefix).log(LogLevel.of(level), tag, message)
            promise.resolve(null)
        }
    }

    /** `Xlog.isLoggable`: whether a record of the level would be written. */
    @ReactMethod
    fun isLoggable(namePrefix: String, level: Int, promise: Promise) {
        rejectIfFailed(promise) {
            promise.resolve(appender(namePrefix).isLoggable(LogLevel.of(level)))
        }
    }

    /** `Xlog.flush`. */
    @ReactMethod
    fun flush(namePrefix: String, sync: Boolean, promise: Promise) {
        rejectIfFailed(promise) {
            appender(namePrefix).flush(sync)
            promise.resolve(null)
        }
    }

    /** `Xlog.level`. */
    @ReactMethod
    fun setLevel(namePrefix: String, level: Int, promise: Promise) {
        rejectIfFailed(promise) {
            appender(namePrefix).level = LogLevel.of(level)
            promise.resolve(null)
        }
    }

    /** `Xlog.level`, read: what `marsrs-jni` answers, and not what Kotlin holds. */
    @ReactMethod
    fun getLevel(namePrefix: String, promise: Promise) {
        rejectIfFailed(promise) {
            promise.resolve(appender(namePrefix).level.ordinal)
        }
    }

    /** `Xlog.mode`. */
    @ReactMethod
    fun setMode(namePrefix: String, mode: Int, promise: Promise) {
        rejectIfFailed(promise) {
            appender(namePrefix).mode = appenderModeOf(mode)
            promise.resolve(null)
        }
    }

    /** `Xlog.consoleLogEnabled`. */
    @ReactMethod
    fun setConsoleLogEnabled(namePrefix: String, enabled: Boolean, promise: Promise) {
        rejectIfFailed(promise) {
            appender(namePrefix).consoleLogEnabled = enabled
            promise.resolve(null)
        }
    }

    /** `Xlog.maxFileSizeBytes`. A `Double` and not a `Long`: the bridge carries
     * every JS number as one, and a file size is below 2^53. */
    @ReactMethod
    fun setMaxFileSize(namePrefix: String, bytes: Double, promise: Promise) {
        rejectIfFailed(promise) {
            appender(namePrefix).maxFileSizeBytes = bytes.toLong()
            promise.resolve(null)
        }
    }

    /** `Xlog.maxAliveTimeSeconds`. */
    @ReactMethod
    fun setMaxAliveTime(namePrefix: String, seconds: Double, promise: Promise) {
        rejectIfFailed(promise) {
            appender(namePrefix).maxAliveTimeSeconds = seconds.toLong()
            promise.resolve(null)
        }
    }

    /** `Xlog.close`: releases the appender `open` made. */
    @ReactMethod
    fun close(namePrefix: String, promise: Promise) {
        rejectIfFailed(promise) {
            appenders.remove(namePrefix)?.close()
            promise.resolve(null)
        }
    }

    /**
     * The appender of [namePrefix], or [IllegalStateException] when there is
     * none: no appender is the process-wide one to `marsrs-jni`, so a call that
     * went on without one would write through whatever appender the rest of the
     * process writes through.
     */
    private fun appender(namePrefix: String): Xlog = appenders[namePrefix]?.takeIf { it.isOpen }
        ?: error("no appender of '$namePrefix' is open: Xlog.open one first")

    /** A JS caller gets a rejection and not a crash: an appender that refuses a
     * configuration, or a write before an `open`, is a caller's mistake to
     * answer. */
    private fun rejectIfFailed(promise: Promise, call: () -> Unit) {
        try {
            call()
        } catch (e: Exception) {
            promise.reject(ERROR, e.message)
        }
    }

    private companion object {
        /** The name `src/index.ts` reads the module out of `NativeModules` by. */
        const val NAME = "Xlog"

        /** What every `promise.reject` answers with: the package a JS caller
         * sees the rejection come from. */
        const val ERROR = "marsrs-react-native-xlog"

        /** `XlogConfig.namePrefix` of the Kotlin, of the Swift and of the TS. */
        const val DEFAULT_NAME_PREFIX = "xlog"

        /** `0` — what the C++ passes on, which is the appender's own 6. */
        const val DEFAULT_COMPRESS_LEVEL = 0

        /** `0` keeps every log file, which is what the C++'s default is. */
        const val NO_CACHE_DAYS = 0

        /**
         * `AppenderMode` of the number `mars_xlog.h` gives a mode: an ordinal
         * the enum does not carry is one the caller sent by mistake, and `ASYNC`
         * is what the C++ opens with.
         */
        fun appenderModeOf(native: Int): AppenderMode = when (native) {
            AppenderMode.SYNC.ordinal -> AppenderMode.SYNC
            else -> AppenderMode.ASYNC
        }

        /** `CompressMode` of the number `mars_xlog.h` gives a compression. */
        fun compressModeOf(native: Int): CompressMode = when (native) {
            CompressMode.ZSTD.ordinal -> CompressMode.ZSTD
            else -> CompressMode.ZLIB
        }
    }
}

/** The number sent for [key], or [fallback] when none was sent: a field
 * `src/index.ts` left out is not one it sent as `null`. */
private fun ReadableMap.int(key: String, fallback: Int): Int = if (hasKey(key)) getInt(key) else fallback

/** The string sent for [key], or the empty one. */
private fun ReadableMap.string(key: String): String = if (hasKey(key)) getString(key).orEmpty() else ""

/** The string sent for [key], or `null` — `cacheDirectory` is the field a caller
 * may leave out, and `null` is what puts the cache in the log directory. */
private fun ReadableMap.optionalString(key: String): String? =
    if (hasKey(key)) getString(key)?.takeIf { it.isNotBlank() } else null
