// The entry point of the Apple demo, and the whole of what an iOS app needs
// besides the two files next to this one.
//
// It opens the appender once, before the first view is built, and closes it once
// when iOS sends the app to the background — the scene phase is the iOS answer
// to "the process may be killed now", and it is the one an app flushes on.
//
// `@main` is what makes these three files an app once they are in an app target:
// Xcode's own iOS template says `@main struct MyApp: App` too, and one file that
// says it is one fewer thing to edit when this is dropped into a project.

import SwiftUI

@main
struct MarsRSDemoApp: App {
    /// The environment this app's scene lives in: `.active` on screen,
    /// `.background` once it is not. Watching it is how an app learns it is
    /// about to be suspended.
    @Environment(\.scenePhase) private var scenePhase

    /// The appender, opened when this object is built.
    ///
    /// `@StateObject` and not a plain `let`: SwiftUI builds an `App` once and
    /// owns what it holds, and the appender has to outlive the view that is
    /// rebuilt when the device rotates.
    @StateObject private var log = LogStore()

    var body: some Scene {
        WindowGroup {
            ContentView(log: log)
        }
        .onChange(of: scenePhase) { _, phase in
            // Closing here is not what keeps the records safe — the port
            // registers this appender with `XlogBackgroundFlush`, which drains
            // it for the app when iOS backgrounds it. It is what makes the
            // choice visible, and what an app replaces with "keep logging, just
            // flush" once it has something to log from the background.
            if phase == .background {
                log.close()
            }
        }
    }
}
