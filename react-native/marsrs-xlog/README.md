# marsrs-react-native-xlog

The React Native module of the xlog half of
[mars-rs](https://github.com/orangeboyChen/mars-rs), a Rust implementation of
[Tencent/mars](https://github.com/Tencent/mars): it writes the `.xlog` the C++
implementation produced, encrypted or plain, zlib- or zstd-compressed, sync or
async.

```ts
import { Xlog, LogLevel } from 'marsrs-react-native-xlog';

const xlog = Xlog.open({
  logDir: RNFS.DocumentDirectoryPath,
  namePrefix: 'marsrs',
  pubKey: '...',
});
xlog.i('Net', 'hello');
xlog.log(LogLevel.Debug, 'Net', 'a debug line');
xlog.flush(true);
xlog.close();
```

`Xlog` is one appender: `Xlog.open(config)` opens it and `close()` closes it —
the same class, under the same name, the Swift package, `kmp/marsrs-xlog` and
the Android AAR publish. Nothing here answers a `Promise` and nothing needs an
`await`: the module is a TurboModule, so a call is made on the JS thread and
returned from, and the settings are the properties they are in Kotlin and in
Swift — `xlog.level = LogLevel.Debug` — and not `setLevel` / `getLevel` pairs.

That is what the New Architecture buys, and it is the only thing asked for in
return: a TurboModule is not a bridge module, so an app still on the old
architecture has no module to `TurboModuleRegistry.getEnforcing`.

## Installing it

`npm install marsrs-react-native-xlog`, and `pod install` for the iOS half: the
module is published to npm, and the release of a tag is what publishes it.

```bash
npm install marsrs-react-native-xlog
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
neither half of the module has to be told the eleven signatures twice.

## What each platform resolves

The two halves do not get their native the same way, and the difference is the
ecosystem's, not the port's:

* **Android** depends on the `xlog` AAR of the same version
  (`io.github.orangeboychen.marsrs:xlog`) from JitPack — the same coordinate,
  resolved the same way, as for an app that takes the AAR directly. The `.so`
  files are the AAR's, so nothing is built here and nothing Rust is needed.
* **iOS** carries `MarsRSXlog.xcframework` with it, in `ios/Frameworks`, and
  `mars_xlog.h` next to it in `ios/include`. CocoaPods cannot resolve the SwiftPM
  binary target of `Package.swift`, and there is no CocoaPods pod for it, so the
  release drops the framework of its own tag into the package before packing it.

Both files arrive with the release and not with this directory: what is checked
in is the module, and `scripts/package_react_native.sh` is what makes a complete
package out of it.

## Why it is called xlog's

The C ABI is 28 `mars_xlog_*` symbols and nothing else, and
`scripts/build_xcframework.sh` fails the day it is not — the same reason the
framework and the AAR are called xlog's. `marsrs-react-native` is the umbrella
the whole port joins under: STN and SDT land there and not here, so an app that
only logs keeps this one and downloads nothing it does not call. Today the two
are the same module under two names, and taking the umbrella costs exactly what
this one costs.

Take one of the two and not both: both carry the same `libmarsrsxlog.so`, and an
app with two of it does not build.
