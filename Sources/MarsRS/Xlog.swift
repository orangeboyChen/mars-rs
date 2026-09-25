// The Swift face of the C ABI in `mars_xlog.h` (crate `mars-ffi`).
//
// Xlog.swift is one file of the `MarsRS` module, not the module: the module is
// the port, and this is the part of it `mars-ffi` exposes today. Stn.swift and
// Sdt.swift belong next to it as soon as the C ABI carries them.
//
// The binary target of Package.swift is a static library plus a module map, so
// what `import MarsRSFFI` gives a caller is the C surface itself: pointers to
// C strings, `int` modes and a config struct that has to be filled field by
// field. This file is that surface in Swift — `String`s, enums and a
// `MarsXlogConfiguration` with defaults — and re-exports the C module, so
// `mars_xlog_open(&config)` and friends are still reachable from here for
// whoever prefers them.
//
// Every call is a straight translation of a symbol in the header; nothing here
// adds behaviour the C ABI does not have.

import Foundation

// Re-exported so that `import MarsRS` also gives the C symbols.
@_exported import MarsRSFFI

/// `TLogLevel`; `.none` is `MARS_LEVEL_NONE`, which the filter understands but
/// the C enum does not carry.
public enum MarsXlogLevel: Int32 {
    case verbose = 0
    case debug = 1
    case info = 2
    case warning = 3
    case error = 4
    case fatal = 5
    case none = 6
}

/// `TAppenderMode`.
public enum MarsXlogMode: Int32 {
    case async = 0
    case sync = 1
}

/// `TCompressMode`.
public enum MarsXlogCompression: Int32 {
    case zlib = 0
    case zstd = 1
}

/// `XLogConfig`, with the defaults the C++ gives the fields it is not told.
///
/// `logDirectory` is the one field that has no default: `mars_xlog_open`
/// answers `MARS_XLOG_ERR_EMPTY_LOG_DIR` without it.
public struct MarsXlogConfiguration {
    /// The level the appender is opened at; `mars_xlog_set_level` after the
    /// open, which is where the C ABI keeps it.
    public var level: MarsXlogLevel = .info
    public var mode: MarsXlogMode = .async
    public var logDirectory: String
    /// Written verbatim, as the C++ does: no default.
    public var namePrefix: String = ""
    /// Empty means the log is written unencrypted.
    public var publicKey: String = ""
    public var compression: MarsXlogCompression = .zlib
    /// `0` keeps the appender's own default (6).
    public var compressionLevel: Int32 = 0
    /// `nil` puts the mmap cache in the log directory.
    public var cacheDirectory: String? = nil
    /// `0` keeps every file.
    public var cacheDays: Int32 = 0

    public init(logDirectory: String) {
        self.logDirectory = logDirectory
    }
}

/// A logger instance: `mars_xlog_new_instance` and friends, which own an
/// appender of their own. Handle `0` is the process-wide one `open` created.
public struct MarsXlogInstance {
    /// The handle, as `mars_xlog_new_instance` gave it.
    public let handle: Int64

    public init(handle: Int64) {
        self.handle = handle
    }

    /// `mars_xlog_get_level`, or `nil` for a handle that is not one.
    public var level: MarsXlogLevel? {
        MarsXlogLevel(rawValue: mars_xlog_get_level(handle))
    }

    /// `mars_xlog_is_enabled_for`.
    public func isEnabled(for level: MarsXlogLevel) -> Bool {
        mars_xlog_is_enabled_for(handle, level.rawValue) != 0
    }

    /// `mars_xlog_set_level_instance`.
    public func setLevel(_ level: MarsXlogLevel) {
        mars_xlog_set_level_instance(handle, level.rawValue)
    }

    /// `mars_xlog_set_mode_instance`.
    public func setMode(_ mode: MarsXlogMode) {
        mars_xlog_set_mode_instance(handle, mode.rawValue)
    }

    /// `mars_xlog_flush_instance`.
    public func flush(sync: Bool) {
        mars_xlog_flush_instance(handle, sync ? 1 : 0)
    }

    /// `mars_xlog_write_instance`.
    public func write(
        _ level: MarsXlogLevel,
        tag: String = "",
        file: String = #file,
        function: String = #function,
        line: Int32 = #line,
        message: String
    ) {
        withCStrings(tag: tag, file: file, function: function, message: message) { tag, file, function, message in
            mars_xlog_write_instance(handle, level.rawValue, tag, file, function, line, message)
        }
    }
}

/// `mars_xlog_open` and the process-wide appender it opens.
public enum MarsXlog {
    /// `mars_xlog_open`, then `mars_xlog_set_level` for the configuration's
    /// level. Returns `MARS_XLOG_OK` or a negative `MARS_XLOG_ERR_*` code.
    @discardableResult
    public static func open(_ configuration: MarsXlogConfiguration) -> Int32 {
        withCStrings(
            tag: configuration.logDirectory,
            file: configuration.namePrefix,
            function: configuration.publicKey,
            message: configuration.cacheDirectory ?? ""
        ) { logDir, namePrefix, publicKey, cacheDir in
            var config = MarsXLogConfig(
                mode: configuration.mode.rawValue,
                log_dir: logDir,
                name_prefix: namePrefix,
                pub_key: publicKey,
                compress_mode: configuration.compression.rawValue,
                compress_level: configuration.compressionLevel,
                cache_dir: cacheDir,
                cache_days: configuration.cacheDays
            )
            let code = mars_xlog_open(&config)
            if code == MARS_XLOG_OK {
                mars_xlog_set_level(configuration.level.rawValue)
            }
            return code
        }
    }

    /// `mars_xlog_close`.
    public static func close() {
        mars_xlog_close()
    }

    /// `mars_xlog_flush` / `mars_xlog_flush_sync`.
    public static func flush(sync: Bool) {
        if sync {
            mars_xlog_flush_sync()
        } else {
            mars_xlog_flush()
        }
    }

    /// `mars_xlog_set_level`.
    public static func setLevel(_ level: MarsXlogLevel) {
        mars_xlog_set_level(level.rawValue)
    }

    /// `mars_xlog_set_console_log`.
    public static func setConsoleLogEnabled(_ enabled: Bool) {
        mars_xlog_set_console_log(enabled ? 1 : 0)
    }

    /// `mars_xlog_set_max_file_size`; `0` stops splitting the file.
    public static func setMaxFileSize(_ bytes: UInt64) {
        mars_xlog_set_max_file_size(bytes)
    }

    /// `mars_xlog_set_max_alive_duration`.
    public static func setMaxAliveTime(_ seconds: Int64) {
        mars_xlog_set_max_alive_duration(seconds)
    }

    /// `mars_xlog_set_mode`.
    public static func setMode(_ mode: MarsXlogMode) {
        mars_xlog_set_mode(mode.rawValue)
    }

    /// `mars_xlog_write`.
    public static func write(
        _ level: MarsXlogLevel,
        tag: String = "",
        file: String = #file,
        function: String = #function,
        line: Int32 = #line,
        message: String
    ) {
        withCStrings(tag: tag, file: file, function: function, message: message) { tag, file, function, message in
            mars_xlog_write(level.rawValue, tag, file, function, line, message)
        }
    }

    /// `mars_xlog_current_log_path`, or `nil` when there is no open file (or
    /// the buffer was too small, which 1024 bytes never is).
    public static var currentLogPath: String? {
        path(of: mars_xlog_current_log_path)
    }

    /// `mars_xlog_current_log_cache_path`.
    public static var currentCachePath: String? {
        path(of: mars_xlog_current_log_cache_path)
    }

    /// `mars_xlog_new_instance` — `nil` when the configuration was refused.
    public static func instance(_ configuration: MarsXlogConfiguration) -> MarsXlogInstance? {
        let handle = withCStrings(
            tag: configuration.logDirectory,
            file: configuration.namePrefix,
            function: configuration.publicKey,
            message: configuration.cacheDirectory ?? ""
        ) { logDir, namePrefix, publicKey, cacheDir -> Int64 in
            var config = MarsXLogConfig(
                mode: configuration.mode.rawValue,
                log_dir: logDir,
                name_prefix: namePrefix,
                pub_key: publicKey,
                compress_mode: configuration.compression.rawValue,
                compress_level: configuration.compressionLevel,
                cache_dir: cacheDir,
                cache_days: configuration.cacheDays
            )
            return mars_xlog_new_instance(&config, configuration.level.rawValue)
        }
        return handle == 0 ? nil : MarsXlogInstance(handle: handle)
    }

    /// `mars_xlog_get_instance`.
    public static func instance(named namePrefix: String) -> MarsXlogInstance? {
        let handle = namePrefix.withCString { mars_xlog_get_instance($0) }
        return handle == 0 ? nil : MarsXlogInstance(handle: handle)
    }

    /// `mars_xlog_release_instance`.
    public static func releaseInstance(named namePrefix: String) {
        namePrefix.withCString { mars_xlog_release_instance($0) }
    }

    private static func path(of body: (UnsafeMutablePointer<CChar>, UInt32) -> Int32) -> String? {
        var buffer = [CChar](repeating: 0, count: 1024)
        let written = body(&buffer, UInt32(buffer.count))
        guard written > 0 else { return nil }
        return String(cString: buffer)
    }
}

/// The C strings a write needs, valid for the length of the closure: the C ABI
/// copies what it needs out of them before it answers.
private func withCStrings<R>(
    tag: String,
    file: String,
    function: String,
    message: String,
    body: (UnsafePointer<CChar>, UnsafePointer<CChar>, UnsafePointer<CChar>, UnsafePointer<CChar>) -> R
) -> R {
    tag.withCString { tag in
        file.withCString { file in
            function.withCString { function in
                message.withCString { message in
                    body(tag, file, function, message)
                }
            }
        }
    }
}
