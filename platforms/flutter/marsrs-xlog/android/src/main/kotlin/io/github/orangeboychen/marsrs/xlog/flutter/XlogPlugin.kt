// The Android half of the `marsrs_xlog` plugin: the fourteen methods of
// the plugin's channel, each of them a straight call of a member of `Xlog` —
// the Kotlin face of `libmarsrsxlog.so` in the `marsrs-xlog` AAR, and the same
// class `platforms/kmp/marsrs-xlog` publishes to a Kotlin Multiplatform app
// and `platforms/apple/MarsRSXlog/Xlog.swift` to a Swift one. So what this
// file is is a channel over an API that already exists, and nothing of the API
// is invented here: `Xlog.open(XlogConfig(...))`, `log`, `isLoggable`,
// `requestFlush`, `flush`,
// `close`, and the five settings, under the names every other platform of the
// port gives them.
//
// `marsrs` is the whole-port plugin, over the `marsrs` AAR, and this
// file is its Kotlin with the two names changed: xlog is the whole C ABI today,
// so the two plugins are one package under two names, and they diverge the day
// STN and SDT land — there, and not here.
//
// Every key below is a field `lib/marsrs_xlog.dart` put there, and every
// number is the one `mars_xlog.h` gives a level, a mode and a compression. The
// appenders are kept by the prefix they were opened with, because that is what
// an appender is known by in the C ABI and in `marsrs-jni`: two `Xlog`s of two
// prefixes are two appenders, and one of a prefix the plugin already has is the
// appender it already has.

package io.github.orangeboychen.marsrs.xlog.flutter

import android.os.Handler
import android.os.Looper
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
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException

/** The Android half of `marsrs_xlog`. */
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

    /**
     * The one thread `flush` drains on, and the one the iOS half drains on for
     * the same reason: a drain blocks the thread it runs on, and the thread a
     * platform channel is handled on is the app's main looper — which is the
     * thread the UI draws on, and not one an app's `await xlog.flush()` may
     * hold up. Serial, so two drains of one appender are one drain after
     * another and not two writers in one file.
     *
     * A `var`, and rebuilt by [onAttachedToEngine], because [onDetachedFromEngine]
     * shuts this one down: a detach is not always the last thing that happens
     * to a plugin instance, and no drain can be put on an executor that is
     * shut down, so the `flush` after one would be a `RejectedExecutionException`
     * where a plugin that is attached again is a plugin that answers.
     */
    private var flushQueue: ExecutorService = Executors.newSingleThreadExecutor()

    /** What a drain is answered on: a `Result` is a reply on the channel, and
     * the channel is the main thread's. */
    private val mainHandler = Handler(Looper.getMainLooper())

    override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        // What [onDetachedFromEngine] shut down, which nothing can be handed
        // again: an attach after a detach is a plugin that answers `flush`
        // again, and not one that throws on the first Dart caller to ask.
        if (flushQueue.isShutdown) {
            flushQueue = Executors.newSingleThreadExecutor()
        }
        channel = MethodChannel(binding.binaryMessenger, CHANNEL).also {
            it.setMethodCallHandler(this)
        }
    }

    override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel?.setMethodCallHandler(null)
        channel = null
        closeAll()
        // The thread `flush` drains on is this plugin's and not the app's, and
        // a plugin outlives the engine it was attached to: a thread left
        // running would be one more per engine, for the life of the process.
        flushQueue.shutdown()
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
                "requestFlush" -> requestFlush(call, result)
                "setLevel" -> setLevel(call, result)
                "setMode" -> setMode(call, result)
                "setConsoleLogEnabled" -> setConsoleLogEnabled(call, result)
                "setMaxFileSize" -> setMaxFileSize(call, result)
                "setMaxAliveTime" -> setMaxAliveTime(call, result)
                "currentLogPath" -> currentLogPath(call, result)
                "logFiles" -> logFiles(call, result)
                "logFileNames" -> logFileNames(call, result)
                "close" -> close(call, result)
                else -> result.notImplemented()
            }
        } catch (e: Exception) {
            result.error(ERROR, e.message, null)
        }
    }

    /**
     * `Xlog.open(XlogConfig(...))`: opens the appender of the configuration the
     * Dart caller sent.
     *
     * The constructor [Xlog.open] calls is what refuses a configuration the
     * appender cannot honour — a blank `logDir` or `namePrefix`, a compression
     * level out of range — and its `IllegalArgumentException` is what the caller
     * is answered with, rather than a handle nothing was opened for.
     */
    private fun open(call: MethodCall, result: Result) {
        // The message the Dart doc of `Xlog.open` promises for an empty
        // `logDir`, and the one the iOS half answers: `Xlog.open` below
        // refuses a blank one too, but with the single message `marsrs-jni`
        // has for every refusal, which names neither the field nor the
        // reason.
        if (call.string("logDir").isBlank()) {
            result.error(ERROR, "logDir is empty", null)
            return
        }
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
        appenders[config.namePrefix] = Xlog.open(config)
        result.success(null)
    }

    /**
     * The directory the appender of `namePrefix` writes its files into, or
     * `null` once it is closed.
     */
    private fun currentLogPath(call: MethodCall, result: Result) {
        result.success(call.appender().currentLogPath)
    }

    /** The day's files that are there — `0` is today, `1` is yesterday. */
    private fun logFiles(call: MethodCall, result: Result) {
        result.success(call.appender().logFiles(call.long("daysAgo", 0)))
    }

    /** The day's names, whether or not the files are there yet. */
    private fun logFileNames(call: MethodCall, result: Result) {
        result.success(call.appender().logFileNames(call.long("daysAgo", 0)))
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

    /**
     * `Xlog.flushNow`: the drain that does not return until the records are on
     * disk, and the one the Dart caller awaits — that caller has no thread of
     * its own to block on, so the wait is this side's and the answer is what
     * it awaits.
     *
     * Off the main thread, and back to it for the answer, the way the iOS half
     * does it: `onMethodCall` runs on the app's main looper, so a drain here
     * would be a drain on the thread the UI draws on.
     */
    private fun flush(call: MethodCall, result: Result) {
        val appender = call.appender()
        flushQueue.execute {
            // Answered either way: a `Result` is one reply and one only, and
            // the Dart caller is awaiting this one — so a drain that throws is
            // an `await` nothing ever settles, and a plugin that looks hung.
            try {
                appender.flushNow()
                mainHandler.post { result.success(null) }
            } catch (e: Exception) {
                mainHandler.post { result.error(ERROR, e.message, null) }
            }
        }
    }

    /** `Xlog.requestFlush`: asks the writer thread to drain, answers nothing. */
    private fun requestFlush(call: MethodCall, result: Result) {
        call.appender().requestFlush()
        result.success(null)
    }

    /** `Xlog.level`. */
    private fun setLevel(call: MethodCall, result: Result) {
        call.appender().level = LogLevel.of(call.int("level", LogLevel.INFO.ordinal))
        result.success(null)
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

    /**
     * `Xlog.close`: releases the appender [open] made.
     *
     * Off the main thread, and back to it for the answer, the way [flush] does
     * it: `Xlog.close` is a drain of everything the appender is still holding,
     * and a write of the banner that ends the file, so an app that closes its
     * appender on the way out would hold the thread the UI draws on for as
     * long as that takes. The one drain that stays on it is the one a queue
     * that was shut down already refuses to take — what that means is that
     * [closeAll] ran, and with it this close.
     */
    private fun close(call: MethodCall, result: Result) {
        val namePrefix = call.string("namePrefix")
        // Taken out of [appenders] here, and not on the queue below: the map is
        // this thread's, so a reopen of the prefix that lands while the drain
        // is still queued is a reopen of an appender this close does not hold.
        val appender = appenders.remove(namePrefix)
        if (appender == null) {
            result.success(null)
            return
        }
        try {
            flushQueue.execute {
                // Answered either way, the way [flush] answers: what a
                // `close()` is awaiting is the drain, and a drain that threw
                // is not one that answered.
                try {
                    appender.close()
                    mainHandler.post { result.success(null) }
                } catch (e: Exception) {
                    mainHandler.post { result.error(ERROR, e.message, null) }
                }
            }
        } catch (e: RejectedExecutionException) {
            // The one drain that is not off this thread: the queue is shut
            // down, so there is no thread to hand it to, and an appender that
            // is going away with the plugin is drained here or not at all.
            appender.close()
            result.success(null)
        }
    }

    /**
     * Every appender [open] made, closed: what a plugin the engine has left
     * owes the files it was writing, and the only drain they will get — the
     * queue is shut down below, so an appender left open is one whose records
     * stay in its buffer, whose writer thread stays alive and whose cache
     * stays claimed for the life of the process.
     *
     * Closed on the queue and not on this thread, which is the app's main
     * looper, because `Xlog.close` is itself a drain; and `shutdown` runs what
     * it was handed before it stops, so these do run. A queue that was shut
     * down already — a detach before this one — was handed them then.
     */
    private fun closeAll() {
        try {
            flushQueue.execute {
                appenders.values.forEach { it.close() }
                appenders.clear()
            }
        } catch (e: RejectedExecutionException) {
            appenders.clear()
        }
    }

    /**
     * The appender of the prefix this call carries, or [IllegalStateException]
     * when there is none: a handle whose appender is gone is a no-op to
     * `marsrs-jni`, so a call that went on without one would silently write
     * nothing.
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
        /** The channel `lib/marsrs_xlog.dart` talks on. */
        private const val CHANNEL = "marsrs_xlog"

        /** What a `PlatformException` of a refused configuration, of a write
         * before an `open`, and of every other caller's mistake, is named. */
        private const val ERROR = "marsrs_xlog"

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
