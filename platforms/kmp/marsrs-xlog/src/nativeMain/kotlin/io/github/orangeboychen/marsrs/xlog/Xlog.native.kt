package io.github.orangeboychen.marsrs.xlog

import io.github.orangeboychen.marsrs.xlog.ffi.MarsXLogConfig
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_current_log_path_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_flush_now_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_get_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_get_level
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_getfilepath_from_timespan_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_is_enabled_for
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_make_logfile_name_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_new_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_release_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_request_flush_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_console_log_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_level_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_max_alive_duration_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_max_file_size_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_mode_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_write_instance
import kotlin.concurrent.Volatile
import kotlinx.cinterop.ByteVar
import kotlinx.cinterop.CPointer
import kotlinx.cinterop.ExperimentalForeignApi
import kotlinx.cinterop.alloc
import kotlinx.cinterop.allocArray
import kotlinx.cinterop.cstr
import kotlinx.cinterop.memScoped
import kotlinx.cinterop.ptr
import kotlinx.cinterop.toKString
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * The Kotlin/Native `actual`: the C ABI of `crates/marsrs-ffi` (`mars_xlog.h`)
 * through cinterop, which is the same source set for every native target —
 * iOS, watchOS, tvOS, macOS, Linux and Windows — because the C ABI is the same
 * header on all of them.
 *
 * An [Xlog] is one `mars_xlog_new_instance`: the C ABI's own named appender, and
 * the same thing `marsrs-jni` answers an [Xlog] of Android with. Every member is
 * the `*_instance` call of the header, so a level, a mode or a size this [Xlog]
 * moves is this appender's and no other's.
 *
 * The strings of the C ABI are Kotlin strings at a call: cinterop maps the
 * `const char*` parameter of `mars_xlog.h` to `String?`, which is what lets the
 * optional ones be passed as `null` — the very thing the header's "NULL is
 * treated as empty" says. The fields of `MarsXLogConfig` stay pointers, so those
 * are converted here, inside the `memScoped` the config lives in.
 */
@OptIn(ExperimentalForeignApi::class)
public actual class Xlog actual constructor(config: XlogConfig) {
    public actual val namePrefix: String = config.namePrefix

    public actual var consoleLogEnabled: Boolean = false
        set(value) {
            mars_xlog_set_console_log_instance(requireOpen(), if (value) CONSOLE_LOG_OPEN else CONSOLE_LOG_CLOSED)
            field = value
        }

    public actual var maxFileSizeBytes: Long = NO_FILE_SIZE_LIMIT
        set(value) {
            val size = value.coerceAtLeast(NO_FILE_SIZE_LIMIT)
            mars_xlog_set_max_file_size_instance(requireOpen(), size.toULong())
            field = size
        }

    public actual var maxAliveTimeSeconds: Long = NO_ALIVE_TIME_LIMIT
        set(value) {
            val seconds = value.coerceAtLeast(NO_ALIVE_TIME_LIMIT)
            mars_xlog_set_max_alive_duration_instance(requireOpen(), seconds)
            field = seconds
        }

    /**
     * The handle [close] takes away, and the one every member is forwarded
     * through. Volatile because [close] may run on another thread than the
     * writes it stops: nothing writes through a handle that is half of the old
     * one and half of the new.
     */
    @Volatile
    private var handle: Long = newInstance(config)

    private var currentMode: AppenderMode = config.mode

    /**
     * Whether this appender is still open: `false` after [close] — on this
     * `Xlog` and on every other one of this `namePrefix`, which is the same
     * appender and is closed with this one.
     *
     * Both halves are needed. A prefix is one appender to the C ABI, so two
     * `Xlog`s of one prefix are answered the same handle and closing either
     * releases it: the handle alone still looks open, and asking the C ABI
     * alone answers a handle the prefix was re-opened under in the meantime.
     */
    public actual val isOpen: Boolean
        get() = openHandle() != NO_HANDLE

    public actual var level: LogLevel
        get() = LogLevel.of(mars_xlog_get_level(requireOpen()))
        set(value) = mars_xlog_set_level_instance(requireOpen(), value.ordinal)

    public actual var mode: AppenderMode
        get() = currentMode
        set(value) {
            mars_xlog_set_mode_instance(requireOpen(), value.ordinal)
            currentMode = value
        }

    /**
     * What `read` writes into the buffer it is handed; `null` when it wrote
     * nothing, which is a negative code or a path of no length.
     *
     * A negative code is `null` whatever it is, `MARS_XLOG_ERR_NO_SPACE` among
     * them: a path that does not fit [PATH_BUFFER_SIZE] ends the walk the way
     * the end of the list does.
     */
    private fun pathAt(read: (CPointer<ByteVar>, UInt) -> Int): String? = memScoped {
        val buffer = allocArray<ByteVar>(PATH_BUFFER_SIZE)
        if (read(buffer, PATH_BUFFER_SIZE.toUInt()) > 0) buffer.toKString() else null
    }

    /**
     * The paths of one day, walked index by index until the symbol answers that
     * there is nothing at that index: the list the C++ fills a `std::vector`
     * with, asked one at a time.
     */
    private fun dayPaths(read: (UInt, CPointer<ByteVar>, UInt) -> Int): List<String> {
        val walked = mutableListOf<String>()
        var index = 0u
        while (true) {
            val found = pathAt { out, len -> read(index, out, len) } ?: break
            walked.add(found)
            index++
        }
        return walked
    }

    public actual val currentLogPath: String?
        get() {
            val opened = openHandle()
            return if (opened == NO_HANDLE) {
                null
            } else {
                pathAt { out, len ->
                    mars_xlog_current_log_path_instance(opened, out, len)
                }
            }
        }

    public actual fun logFiles(daysAgo: Long): List<String> {
        val opened = openHandle()
        return if (opened == NO_HANDLE) {
            emptyList()
        } else {
            dayPaths { index, out, len ->
                mars_xlog_getfilepath_from_timespan_instance(opened, daysAgoOf(daysAgo), index, out, len)
            }
        }
    }

    public actual fun logFileNames(daysAgo: Long): List<String> {
        val opened = openHandle()
        return if (opened == NO_HANDLE) {
            emptyList()
        } else {
            dayPaths { index, out, len ->
                mars_xlog_make_logfile_name_instance(opened, daysAgoOf(daysAgo), index, out, len)
            }
        }
    }

    public actual fun isLoggable(level: LogLevel): Boolean {
        val opened = openHandle()
        return opened != NO_HANDLE && mars_xlog_is_enabled_for(opened, level.ordinal) != DISABLED
    }

    public actual fun log(level: LogLevel, tag: String, message: String) {
        val opened = openHandle()
        if (opened != NO_HANDLE) {
            mars_xlog_write_instance(opened, level.ordinal, tag, EMPTY, EMPTY, NO_LINE, message)
        }
    }

    public actual fun v(tag: String, message: String) = log(LogLevel.VERBOSE, tag, message)

    public actual fun d(tag: String, message: String) = log(LogLevel.DEBUG, tag, message)

    public actual fun i(tag: String, message: String) = log(LogLevel.INFO, tag, message)

    public actual fun w(tag: String, message: String) = log(LogLevel.WARNING, tag, message)

    public actual fun e(tag: String, message: String) = log(LogLevel.ERROR, tag, message)

    public actual fun f(tag: String, message: String) = log(LogLevel.FATAL, tag, message)

    public actual fun requestFlush() {
        val opened = openHandle()
        if (opened != NO_HANDLE) {
            mars_xlog_request_flush_instance(opened)
        }
    }

    public actual fun flushNow() {
        val opened = openHandle()
        if (opened != NO_HANDLE) {
            mars_xlog_flush_now_instance(opened)
        }
    }

    public actual suspend fun flush() = withContext(Dispatchers.IO) {
        flushNow()
    }

    /**
     * `daysAgo` as the `Int` the C ABI takes. A `Long` outside `Int`'s range
     * wraps rather than fails — `Long.MAX_VALUE` would ask the appender for a
     * day in the future — so what crosses is the day clamped into it.
     */
    private fun daysAgoOf(daysAgo: Long): Int = daysAgo.coerceIn(NO_DAYS_AGO, Int.MAX_VALUE.toLong()).toInt()

    public actual fun close() {
        if (openHandle() == NO_HANDLE) {
            return
        }
        mars_xlog_release_instance(namePrefix)
        handle = NO_HANDLE
    }

    /**
     * The handle of this appender, or [IllegalStateException] when there is none
     * left to forward: a handle whose appender is gone is a no-op to the C ABI,
     * so a closed [Xlog] that handed it on would silently write nothing — and
     * one whose prefix has since been re-opened would move an appender that is
     * not this one's.
     */
    private fun requireOpen(): Long {
        val opened = openHandle()
        check(opened != NO_HANDLE) {
            "no appender of this Xlog is open ('$namePrefix'): Xlog.open(XlogConfig(...)) another to log again"
        }
        return opened
    }

    /**
     * The handle of this appender, [NO_HANDLE] when it is closed, read once:
     * [handle] is what `mars_xlog_new_instance` answered, and the registry is
     * what says the handle is still this [Xlog]'s — an [Xlog] of the same
     * [namePrefix] is closed with this one, and the registry is where that
     * shows.
     *
     * One read and not two, because the two it replaces are not one answer:
     * `isOpen` and then `handle` is a window a [close] on another thread
     * lands in, and what comes out of it is the `0` that [close] wrote. A
     * handle of `0` is one no appender is open for, so the level a wrapper
     * read through it is the `-1` `mars_xlog_get_level` answers for one —
     * which [LogLevel.of] reads as `VERBOSE`, the level that logs
     * everything: an [Xlog] that was closed answered the opposite of what it
     * was asked. A setter handed a `0` is quieter and no better: it moves
     * nothing and still answers that it took the setting.
     */
    private fun openHandle(): Long =
        handle.takeIf { it != NO_HANDLE && it == mars_xlog_get_instance(namePrefix) } ?: NO_HANDLE

    public actual companion object {
        /** What a path symbol writes into: a path never fills it, and a symbol
         * that answers a length rather than a pointer is the C ABI's way of
         * saying the caller decides how much it can hold. */
        const val PATH_BUFFER_SIZE = 1024

        /**
         * Opens an appender of its own: the constructor of this actual, under the
         * one name every platform of the port opens one with.
         */
        public actual fun open(config: XlogConfig): Xlog = Xlog(config)

        /** What `mars_xlog_new_instance` answers for a config it opened nothing for. */
        const val NO_HANDLE = 0L

        /** `0` is today, and no day is before it. */
        const val NO_DAYS_AGO = 0L

        /** `mars_xlog_set_console_log_instance` reads a non-zero `open` as on. */
        const val CONSOLE_LOG_OPEN = 1

        const val CONSOLE_LOG_CLOSED = 0

        /** `mars_xlog_is_enabled_for` answers `0` for a level the appender drops. */
        const val DISABLED = 0

        /** The C ABI reads a max file size of `0` as "never split". */
        const val NO_FILE_SIZE_LIMIT = 0L

        /** The C ABI reads a max alive time of `0` as the C++'s own ten days. */
        const val NO_ALIVE_TIME_LIMIT = 0L

        /** What a Kotlin caller has no macro to fill in: no file, no function, no line. */
        const val EMPTY = ""

        const val NO_LINE = 0

        fun newInstance(config: XlogConfig): Long = memScoped {
            val native = alloc<MarsXLogConfig>()
            native.mode = config.mode.ordinal
            native.log_dir = config.logDir.cstr.ptr
            native.name_prefix = config.namePrefix.cstr.ptr
            native.pub_key = config.pubKey.cstr.ptr
            native.compress_mode = config.compressMode.ordinal
            native.compress_level = config.compressLevel
            native.cache_dir = config.cacheDir?.cstr?.ptr
            native.cache_days = config.cacheDays
            val handle = mars_xlog_new_instance(native.ptr, config.level.ordinal)
            require(handle != NO_HANDLE) {
                "mars_xlog_new_instance opened no appender for ${config.namePrefix} in ${config.logDir}"
            }
            handle
        }
    }
}
