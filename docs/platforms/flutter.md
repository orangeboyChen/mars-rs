# Flutter

Two plugins, one API: `marsrs_xlog` for an app that only logs, and `marsrs` for
the whole port — which today is the same thing, because what the plugin exposes
is the logger. STN and SDT land in `marsrs` and in nothing else: the C ABI
behind both carries them — see [the C ABI](/platforms/c-abi#the-task-pipeline)
— but neither is in the Dart yet. Take one of the two, not both: both carry the
same native library, and an app with two of it does not build.

## Install

```bash
flutter pub add marsrs_xlog    # or marsrs, for the whole port
```

The plugin is on pub.dev: `flutter pub add` takes the newest version there. A
release also carries the plugin itself, as `marsrs-flutter-xlog-<version>.tar.gz`,
and a `path:` dependency on the directory inside the tarball installs that one:

```yaml
# pubspec.yaml
dependencies:
  marsrs_xlog:
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
import 'package:marsrs_xlog/marsrs_xlog.dart';

final dir = await getTemporaryDirectory();          // path_provider
final xlog = await Xlog.open(
  XlogConfig(
    logDir: '${dir.path}/xlog',
    namePrefix: 'marsrs',
    level: LogLevel.info,
    mode: AppenderMode.async,
  ),
);
xlog.consoleLogEnabled = kDebugMode;

xlog.i('startup', 'hello from mars');

await xlog.flush(sync: true);   // before the app reads or uploads the files
await xlog.close();
```

`logDir` is the one option with no default — the rest are on
[the configuration page](/configuration), under the names the Kotlin of the port
gives them.

## What an app awaits

The plugin is a method channel and not `dart:ffi`: the Apple binary is a static
library inside `MarsRSXlog.xcframework`, and `DynamicLibrary.open` has nothing
to open for one. So what crosses to the platform thread is a message and not a
call, and a call that answers nothing waits for nothing — a write and a setting
hand the message over and return, `xlog.i('startup', '…')` the way it does in
Kotlin, in Swift and in TypeScript. The channel keeps the order the messages were
handed over in, so a record is written after the one handed over before it.

Four of them answer a `Future`, because four of them have something an app can
act on: the appender `Xlog.open` opens, the drain `flush` and `close` wait for,
and the answer `isLoggable` gives.

The five settings are the properties they are on every other platform of the
port, and not a `setLevel` / `getLevel` pair. What one of them answers is what
this side last wrote, and not what the appender holds: a getter answers in the
call it is read in, and what the platform side holds is a channel call away.
`isLoggable` is the appender's own answer, and it is the one an app awaits.

## Writing

The write is `android.util.Log`'s shape — `v`/`d`/`i`/`w`/`e`/`f`, each of them a
tag and a message, and `log(level, tag, message)` when the level is not known
until the call.

```dart
xlog.v('net', '…');
xlog.d('net', '…');
xlog.i('startup', '…');
xlog.w('net', '…');
xlog.e('login', '…');
xlog.f('login', '…');

xlog.log(LogLevel.debug, 'net', '…');
```

A record below the level the appender was opened at is dropped before anything is
formatted. A message that is expensive to build is worth asking about first —
the record is dropped either way, and what `isLoggable` saves is the string:

```dart
if (await xlog.isLoggable(LogLevel.debug)) {
  xlog.d('net', expensiveDescription());
}
```

## While it is open

| what | how |
|---|---|
| move the level | `xlog.level = LogLevel.warning` |
| read the level back | `xlog.level` |
| switch async / sync | `xlog.mode = AppenderMode.sync` |
| mirror records to the console | `xlog.consoleLogEnabled = true` |
| close a file at a size | `xlog.maxFileSizeBytes = 8 * 1024 * 1024` |
| drop a file at an age | `xlog.maxAliveTimeSeconds = 10 * 24 * 3600` |
| is it still open | `xlog.isOpen` |
| drain the cache | `await xlog.flush(sync: true)` |

A setting is written and not awaited, and what reading one back answers is what
this side last wrote — see [what an app awaits](#what-an-app-awaits).

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
`StnLogic`, no `SdtLogic`, no task and no check. What is missing is the Dart — the
C ABI the plugin is built over carries both, see
[the C ABI](/platforms/c-abi#the-task-pipeline). The native side it resolves does
not carry the same thing on both platforms, though:

| | what the plugin resolves | so a task today |
|---|---|---|
| Android | `io.github.orangeboychen.marsrs:marsrs` — the whole-port AAR, STN and SDT in it | the app's own Android code can start one, and a channel of its own can carry it to the Dart |
| iOS | `marsrs-xlog.xcframework` and `mars_xlog.h` — the logger, and nothing else | no: `MarsRSNet` is not vendored, so there is no `MarsStn` and no `MarsSdt` to link |

Which is why "start it from the platform side" is half an answer: it is one on
Android, and on iOS the net framework and the two net headers would have to be
packaged beside the xlog ones before it is one there.
