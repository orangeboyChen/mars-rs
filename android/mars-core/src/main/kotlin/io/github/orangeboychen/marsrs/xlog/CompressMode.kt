package io.github.orangeboychen.marsrs.xlog

/**
 * What an appender compresses a log file with once it is closed and no longer
 * written to, the pair the C++ project's `ZLIB_MODE`/`ZSTD_MODE` are.
 *
 * [native] is the number `mars-jni` reads out of `Xlog.XLogConfig.compressmode`.
 */
enum class CompressMode(internal val native: Int) {
    /**
     * zlib, the mode the C++ project opens with and the one every reader of a
     * mars log file understands.
     */
    ZLIB(0),

    /**
     * Zstandard: smaller at the same speed, and unreadable by a tool that
     * only knows zlib.
     */
    ZSTD(1)
}
