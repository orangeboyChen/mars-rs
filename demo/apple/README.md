# The Apple demo

The sources of an iOS app: one SwiftUI screen, one button, six records into a
`.xlog` file in the app's Application Support directory.

They are sources and not a project, because an iOS app is a bundle and a bundle
is not a thing a repository carries as text: an Xcode project is a
`project.pbxproj` of generated identifiers. It is also not buildable from the
command line here — the Swift package of the port declares iOS 12 and watchOS 10
and no macOS, so there is no macOS slice to link and no `swift build` that runs
it. Three files and a new project is the short way in.

## Make the app

1. **File ▸ New ▸ Project ▸ iOS ▸ App**, SwiftUI, Swift. Name it what you like.
2. **Add the package.** File ▸ Add Package Dependencies ▸
   `https://github.com/orangeboyChen/mars-rs`, and tick **MarsRSXlog** — the
   logging half. `MarsRS` is the whole port; an app takes one of the two.
   The package needs a version: `0.1.0-alpha.3` is the newest the xcframeworks
   were built for.
3. **Drop these three files in**, replacing the `ContentView.swift` and the
   `App` file the template wrote:

   ```text
   Sources/LogStore.swift          the appender
   Sources/ContentView.swift       the screen
   Sources/MarsRSDemoApp.swift     @main
   ```

4. **Run.** Six records land in Application Support, and the console shows them.

## What to look at

- `Xlog.open(config)` in `LogStore` — the same call it is in Kotlin, in Dart and
  in ArkTS, and `XlogConfig` is the class the Kotlin one is: the defaults are on
  its properties, so an app names the three it cares about.
- `log.info(message:tag:)` — Swift takes the message first and lets the tag
  default to empty, and fills `#file`, `#function` and `#line` in at the call
  site, which is where the C++ and the C demo write them by hand.
- `log.isEnabled(for:)` — the check that goes before a message that is expensive
  to build.
- `XlogBackgroundFlush` — the port registers the appender with it, so iOS
  draining the app when it is backgrounded is not something the app has to do.
- `scenePhase` in `MarsRSDemoApp` — where an app learns it may be suspended, and
  the iOS answer to the `onDestroy` the Android demo closes in.
- `error` in `LogStore.init` — `Xlog.open` throws `XlogError` when the C ABI
  refuses the config, and an app that pretends it cannot is an app that crashes
  on a device whose storage is full.
