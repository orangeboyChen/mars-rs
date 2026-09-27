package io.github.orangeboychen.marsrs.xlog

/**
 * The levels of a record: `TLogLevel` of the C++ project (`xloggerbase.h`),
 * which the C ABI numbers the same way — `MarsLogLevel` of `mars_xlog.h`.
 *
 * The order is the order of the C enum, so an entry's `ordinal` *is* the number
 * both bridges put on the wire, and the two `actual`s send exactly that.
 * `None` is the level that drops every record: `kLevelNone` in the C++,
 * `MARS_LEVEL_NONE` in the header, and one past `Fatal`, which is what the
 * filter of [Log] compares against.
 */
public enum class LogLevel {
    Verbose,
    Debug,
    Info,
    Warning,
    Error,
    Fatal,
    None
}
