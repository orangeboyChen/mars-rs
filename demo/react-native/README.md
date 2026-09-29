# The React Native demo

A React Native app's `App.tsx`, and not the whole project: the `android/` and
`ios/` halves of one are React Native's to generate — an Xcode project, a Gradle
build and a few hundred files in between.

```bash
npx @react-native-community/cli init MarsRSDemo
cd MarsRSDemo
npm install marsrs-react-native-xlog react-native-fs
cd ios && pod install && cd ..
npx react-native run-android        # or run-ios
```

and then replace the `App.tsx` the CLI wrote with this one. React Native 0.74 or
newer with the New Architecture on: the module is a TurboModule, and an app
still on the bridge has no module to `TurboModuleRegistry.getEnforcing`.

## What to look at

- `Xlog.open(config)` — not a future. A TurboModule's method queue is
  `RCTJSThread`, so a call is made on the JS thread and returns from there: a
  call that answers a value answers it before the next line runs, and none of
  them answers a `Promise`.
- `xlog.i(tag, message)` and `xlog.flushNow()` — the same, and the flush is
  what waits for the write. `xlog.flush()` is the drain as a `Promise` and
  `xlog.requestFlush()` is the one that asks for it and returns at once.
- `RNFS.DocumentDirectoryPath` — why the appender is opened in an effect rather
  than at module scope: JavaScript has no way to ask where an app may write
  until the native side answers, and `react-native-fs` is what asks.
- `return () => opened.xlog?.close()` — the effect's cleanup, which is the
  React Native answer to the `onDestroy` the Android demo closes in.

`marsrs-react-native-xlog` is the package an app that only logs takes;
`marsrs-react-native` is the whole port. An app takes one of the two and never
both — both carry the same native library, and an app with two of it does not
build.
