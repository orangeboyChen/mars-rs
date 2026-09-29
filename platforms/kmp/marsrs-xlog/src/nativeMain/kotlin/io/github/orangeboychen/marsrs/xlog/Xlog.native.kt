package io.github.orangeboychen.marsrs.xlog

import io.github.orangeboychen.marsrs.xlog.ffi.MarsXLogConfig
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_flush_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_get_level
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_is_enabled_for
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_new_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_release_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_console_log_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_level_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_max_alive_duration_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_max_file_size_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_mode_instance
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_write_instance
import kotlinx.cinterop.ExperimentalForeignApi
import kotlinx.cinterop.alloc
import kotlinx.cinterop.cstr
import kotlinx.cinterop.memScoped
import kotlinx.cinterop.ptr
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

    private var handle: Long = newInstance(config)

    private var currentMode: AppenderMode = config.mode

    public actual val isOpen: Boolean
        get() = handle != NO_HANDLE

    public actual var level: LogLevel
        get() = LogLevel.of(mars_xlog_get_level(requireOpen()))
        set(value) = mars_xlog_set_level_instance(requireOpen(), value.ordinal)

    public actual var mode: AppenderMode
        get() = currentMode
        set(value) {
            mars_xlog_set_mode_instance(requireOpen(), value.ordinal)
            currentMode = value
        }

    public actual fun isLoggable(level: LogLevel): Boolean =
        isOpen && mars_xlog_is_enabled_for(handle, level.ordinal) != DISABLED

    public actual fun log(level: LogLevel, tag: String, message: String) {
        if (isOpen) {
            mars_xlog_write_instance(handle, level.ordinal, tag, EMPTY, EMPTY, NO_LINE, message)
        }
    }

    public actual fun v(tag: String, message: String) = log(LogLevel.VERBOSE, tag, message)

    public actual fun d(tag: String, message: String) = log(LogLevel.DEBUG, tag, message)

    public actual fun i(tag: String, message: String) = log(LogLevel.INFO, tag, message)

    public actual fun w(tag: String, message: String) = log(LogLevel.WARNING, tag, message)

    public actual fun e(tag: String, message: String) = log(LogLevel.ERROR, tag, message)

    public actual fun f(tag: String, message: String) = log(LogLevel.FATAL, tag, message)

    public actual fun signalFlush() {
        if (isOpen) {
            mars_xlog_flush_instance(handle, ASYNC)
        }
    }

    public actual fun flushNow() {
        if (isOpen) {
            mars_xlog_flush_instance(handle, SYNC)
        }
    }

    public actual suspend fun flush() = withContext(Dispatchers.IO) {
        flushNow()
    }

    public actual fun close() {
        if (!isOpen) {
            return
        }
        mars_xlog_release_instance(namePrefix)
        handle = NO_HANDLE
    }

    /**
     * The handle of this appender, or [IllegalStateException] when there is none
     * left to forward: no handle is the process-wide appender of `mars_xlog_open`
     * to the C ABI, so a closed [Xlog] that handed it on would read and move the
     * appender every other part of the process writes through.
     */
    private fun requireOpen(): Long {
        check(isOpen) {
            "no appender of this Xlog is open ('$namePrefix'): Xlog.open(XlogConfig(...)) another to log again"
        }
        return handle
    }

    public actual companion object {
        /**
         * Opens an appender of its own: the constructor of this actual, under the
         * one name every platform of the port opens one with.
         */
        public actual fun open(config: XlogConfig): Xlog = Xlog(config)

        /** What `mars_xlog_new_instance` answers for a config it opened nothing for. */
        const val NO_HANDLE = 0L

        /** `mars_xlog_set_console_log_instance` reads a non-zero `open` as on. */
        const val CONSOLE_LOG_OPEN = 1

        const val CONSOLE_LOG_CLOSED = 0

        /** `mars_xlog_is_enabled_for` answers `0` for a level the appender drops. */
        const val DISABLED = 0

        /** `mars_xlog_flush_instance` reads a non-zero `sync` as "wait for the write". */
        const val SYNC = 1

        const val ASYNC = 0

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
