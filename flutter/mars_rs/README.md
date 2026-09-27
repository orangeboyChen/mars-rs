# mars_rs

The Flutter plugin of the whole of [mars-rs](https://github.com/orangeboyChen/mars-rs),
a Rust implementation of [Tencent/mars](https://github.com/Tencent/mars): the
`.xlog` the C++ implementation produced — encrypted or plain, zlib- or
zstd-compressed, sync or async — today, and STN and SDT when the C ABI carries
them.

```dart
import 'package:mars_rs/mars_rs.dart';

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
`Xlog.newXlogInstance` of the Android AAR are. Its six methods and the fields of
`MarsXlogConfig` are the C ABI of `mars_xlog.h` and nothing more.

## Installing it

The release of a tag publishes `mars-rs-flutter-<version>.tar.gz`; unpack it next
to the app and depend on the directory:

```yaml
dependencies:
  mars_rs:
    path: ../mars_rs
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

* **Android** depends on the `mars-rs` AAR of the same version
  (`io.github.orangeboychen:mars-rs`) from JitPack — the same coordinate,
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

`mars_rs_xlog` is this plugin with the logging half only, and the two are the
pair the AARs of `android/` are — `mars-rs` and `mars-rs-xlog` — and the pair
`MarsRS` and `MarsRSXlog` of `Package.swift` are. Today they are the same package
under two names: the C ABI is 28 `mars_xlog_*` symbols and nothing else, and
`scripts/build_xcframework.sh` fails the day it is not, so taking this one costs
exactly what `mars_rs_xlog` costs and the difference between the two is the
promise, not the bytes. STN and SDT land here and in nothing else, which is what
an app that wants them is buying.

Take one of the two and not both: both carry the same `libmarsxlog.so`, and an
app with two of it does not build.
