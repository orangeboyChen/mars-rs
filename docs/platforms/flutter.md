# Flutter

Two plugins, one API: `marsrs_flutter_xlog` for an app that only logs, and
`marsrs_flutter` for the whole port — which today is the same thing, because what
the plugin exposes is the logger. STN and SDT land in `marsrs_flutter` and in
nothing else: the C ABI behind both carries them — see
[the C ABI](/platforms/c-abi#the-task-pipeline) — but neither is in the Dart
yet. Take one of the two, not both: both carry the same native library, and an
app with two of it does not build.

## Install

```bash
flutter pub add marsrs_flutter_xlog        # or marsrs_flutter, for the whole port
```

The plugin is on pub.dev: `flutter pub add` takes the newest version there. A
release also carries the plugin itself, as `marsrs-flutter-xlog-<version>.tar.gz`,
and a `path:` dependency on the directory inside the tarball installs that one:

```yaml
# pubspec.yaml
dependencies:
  marsrs_flutter_xlog:
    path: marsrs-xlog
```

What the plugin carries that a checkout does not is that framework: CocoaPods
cannot resolve the SwiftPM binary target of `Package.swift`, so iOS needs it
inside the plugin, and the repository keeps none. Android needs nothing extra: it
resolves `io.github.orangeboychen.marsrs:xlog:<version>` from JitPack, the same
coordinate an app that takes the AAR directly does.

`flutter pub get` next. `pod install` is `flutter build ios`'s own business — the
plugin is a normal Flutter plugin with a podspec, and nothing in the app's
`Podfile` has to name it.

## Open, write, flush

```dart
import 'package:marsrs_flutter_xlog/marsrs_flutter_xlog.dart';

final dir = await getTemporaryDirectory();          // path_provider
final xlog = await Xlog.open(
  XlogConfig(
    logDir: '${dir.path}/xlog',
    namePrefix: 'marsrs',
    level: LogLevel.info,
    mode: AppenderMode.async,
  ),
);
await xlog.setConsoleLogEnabled(kDebugMode);

await xlog.i('startup', 'hello from mars');

await xlog.flush(sync: true);   // before the app reads or uploads the files
await xlog.close();
```

`logDir` is the one option with no default — the rest are on
[the configuration page](/configuration), under the names the Kotlin of the port
gives them.

## Every call is a `Future`

The plugin is a method channel and not `dart:ffi`: the Apple binary is a static
library inside `MarsRSXlog.xcframework`, and `DynamicLibrary.open` has nothing
to open for one. So every call below crosses to the platform thread and answers a
`Future` — `await` it, or hand it to `unawaited()` when the caller does not want
to wait for a record:

```dart
unawaited(xlog.i('startup', 'cold start'));
```

That is the one thing this surface cannot share with the Swift and the Kotlin of
the port, where a record costs a call and nothing else, and it is why the
settings are `setLevel(…)` and not `level = …`.

## Writing

The write is `android.util.Log`'s shape — `v`/`d`/`i`/`w`/`e`/`f`, each of them a
tag and a message, and `log(level, tag, message)` when the level is not known
until the call.

```dart
await xlog.v('net', '…');
await xlog.d('net', '…');
await xlog.i('startup', '…');
await xlog.w('net', '…');
await xlog.e('login', '…');
await xlog.f('login', '…');

await xlog.log(LogLevel.debug, 'net', '…');
```

A record below the level the appender was opened at is dropped before anything is
formatted. A message that is expensive to build is worth asking about first —
the record is dropped either way, and what `isLoggable` saves is the string:

```dart
if (await xlog.isLoggable(LogLevel.debug)) {
  await xlog.d('net', expensiveDescription());
}
```

## While it is open

| what | how |
|---|---|
| move the level | `await xlog.setLevel(LogLevel.warning)` |
| read the level back | `await xlog.getLevel()` |
| switch async / sync | `await xlog.setMode(AppenderMode.sync)` |
| mirror records to the console | `await xlog.setConsoleLogEnabled(true)` |
| close a file at a size | `await xlog.setMaxFileSize(8 * 1024 * 1024)` |
| drop a file at an age | `await xlog.setMaxAliveTime(10 * 24 * 3600)` |
| is it still open | `xlog.isOpen` |
| drain the cache | `await xlog.flush(sync: true)` |

`close()` drains what is left and closes the appender. Two `Xlog`s of one
`namePrefix` are one appender — the native side is one plugin holding one
appender per prefix, and every call carries the prefix it is about — so give a
part of the app whose logs are read apart from the rest a prefix of its own.

## What is not in it

The file, the function and the line of a record are empty: there is no Dart frame
to name, and the C++ writes an empty one too. Where the current file is is not
answered — no `mars_xlog_current_log_path` and no `Xlog.currentLogPath` — because
the path is a thing the app asks of the directory it gave, and
[the log files page](/log-files) is what names it.

Neither [the task pipeline](/stn) nor [the network diagnosis](/sdt) is in it: no
`StnLogic`, no `SdtLogic`, no task and no check. The C ABI the plugin is built
over carries both, and the two AARs and frameworks it resolves carry both, so
what is missing is the Dart — it lands in `marsrs_flutter` and nowhere else. An
app that needs a task today starts it from the platform side of its own plugin,
or waits for the call to land.
