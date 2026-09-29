# marsrs-react-native

The React Native module of the whole of [mars-rs](https://github.com/orangeboyChen/mars-rs),
a Rust implementation of [Tencent/mars](https://github.com/Tencent/mars): the
`.xlog` the C++ implementation produced — encrypted or plain, zlib- or
zstd-compressed, sync or async — today, and STN and SDT when the C ABI carries
them.

```ts
import { Xlog, LogLevel } from 'marsrs-react-native';

const xlog = Xlog.open({
  logDir: RNFS.DocumentDirectoryPath,
  namePrefix: 'marsrs',
  pubKey: '...',
});
xlog.i('Net', 'hello');
xlog.log(LogLevel.Debug, 'Net', 'a debug line');
xlog.flushNow();
xlog.close();
```

`Xlog` is one appender: `Xlog.open(config)` opens it and `close()` closes it —
the same class, under the same name, the Swift package,
`platforms/kmp/marsrs-xlog` and the Android AAR publish. The settings are the
properties they are in Kotlin and in Swift — `xlog.level = LogLevel.Debug` —
and not `setLevel` / `getLevel` pairs.

The drain is three calls and not one with a flag: `xlog.signalFlush()` tells
the writer thread it may take the cache to the file and returns at once,
`xlog.flushNow()` does the same on the calling thread and is over when it
returns, and `await xlog.flush()` is `flushNow` off the JS thread — the one
call here that answers a `Promise`, because a drain blocks the thread it runs
on and the JS thread is not one to block.

That is what the New Architecture buys, and it is the only thing asked for in
return: a TurboModule is not a bridge module, so an app still on the old
architecture has no module to `TurboModuleRegistry.getEnforcing`.

## Installing it

`npm install marsrs-react-native`, and `pod install` for the iOS half: the
module is published to npm, and the release of a tag is what publishes it.

```bash
npm install marsrs-react-native
cd ios && pod install
```

| | |
|---|---|
| version | the version of the release; `npm install` takes the newest published one |
| React Native | >= 0.74, on the New Architecture |
| iOS | 12.0 |
| Android | `minSdk` 24 |

The package ships its TypeScript as the `main` entry, which is what Metro
compiles — there is no build step, and `npm run typecheck` is the only script.
`src/NativeXlog.ts` is the spec React Native's codegen reads, and
`codegenConfig` in `package.json` is what points it at `src`: `NativeXlogSpec`
is generated into the app's `React-Codegen` pod and into the Android build, and
neither half of the module has to be told the thirteen signatures twice.

## What each platform resolves

The two halves do not get their native the same way, and the difference is the
ecosystem's, not the port's:

* **Android** depends on the `marsrs` AAR of the same version
  (`io.github.orangeboychen.marsrs:marsrs`) from JitPack — the same coordinate,
  resolved the same way, as for an app that takes the AAR directly. The `.so`
  files are the AAR's, so nothing is built here and nothing Rust is needed.
* **iOS** carries `marsrs-xlog.xcframework` with it, in `ios/Frameworks`, and
  `mars_xlog.h` next to it in `ios/include`. CocoaPods cannot resolve the SwiftPM
  binary target of `Package.swift`, and there is no CocoaPods pod for it, so the
  release drops the framework of its own tag into the package before packing it.

Both files arrive with the release and not with this directory: what is checked
in is the module, and `scripts/package_react_native.sh` is what makes a complete
package out of it.

## Why there are two modules

`marsrs-react-native-xlog` is this module with the logging half only, and the
two are the pair the AARs of `platforms/android/` are — `mars-rs` and
`marsrs-xlog` — and the pair `MarsRS` and `MarsRSXlog` of `Package.swift` are.
Today they are the same package under two names: the C ABI is 28 `mars_xlog_*`
symbols and nothing else, and `scripts/build_xcframework.sh` fails the day it
is not, so taking this one costs exactly what `marsrs-react-native-xlog` costs
and the difference between the two is the promise, not the bytes. STN and SDT
land here and in nothing else, which is what an app that wants them is buying.

Take one of the two and not both: both carry the same `libmarsrsxlog.so`, and an
app with two of it does not build.
