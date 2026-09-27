// The Swift face of `mars_xlog.h` that carries no behaviour: the levels, the
// modes and the config an `Xlog` is opened with, and the error an `Xlog`
// answers when it cannot be opened.
//
// These are the numbers the C ABI speaks — `MarsLogLevel`, `MarsAppenderMode`
// and the fields of `MarsXLogConfig` — under the names the Android Kotlin of
// the port spells them with, so that one app writes `XlogConfig(logDirectory:)`
// and `LogLevel.info` whichever platform it is on. Nothing here is reached by
// the C ABI: `Xlog` hands the raw values over when it opens an appender.

/// `TLogLevel`; `.none` is `MARS_LEVEL_NONE`, which the filter understands but
/// the C enum does not carry.
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
public enum AppenderMode: Int32 {
    case async = 0
    case sync = 1
}

/// `TCompressMode`.
public enum CompressMode: Int32 {
    case zlib = 0
    case zstd = 1
}

/// Why an `Xlog` was not opened. The C ABI answers a config it refuses with a
/// handle of `0`, so what an app gets is this and not a logger that writes
/// nowhere.
public enum XlogError: Error, CustomStringConvertible {
    /// `MARS_XLOG_ERR_EMPTY_LOG_DIR`: `logDirectory` was empty.
    case emptyLogDirectory

    /// `namePrefix` was empty: it is what the appender is known by, and what
    /// `close()` asks the C ABI to release it by.
    case emptyNamePrefix

    /// `compressionLevel` outside what the compressor takes: `0...9` for
    /// `.zlib`, `0...22` for `.zstd`.
    case invalidCompressionLevel

    /// `cacheDays` was negative.
    case negativeCacheDays

    /// `mars_xlog_new_instance` answered `0`: the appender refused the config.
    case refused

    public var description: String {
        switch self {
        case .emptyLogDirectory:
            return "mars-xlog opened no appender without a log directory"

        case .emptyNamePrefix:
            return "mars-xlog opened no appender without a name prefix"

        case .invalidCompressionLevel:
            return "compressionLevel must be in 0...9 for zlib, 0...22 for zstd"

        case .negativeCacheDays:
            return "cacheDays must not be negative"

        case .refused:
            return "mars_xlog_new_instance refused the configuration"
        }
    }
}

/// `XLogConfig`, with the defaults the C++ gives the fields it is not told.
///
/// `logDirectory` is the one field that has no default: an `Xlog` is not opened
/// without it.
public struct XlogConfig {
    /// The level the appender is opened at; `Xlog.level` moves it afterwards.
    public var level: LogLevel = .info

    /// Whether a write reaches the file before it returns.
    public var mode: AppenderMode = .async

    /// Where the log files go: the one field with no default.
    public var logDirectory: String

    /// What every file of this appender starts with, and what it is known by.
    public var namePrefix: String = "xlog"

    /// Empty means the log is written unencrypted.
    public var publicKey: String = ""

    /// How the log is compressed; `.zlib` is what the C++ defaults to.
    public var compression: CompressMode = .zlib

    /// `0` keeps the appender's own default (6); the ceiling is the one of the
    /// compressor: `9` for `.zlib`, `22` for `.zstd`.
    public var compressionLevel: Int32 = 0

    /// `nil` puts the mmap cache in the log directory.
    public var cacheDirectory: String?

    /// `0` keeps every file.
    public var cacheDays: Int32 = 0

    /// The four fields an app has at hand when it opens a logger of its own:
    /// the one an `Xlog` is not opened without, and the three the example on
    /// `Xlog` names.
    ///
    /// - Parameters:
    ///   - logDirectory: where the log files go.
    ///   - cacheDirectory: where the mmap cache goes; `nil` puts it in the log
    ///                     directory.
    ///   - namePrefix: what every file starts with, and what the appender is
    ///                 known by.
    ///   - level: the level the appender is opened at.
    public init(
        logDirectory: String,
        cacheDirectory: String? = nil,
        namePrefix: String = "xlog",
        level: LogLevel = .info
    ) {
        self.init(logDirectory: logDirectory)
        self.cacheDirectory = cacheDirectory
        self.namePrefix = namePrefix
        self.level = level
    }

    /// Every field but `logDirectory` at the default it carries beside it.
    public init(logDirectory: String) {
        self.logDirectory = logDirectory
    }
}
