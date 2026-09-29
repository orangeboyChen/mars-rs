package io.github.orangeboychen.marsrs.xlog

/**
 * Whether a write reaches the file before it returns, the pair the C++ project's
 * `AppednerModeAsync`/`AppednerModeSync` are.
 *
 * [native] is the number `marsrs-jni` reads out of `Xlog.XLogConfig.mode` and
 * out of `Xlog.setAppenderMode`.
 */
enum class AppenderMode(internal val native: Int) {
    /**
     * The record goes into the memory-mapped cache file and a writer thread
     * takes it to the log file. Cheap enough to log from anywhere, and the
     * only mode that keeps its ordering across threads; the last records of a
     * process that dies are in the cache and not in the file, which is what
     * [Xlog.flushNow] is for.
     */
    ASYNC(0),

    /**
     * The record is in the log file before the call returns, at the cost of a
     * write and a lock per line. For a handful of lines that must survive a
     * crash, not for the whole app.
     */
    SYNC(1)
}
