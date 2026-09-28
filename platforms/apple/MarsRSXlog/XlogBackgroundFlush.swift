// The flush an `Xlog` gives itself when the app leaves the screen: the one an
// app cannot be asked to call, because iOS ends a backgrounded app without
// telling it anything at all.
//
// Nothing is lost without it — a record in the cache sits in a file the kernel
// holds and not in the process, and the next `Xlog` of the same prefix takes it
// to the log file as it opens — so what it buys is a log file complete when the
// session ends, and not one the next start completes.
//
// The observers are the process's and not an appender's: an app that builds a
// logger per component would otherwise hold a set per component, and a
// notification is a broadcast with nothing in it to tell them apart. So there is
// one set, and a weak reference per appender for it to drain.

import Foundation
#if os(iOS)
import UIKit
#elseif os(watchOS)
import WatchKit
#endif

/// The flush every `Xlog` of this process gives itself when the app leaves the
/// screen: the appenders it holds weakly, and the one set of observers that says
/// the screen is gone.
internal final class XlogBackgroundFlush {
    /// The one there is: the observers are the process's, not an appender's.
    internal static let shared = XlogBackgroundFlush()

    /// The appenders still alive. Weak, because one that is gone has nothing
    /// left to drain, and [Xlog.close()] has drained it already.
    private var appenders: [WeakAppender] = []

    /// What keeps the observers reachable, and what [deinit] takes back; empty
    /// where there is no UIKit to post them.
    private var observers: [NSObjectProtocol] = []

    /// An appender is opened wherever an app opens it, and the notification
    /// arrives on the main thread, so the list is the one thing guarded.
    private let lock = NSLock()

    private init() {
        // `queue: nil` runs the block on the thread that posts the notification
        // — the main one — and synchronously, so the drain is over before the
        // app is suspended. That is the point of it.
        observers = Self.names.map { name in
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: nil) { [weak self] _ in
                self?.flushAll()
            }
        }
    }

    deinit {
        for observer in observers {
            NotificationCenter.default.removeObserver(observer)
        }
    }

    /// Takes `xlog` into the appenders the next notification drains.
    ///
    /// - Parameter xlog: an appender to flush when the app leaves the screen.
    internal func add(_ xlog: Xlog) {
        lock.lock()
        appenders.append(WeakAppender(appender: xlog))
        lock.unlock()
    }

    /// Drains every appender still alive, and forgets the ones that are not.
    private func flushAll() {
        lock.lock()
        let live = appenders
        appenders = live.filter { $0.appender != nil }
        lock.unlock()
        // `flush` does nothing once [Xlog.close()] ran, so an appender closed
        // between the notification and here loses nothing by being asked.
        for appender in live {
            appender.appender?.flush(sync: true)
        }
    }

    /// The notifications that say the app's UI is no longer on screen — the last
    /// thing the OS says before it can end the process without another word. A
    /// scene-based app is never sent the `UIApplication` lifecycle ones, so both
    /// are watched from iOS 13 on; watchOS has no UIKit at all, and its own
    /// `WKExtension` posts the one it does have.
    private static var names: [Notification.Name] {
        #if os(iOS)
        if #available(iOS 13.0, *) {
            return [
                UIApplication.didEnterBackgroundNotification,
                UIScene.didEnterBackgroundNotification,
                UIApplication.willTerminateNotification
            ]
        }
        return [UIApplication.didEnterBackgroundNotification, UIApplication.willTerminateNotification]
        #elseif os(watchOS)
        return [WKExtension.applicationDidEnterBackgroundNotification]
        #else
        return []
        #endif
    }

    /// An appender this holds without keeping it alive: a strong reference would
    /// be a cache slot a dropped `Xlog` never gives back.
    private struct WeakAppender {
        weak var appender: Xlog?
    }
}
