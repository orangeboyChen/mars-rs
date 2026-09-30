// The Swift face of the C ABI in `mars_xlog.h` (crate `marsrs-ffi`).
//
// This is the `MarsRSXlog` module, the xlog half of the port: `import MarsRSXlog`
// is what an app that only logs takes. `MarsRS` is the whole port and re-exports
// this module, so `Stn.swift` and `Sdt.swift` join the umbrella as `marsrs-ffi`
// grows past the logging half.
//
// The binary target of Package.swift is a static library plus a module map, so
// what `import MarsRSFFI` gives a caller is the C surface itself: pointers to
// C strings, `int` modes and a config struct that has to be filled field by
// field. This file is that surface in Swift — `String`s, an `XlogConfig` with
// defaults and a value an app writes through — and it re-exports the C module,
// so `mars_xlog_new_instance(&config, level)` and friends stay reachable for
// whoever prefers them.
//
// The shape is the one the Android `Xlog` has, and the one the C ABI spells with
// a handle: `Xlog.open(config)` opens an appender of its own and answers it, and
// the app writes through what it was given. Nothing here is deprecated, because
// there is no older Swift API to keep — an instance is the only way in, and
// `import MarsRSFFI` reaches the symbols behind it. Every question about a file is asked of the `Xlog` it belongs to —
// [`currentLogPath`], [`logFiles(daysAgo:)`] and [`logFileNames(daysAgo:)`] —
// which is the shape the Kotlin Multiplatform module has: `Xlog.open` is the one
// call that is not an appender's own.
//
// Every call is a straight translation of a symbol in the header; nothing here
// adds behaviour the C ABI does not have.
//
// The class and every member an app reaches are `@objc`, and `Xlog` is an
// `NSObject` — but for the `async` `flush()`, which no Objective-C caller can
// `await`. An app written in Objective-C takes the port
// through `[[Xlog alloc] initWithConfig:error:]`, and the `Xlog` it writes is
// the one the compiler writes out of this file, into `MarsRSXlog-Swift.h` —
// there is no Objective-C source in the port, the linkage is Swift's own.

import Foundation

// Re-exported so that `import MarsRSXlog` also gives the C symbols.
@_exported import MarsRSFFI

/// An appender of an app's own — build one when the app starts, then write
/// through it from wherever there is something to say.
///
/// ```swift
/// let log = try Xlog.open(
///     XlogConfig(
///         logDirectory: logDirectory.path,
///         cacheDirectory: cacheDirectory.path,
///         namePrefix: "marsrs",
///         level: .info
///     )
/// )
/// log.isConsoleLogEnabled = true
///
/// log.info(message: "cold start in \(elapsedMillis) ms", tag: "startup")
/// log.error(message: "login failed\n\(error)", tag: "login")
/// ```
///
/// A write is a message with a tag beside it, the pair every platform of the
/// port spells; Swift takes the message first and lets the tag default to
/// empty, where the Android `xlog.i(tag, message)` asks for both — a Kotlin
/// default argument is a second overload, and a Swift one is free. Swift adds
/// what Swift can fill in for itself — `file`, `function` and `line` come from
/// the call site, because the C ABI carries them and `#file` costs no stack
/// walk. A record is dropped before anything is formatted when its level is
/// below [level], and a message that is expensive to build is worth an
/// `if log.isEnabled(for: .debug)` first.
///
/// Two `Xlog`s of one `namePrefix` are one appender: the C ABI answers the
/// handle it already has, so closing one of them closes what the other writes
/// through.
@objc
public final class Xlog: NSObject {
    /// What every file of this appender starts with, and what it is known by.
    @objc public let namePrefix: String

    /// Whether this appender is still open: `false` after [close()] — on this
    /// `Xlog` and on every other one of this `namePrefix`, which is the same
    /// appender and is closed with this one.
    ///
    /// The prefix and not the handle alone: a prefix is one appender to the C
    /// ABI, so two `Xlog`s of one prefix are answered the same handle and
    /// closing either releases it — the handle this one cached still looks
    /// open, and a write through it would silently write nothing.
    @objc public var isOpen: Bool {
        openHandle() != Self.noHandle
    }

    /// The level of this appender: a record less severe than this is dropped.
    ///
    /// Read from the C ABI and not mirrored here, so a level another part of
    /// the app set is the one this answers with.
    @objc public var level: LogLevel {
        get {
            // A closed `Xlog` — one whose twin closed the appender they
            // share — writes nothing, which is `none` and not "log
            // everything": the `-1` `mars_xlog_get_level` answers for a
            // handle that is not one is `(TLogLevel)-1`, the C++'s own
            // spelling of the opposite.
            let opened = openHandle()
            guard opened != Self.noHandle else {
                return .none
            }
            // A raw value [LogLevel] does not carry is `none` and not
            // `verbose`, and for the reason above: the value an appender that
            // is not there answers is `-1`, and `verbose` is the level that
            // logs everything — the opposite of what was asked.
            return LogLevel(rawValue: mars_xlog_get_level(opened)) ?? .none
        }
        set {
            withHandle { mars_xlog_set_level_instance($0, newValue.rawValue) }
        }
    }

    /// Whether a write reaches the file before it returns: what the
    /// `XlogConfig` gave, until this says otherwise. The C ABI has no getter
    /// for it, so this is the last value this side wrote.
    @objc public var mode: AppenderMode {
        get {
            currentMode
        }
        set {
            currentMode = newValue
            withHandle { mars_xlog_set_mode_instance($0, newValue.rawValue) }
        }
    }

    /// Whether the console prints the log too — off until an app turns it on.
    @objc public var isConsoleLogEnabled: Bool = false {
        didSet {
            withHandle { mars_xlog_set_console_log_instance($0, isConsoleLogEnabled ? 1 : 0) }
        }
    }

    /// How many bytes a log file may reach before it is closed and a new one
    /// opened; `0` is "never split".
    @objc public var maxFileSizeBytes: UInt64 = 0 {
        didSet {
            withHandle { mars_xlog_set_max_file_size_instance($0, maxFileSizeBytes) }
        }
    }

    /// How many seconds a log file is kept; `0` is the C++'s own ten days.
    @objc public var maxAliveTimeSeconds: Int64 = 0 {
        didSet {
            withHandle { mars_xlog_set_max_alive_duration_instance($0, maxAliveTimeSeconds) }
        }
    }

    /// Whether a record of `level` would be written: what an app asks before it
    /// builds a message that is expensive to build.
    @objc
    public func isEnabled(for level: LogLevel) -> Bool {
        let opened = openHandle()
        guard opened != Self.noHandle else {
            return false
        }
        return mars_xlog_is_enabled_for(opened, level.rawValue) != 0
    }

    /// Writes a record of `level`.
    ///
    /// `file`, `function` and `line` come from the call site in Swift, and are
    /// written out by the caller in Objective-C.
    @objc
    public func log(
        _ level: LogLevel,
        message: String,
        tag: String = "",
        file: String = #file,
        function: String = #function,
        line: Int32 = #line
    ) {
        let opened = openHandle()
        guard opened != Self.noHandle else {
            return
        }
        // `cTag` and friends: the same four strings as C pointers, which is
        // what the closure hands back and what the C ABI copies out of.
        withCStrings(first: tag, second: file, third: function, fourth: message) { cTag, cFile, cFunction, cMessage in
            mars_xlog_write_instance(opened, level.rawValue, cTag, cFile, cFunction, line, cMessage)
        }
    }

    /// Writes a record of `level` the way Objective-C writes one:
    /// `[log writeWithLevel:message:tag:]`.
    ///
    /// Swift writes [log(_:message:tag:file:function:line:)], which fills the
    /// file, the function and the line in at the call site; Objective-C has no
    /// default argument to fill in and no `#file` to fill one with, so a record
    /// written here carries an empty file, an empty function and the line 0 —
    /// what the C ABI does with them. Where a record was written goes *in* the
    /// record through [log(_:message:tag:file:function:line:)], which is where
    /// `__FILE__`, `__PRETTY_FUNCTION__` and `__LINE__` go.
    @objc(writeWithLevel:message:tag:)
    public func write(_ level: LogLevel, message: String, tag: String) {
        log(level, message: message, tag: tag, file: "", function: "", line: 0)
    }

    /// `LogLevel.verbose`.
    public func verbose(
        message: String,
        tag: String = "",
        file: String = #file,
        function: String = #function,
        line: Int32 = #line
    ) {
        log(.verbose, message: message, tag: tag, file: file, function: function, line: line)
    }

    /// `LogLevel.debug`.
    public func debug(
        message: String,
        tag: String = "",
        file: String = #file,
        function: String = #function,
        line: Int32 = #line
    ) {
        log(.debug, message: message, tag: tag, file: file, function: function, line: line)
    }

    /// `LogLevel.info`.
    public func info(
        message: String,
        tag: String = "",
        file: String = #file,
        function: String = #function,
        line: Int32 = #line
    ) {
        log(.info, message: message, tag: tag, file: file, function: function, line: line)
    }

    /// `LogLevel.warning`.
    public func warning(
        message: String,
        tag: String = "",
        file: String = #file,
        function: String = #function,
        line: Int32 = #line
    ) {
        log(.warning, message: message, tag: tag, file: file, function: function, line: line)
    }

    /// `LogLevel.error`.
    public func error(
        message: String,
        tag: String = "",
        file: String = #file,
        function: String = #function,
        line: Int32 = #line
    ) {
        log(.error, message: message, tag: tag, file: file, function: function, line: line)
    }

    /// `LogLevel.fatal`.
    public func fatal(
        message: String,
        tag: String = "",
        file: String = #file,
        function: String = #function,
        line: Int32 = #line
    ) {
        log(.fatal, message: message, tag: tag, file: file, function: function, line: line)
    }

    /// Tells the writer thread to take what is in the cache to the log file,
    /// and returns at once: the drain is the writer's, and nothing here says
    /// when it is over. What it is for is a drain an app wants soon and does
    /// not want to wait for — a record still in the cache is in a file the
    /// kernel holds, so nothing is lost by a drain that has not happened yet.
    @objc
    public func requestFlush() {
        withHandle { mars_xlog_request_flush_instance($0) }
    }

    /// Takes what is in the cache to the log file on the calling thread, and
    /// hands the file's own buffer to the OS — the last few KiB of a log file
    /// are in a `FILE*` until this runs, so a reader in another process cannot
    /// see them yet. The records are on disk when it returns, and what it
    /// costs is the time the drain takes; `await flush()` waits for the
    /// same thing off this thread.
    @objc
    public func flushNow() {
        withHandle { mars_xlog_flush_now_instance($0) }
    }

    /// [flushNow()] for a caller that can wait without holding a thread: the
    /// records are on disk when this returns, and what waited for them is a
    /// parked queue thread.
    ///
    /// A drain blocks the thread it runs on, and blocking a thread of the
    /// cooperative pool is what `async` asks a caller not to do — so this one
    /// goes to a queue. It is not cancelled with the task that asked for it.
    @available(iOS 13.0, macOS 10.15, *)
    public func flush() async {
        // The handle and not `self`: the closure runs on another thread, and
        // `Xlog` is not `Sendable`, so strict concurrency refuses the capture.
        let opened = openHandle()
        guard opened != Self.noHandle else {
            return
        }
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            Self.flushQueue.async {
                mars_xlog_flush_now_instance(opened)
                continuation.resume()
            }
        }
    }

    /// Closes this appender: drains what is left and drops it. Writing through
    /// this `Xlog` afterwards writes nothing, and asking the C ABI for this
    /// `namePrefix` answers `0`. Safe to call twice.
    @objc
    public func close() {
        Self.registryLock.lock()
        defer { Self.registryLock.unlock() }
        guard openHandle() != Self.noHandle else {
            return
        }
        // Releasing takes the prefix and not the handle, so it drops whatever
        // the prefix answers *now* — and [openHandle] has just asked the
        // registry and been answered this one, with [registryLock] holding an
        // open of that prefix outside the window between the question and the
        // release. A prefix is one appender to the C ABI, so two `Xlog`s of
        // one prefix hold one handle between them and this closes both.
        namePrefix.withCString { prefix in
            mars_xlog_release_instance(prefix)
        }
        handleLock.lock()
        handle = Self.noHandle
        handleLock.unlock()
    }

    /// `mars_xlog_current_log_path_instance`: the directory this appender writes
    /// its files into, or `nil` once it is closed (or the buffer was too small,
    /// which 1024 bytes never is).
    ///
    /// It is a directory and not a file, because that is what the C++ answers
    /// (`XloggerAppender::GetCurrentLogPath` hands back `sg_logdir`); the day's
    /// file is what [`logFiles(daysAgo:)`] names.
    @objc public var currentLogPath: String? {
        let opened = openHandle()
        guard opened != Self.noHandle else {
            return nil
        }
        return path { out, len in mars_xlog_current_log_path_instance(opened, out, len) }
    }

    /// `mars_xlog_getfilepath_from_timespan_instance`: the log files of `daysAgo` days ago
    /// that are *there* — what an app that uploads yesterday's opens. `[]` when
    /// the directory holds none of that day's. `0` is today, `1` is yesterday,
    /// and so on.
    ///
    /// This is a day of files and not the file being written: what
    /// [`currentLogPath`] answers is the directory, and this names the day's
    /// files in it — the day of *this* appender, its own prefix and directory.
    ///
    /// `daysAgo` is an `Int`, and the C ABI takes an `Int32`: what crosses is
    /// `Int32(exactly:)` and not `Int32(_:)`, which traps on a value it cannot
    /// represent rather than answering nothing.
    @objc
    public func logFiles(daysAgo: Int) -> [String] {
        let opened = openHandle()
        guard opened != Self.noHandle, let timespan = Int32(exactly: daysAgo) else {
            return []
        }
        return paths { index, out, len in
            mars_xlog_getfilepath_from_timespan_instance(opened, timespan, index, out, len)
        }
    }

    /// `mars_xlog_make_logfile_name_instance`: the paths of the log files of `daysAgo` days
    /// ago whether or not they are *there yet* — the name an app that is about to
    /// write, or that is naming a file to someone else, asks for.
    ///
    /// A day's answer is the log-dir file and, when a cache dir is configured and
    /// the file exists, its cache-dir twin, so this can answer two where
    /// [`logFiles(daysAgo:)`] answers one.
    @objc
    public func logFileNames(daysAgo: Int) -> [String] {
        let opened = openHandle()
        guard opened != Self.noHandle, let timespan = Int32(exactly: daysAgo) else {
            return []
        }
        return paths { index, out, len in
            mars_xlog_make_logfile_name_instance(opened, timespan, index, out, len)
        }
    }

    /// Opens an appender of its own: its own log directory, file name prefix,
    /// key, mode and cache file, all of them `config`'s — the one call every
    /// platform of the port opens one with, under the one name:
    /// `Xlog.open(config)` in Kotlin, in TypeScript and in Dart. Swift has a
    /// constructor that does the same thing, and this is the name an app still
    /// wants: an appender is opened for the process, not for an expression.
    ///
    /// The prefix is what the appender is known by — and what every one of its
    /// files starts with — so an app that wants two gives them two. Asking for
    /// a prefix that is already open answers the appender that is open and not
    /// a second one.
    ///
    /// - Parameter config: what to open it with.
    /// - Returns: the appender `config` asked for.
    /// - Throws: `XlogError` when the config is one the C ABI refuses, which is
    ///           what an empty log directory comes to. Objective-C reads the
    ///           same thing out of the `NSError` it hands in.
    public static func open(_ config: XlogConfig) throws -> Xlog {
        try Xlog(config)
    }

    /// `Xlog.open(_:)`, as a constructor: the same appender, and the same
    /// `XlogError` for a configuration the C ABI refuses — which Objective-C
    /// reads out of the `NSError` it hands in.
    @objc(initWithConfig:error:)
    public init(_ config: XlogConfig) throws {
        guard !config.logDirectory.isEmpty else {
            throw XlogError.emptyLogDirectory
        }
        guard !config.namePrefix.isEmpty else {
            throw XlogError.emptyNamePrefix
        }
        let ceiling = Self.maxCompressionLevel(for: config.compression)
        guard config.compressionLevel >= 0, config.compressionLevel <= ceiling else {
            throw XlogError.invalidCompressionLevel
        }
        guard config.cacheDays >= 0 else {
            throw XlogError.negativeCacheDays
        }

        // An open is the other half of [registryLock]: a close that has asked
        // the registry and been answered cannot have this prefix's new handle
        // taken out from under it by the open that follows.
        Self.registryLock.lock()
        let opened = withCStrings(
            first: config.logDirectory,
            second: config.namePrefix,
            third: config.publicKey,
            fourth: config.cacheDirectory ?? ""
        ) { directory, prefix, key, cache -> Int64 in
            var cConfig = MarsXLogConfig(
                mode: config.mode.rawValue,
                log_dir: directory,
                name_prefix: prefix,
                pub_key: key,
                compress_mode: config.compression.rawValue,
                compress_level: config.compressionLevel,
                cache_dir: cache,
                cache_days: config.cacheDays
            )
            return mars_xlog_new_instance(&cConfig, config.level.rawValue)
        }
        Self.registryLock.unlock()
        guard opened != Self.noHandle else {
            throw XlogError.refused
        }

        self.namePrefix = config.namePrefix
        self.handle = opened
        self.currentMode = config.mode
        super.init()
        XlogBackgroundFlush.shared.add(self)
    }

    deinit {
        close()
    }

    /// The handle `mars_xlog_new_instance` answered with; `0` once [close()] ran.
    ///
    /// Under [handleLock] and not plain: [close()] writes it from whichever
    /// thread an app closed on, and every member reads it from whichever
    /// thread it was called on. Two threads touching one property without
    /// that is a race on eight bytes, and a torn read of a handle is a write
    /// through an appender that is not this one's.
    private var handle: Int64

    /// What makes [handle] one value across threads: [close()] takes it on the
    /// thread it was called on, and a record is written from any thread at
    /// all.
    private let handleLock = NSLock()

    /// What [mode] answers while this side is the only one that knows it.
    private var currentMode: AppenderMode

    /// The queue an `await flush()` waits for the disk on: serial, because a
    /// drain holds the appender's lock from the cache to the OS anyway.
    private static let flushQueue = DispatchQueue(label: "io.github.orangeboychen.marsrs.xlog.flush")

    /// The handle the C ABI answers for an appender it did not open.
    private static let noHandle: Int64 = 0

    /// What makes [close()] one step and not two: a close releases by prefix,
    /// so the question it asks of the registry and the release that trusts the
    /// answer are held together, and an open of the same prefix — the thing
    /// that would otherwise land between them and be closed in this one's
    /// place — waits outside. One lock for every `Xlog` of the process,
    /// because the appender a prefix names is one for the whole process.
    private static let registryLock = NSLock()

    /// `COMPRESS_LEVEL9`: the hardest deflate is asked to try.
    private static let maxZlibCompressionLevel: Int32 = 9

    /// `ZSTD_maxCLevel()`: the hardest zstd is asked to try, and the ceiling
    /// of the table a level like 19 is read out of.
    private static let maxZstdCompressionLevel: Int32 = 22

    /// The hardest the compressor of `mode` is asked to try: nine levels is
    /// all a deflate has, where zstd's table runs to `ZSTD_maxCLevel()`.
    private static func maxCompressionLevel(for mode: CompressMode) -> Int32 {
        mode == .zstd ? Self.maxZstdCompressionLevel : Self.maxZlibCompressionLevel
    }

    /// Runs `body` with this appender's handle, and runs nothing at all once
    /// [close()] has: handle `0` names no appender at all, so a call through
    /// it would silently write nothing.
    private func withHandle(_ body: (Int64) -> Void) {
        let opened = openHandle()
        guard opened != Self.noHandle else {
            return
        }
        body(opened)
    }

    /// The handle of this appender, [noHandle] when it is closed: what every
    /// member reads once and then makes its call through, rather than asking
    /// [isOpen] and reading [handle] again for the call that question guards.
    ///
    /// One read and not two, because the two are not one answer: a [close()]
    /// on another thread lands between them, and what the second one reads
    /// is the `0` the close wrote. Handle `0` is one no appender is open
    /// for, so the level this `Xlog` answered through it was the `-1` the C
    /// ABI answers — `(TLogLevel)-1`, which [LogLevel] reads as `verbose`,
    /// the level that logs everything: a closed `Xlog` answered the opposite
    /// of what it was asked. A setter handed a `0` is quieter and no better:
    /// it moves nothing, and still answers that it took the setting.
    ///
    /// Asked of the C ABI's registry and not of the handle alone, which is
    /// the other half of [isOpen]: a prefix is one appender, so an `Xlog` of
    /// the same [namePrefix] is closed with this one, and the registry is
    /// where that shows.
    private func openHandle() -> Int64 {
        handleLock.lock()
        defer { handleLock.unlock() }
        let opened = handle
        guard opened != Self.noHandle else {
            return Self.noHandle
        }
        let owns = namePrefix.withCString { prefix in
            mars_xlog_get_instance(prefix) == opened
        }
        return owns ? opened : Self.noHandle
    }
}
