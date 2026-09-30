package io.github.orangeboychen.marsrs.xlog

/**
 * `TAppenderMode` of the C++ project (`appender.h`), which the C ABI spells
 * `MarsAppenderMode`. As in [LogLevel], the order is the order of the C enum,
 * so `ordinal` is the number on the wire, and the entries are the ones the
 * Android AAR publishes.
 *
 * `ASYNC` is what the C++ opens with: the appender writes from its own thread
 * and [Xlog.flushNow] is what drains it. `SYNC` is what a caller who cannot afford
 * to lose the last records of a process opens with.
 */
public enum class AppenderMode {
    ASYNC,
    SYNC
}

/**
 * `TCompressMode` of the C++ project, `MarsCompressMode` of the C ABI: how a
 * log file is compressed once it is closed. `ZLIB` is the default of both, and
 * `ZSTD` is what the port added — the C++ project's own `mars-xlog` has no
 * zstd, so a file written this way is one its reader has to be told about.
 *
 * The order is the order of the C enum, so `ordinal` is the number on the wire.
 */
public enum class CompressMode {
    ZLIB,
    ZSTD
}

/**
 * `XLogConfig` of the C++ project (`appender.h`) in Kotlin: what an [Xlog] is
 * built with, instead of the string of positional arguments its Java takes.
 *
 * Every property has the default the C++ project's own `XLogConfig` carries, so
 * the only one an app has to give is [logDir], and a config the appender cannot
 * honour is refused here and not silently by the bridge: `mars_xlog_open`
 * answers a config it does not like by opening nothing, and an app that finds
 * out three days later that it has no logs has no way back.
 *
 * @property logDir the directory the log files are written to. It is created if
 *   it is missing, and it is the one field the C ABI insists on: without it,
 *   `mars_xlog_open` answers `MARS_XLOG_ERR_EMPTY_LOG_DIR` and opens nothing.
 * @property namePrefix the name every log file starts with (`marsrs_20260927.xlog`),
 *   and the name the appender is known by — an app that writes through two of
 *   them gives them two.
 * @property level the level the appender is opened at: a record below it is
 *   dropped before it is formatted. [Xlog.level] moves it afterwards.
 * @property mode whether the appender writes from its own thread.
 * @property pubKey the ECDH public key a log file is encrypted with; empty
 *   writes it unencrypted, the way an empty key does in the C++.
 * @property compressMode how a closed log file is compressed.
 * @property compressLevel the level of that compression; `0` is what the C++
 *   passes on — 6, the appender's own default. The ceiling is the compressor's
 *   and not one number for both: `9` for [CompressMode.ZLIB] and `22` for
 *   [CompressMode.ZSTD].
 * @property cacheDir where the mmap cache the appender keeps lives; `null` puts
 *   it in [logDir], as the C++ does.
 * @property cacheDays how many days of log files are kept; `0` keeps all of
 *   them, which is what the C++'s default is.
 */
public data class XlogConfig(
    public val logDir: String,
    public val namePrefix: String = DEFAULT_NAME_PREFIX,
    public val level: LogLevel = LogLevel.INFO,
    public val mode: AppenderMode = AppenderMode.ASYNC,
    public val pubKey: String = DEFAULT_PUB_KEY,
    public val compressMode: CompressMode = CompressMode.ZLIB,
    public val compressLevel: Int = DEFAULT_COMPRESS_LEVEL,
    public val cacheDir: String? = null,
    public val cacheDays: Int = NO_CACHE_DAYS
) {
    init {
        require(logDir.isNotBlank()) { "logDir must not be blank: no appender is opened without one" }
        require(namePrefix.isNotBlank()) { "namePrefix must not be blank: it is what an appender is known by" }
        require(cacheDays >= 0) { "cacheDays must not be negative, was $cacheDays" }
        // The ceiling is the compressor's, and not one number for both: zstd
        // takes levels zlib has no meaning for, and a config that opens on
        // every other platform of the port — the Android AAR's own, and
        // Swift's, which reads `0...22` for zstd — was refused here for being
        // above zlib's `9`.
        val maxLevel = if (compressMode == CompressMode.ZSTD) MAX_ZSTD_COMPRESS_LEVEL else MAX_ZLIB_COMPRESS_LEVEL
        require(compressLevel in DEFAULT_COMPRESS_LEVEL..maxLevel) {
            "compressLevel must be in $DEFAULT_COMPRESS_LEVEL..$maxLevel for $compressMode, was $compressLevel"
        }
    }

    private companion object {
        const val DEFAULT_NAME_PREFIX = "xlog"

        const val DEFAULT_PUB_KEY = ""

        const val DEFAULT_COMPRESS_LEVEL = 0

        const val MAX_ZLIB_COMPRESS_LEVEL = 9

        /** `ZSTD_maxCLevel()`, and what the appender refuses a level above. */
        const val MAX_ZSTD_COMPRESS_LEVEL = 22

        const val NO_CACHE_DAYS = 0
    }
}
