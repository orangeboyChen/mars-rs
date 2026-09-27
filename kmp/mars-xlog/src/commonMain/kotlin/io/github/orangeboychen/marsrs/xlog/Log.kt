package io.github.orangeboychen.marsrs.xlog

/**
 * `Log.java` of the C++ project: `Log.d(tag, message)` and friends over [Xlog],
 * plus a level of its own on top of the appender's.
 *
 * That level is [LogLevel.None] until an app says otherwise, so a library that
 * writes through this facade before the app has asked for anything writes
 * nothing — the opposite of what a `Log` that defaults to verbose does.
 */
public object Log {
    private var level: LogLevel = LogLevel.None

    /** The level below which a record is dropped before [Xlog] is asked for it. */
    public fun setLevel(level: LogLevel) {
        this.level = level
    }

    public fun v(tag: String, message: String) {
        write(LogLevel.Verbose, tag, message)
    }

    public fun d(tag: String, message: String) {
        write(LogLevel.Debug, tag, message)
    }

    public fun i(tag: String, message: String) {
        write(LogLevel.Info, tag, message)
    }

    public fun w(tag: String, message: String) {
        write(LogLevel.Warning, tag, message)
    }

    public fun e(tag: String, message: String) {
        write(LogLevel.Error, tag, message)
    }

    public fun f(tag: String, message: String) {
        write(LogLevel.Fatal, tag, message)
    }

    private fun write(level: LogLevel, tag: String, message: String) {
        if (level.ordinal >= this.level.ordinal) {
            Xlog.write(level, tag, message)
        }
    }
}
