// The iOS half of `marsrs-react-native`: the thirteen methods of the `Xlog`
// native module, each of them a straight call of a `mars_xlog_*` symbol — the
// C ABI of `crates/marsrs-ffi`, in the `marsrs-xlog.xcframework` the pod carries.
//
// Swift, and not Objective-C — except for `Xlog.mm`, which is the one file of
// the module Swift cannot be, and which says why in its own header: a
// TurboModule is made of a factory that answers a `std::shared_ptr` of a C++
// class, and there is no Swift for either.
//
// The module is a TurboModule and not a bridge module. `src/NativeXlog.ts` is
// the spec codegen reads, and what it makes of it — `NativeXlogSpec`, the
// protocol, and `NativeXlogSpecJSI`, the C++ class behind it — is what lets a
// call land where it is asked for rather than cross a bridge on a queue: the
// module answers `RCTJSThread` as its method queue, and every method of it is
// therefore made on the JS thread and returned from. A method that answers no
// promise is a call and not an `await`, which is why `src/index.ts` can be the
// Kotlin and the Swift of the port member for member.
//
// The appenders are kept by the prefix they were opened with, because that is
// what an appender is known by in the C ABI: two `Xlog`s of two prefixes are two
// appenders, and one of a prefix the module already has is the appender it
// already has.
//
// The xlog-only module is `marsrs-react-native-xlog`, and this file is its Swift
// with the two names changed: xlog is the whole C ABI today, so the two are one
// package under two names, and they diverge the day STN and SDT land — here, and
// not there.

import Foundation

/// The iOS half of `Xlog`.
///
/// `Xlog` and no suffix: the name `src/index.ts` reads the module out of
/// `TurboModuleRegistry` by is the class's.
@objc(Xlog)
internal final class Xlog: NSObject {
    /// The handle of the appender of every prefix `open` has opened, by the
    /// prefix: what keeps two `Xlog`s apart, which one handle cannot, because
    /// the appender a `mars_xlog_release_instance` releases is the one of the
    /// prefix it is given.
    ///
    /// Written and read on the JS thread and nowhere else, which is what the
    /// module's `RCTJSThread` method queue makes true: the dictionary is not
    /// locked, and a logger asked to wait for one would be a logger an app
    /// routes around.
    private var handles: [String: Int64] = [:]

    /// `mars_xlog_new_instance`: opens the appender of `config` and answers
    /// whether it took the configuration.
    ///
    /// `false` is a configuration it refused — an empty `logDir` or
    /// `namePrefix`, or a directory it cannot write to — and it is what the JS
    /// caller turns into a throw rather than a handle it would write through
    /// the process-wide appender with.
    @objc(open:)
    internal func openAppender(_ config: [AnyHashable: Any]) -> Bool {
        let logDir = string(config, "logDir")
        guard !logDir.isEmpty else {
            return false
        }
        let namePrefix = string(config, "namePrefix").isEmpty
            ? Self.defaultNamePrefix
            : string(config, "namePrefix")
        // One appender per prefix: a prefix this module has already opened is
        // answered as it is, and not opened again over the first, which would be
        // a handle nothing releases. `src/index.ts` keeps the same map, so the
        // two names an app holds for one prefix are one appender, and `close` on
        // either is `close` on both.
        guard handles[namePrefix] == nil else {
            return true
        }
        let level = int(config, "level", Int32(MarsLevelInfo.rawValue))
        let mode = int(config, "mode", Int32(MarsAppenderAsync.rawValue))
        let compressMode = int(config, "compressMode", Int32(MarsCompressZlib.rawValue))
        let compressLevel = int(config, "compressLevel", Self.defaultCompressLevel)
        let cacheDays = int(config, "cacheDays", Self.keepEveryFile)

        var handle: Int64 = 0
        // `pubKey` and `cacheDir` are the two fields a caller may leave out, and
        // `mars_xlog.h` reads the empty string the way it reads `NULL`.
        withCStrings([logDir, namePrefix, string(config, "pubKey"), string(config, "cacheDir")]) { pointers in
            var native = MarsXLogConfig(
                mode: mode,
                log_dir: pointers[Field.logDir.rawValue],
                name_prefix: pointers[Field.namePrefix.rawValue],
                pub_key: pointers[Field.pubKey.rawValue],
                compress_mode: compressMode,
                compress_level: compressLevel,
                cache_dir: pointers[Field.cacheDir.rawValue],
                cache_days: cacheDays
            )
            handle = mars_xlog_new_instance(&native, level)
        }
        // A refusal is a negative `MARS_XLOG_ERR_*` code and never `0`, which is
        // the process-wide appender: a handle of `0` in the table below would
        // send every write through a logger this module never opened.
        guard handle > 0 else {
            return false
        }
        handles[namePrefix] = handle
        return true
    }

    /// `mars_xlog_write_instance`. The file, the function and the line are left
    /// empty: there is no JS frame to name, and the C++ writes an empty one too.
    @objc(log:level:tag:message:)
    internal func log(_ namePrefix: String, level: Double, tag: String, message: String) {
        guard let handle = handles[namePrefix], let level = int32(level) else {
            return
        }
        tag.withCString { cTag in
            message.withCString { cMessage in
                mars_xlog_write_instance(handle, level, cTag, nil, nil, 0, cMessage)
            }
        }
    }

    /// `mars_xlog_is_enabled_for`: whether a record of the level would be
    /// written, which an app asks before it builds a message that is expensive
    /// to build.
    @objc(isLoggable:level:)
    internal func isLoggable(_ namePrefix: String, level: Double) -> Bool {
        guard let handle = handles[namePrefix], let level = int32(level) else {
            return false
        }
        return mars_xlog_is_enabled_for(handle, level) != 0
    }

    /// `mars_xlog_get_level`: what the appender answers, and not what JS holds.
    @objc(getLevel:)
    internal func level(of namePrefix: String) -> Double {
        guard let handle = handles[namePrefix] else {
            return Double(MARS_LEVEL_NONE)
        }
        return Double(mars_xlog_get_level(handle))
    }

    /// `mars_xlog_request_flush_instance`: tells the writer thread it may take what is
    /// in the cache to the log file, and returns at once — nothing waits, and
    /// nothing is in the file because this returned. A record still in the cache
    /// is in a file the kernel holds, so one whose process dies keeps it; what
    /// an app wants before it reads the file or uploads it is `flushNow` or
    /// `flush`.
    @objc(requestFlush:)
    internal func requestFlush(_ namePrefix: String) {
        guard let handle = handles[namePrefix] else {
            return
        }
        mars_xlog_request_flush_instance(handle)
    }

    /// `mars_xlog_flush_now_instance`: the drain is this thread's, so the records are
    /// in the log file — handed to the OS, and not left in the file's own
    /// `FILE*` — when it returns, and what it costs is the time the drain takes,
    /// on the JS thread and nowhere else.
    @objc(flushNow:)
    internal func flushNow(_ namePrefix: String) {
        guard let handle = handles[namePrefix] else {
            return
        }
        mars_xlog_flush_now_instance(handle)
    }

    /// `flushNow` off the JS thread: the same drain, on a queue of this
    /// module's own, and `resolve` called when it is over.
    ///
    /// The handle and not `self`: the closure runs on another thread.
    /// `resolve` and `reject` are `RCTPromiseResolveBlock` and
    /// `RCTPromiseRejectBlock`, spelled out because the bridging header shows
    /// this file the C ABI and nothing else — a TurboModule method codegen
    /// answers a `Promise` from is given the two blocks and resolves one of
    /// them, and there is nothing here that can fail, so `reject` is never
    /// called.
    @objc(flush:resolve:reject:)
    internal func flush(
        _ namePrefix: String,
        resolve: @escaping (Any?) -> Void,
        reject: @escaping (String?, String?, Error?) -> Void
    ) {
        guard let handle = handles[namePrefix] else {
            resolve(nil)
            return
        }
        Self.flushQueue.async {
            mars_xlog_flush_now_instance(handle)
            resolve(nil)
        }
    }

    /// `mars_xlog_set_level_instance`.
    @objc(setLevel:level:)
    internal func setLevel(_ namePrefix: String, level: Double) {
        guard let handle = handles[namePrefix], let level = int32(level) else {
            return
        }
        mars_xlog_set_level_instance(handle, level)
    }

    /// `mars_xlog_set_mode_instance`.
    @objc(setMode:mode:)
    internal func setMode(_ namePrefix: String, mode: Double) {
        guard let handle = handles[namePrefix], let mode = int32(mode) else {
            return
        }
        mars_xlog_set_mode_instance(handle, mode)
    }

    /// `mars_xlog_set_console_log_instance`.
    @objc(setConsoleLogEnabled:enabled:)
    internal func setConsoleLogEnabled(_ namePrefix: String, enabled: Bool) {
        guard let handle = handles[namePrefix] else {
            return
        }
        mars_xlog_set_console_log_instance(handle, enabled ? 1 : 0)
    }

    /// `mars_xlog_set_max_file_size_instance`. A `Double` and not a `UInt64`:
    /// the module carries every JS number as one, and a file size is below
    /// 2^53.
    ///
    /// Narrowed with `exactly` and not with `UInt64(_:)`, which traps: a
    /// negative, a NaN — what a `number` a sum with `undefined` in it answered
    /// — and anything above `UInt64.max` are three traps, and a trap in a
    /// method the JS thread called into is the app going down and not a setting
    /// that was not taken. What does not fit is `0`, the one `mars_xlog.h`
    /// reads as "do not split". The Kotlin half casts the same number instead
    /// of narrowing it, so a size that does not fit is a saturated `Long`
    /// there and `0` here — two answers to a number a JS caller has no use
    /// for, and neither of them a crash.
    @objc(setMaxFileSize:bytes:)
    internal func setMaxFileSize(_ namePrefix: String, bytes: Double) {
        guard let handle = handles[namePrefix] else {
            return
        }
        mars_xlog_set_max_file_size_instance(handle, UInt64(exactly: bytes.rounded(.down)) ?? 0)
    }

    /// `mars_xlog_set_max_alive_duration_instance`. Narrowed the way the size
    /// above is, and for the same reason: `Int64(_:)` traps on a NaN too, and
    /// the C ABI's own clamp is downstream of a conversion that would never
    /// reach it.
    @objc(setMaxAliveTime:seconds:)
    internal func setMaxAliveTime(_ namePrefix: String, seconds: Double) {
        guard let handle = handles[namePrefix] else {
            return
        }
        mars_xlog_set_max_alive_duration_instance(handle, Int64(exactly: seconds.rounded(.down)) ?? 0)
    }

    /// `mars_xlog_release_instance`: closes the appender `open` made.
    @objc(close:)
    internal func close(_ namePrefix: String) {
        guard handles.removeValue(forKey: namePrefix) != nil else {
            return
        }
        namePrefix.withCString { mars_xlog_release_instance($0) }
    }

    /// Closes what `open` opened and `close` was not asked about: the module
    /// goes away with the bridge, and an appender is a file its writer thread
    /// holds open.
    deinit {
        for namePrefix in handles.keys {
            namePrefix.withCString { mars_xlog_release_instance($0) }
        }
        handles.removeAll()
    }

    /// `XlogConfig.namePrefix` of the Kotlin, of the Swift and of the TS: what
    /// an appender is opened with when the caller gave none.
    private static let defaultNamePrefix = "xlog"

    /// What the C++ passes on, which is the appender's own 6.
    private static let defaultCompressLevel: Int32 = 0

    /// The queue `flush` drains on, and why it is one queue and not the JS
    /// thread: a drain blocks the thread it runs on, and serial is what keeps
    /// two drains of one appender from being two writers in one file.
    private static let flushQueue = DispatchQueue(label: "io.github.orangeboychen.marsrs.reactnative.xlog.flush")

    /// `0` keeps every cache file, which is what the C++'s default is.
    private static let keepEveryFile: Int32 = 0

    /// The four fields of `MarsXLogConfig` that are C strings, in the order the
    /// copies are made in: what names the index a pointer is read out of, which
    /// is what a `pointers[2]` is not.
    private enum Field: Int {
        case logDir = 0
        case namePrefix = 1
        case pubKey = 2
        case cacheDir = 3
    }

    /// Runs `body` with every one of `strings` as a C string, and frees the
    /// copies it made: `MarsXLogConfig` holds pointers rather than copies, and a
    /// Swift `String` has no C string of its own to hand out — `withCString` is
    /// the answer for one, and a configuration is four at once.
    private func withCStrings(_ strings: [String], body: ([UnsafePointer<CChar>?]) -> Void) {
        let copies: [UnsafeMutablePointer<CChar>?] = strings.map { strdup($0) }
        defer { copies.forEach { free($0) } }
        body(copies.map { $0.map { UnsafePointer($0) } })
    }

    /// What the caller sent for `key`, read as a string; a field it left out is
    /// not one it sent as `null`, and the empty string is what every field of
    /// the configuration reads as "not given".
    private func string(_ config: [AnyHashable: Any], _ key: String) -> String {
        (config[key] as? String) ?? ""
    }

    /// What the caller sent for `key`, read as an `Int32`, or `fallback` when it
    /// sent nothing: a `Number`, because JS has one number type and the codec
    /// hands over whichever the value fits in.
    private func int(_ config: [AnyHashable: Any], _ key: String, _ fallback: Int32) -> Int32 {
        (config[key] as? NSNumber)?.int32Value ?? fallback
    }

    /// What a level or a mode the caller sent is read as an `Int32` with: `nil`
    /// when it is not one — a `NaN`, an infinity, anything past 2^31.
    /// `Int32(_:)` traps on all three, and a trap in a method the JS thread
    /// called into is the app going down for a number the C ABI answers "no
    /// such level" for; the size and the time above are narrowed with `exactly`
    /// for the same reason. What is left out is what the appender was set to:
    /// a level that does not exist is not one the appender is moved to, and a
    /// record of one is not one it writes.
    private func int32(_ value: Double) -> Int32? {
        guard value.isFinite, value >= Double(Int32.min), value <= Double(Int32.max) else {
            return nil
        }
        return Int32(value)
    }
}
