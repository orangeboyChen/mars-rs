// The Swift face of `mars_xlog.h` that carries no behaviour: the levels, the
// modes and the config an `Xlog` is opened with, and the error an `Xlog`
// answers when it cannot be opened.
//
// These are the numbers the C ABI speaks — `MarsLogLevel`, `MarsAppenderMode`
// and the fields of `MarsXLogConfig` — under the names the Android Kotlin of
// the port spells them with, so that one app writes `XlogConfig(logDirectory:)`
// and `LogLevel.info` whichever platform it is on. Nothing here is reached by
// the C ABI: `Xlog` hands the raw values over when it opens an appender.
//
// Every one of them is `@objc`, and `XlogConfig` is a class: an app written in
// Objective-C asks for the logger the same way a Swift one does —
// `[[Xlog alloc] initWithConfig:config error:&error]` — and a Swift `struct` is
// not a value an Objective-C file can hold. The Kotlin `XlogConfig` of the port
// is a class for the same reason. What Swift loses is the copying a `struct`
// would do: an `Xlog` reads the fields it was given once, when it opens.

import Foundation

/// `TLogLevel`; `.none` is `MARS_LEVEL_NONE`, which the filter understands but
/// the C enum does not carry.
@objc
public enum LogLevel: Int32 {
    case verbose = 0
    case debug = 1
    case info = 2
    case warning = 3
    case error = 4
    case fatal = 5
    case none = 6
}

/// `TAppenderMode`.
@objc
public enum AppenderMode: Int32 {
    case async = 0
    case sync = 1
}

/// `TCompressMode`.
@objc
public enum CompressMode: Int32 {
    case zlib = 0
    case zstd = 1
}

/// Why an `Xlog` was not opened. The C ABI answers a config it refuses with a
/// handle of `0`, so what an app gets is this and not a logger that writes
/// nowhere.
///
/// The raw values are the ones Objective-C reads: `initWithConfig:error:` hands
/// an `NSError` back whose `code` is one of them and whose
/// `localizedDescription` is [description], which is what a Swift error carries
/// into Objective-C once it says which one of its strings is the description.
@objc
public enum XlogError: Int32, Error, CustomStringConvertible, LocalizedError {
    /// `MARS_XLOG_ERR_EMPTY_LOG_DIR`: `logDirectory` was empty.
    case emptyLogDirectory = 0

    /// `namePrefix` was empty: it is what the appender is known by, and what
    /// `close()` asks the C ABI to release it by.
    case emptyNamePrefix = 1

    /// `compressionLevel` outside what the compressor takes: `0...9` for
    /// `.zlib`, `0...22` for `.zstd`.
    case invalidCompressionLevel = 2

    /// `cacheDays` was negative.
    case negativeCacheDays = 3

    /// `mars_xlog_new_instance` answered `0`: the appender refused the config.
    case refused = 4

    public var description: String {
        switch self {
        case .emptyLogDirectory:
            return "marsrs-xlog opened no appender without a log directory"

        case .emptyNamePrefix:
            return "marsrs-xlog opened no appender without a name prefix"

        case .invalidCompressionLevel:
            return "compressionLevel must be in 0...9 for zlib, 0...22 for zstd"

        case .negativeCacheDays:
            return "cacheDays must not be negative"

        case .refused:
            return "mars_xlog_new_instance refused the configuration"
        }
    }

    /// What `NSError.localizedDescription` answers, and [description] verbatim:
    /// a Swift error reaches Objective-C as an `NSError` with a code and no
    /// message, and this is the key that gives it one.
    public var errorDescription: String? {
        description
    }
}

/// `XLogConfig`, with the defaults the C++ gives the fields it is not told.
///
/// `logDirectory` is the one field that has no default: an `Xlog` is not opened
/// without it.
///
/// ```swift
/// var config = XlogConfig(logDirectory: logDirectory.path)
/// config.namePrefix = "marsrs"
/// ```
///
/// ```objc
/// XlogConfig *config = [[XlogConfig alloc] initWithLogDirectory:logDirectory.path];
/// config.namePrefix = @"marsrs";
/// ```
///
/// A class and not a struct, because an app that writes Objective-C fills one in
/// too: `[[XlogConfig alloc] initWithLogDirectory:]` needs an object to return,
/// and a struct is not one. The Kotlin `XlogConfig` of the port is a class as
/// well. What Swift gives up is the copy a `struct` would make — an `Xlog`
/// reads the fields it was handed once, while it opens.
@objc
public final class XlogConfig: NSObject {
    /// The level the appender is opened at; `Xlog.level` moves it afterwards.
    @objc public var level: LogLevel = .info

    /// Whether a write reaches the file before it returns.
    @objc public var mode: AppenderMode = .async

    /// Where the log files go: the one field with no default.
    @objc public var logDirectory: String

    /// What every file of this appender starts with, and what it is known by.
    @objc public var namePrefix: String = "xlog"

    /// Empty means the log is written unencrypted.
    @objc public var publicKey: String = ""

    /// How the log is compressed; `.zlib` is what the C++ defaults to.
    @objc public var compression: CompressMode = .zlib

    /// `0` keeps the appender's own default (6); the ceiling is the one of the
    /// compressor: `9` for `.zlib`, `22` for `.zstd`.
    @objc public var compressionLevel: Int32 = 0

    /// `nil` puts the mmap cache in the log directory.
    @objc public var cacheDirectory: String?

    /// `0` keeps every file.
    @objc public var cacheDays: Int32 = 0

    /// The four fields an app has at hand when it opens a logger: the directory
    /// an `Xlog` is not opened without, and the three the example on `Xlog`
    /// fills in.
    ///
    /// - Parameters:
    ///   - logDirectory: where the log files go.
    ///   - cacheDirectory: where the mmap cache goes; `nil` puts it in the log
    ///                     directory.
    ///   - namePrefix: what every file starts with, and what the appender is
    ///                 known by.
    ///   - level: the level the appender is opened at.
    @objc
    public init(
        logDirectory: String,
        cacheDirectory: String? = nil,
        namePrefix: String = "xlog",
        level: LogLevel = .info
    ) {
        self.logDirectory = logDirectory
        super.init()
        self.cacheDirectory = cacheDirectory
        self.namePrefix = namePrefix
        self.level = level
    }

    /// Every field but `logDirectory` left at the default it declares.
    @objc
    public init(logDirectory: String) {
        self.logDirectory = logDirectory
        super.init()
    }

    deinit {
        // Nothing to release: the strings are all this object holds, and they
        // go with it. The declaration is what `required_deinit` asks a class
        // for, and it is the one member of this type that is not `@objc` — an
        // Objective-C caller releases what it allocated and never calls it.
    }
}
