// The Android half of the `marsrs_flutter_xlog` plugin: the eleven methods of
// the plugin's channel, each of them a straight call of a member of `Xlog` —
// the Kotlin face of `libmarsrsxlog.so` in the `marsrs-xlog` AAR, and the same
// class `kmp/marsrs-xlog` publishes to a Kotlin Multiplatform app and
// `Sources/MarsRSXlog/Xlog.swift` to a Swift one. So what this file is is a
// channel over an API that already exists, and nothing of it is invented here:
// `Xlog(XlogConfig(...))`, `log`, `isLoggable`, `flush`, `close`, and the five
// settings, under the names every other platform of the port gives them.
//
// `marsrs_flutter` is the whole-port plugin, over the `marsrs` AAR, and this
// file is its Kotlin with the two names changed: xlog is the whole C ABI today,
// so the two plugins are one package under two names, and they diverge the day
// STN and SDT land — there, and not here.
//
// Every key below is a field `lib/marsrs_flutter_xlog.dart` put there, and every
// number is the one `mars_xlog.h` gives a level, a mode and a compression. The
// appenders are kept by the prefix they were opened with, because that is what
// an appender is known by in the C ABI and in `marsrs-jni`: two `Xlog`s of two
// prefixes are two appenders, and one of a prefix the plugin already has is the
// appender it already has.

package io.github.orangeboychen.marsrs.xlog.flutter

import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.MethodChannel.MethodCallHandler
import io.flutter.plugin.common.MethodChannel.Result
import io.github.orangeboychen.marsrs.xlog.AppenderMode
import io.github.orangeboychen.marsrs.xlog.CompressMode
import io.github.orangeboychen.marsrs.xlog.LogLevel
import io.github.orangeboychen.marsrs.xlog.Xlog
import io.github.orangeboychen.marsrs.xlog.XlogConfig
import java.util.concurrent.ConcurrentHashMap

/** The Android half of `marsrs_flutter_xlog`. */
class XlogPlugin :
    FlutterPlugin,
    MethodCallHandler {

    private var channel: MethodChannel? = null

    /**
     * The appender of every prefix `open` has opened, by the prefix: what keeps
     * two of them apart, which a single field cannot — the C ABI and
     * `marsrs-jni` both answer an appender by the name it was opened with.
     */
    private val appenders = ConcurrentHashMap<String, Xlog>()

    override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel = MethodChannel(binding.binaryMessenger, CHANNEL).also {
            it.setMethodCallHandler(this)
        }
    }

    override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel?.setMethodCallHandler(null)
        channel = null
    }

    override fun onMethodCall(call: MethodCall, result: Result) {
        // A Dart caller gets a `PlatformException` and not a crash: an appender
        // that refuses a configuration, or a write before an `open`, is a
        // caller's mistake to answer.
        try {
            when (call.method) {
                "open" -> open(call, result)
                "log" -> log(call, result)
                "isLoggable" -> isLoggable(call, result)
                "flush" -> flush(call, result)
                "setLevel" -> setLevel(call, result)
                "getLevel" -> getLevel(call, result)
                "setMode" -> setMode(call, result)
                "setConsoleLogEnabled" -> setConsoleLogEnabled(call, result)
                "setMaxFileSize" -> setMaxFileSize(call, result)
                "setMaxAliveTime" -> setMaxAliveTime(call, result)
                "close" -> close(call, result)
                else -> result.notImplemented()
            }
        } catch (e: Exception) {
            result.error(ERROR, e.message, null)
        }
    }

    /**
     * `Xlog(XlogConfig(...))`: opens the appender of the configuration the Dart
     * caller sent.
     *
     * The constructor is what refuses a configuration the appender cannot honour
     * — a blank `logDir` or `namePrefix`, a compression level out of range — and
     * its `IllegalArgumentException` is what the caller is answered with, rather
     * than a handle it would write through the process-wide appender with.
     */
    private fun open(call: MethodCall, result: Result) {
        val config = XlogConfig(
            logDir = call.string("logDir"),
            namePrefix = call.string("namePrefix").ifBlank { DEFAULT_NAME_PREFIX },
            level = LogLevel.of(call.int("level", LogLevel.INFO.ordinal)),
            mode = appenderMode(call.int("mode", AppenderMode.ASYNC.ordinal)),
            pubKey = call.string("pubKey"),
            compressMode = compressMode(call.int("compressMode", CompressMode.ZLIB.ordinal)),
            compressLevel = call.int("compressLevel", DEFAULT_COMPRESS_LEVEL),
            cacheDir = call.optionalString("cacheDir"),
            cacheDays = call.int("cacheDays", NO_CACHE_DAYS)
        )
        appenders[config.namePrefix] = Xlog(config)
        result.success(null)
    }

    /**
     * `Xlog.log`. The file, the function and the line are the C++'s own
     * defaults: there is no Dart frame to name, and the C++ writes an empty one
     * too.
     */
    private fun log(call: MethodCall, result: Result) {
        call.appender().log(
            LogLevel.of(call.int("level", LogLevel.INFO.ordinal)),
            call.string("tag"),
            call.string("message")
        )
        result.success(null)
    }

    /** `Xlog.isLoggable`: whether a record of the level would be written. */
    private fun isLoggable(call: MethodCall, result: Result) {
        val level = LogLevel.of(call.int("level", LogLevel.INFO.ordinal))
        result.success(call.appender().isLoggable(level))
    }

    /** `Xlog.flush`. */
    private fun flush(call: MethodCall, result: Result) {
        call.appender().flush(call.boolean("sync", false))
        result.success(null)
    }

    /** `Xlog.level`. */
    private fun setLevel(call: MethodCall, result: Result) {
        call.appender().level = LogLevel.of(call.int("level", LogLevel.INFO.ordinal))
        result.success(null)
    }

    /** `Xlog.level`, read: what `marsrs-jni` answers, and not what Kotlin holds. */
    private fun getLevel(call: MethodCall, result: Result) {
        result.success(call.appender().level.ordinal)
    }

    /** `Xlog.mode`. */
    private fun setMode(call: MethodCall, result: Result) {
        call.appender().mode = appenderMode(call.int("mode", AppenderMode.ASYNC.ordinal))
        result.success(null)
    }

    /** `Xlog.consoleLogEnabled`. */
    private fun setConsoleLogEnabled(call: MethodCall, result: Result) {
        call.appender().consoleLogEnabled = call.boolean("enabled", false)
        result.success(null)
    }

    /** `Xlog.maxFileSizeBytes`. */
    private fun setMaxFileSize(call: MethodCall, result: Result) {
        call.appender().maxFileSizeBytes = call.long("bytes", NO_FILE_SIZE_LIMIT)
        result.success(null)
    }

    /** `Xlog.maxAliveTimeSeconds`. */
    private fun setMaxAliveTime(call: MethodCall, result: Result) {
        call.appender().maxAliveTimeSeconds = call.long("seconds", NO_ALIVE_TIME_LIMIT)
        result.success(null)
    }

    /** `Xlog.close`: releases the appender `open` made. */
    private fun close(call: MethodCall, result: Result) {
        val namePrefix = call.string("namePrefix")
        appenders.remove(namePrefix)?.close()
        result.success(null)
    }

    /**
     * The appender of the prefix this call carries, or [IllegalStateException]
     * when there is none: no appender is the process-wide one to `marsrs-jni`,
     * so a call that went on without one would write through whatever appender
     * the rest of the process writes through.
     */
    private fun MethodCall.appender(): Xlog {
        val namePrefix = string("namePrefix")
        return appenders[namePrefix]?.takeIf { it.isOpen }
            ?: error("no appender of '$namePrefix' is open: Xlog.open one first")
    }

    /** What the channel carries, read as a string; a missing one is the empty
     * string, which every field of the configuration treats as "not given". */
    private fun MethodCall.string(key: String): String = argument<String>(key).orEmpty()

    /** `null` for a missing or blank one, because a blank path is a path the
     * appender was not given: `cacheDir` of `null` is the log directory. */
    private fun MethodCall.optionalString(key: String): String? = argument<String>(key)?.takeIf { it.isNotBlank() }

    /** `Number`, and not `Int` or `Long`: the codec hands over whichever of the
     * two the Dart `int` fit in. */
    private fun MethodCall.int(key: String, default: Int): Int = argument<Number>(key)?.toInt() ?: default

    private fun MethodCall.long(key: String, default: Long): Long = argument<Number>(key)?.toLong() ?: default

    private fun MethodCall.boolean(key: String, default: Boolean): Boolean = argument<Boolean>(key) ?: default

    /**
     * An ordinal the caller sent, as the entry of that ordinal — or [default],
     * because a number `mars_xlog.h` does not give a mode is not one this
     * channel is bound to answer with an `IndexOutOfBoundsException`.
     */
    private fun appenderMode(ordinal: Int): AppenderMode =
        AppenderMode.entries.getOrElse(ordinal) { AppenderMode.ASYNC }

    private fun compressMode(ordinal: Int): CompressMode = CompressMode.entries.getOrElse(ordinal) { CompressMode.ZLIB }

    private companion object {
        /** The channel `lib/marsrs_flutter_xlog.dart` talks on. */
        private const val CHANNEL = "marsrs_flutter_xlog"

        /** What a `PlatformException` of a refused configuration, of a write
         * before an `open`, and of every other caller's mistake, is named. */
        private const val ERROR = "marsrs_flutter_xlog"

        /** `XlogConfig.namePrefix` of the Kotlin, and of the Swift and the Dart. */
        private const val DEFAULT_NAME_PREFIX = "xlog"

        private const val DEFAULT_COMPRESS_LEVEL = 0

        private const val NO_CACHE_DAYS = 0

        /** The C ABI reads a max file size of `0` as "never split". */
        private const val NO_FILE_SIZE_LIMIT = 0L

        /** The C ABI reads a max alive time of `0` as the C++'s own ten days. */
        private const val NO_ALIVE_TIME_LIMIT = 0L
    }
}
