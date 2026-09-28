// The Android half of `marsrs-react-native-xlog`: the eleven methods of the
// `Xlog` native module, each of them a straight call of a member of `Xlog` — the
// Kotlin face of `libmarsrsxlog.so` in the `marsrs-xlog` AAR, and the same class
// `kmp/marsrs-xlog` publishes to a Kotlin Multiplatform app and
// `Sources/MarsRSXlog/Xlog.swift` to a Swift one. So what this file is is a
// native module over an API that already exists, and nothing of the API is
// invented here: `Xlog.open(XlogConfig(...))`, `log`, `isLoggable`, `flush`,
// `close`, and the five settings, under the names every other platform of the
// port gives them.
//
// A TurboModule, and not a bridge module: `src/NativeXlog.ts` is the spec
// codegen reads, and `NativeXlogSpec` — the abstract class it generates into
// this package — is what this class extends. What that buys is a call that is
// made where it is asked for and returned from: a method that answers no promise
// is a call and not an `await`, and one that answers a value answers it before
// the call returns. That, and only that, is why `src/index.ts` has no `Promise`
// in it and no `await` in front of a line.
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

import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReadableMap
import io.github.orangeboychen.marsrs.xlog.AppenderMode
import io.github.orangeboychen.marsrs.xlog.CompressMode
import io.github.orangeboychen.marsrs.xlog.LogLevel
import io.github.orangeboychen.marsrs.xlog.Xlog
import io.github.orangeboychen.marsrs.xlog.XlogConfig
import java.util.concurrent.ConcurrentHashMap

/** The Android half of `Xlog`. */
class XlogModule(reactContext: ReactApplicationContext) : NativeXlogSpec(reactContext) {

    /**
     * The appender of every prefix `open` has opened, by the prefix: what keeps
     * two `Xlog`s apart, which a single field cannot — the C ABI and
     * `marsrs-jni` both answer an appender by the name it was opened with.
     */
    private val appenders = ConcurrentHashMap<String, Xlog>()

    /**
     * `Xlog.open(XlogConfig(...))`: opens the appender of the configuration
     * `src/index.ts` sent, and answers whether it took it.
     *
     * One appender per prefix: a prefix [appenders] already holds is answered
     * as `true` without a second `Xlog` over the first, which would be a handle
     * nothing releases. `src/index.ts` does the same, so the two names an app
     * holds for one prefix are one appender and `close` on either closes it for
     * both.
     *
     * [Xlog.open] is what refuses a configuration the appender cannot honour —
     * a blank `logDir` or `namePrefix`, a compression level out of range — and
     * its `IllegalArgumentException` is what would cross back into JS as a
     * crash, so it is caught here and answered as `false`: the caller's mistake
     * to throw on, and not one to take the app with.
     */
    override fun open(config: ReadableMap): Boolean {
        val namePrefix = config.string("namePrefix").ifBlank { DEFAULT_NAME_PREFIX }
        if (appenders.containsKey(namePrefix)) {
            return true
        }
        val xlogConfig = XlogConfig(
            logDir = config.string("logDir"),
            namePrefix = namePrefix,
            level = LogLevel.of(config.int("level", LogLevel.INFO.ordinal)),
            mode = appenderModeOf(config.int("mode", AppenderMode.ASYNC.ordinal)),
            pubKey = config.string("pubKey"),
            compressMode = compressModeOf(config.int("compressMode", CompressMode.ZLIB.ordinal)),
            compressLevel = config.int("compressLevel", DEFAULT_COMPRESS_LEVEL),
            cacheDir = config.optionalString("cacheDir"),
            cacheDays = config.int("cacheDays", NO_CACHE_DAYS)
        )
        val xlog = try {
            Xlog.open(xlogConfig)
        } catch (e: IllegalArgumentException) {
            return false
        }
        appenders[xlogConfig.namePrefix] = xlog
        return true
    }

    /**
     * `Xlog.log`. The file, the function and the line are the C++'s own
     * defaults: there is no JS frame to name, and the C++ writes an empty one
     * too.
     */
    override fun log(namePrefix: String, level: Double, tag: String, message: String) {
        appender(namePrefix)?.log(LogLevel.of(level.toInt()), tag, message)
    }

    /** `Xlog.isLoggable`: whether a record of the level would be written. */
    override fun isLoggable(namePrefix: String, level: Double): Boolean =
        appender(namePrefix)?.isLoggable(LogLevel.of(level.toInt())) ?: false

    /** `Xlog.level`, read: what `marsrs-jni` answers, and not what JS holds. */
    override fun getLevel(namePrefix: String): Double =
        appender(namePrefix)?.level?.ordinal?.toDouble() ?: LogLevel.NONE.ordinal.toDouble()

    /** `Xlog.flush`. */
    override fun flush(namePrefix: String, sync: Boolean) {
        appender(namePrefix)?.flush(sync)
    }

    /** `Xlog.level`. */
    override fun setLevel(namePrefix: String, level: Double) {
        appender(namePrefix)?.level = LogLevel.of(level.toInt())
    }

    /** `Xlog.mode`. */
    override fun setMode(namePrefix: String, mode: Double) {
        appender(namePrefix)?.mode = appenderModeOf(mode.toInt())
    }

    /** `Xlog.consoleLogEnabled`. */
    override fun setConsoleLogEnabled(namePrefix: String, enabled: Boolean) {
        appender(namePrefix)?.consoleLogEnabled = enabled
    }

    /** `Xlog.maxFileSizeBytes`. A `Double` and not a `Long`: the module carries
     * every JS number as one, and a file size is below 2^53. */
    override fun setMaxFileSize(namePrefix: String, bytes: Double) {
        appender(namePrefix)?.maxFileSizeBytes = bytes.toLong()
    }

    /** `Xlog.maxAliveTimeSeconds`. */
    override fun setMaxAliveTime(namePrefix: String, seconds: Double) {
        appender(namePrefix)?.maxAliveTimeSeconds = seconds.toLong()
    }

    /** `Xlog.close`: releases the appender `open` made. */
    override fun close(namePrefix: String) {
        appenders.remove(namePrefix)?.close()
    }

    /**
     * The appender of [namePrefix], or `null` when there is none — which is a
     * no-op and not a crash, the same answer the Swift and the Kotlin give an
     * `Xlog` that is closed: no appender is the process-wide one to
     * `marsrs-jni`, so a call that went on without one would write through
     * whatever appender the rest of the process writes through.
     */
    private fun appender(namePrefix: String): Xlog? = appenders[namePrefix]?.takeIf { it.isOpen }

    internal companion object {
        /** The name `src/index.ts` reads the module out of
         * `TurboModuleRegistry` by — and the one codegen gave the generated
         * `NativeXlogSpec`. */
        const val NAME = "Xlog"

        /** `XlogConfig.namePrefix` of the Kotlin, of the Swift and of the TS. */
        private const val DEFAULT_NAME_PREFIX = "xlog"

        /** `0` — what the C++ passes on, which is the appender's own 6. */
        private const val DEFAULT_COMPRESS_LEVEL = 0

        /** `0` keeps every log file, which is what the C++'s default is. */
        private const val NO_CACHE_DAYS = 0

        /**
         * `AppenderMode` of the number `mars_xlog.h` gives a mode: an ordinal
         * the enum does not carry is one the caller sent by mistake, and `ASYNC`
         * is what the C++ opens with.
         */
        fun appenderModeOf(ordinal: Int): AppenderMode = when (ordinal) {
            AppenderMode.SYNC.ordinal -> AppenderMode.SYNC
            else -> AppenderMode.ASYNC
        }

        /** `CompressMode` of the number `mars_xlog.h` gives a compression. */
        fun compressModeOf(ordinal: Int): CompressMode = when (ordinal) {
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

/** The string sent for [key], or `null` — `cacheDir` is the field a caller may
 * leave out, and `null` is what puts the cache in the log directory. */
private fun ReadableMap.optionalString(key: String): String? =
    if (hasKey(key)) getString(key)?.takeIf { it.isNotBlank() } else null
