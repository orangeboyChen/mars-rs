# mars_rs_xlog

The Flutter plugin of the xlog half of [mars-rs](https://github.com/orangeboyChen/mars-rs),
a Rust implementation of [Tencent/mars](https://github.com/Tencent/mars): it writes
the `.xlog` the C++ implementation produced, encrypted or plain, zlib- or
zstd-compressed, sync or async.

```dart
import 'package:mars_rs_xlog/mars_rs_xlog.dart';

final config = MarsXlogConfig(
  logDirectory: (await getTemporaryDirectory()).path,
  namePrefix: 'Ham',
  publicKey: '...',
);

await MarsRsXlog.open(config);
await MarsRsXlog.write(MarsXlogLevel.info, 'hello', tag: 'Net');
await MarsRsXlog.flush(sync: true);
await MarsRsXlog.close();
```

`MarsRsXlog` is one appender, opened with a configuration and closed by the name
it was opened with — what `MarsXlogInstance` of the Swift package and
`Xlog.newXlogInstance` of the Android AAR are. Its five methods and the fields of
`MarsXlogConfig` are the C ABI of `mars_xlog.h` and nothing more.

## Installing it

The release of a tag publishes `mars-rs-flutter-<version>.tar.gz`; unpack it next
to the app and depend on the directory:

```yaml
dependencies:
  mars_rs_xlog:
    path: ../mars_rs_xlog
```

| | |
|---|---|
| version | the version of the release the plugin was unpacked from |
| Flutter | >= 3.24 |
| iOS | 12.0 |
| Android | `minSdk` 21 |

## What each platform resolves

The two halves do not get their native the same way, and the difference is the
ecosystem's, not the port's:

* **Android** depends on the `mars-rs-xlog` AAR of the same version
  (`io.github.orangeboychen:mars-rs-xlog`) from JitPack — the same coordinate,
  resolved the same way, as for an app that takes the AAR directly. The `.so`
  files are the AAR's, so nothing is built here and nothing Rust is needed.
* **iOS** carries `MarsRSXlog.xcframework` with it, in `ios/Frameworks`, and
  `mars_xlog.h` next to it in `ios/include`. CocoaPods cannot resolve the SwiftPM
  binary target of `Package.swift`, and there is no CocoaPods pod for it, so the
  release drops the framework of its own tag into the plugin before packaging it.

Both files arrive with the release and not with this directory: what is checked
in is the plugin, and `scripts/package_flutter.sh` is what makes a complete
package out of it.

## Why it is called xlog's

The C ABI is 28 `mars_xlog_*` symbols and nothing else, and
`scripts/build_xcframework.sh` fails the day it is not — the same reason the
framework and the AAR are called xlog's. `MarsRS` is the umbrella the whole port
joins under, and a plugin of that name lands with the STN and SDT halves of the
C ABI.
