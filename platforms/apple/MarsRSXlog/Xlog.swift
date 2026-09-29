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
// so the symbols an `Xlog` is written over stay reachable for whoever prefers
// them.
//
// The shape is the one the Android `Xlog` has, and the one the C ABI spells with
// a handle: `Xlog.open(config)` opens an appender of its own and answers it, and
// the app writes through what it was given. Nothing here is deprecated, because
// there is no older Swift API to keep — the process-wide appender the JNI
// bridge installs is a set of C symbols, and `import MarsRSFFI` reaches
// them. `currentLogPath` and `currentCachePath` are the exception: they are that
// appender's, and they say so.
//
// Every call is a straight translation of a symbol in the header; nothing here
// adds behaviour the C ABI does not have.
//
// The class and every member an app reaches are `@objc`, and `Xlog` is an
// `NSObject` — but for `setConsoleSink(_:)`, which takes a C function pointer
// and so has no Objective-C spelling, and for the `async` `flush()`, which
// no Objective-C caller can `await`. An app written in Objective-C takes the port
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

    /// Whether this appender is still open: `false` after [close()].
    @objc public var isOpen: Bool {
        handle != Self.noHandle
    }

    /// The level of this appender: a record less severe than this is dropped.
    ///
    /// Read from the C ABI and not mirrored here, so a level another part of
    /// the app set is the one this answers with.
    @objc public var level: LogLevel {
        get {
            // `-1` is what `mars_xlog_get_level` answers for a handle that is
            // not one, and it is `(TLogLevel)-1`, the C++'s "log everything".
            LogLevel(rawValue: mars_xlog_get_level(handle)) ?? .verbose
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

    /// `mars_xlog_set_console_fun`: where the console copy of a record goes
    /// instead of the built-in sink, which is standard error on every platform
    /// of the port — `os_log_with_type` is a macro with no symbol to link
    /// against, so nothing here can write to the system log.
    ///
    /// An Apple app that wants its records there is what this is for: the C++
    /// has an Apple enum of three sinks of its own, `kConsoleOSLog` among them,
    /// and the app is the only code that can call `os_log` — which Swift can.
    ///
    /// ```swift
    /// Xlog.setConsoleSink { level, tag, file, function, line, log in
    ///     os_log(.default, "%{public}@", String(cString: log))
    /// }
    /// ```
    ///
    /// `nil` takes the sink away, and the console copy is standard error's
    /// again. What a sink is handed is the record unformatted — the level, the
    /// tag, where the call site is and the message — and it is called on the
    /// thread that wrote the record, the writer thread of an async appender
    /// included.
    ///
    /// One sink for the process and not one for this `Xlog`: the C ABI has no
    /// instance of it. Objective-C sets the same one through
    /// `mars_xlog_set_console_fun`, which `import MarsRSXlog` re-exports.
    ///
    /// - Parameter sink: what the console copy is handed to, or `nil`.
    public static func setConsoleSink(_ sink: MarsXLogConsoleFun?) {
        mars_xlog_set_console_fun(sink)
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
        guard isOpen else {
            return false
        }
        return mars_xlog_is_enabled_for(handle, level.rawValue) != 0
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
        guard isOpen else {
            return
        }
        // `cTag` and friends: the same four strings as C pointers, which is
        // what the closure hands back and what the C ABI copies out of.
        withCStrings(first: tag, second: file, third: function, fourth: message) { cTag, cFile, cFunction, cMessage in
            mars_xlog_write_instance(handle, level.rawValue, cTag, cFile, cFunction, line, cMessage)
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
    public func signalFlush() {
        withHandle { mars_xlog_signal_flush_instance($0) }
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
        let opened = self.handle
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
        guard isOpen else {
            return
        }
        // A prefix is one appender to the C ABI, so two `Xlog`s of one prefix
        // hold one handle between them — and releasing takes the prefix and
        // not the handle, which drops whatever the prefix answers *now*. Once
        // this object's twin closed the appender and a third one reopened the
        // prefix, releasing here would close an appender that is not ours.
        let ownsPrefix = namePrefix.withCString { prefix in
            mars_xlog_get_instance(prefix) == handle
        }
        if ownsPrefix {
            namePrefix.withCString { prefix in
                mars_xlog_release_instance(prefix)
            }
        }
        handle = Self.noHandle
    }

    /// The path of the file the *process-wide* appender is writing — the one
    /// the JNI bridge installs, not the one of an `Xlog` — or `nil` when there is
    /// no open file (or the buffer was too small, which 1024 bytes never is).
    @objc public static var currentLogPath: String? {
        path(of: mars_xlog_current_log_path)
    }

    /// `mars_xlog_current_log_cache_path`: the cache file of the process-wide
    /// appender. An `Xlog` of its own keeps its cache in its `XlogConfig`'s
    /// `cacheDirectory ?? logDirectory`.
    @objc public static var currentCachePath: String? {
        path(of: mars_xlog_current_log_cache_path)
    }

    /// `mars_xlog_getfilepath_from_timespan`: the log files of `daysAgo` days
    /// ago that are *there* — what an app that uploads yesterday's opens. `[]`
    /// when the directory holds none of that day's.
    ///
    /// This is a day of files and not the file being written: what `currentLogPath`
    /// answers is one, and it is the process-wide appender's, while this takes
    /// the prefix and the directory of the files it is asked about.
    ///
    /// - Parameters:
    ///   - daysAgo: `0` is today, `1` yesterday, and so on.
    ///   - prefix: what every file of that appender's starts with.
    ///   - logDirectory: the directory those files are in.
    @objc
    public static func logFiles(daysAgo: Int, prefix: String, logDirectory: String) -> [String] {
        paths { index, out, len in
            prefix.withCString { name in
                logDirectory.withCString { directory in
                    mars_xlog_getfilepath_from_timespan(Int32(daysAgo), name, directory, index, out, len)
                }
            }
        }
    }

    /// `mars_xlog_make_logfile_name`: the paths of the log files of `daysAgo`
    /// days ago whether or not they are *there yet* — the name an app that is
    /// about to write, or that is naming a file to someone else, asks for.
    ///
    /// A day's answer is the log-dir file and, when a cache dir is configured
    /// and the file exists, its cache-dir twin, so this can answer two where
    /// [`logFiles(daysAgo:prefix:logDirectory:)`] answers one.
    ///
    /// - Parameters:
    ///   - daysAgo: `0` is today, `1` yesterday, and so on.
    ///   - prefix: what every file of that appender's starts with.
    ///   - logDirectory: the directory those files are written into.
    @objc
    public static func logFileNames(daysAgo: Int, prefix: String, logDirectory: String) -> [String] {
        paths { index, out, len in
            prefix.withCString { name in
                logDirectory.withCString { directory in
                    mars_xlog_make_logfile_name(Int32(daysAgo), name, directory, index, out, len)
                }
            }
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
    private var handle: Int64

    /// What [mode] answers while this side is the only one that knows it.
    private var currentMode: AppenderMode

    /// The queue an `await flush()` waits for the disk on: serial, because a
    /// drain holds the appender's lock from the cache to the OS anyway.
    private static let flushQueue = DispatchQueue(label: "io.github.orangeboychen.marsrs.xlog.flush")

    /// The handle the C ABI answers for an appender it did not open.
    private static let noHandle: Int64 = 0

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
    /// [close()] has: handle `0` is the process-wide appender to the C ABI,
    /// so a call through it would move a logger this object does not own.
    private func withHandle(_ body: (Int64) -> Void) {
        guard isOpen else {
            return
        }
        body(handle)
    }
}
