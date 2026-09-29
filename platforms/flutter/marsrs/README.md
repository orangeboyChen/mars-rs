# marsrs

The Flutter plugin of the whole of [mars-rs](https://github.com/orangeboyChen/mars-rs),
a Rust implementation of [Tencent/mars](https://github.com/Tencent/mars): the
`.xlog` the C++ implementation produced — encrypted or plain, zlib- or
zstd-compressed, sync or async — today, and STN and SDT when the C ABI carries
them.

```dart
import 'package:marsrs/marsrs.dart';

final xlog = await Xlog.open(
  XlogConfig(
    logDir: (await getTemporaryDirectory()).path,
    namePrefix: 'marsrs',
    pubKey: '...',
  ),
);

xlog.i('Net', 'hello');
await xlog.flush();
await xlog.close();
```

`Xlog` is one appender: `Xlog.open(config)` opens it and `close()` closes it —
the same class, under the same name, the Swift package,
`platforms/kmp/marsrs-xlog` and the Android AAR publish. A write and a setting
are calls and not `await`s, and the five settings are the properties they are in
Kotlin and in Swift: the channel is crossed without the caller waiting for it. `requestFlush()` is a call
of that kind too — it tells the writer thread to drain and returns at once —
and `flush()` is the drain an app waits for. What answers a `Future` is
what an app can act on — the appender `open` opens, the drain `flush()`
and `close()` wait for, and the answer `isLoggable` gives.

## Installing it

`flutter pub add marsrs`: the plugin is published to pub.dev, and the
release of a tag is what publishes it.

| | |
|---|---|
| version | the version of the release; `flutter pub add` takes the newest published one |
| Flutter | >= 3.24 |
| iOS | 12.0 |
| Android | `minSdk` 21 |

## What each platform resolves

The two halves do not get their native the same way, and the difference is the
ecosystem's, not the port's:

* **Android** depends on the `marsrs` AAR of the same version
  (`io.github.orangeboychen.marsrs:marsrs`) from JitPack — the same coordinate,
  resolved the same way, as for an app that takes the AAR directly. The `.so`
  files are the AAR's, so nothing is built here and nothing Rust is needed.
* **iOS** carries `MarsRSXlog.xcframework` with it, in `ios/Frameworks`, and
  `mars_xlog.h` next to it in `ios/include`. CocoaPods cannot resolve the SwiftPM
  binary target of `Package.swift`, and there is no CocoaPods pod for it, so the
  release drops the framework of its own tag into the plugin before packaging it.

Both files arrive with the release and not with this directory: what is checked
in is the plugin, and `scripts/package_flutter.sh` is what makes a complete
package out of it.

## Why there are two plugins

`marsrs_xlog` is this plugin with the logging half only, and the two
are the pair the AARs of `platforms/android/` are — `mars-rs` and `marsrs-xlog`
— and the pair `MarsRS` and `MarsRSXlog` of `Package.swift` are. Today they are
the same package under two names: the C ABI is 28 `mars_xlog_*` symbols and
nothing else, and `scripts/build_xcframework.sh` fails the day it is not, so
taking this one costs exactly what `marsrs_xlog` costs and the
difference between the two is the promise, not the bytes. STN and SDT land here
and in nothing else, which is what an app that wants them is buying.

Take one of the two and not both: both carry the same `libmarsrsxlog.so`, and an
app with two of it does not build.
