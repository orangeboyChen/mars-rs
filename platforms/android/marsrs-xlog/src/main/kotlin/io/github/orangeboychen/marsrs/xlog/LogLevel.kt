// The levels, the modes and the config of the Kotlin API — what `Xlog(...)` and
// `xlog.i(tag, message)` are written in. Nothing here is a native, and
// nothing here is reached by JNI: the numbers these carry are handed to `Xlog`,
// whose statics are the symbols `marsrs-jni` exports, and that class is the only
// door to the `.so`.
//
// They are named the way the Kotlin of this port already names things, which
// is why `XlogConfig` sits next to `Xlog.XLogConfig` and `LogLevel.INFO`
// next to `Xlog.LEVEL_INFO`: the first of each pair is what new code writes,
// the second is what the API the C++ project's Java spelled still answers.
// Both reach one appender.

package io.github.orangeboychen.marsrs.xlog

// The numbers `marsrs-jni` speaks, one per level of the C++ project's
// `TLogLevel`. They sit outside the enum because an enum entry's argument
// cannot reach into the enum's own companion object — `VERBOSE(0)` and
// `VERBOSE(LEVEL_VERBOSE)` are the same class file otherwise.

private object NativeLevel {
    const val VERBOSE = 0
    const val DEBUG = 1
    const val INFO = 2
    const val WARNING = 3
    const val ERROR = 4
    const val FATAL = 5
    const val NONE = 6
}

/**
 * How severe a log record is, in the order the C++ project's `TLogLevel` is —
 * [VERBOSE] lets everything through and [NONE] lets nothing through.
 *
 * [native] is the number `marsrs-jni` speaks: the one `Xlog.getLogLevel`
 * answers with, the one `Xlog.setLogLevel` reads, and the one the
 * `Xlog.LEVEL_*` constants of the older API spell. [of] is the way back from
 * that number, for a level that arrives from somewhere else — a remote
 * config, a saved preference.
 *
 * A record is written when the level of the appender it goes through is at
 * most its own: an appender opened at [INFO] keeps [WARNING] and drops
 * [DEBUG].
 */
enum class LogLevel(internal val native: Int) {
    VERBOSE(NativeLevel.VERBOSE),

    DEBUG(NativeLevel.DEBUG),

    INFO(NativeLevel.INFO),

    WARNING(NativeLevel.WARNING),

    ERROR(NativeLevel.ERROR),

    FATAL(NativeLevel.FATAL),

    /**
     * Nothing is written, not even [FATAL]: the level of an appender that is
     * meant to keep quiet without being closed.
     */
    NONE(NativeLevel.NONE)
    ;

    /**
     * Whether an appender sitting at this level writes a record of [level].
     */
    fun isEnabledFor(level: LogLevel): Boolean = native <= level.native

    companion object {
        /**
         * The level of a number `marsrs-jni` speaks.
         *
         * A number outside `0..6` is read the way `marsrs-jni` reads it: a
         * negative one is [VERBOSE] — what `(TLogLevel)-1`, the C++'s "log
         * everything", meant — and anything above [NONE] is [FATAL].
         */
        @JvmStatic
        fun of(native: Int): LogLevel = entries.firstOrNull { it.native == native }
            ?: if (native < 0) VERBOSE else FATAL
    }
}
