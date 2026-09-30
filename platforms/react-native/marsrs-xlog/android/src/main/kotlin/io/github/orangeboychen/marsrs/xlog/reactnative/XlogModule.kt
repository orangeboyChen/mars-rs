// The Android half of `marsrs-react-native-xlog`: the sixteen methods of the
// `Xlog` native module, each of them a straight call of a member of `Xlog` —
// the Kotlin face of `libmarsrsxlog.so` in the `marsrs-xlog` AAR, and the same
// class `platforms/kmp/marsrs-xlog` publishes to a Kotlin Multiplatform app
// and `platforms/apple/MarsRSXlog/Xlog.swift` to a Swift one. So what this
// file is is a native module over an API that already exists, and nothing of
// the API is invented here: `Xlog.open(XlogConfig(...))`, `log`,
// `isLoggable`, `requestFlush`, `flushNow`, `flush`, `close`, and the five
// settings, under the names every other platform of the port gives them.
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

import com.facebook.react.bridge.Arguments
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReadableMap
import com.facebook.react.bridge.WritableArray
import io.github.orangeboychen.marsrs.xlog.AppenderMode
import io.github.orangeboychen.marsrs.xlog.CompressMode
import io.github.orangeboychen.marsrs.xlog.LogLevel
import io.github.orangeboychen.marsrs.xlog.Xlog
import io.github.orangeboychen.marsrs.xlog.XlogConfig
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException

/** The Android half of `Xlog`. */
class XlogModule(reactContext: ReactApplicationContext) : NativeXlogSpec(reactContext) {

    /**
     * The appender of every prefix `open` has opened, by the prefix: what keeps
     * two `Xlog`s apart, which a single field cannot — the C ABI and
     * `marsrs-jni` both answer an appender by the name it was opened with.
     */
    private val appenders = ConcurrentHashMap<String, Xlog>()

    /**
     * The one thread `flush` drains on: a drain blocks the thread it runs on,
     * and the JS thread is not one to block.
     */
    private val flushQueue = Executors.newSingleThreadExecutor()

    /**
     * `Xlog.open(XlogConfig(...))`: opens the appender of the configuration
     * `src/index.ts` sent, and answers whether it took it.
     *
     * One appender per prefix: `Xlog.open` answers the appender the prefix
     * already has rather than a second one over its files, so asking again is
     * how a prefix another part of the app closed is opened again — and a
     * prefix whose appender is still open is answered as it is. `src/index.ts`
     * does the same, so the two names an app holds for one prefix are one
     * appender and `close` on either closes it for both.
     *
     * [Xlog.open] is what refuses a configuration the appender cannot honour —
     * a blank `logDir` or `namePrefix`, a compression level out of range — and
     * its `IllegalArgumentException` is what would cross back into JS as a
     * crash, so it is caught here and answered as `false`: the caller's mistake
     * to throw on, and not one to take the app with.
     */
    override fun open(config: ReadableMap): Boolean {
        val namePrefix = config.string("namePrefix").ifBlank { DEFAULT_NAME_PREFIX }
        // [appenders] holding one is not the same as there being one: the
        // appender of a prefix is one for the whole process, so an `Xlog` of
        // this prefix the app closed itself took it away, and answering `true`
        // for it would leave every later call writing through a handle that
        // has no appender behind it.
        appenders[namePrefix]?.takeIf { it.isOpen }?.let { return true }
        // Both calls and not the open only: `XlogConfig` is where a blank
        // `logDir`, a negative `cacheDays` and a compression level out of
        // range are refused, and it throws before `Xlog.open` is ever reached
        // — so a `try` around the open alone is a `try` around nothing.
        val xlog = try {
            Xlog.open(
                XlogConfig(
                    logDir = config.string("logDir"),
                    namePrefix = namePrefix,
                    level = LogLevel.of(config.int("level", LogLevel.INFO.ordinal)),
                    mode = appenderModeOf(config.int("mode", AppenderMode.ASYNC.ordinal)),
                    pubKey = config.string("pubKey"),
                    compressMode = compressModeOf(
                        config.int("compressMode", CompressMode.ZLIB.ordinal)
                    ),
                    compressLevel = config.int("compressLevel", DEFAULT_COMPRESS_LEVEL),
                    cacheDir = config.optionalString("cacheDir"),
                    cacheDays = config.int("cacheDays", NO_CACHE_DAYS)
                )
            )
        } catch (e: IllegalArgumentException) {
            return false
        }
        appenders[xlog.namePrefix] = xlog
        return true
    }

    /**
     * `Xlog.log`. The file, the function and the line are the C++'s own
     * defaults: there is no JS frame to name, and the C++ writes an empty one
     * too.
     */
    override fun log(namePrefix: String, level: Double, tag: String, message: String) {
        val native = int32(level) ?: return
        appender(namePrefix)?.log(LogLevel.of(native), tag, message)
    }

    /** `Xlog.currentLogPath`: the directory this appender writes its files
     * into, or `null` once it is closed. */
    override fun currentLogPath(namePrefix: String): String? = appender(namePrefix)?.currentLogPath

    /** `Xlog.logFiles`: the day's files that are there. */
    override fun logFiles(namePrefix: String, daysAgo: Double): WritableArray? {
        val day = int32(daysAgo) ?: return Arguments.createArray()
        return appender(namePrefix)?.logFiles(day.toLong())?.let { Arguments.fromList(it) }
            ?: Arguments.createArray()
    }

    /** `Xlog.logFileNames`: the day's names, whether or not they are there yet. */
    override fun logFileNames(namePrefix: String, daysAgo: Double): WritableArray? {
        val day = int32(daysAgo) ?: return Arguments.createArray()
        return appender(namePrefix)?.logFileNames(day.toLong())?.let { Arguments.fromList(it) }
            ?: Arguments.createArray()
    }

    /** `Xlog.isLoggable`: whether a record of the level would be written. */
    override fun isLoggable(namePrefix: String, level: Double): Boolean {
        val native = int32(level) ?: return false
        return appender(namePrefix)?.isLoggable(LogLevel.of(native)) ?: false
    }

    /**
     * `Xlog.level`, read: what `marsrs-jni` answers, and not what JS holds.
     *
     * `NONE` and not a level the enum does not carry, which is what the iOS
     * half of this module answers for the same numbers and the answer that is
     * safe: `VERBOSE` is the ordinal `0`, so a `-1` read as a level is the
     * level that logs everything — the opposite of what an appender that
     * writes nothing was asked. An appender there is none of is `NONE` on
     * both halves.
     */
    override fun getLevel(namePrefix: String): Double =
        appender(namePrefix)?.level?.ordinal?.toDouble() ?: LogLevel.NONE.ordinal.toDouble()

    /** `Xlog.requestFlush`: asks the writer thread to drain, answers nothing. */
    override fun requestFlush(namePrefix: String) {
        appender(namePrefix)?.requestFlush()
    }

    /** `Xlog.flushNow`: the drain that is over, records on disk, when it returns. */
    override fun flushNow(namePrefix: String) {
        appender(namePrefix)?.flushNow()
    }

    /**
     * `Xlog.flushNow` off the thread JS runs on: the same drain, on a thread of
     * this module's own, and `promise` settled when it is over.
     *
     * A TurboModule method that answers a promise is the one codegen calls off
     * the JS thread, which is the whole reason this one answers one: a drain
     * blocks the thread it runs on, and the JS thread is not one to block.
     *
     * A flush asked of a module [invalidate] has already shut the queue down
     * is no drain and a settled promise: the `await` in JS does not return
     * from a promise nobody resolved, and there is no appender left to drain.
     */
    override fun flush(namePrefix: String, promise: Promise) {
        try {
            flushQueue.execute {
                appender(namePrefix)?.flushNow()
                promise.resolve(null)
            }
        } catch (e: RejectedExecutionException) {
            // The queue is shut down, so the drain it was handed is one it
            // will never run: settle here, and not in it.
            promise.resolve(null)
        }
    }

    /** `Xlog.level`. */
    override fun setLevel(namePrefix: String, level: Double) {
        val native = int32(level) ?: return
        appender(namePrefix)?.level = LogLevel.of(native)
    }

    /** `Xlog.mode`. */
    override fun setMode(namePrefix: String, mode: Double) {
        val native = int32(mode) ?: return
        appender(namePrefix)?.mode = appenderModeOf(native)
    }

    /** `Xlog.consoleLogEnabled`. */
    override fun setConsoleLogEnabled(namePrefix: String, enabled: Boolean) {
        appender(namePrefix)?.consoleLogEnabled = enabled
    }

    /** `Xlog.maxFileSizeBytes`. A `Double` and not a `Long`: the module carries
     * every JS number as one, and a file size is below 2^53. */
    override fun setMaxFileSize(namePrefix: String, bytes: Double) {
        val native = int64(bytes) ?: return
        appender(namePrefix)?.maxFileSizeBytes = native
    }

    /** `Xlog.maxAliveTimeSeconds`. */
    override fun setMaxAliveTime(namePrefix: String, seconds: Double) {
        val native = int64(seconds) ?: return
        appender(namePrefix)?.maxAliveTimeSeconds = native
    }

    /**
     * `Xlog.close`: releases the appender `open` made.
     *
     * A method that answers no promise is a call codegen makes on the JS
     * thread, and `Xlog.close` is a drain of everything the appender is still
     * holding and a write of the banner that ends the file — so this one is
     * handed to the queue, the way [flush] is, and the JS thread is free again
     * at once.
     *
     * Taken out of [appenders] here and not on the queue: a reopen of the
     * prefix that lands while the drain is queued is otherwise an appender
     * this close never held. What a queue that was shut down already means is
     * that [closeAll] ran, and with it this close.
     */
    override fun close(namePrefix: String) {
        val appender = appenders.remove(namePrefix) ?: return
        try {
            flushQueue.execute { appender.close() }
        } catch (e: RejectedExecutionException) {
            appender.close()
        }
    }

    /**
     * What React Native calls when the bridge this module was made for goes
     * away: the one place every appender [open] made is closed — see
     * [closeAll] — and the one place the thread [flushQueue] runs on is shut
     * down:
     * `Executors.newSingleThreadExecutor` keeps a non-daemon thread alive
     * until something shuts it, so a module that did not would leave one
     * behind per bridge reload — each of them idle, and each of them a
     * process that will not end while they are there.
     */
    override fun invalidate() {
        super.invalidate()
        closeAll()
        flushQueue.shutdown()
    }

    /**
     * Every appender [open] made, closed: what a module that is going away
     * owes the files it was writing, and the only drain they will get — the
     * queue is shut down below, so an appender left open is one whose records
     * stay in its buffer, whose writer thread stays alive and whose cache
     * stays claimed for the life of the process.
     *
     * Closed on the queue and not on this thread, which is the JS thread,
     * because `Xlog.close` is itself a drain; and `shutdown` runs what it was
     * handed before it stops, so these do run. A queue that was shut down
     * already — an `invalidate` before this one — was handed them then.
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
     * The appender of [namePrefix], or `null` when there is none — which is a
     * no-op and not a crash, the same answer the Swift and the Kotlin give an
     * `Xlog` that is closed: a handle whose appender is gone is a no-op to
     * `marsrs-jni`, so a call that went on without one would silently write
     * nothing.
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

        /** `0` keeps every cache file, which is what the C++'s default is. */
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

        /**
         * The number JS sent for a level, a mode or a day, and `null` when it
         * is not one: a `NaN`, an infinity, and anything the `Int` the C ABI
         * takes cannot hold.
         *
         * `Double.toInt()` answers `0` for a `NaN`, and `LogLevel.of` answers
         * `VERBOSE` for a `0`, so a level JS computed and came to no level
         * with — a sum with an `undefined` in it — was one the appender was
         * *moved* to, and the one it was moved to is the level that logs
         * everything: an app that meant to log nothing logged all of it. The
         * iOS half of this module read the same number with a trap rather
         * than a `0` — `Int32(_:)` on a `NaN` ends the app — and neither is
         * an answer, so both leave the appender alone now.
         */
        private fun int32(value: Double): Int? {
            if (!value.isFinite() || value < Int.MIN_VALUE.toDouble() || value > Int.MAX_VALUE.toDouble()) {
                return null
            }
            return value.toInt()
        }

        /**
         * The number JS sent for a size or a file lifetime, and `null` when it
         * is not one: the same three, over the `Long` the Kotlin takes. A
         * `NaN` was answered as `0` — "never split" for a size, and the
         * appender's own ten days for a lifetime — which is a setting an app
         * never asked for and not one it was told had been refused.
         */
        private fun int64(value: Double): Long? {
            if (!value.isFinite() ||
                value < Long.MIN_VALUE.toDouble() ||
                value >= Long.MAX_VALUE.toDouble()
            ) {
                return null
            }
            return value.toLong()
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
