// The four settings of an appender the C ABI has no getter for.
//
// `mars_xlog.h` gives a setter for each of a mode, a console log, a maximum
// file size and a maximum alive time, and a getter for none of them, so what
// an `Xlog` answers when it is asked for one is the last value this side
// wrote. What that value belongs to is the prefix and not the `Xlog`: a prefix
// is one appender to the C ABI, so two `Xlog`s of one prefix write through the
// same one and a setting either of them set is the other's — and a second
// `Xlog` of a prefix that is open is answered the appender that is open
// *without* the config it asked for being applied to it, which is why the open
// that joins one adopts the values it finds rather than writing its own.
//
// It is here and not in [Xlog] for the reason `path(of:)` is in `LogPath.swift`
// and not beside the call that needs it: the class an app is given is the whole
// surface of `mars_xlog.h` already, and these four are a seam it reads and
// writes them through.

import Foundation

/// The last values the appender of a prefix was given, by the prefix.
internal enum XlogSettings {
    /// The four settings of one appender, at what a fresh one is opened with.
    internal struct Values {
        /// Whether a write reaches the file before it returns.
        internal var mode: AppenderMode = .async

        /// Whether the console prints the record too.
        internal var consoleLogEnabled = false

        /// How many bytes a file may reach before it is closed; `0`, never split.
        internal var fileSizeLimit: UInt64 = 0

        /// How many seconds a file is kept; `0`, the C++'s own ten days.
        internal var aliveTimeLimit: Int64 = 0
    }

    /// What the appender of `namePrefix` was last given, or what a fresh one is
    /// opened with: a prefix no `Xlog` ever opened is one no appender is there
    /// for, and every member that asks writes nothing through it anyway.
    internal static func values(of namePrefix: String) -> Values {
        lock.lock()
        defer { lock.unlock() }
        return byPrefix[namePrefix] ?? Values()
    }

    /// Sets one setting of the appender of `namePrefix`: the whole read and the
    /// whole write are under [lock], so a setting one thread is moving is not
    /// lost under one another thread is moving beside it.
    internal static func set<Value>(
        _ key: WritableKeyPath<Values, Value>,
        of namePrefix: String,
        to value: Value
    ) {
        lock.lock()
        defer { lock.unlock() }
        var updated = byPrefix[namePrefix] ?? Values()
        updated[keyPath: key] = value
        byPrefix[namePrefix] = updated
    }

    /// What an `open` writes for a prefix it made an appender for — and what an
    /// `open` that joined an appender which was there already leaves alone.
    internal static func replace(of namePrefix: String, with values: Values) {
        lock.lock()
        defer { lock.unlock() }
        byPrefix[namePrefix] = values
    }

    /// The values of every prefix, and what makes them one value across
    /// threads: a setting is set from whichever thread an app set it on and
    /// read from whichever thread asks, and Swift gives a stored property no
    /// lock of its own.
    private static var byPrefix: [String: Values] = [:]

    /// What [byPrefix] is read and written under.
    private static let lock = NSLock()
}
