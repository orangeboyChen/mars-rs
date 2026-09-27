package io.github.orangeboychen.marsrs.xlog

/**
 * `TAppenderMode` of the C++ project (`appender.h`), which the C ABI spells
 * `MarsAppenderMode`. As in [LogLevel], the order is the order of the C enum,
 * so `ordinal` is the number on the wire.
 *
 * `Async` is what the C++ opens with: the appender writes from its own thread
 * and [Xlog.flush] is what drains it. `Sync` is what a caller who cannot afford
 * to lose the last records of a process opens with.
 */
public enum class AppenderMode {
    Async,
    Sync
}

/**
 * `TCompressMode` of the C++ project, `MarsCompressMode` of the C ABI: how a
 * log file is compressed once it is closed. `Zlib` is the default of both, and
 * `Zstd` is what the port added — the C++ project's own `mars-xlog` has no
 * zstd, so a file written this way is one its reader has to be told about.
 *
 * The order is the order of the C enum, so `ordinal` is the number on the wire.
 */
public enum class CompressMode {
    Zlib,
    Zstd
}

/**
 * `XLogConfig` of the C++ project (`appender.h`) in Kotlin: what [Xlog.open] is
 * handed, instead of the string of positional arguments its Java takes.
 *
 * @property logDir the directory the log files are written to. It is created if
 *   it is missing, and it is the one field the C ABI insists on: without it,
 *   `mars_xlog_open` answers `MARS_XLOG_ERR_EMPTY_LOG_DIR` and opens nothing.
 * @property namePrefix the name every log file starts with, as it is in the
 *   C++; `null` writes files the C++'s own default name would give them.
 * @property level the level the appender is opened at: a record below it is
 *   dropped before it is formatted.
 * @property mode whether the appender writes from its own thread.
 * @property pubKey the ECDH public key a log file is encrypted with; `null`
 *   writes it unencrypted, the way an empty key does in the C++.
 * @property compressMode how a closed log file is compressed.
 * @property compressLevel the level of that compression; `0` is what the C++
 *   passes on — 6, the appender's own default.
 * @property cacheDir where the mmap cache the appender keeps lives; `null` puts
 *   it in [logDir], as the C++ does.
 * @property cacheDays how many days of log files are kept; `0` keeps all of
 *   them, which is what the C++'s default is.
 */
public data class XlogConfig(
    public val logDir: String,
    public val namePrefix: String? = null,
    public val level: LogLevel = LogLevel.Info,
    public val mode: AppenderMode = AppenderMode.Async,
    public val pubKey: String? = null,
    public val compressMode: CompressMode = CompressMode.Zlib,
    public val compressLevel: Int = 0,
    public val cacheDir: String? = null,
    public val cacheDays: Int = 0
)
