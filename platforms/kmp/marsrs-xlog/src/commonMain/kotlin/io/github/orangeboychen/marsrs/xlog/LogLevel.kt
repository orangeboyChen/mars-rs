package io.github.orangeboychen.marsrs.xlog

/**
 * The levels of a record: `TLogLevel` of the C++ project (`xloggerbase.h`),
 * which the C ABI numbers the same way — `MarsLogLevel` of `mars_xlog.h`.
 *
 * The order is the order of the C enum, so an entry's `ordinal` *is* the number
 * both bridges put on the wire, and the two `actual`s send exactly that.
 * `NONE` is the level that drops every record: `kLevelNone` in the C++,
 * `MARS_LEVEL_NONE` in the header, and one past `FATAL`.
 *
 * The entries are the ones the Android AAR publishes — `LogLevel.INFO` here is
 * `LogLevel.INFO` there — because a shared module that moves between `xlog-kmp`
 * and `xlog` has nothing to rename when it does.
 */
public enum class LogLevel {
    VERBOSE,

    DEBUG,

    INFO,

    WARNING,

    ERROR,

    FATAL,

    /**
     * Nothing is written, not even [FATAL]: the level of an appender that is
     * meant to keep quiet without being closed.
     */
    NONE;

    /**
     * Whether an appender sitting at this level writes a record of [level].
     *
     * A record of [NONE] is the one exception: it is not a severity a record
     * can have, so no appender writes one whatever it is sitting at, and
     * `ordinal <= level.ordinal` — which is the rule for every other level —
     * answers `true` for it. An app that asks before it builds a message and
     * is told `true` spends the cost of building one and gets silence.
     */
    public fun isEnabledFor(level: LogLevel): Boolean = level != NONE && ordinal <= level.ordinal

    public companion object {
        /**
         * The level of a number the bridge speaks.
         *
         * A number outside `0..6` is read the way the bridge reads it: a
         * negative one is [VERBOSE] — what `(TLogLevel)-1`, the C++'s "log
         * everything", meant — and anything above [NONE] is [FATAL].
         */
        public fun of(native: Int): LogLevel = entries.firstOrNull { it.ordinal == native }
            ?: if (native < 0) VERBOSE else FATAL
    }
}
