// The appender of the Apple demo: the one object that owns the `Xlog`, opened
// once when the app starts and closed once when it goes.
//
// It is the same three things every demo in `demo/` does — open, write one
// record at every level, flush — under the one name the port gives that call on
// every platform: `Xlog.open(config)`. What is Apple's is the directory the
// files go in, which on iOS is the one `FileManager` hands the app and never a
// path an app writes out for itself, and the `XlogBackgroundFlush` the port
// registers this appender with, which drains it when iOS backgrounds the app.
//
// Copy this file, `ContentView.swift` and `MarsRSDemoApp.swift` into an iOS app
// that has the `mars-rs` package added to it — `demo/apple/README.md` says how,
// step by step.

import Foundation
import MarsRSXlog
import SwiftUI

// An `ObservableObject` and not a `struct` because what it holds is not a value:
// it holds an appender, which is a handle into Rust, and a second copy of it
// would be the same appender twice over.
final class LogStore: ObservableObject {
    /// What the view shows: the directory being written into, or why nothing is
    /// being written at all.
    @Published private(set) var status = "opening the appender"

    /// Where the `.xlog` files are. The view shows it, and an app that lets its
    /// user send in a bug report reads the files out of it.
    let directory: String

    /// Whether there is an appender to write through: `false` when `Xlog.open`
    /// threw, which is what a build with no log directory comes to.
    var isOpen: Bool {
        log != nil
    }

    /// The appender, or `nil` when it could not be opened.
    ///
    /// Optional and not force-unwrapped because opening one really can fail —
    /// `Xlog.open(_:)` throws `XlogError` when the C ABI refuses the config,
    /// which an empty log directory comes to — and an app that pretends it
    /// cannot is an app that crashes on a device whose storage is full.
    private var log: Xlog?

    init() {
        directory = LogStore.makeLogDirectory()
        do {
            // Swift's `XlogConfig` is a class with the defaults on its
            // properties, so an app names the three it cares about and leaves
            // the rest: `.zlib`, no public key, the cache beside the files.
            let config = XlogConfig(
                logDirectory: directory,
                // `nil` puts the mmap cache in the log directory. On iOS there
                // is nowhere better: a file in `Caches/` is one the OS deletes
                // when space runs short, which is exactly when the log matters,
                // and the appender needs the cache to still be there to write
                // what it was holding.
                cacheDirectory: nil,
                namePrefix: "marsrs",
                // Verbose so that all six records survive to be read back. An
                // app that ships sets `.info`, and `.none` for a build that
                // writes nothing at all.
                level: .verbose
            )
            // `.async` is the default: the appender writes from its own thread
            // and a log call does not block the one that made it.
            config.mode = .async

            let log = try Xlog.open(config)

            // Mirror every record to the console as well, so Xcode's console
            // shows what went into the file. Off in a build that ships.
            log.isConsoleLogEnabled = true
            // Close a file at 8 MiB and drop one at ten days. Both start at 0,
            // which is not the same 0 twice: a maximum size of 0 never splits a
            // file, and a lifetime of 0 is the C++'s own ten days.
            log.maxFileSizeBytes = 8 * 1024 * 1024
            log.maxAliveTimeSeconds = 10 * 24 * 60 * 60

            self.log = log
            status = "writing into \(directory)"
        } catch {
            // `XlogError` says which of the four refusals it was; an app shows
            // it once and carries on, because an app that cannot log is still
            // an app.
            status = "no appender: \(error)"
        }
    }

    /// One record at every level, and the flush that leaves them on disk.
    ///
    /// The same six `demo/rust`, `demo/c`, `demo/android` and `demo/kmp` write,
    /// in the same order, so that the five read side by side.
    func writeDemoRecords() {
        guard let log else {
            return
        }

        // Swift takes the message first and lets the tag default to empty;
        // Android's Kotlin asks for both. `#file`, `#function` and `#line` are
        // filled in at the call site, so a record carries where it was written
        // without the caller spelling it out.
        log.verbose(message: "the finest record there is", tag: "trace")
        log.debug(message: "resolved 3 addresses for example.com", tag: "net")
        log.info(message: "cold start in 412 ms", tag: "startup")
        log.warning(message: "retrying after 1204 ms", tag: "net")
        log.error(message: "login failed: token expired", tag: "login")
        log.fatal(message: "giving up after 3 attempts", tag: "login")

        // A message that is expensive to build is worth asking about first: a
        // record the level drops still costs its caller the string.
        if log.isEnabled(for: .debug) {
            log.debug(message: "the appender is \(log.namePrefix)", tag: "startup")
        }

        // `flushNow` waits for the write, so every record above is on disk
        // when this returns; `signalFlush` is the one that asks the writer
        // thread and does not wait, and `flush()` is the `async` spelling of
        // the same drain. The port flushes for the app when iOS backgrounds
        // it; an app calls this before it reads the files or uploads them.
        log.flushNow()
    }

    /// Closes the appender: drains what is left and drops it.
    ///
    /// `Xlog` closes itself in its `deinit` too, so this is not what keeps the
    /// last records from being lost — it is what makes *when* they are written
    /// the app's own choice, rather than the next time the object is released.
    func close() {
        log?.close()
        log = nil
    }

    /// The directory the files go in: `Application Support/MarsRSDemo`, created
    /// if it is not there.
    ///
    /// Application Support and not `Caches/` or `Documents/`: a file in
    /// `Caches/` is one iOS deletes when space runs short, and a file in
    /// `Documents/` is one iTunes shows the user and one an app is asked to
    /// account for at review. A log is neither — it is the app's own, it is
    /// backed up, and no one browses it.
    private static func makeLogDirectory() -> String {
        let root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
        guard let directory = root.first?.appendingPathComponent("MarsRSDemo", isDirectory: true) else {
            // No Application Support directory is a device this app cannot run
            // on; the appender is not opened, and the view says so.
            return ""
        }
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory.path
    }
}
