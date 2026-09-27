package io.github.orangeboychen.marsrs.xlog

import io.github.orangeboychen.marsrs.xlog.ffi.MarsXLogConfig
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_close
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_flush
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_flush_sync
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_open
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_console_log
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_level
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_max_alive_duration
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_set_max_file_size
import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_write
import kotlinx.cinterop.ExperimentalForeignApi
import kotlinx.cinterop.alloc
import kotlinx.cinterop.cstr
import kotlinx.cinterop.memScoped
import kotlinx.cinterop.ptr

/**
 * The Kotlin/Native `actual`: the C ABI of `crates/mars-ffi` (`mars_xlog.h`)
 * through cinterop, which is the same source set for every native target —
 * iOS, watchOS, tvOS, macOS, Linux and Windows — because the C ABI is the same
 * header on all of them.
 *
 * `mars_xlog_open` answers an `MARS_XLOG_ERR_*` code that the common API has no
 * room for, the JNI bridge answers nothing at all, and the C++'s
 * `appender_open` answers `void` as well: a caller who wants to know whether the
 * appender opened is a caller of one platform.
 *
 * The strings of the C ABI are Kotlin strings at a call: cinterop maps the
 * `const char*` parameter of `mars_xlog.h` to `String?`, which is what lets the
 * optional ones be passed as `null` — the very thing the header's "NULL is
 * treated as empty" says. The fields of `MarsXLogConfig` stay pointers, so
 * those four are converted here, inside the `memScoped` the config lives in.
 */
@OptIn(ExperimentalForeignApi::class)
public actual object Xlog {
    public actual fun open(config: XlogConfig) {
        memScoped {
            val native = alloc<MarsXLogConfig>()
            native.mode = config.mode.ordinal
            native.log_dir = config.logDir.cstr.ptr
            native.name_prefix = config.namePrefix?.cstr?.ptr
            native.pub_key = config.pubKey?.cstr?.ptr
            native.compress_mode = config.compressMode.ordinal
            native.compress_level = config.compressLevel
            native.cache_dir = config.cacheDir?.cstr?.ptr
            native.cache_days = config.cacheDays
            mars_xlog_open(native.ptr)
        }
        // The level is not part of `MarsXLogConfig`: the C++ sets it after
        // `appender_open` too, and the JNI bridge reads it out of its config and
        // does the same.
        mars_xlog_set_level(config.level.ordinal)
    }

    public actual fun write(level: LogLevel, tag: String, message: String, file: String, function: String, line: Int) {
        mars_xlog_write(level.ordinal, tag, file, function, line, message)
    }

    public actual fun flush(sync: Boolean) {
        if (sync) {
            mars_xlog_flush_sync()
        } else {
            mars_xlog_flush()
        }
    }

    public actual fun close() {
        mars_xlog_close()
    }

    public actual fun setLevel(level: LogLevel) {
        mars_xlog_set_level(level.ordinal)
    }

    public actual fun setConsoleLog(open: Boolean) {
        mars_xlog_set_console_log(if (open) CONSOLE_LOG_OPEN else CONSOLE_LOG_CLOSED)
    }

    public actual fun setMaxFileSize(bytes: Long) {
        mars_xlog_set_max_file_size(bytes.toULong())
    }

    public actual fun setMaxAliveTime(seconds: Long) {
        mars_xlog_set_max_alive_duration(seconds)
    }

    private const val CONSOLE_LOG_OPEN = 1

    private const val CONSOLE_LOG_CLOSED = 0
}
