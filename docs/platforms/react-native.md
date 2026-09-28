# React Native

Two packages, one API: `marsrs-react-native-xlog` for an app that only logs, and
`marsrs-react-native` for the whole port — which today is the same thing, because
the C ABI is 28 `mars_xlog_*` symbols and nothing else. STN and SDT land in
`marsrs-react-native` and in nothing else. Take one of the two, not both: both
carry the same native library, and an app with two of it does not build.

The module is a TurboModule, which is what the New Architecture is for: React
Native 0.74 or newer, with the bridge switched off. It is the one thing this
surface asks of the app, and it is what pays for the rest — a TurboModule is not
a bridge module, so an app still on the old architecture has no module to
`TurboModuleRegistry.getEnforcing`.

## Install

```bash
npm install marsrs-react-native-xlog        # or marsrs-react-native, for the whole port
cd ios && pod install
```

The package is on npm: `npm install` takes the newest version there. A release
also carries the module, as `marsrs-react-native-xlog-<version>.tgz`. `npm install
./marsrs-react-native-xlog-<version>.tgz` installs that one.

What the package carries that a checkout does not is that framework: CocoaPods
cannot resolve the SwiftPM binary target of `Package.swift`, so iOS needs it
inside the package, and the repository keeps none. Android needs nothing extra: it
resolves `io.github.orangeboychen.marsrs:xlog:<version>` from JitPack, the same
coordinate an app that takes the AAR directly does.

Nothing in the app has to name the module: autolinking finds the `ReactPackage`
in `android/` and the pod in `ios/`, which is what puts `Xlog` in
`TurboModuleRegistry`. `src/NativeXlog.ts` is the spec React Native's codegen
reads, and `codegenConfig` in `package.json` is what points it at `src`:
`NativeXlogSpec` is generated into the app's `React-Codegen` pod and into the
Android build, and neither half of the module has to be told the eleven
signatures twice.

## Open, write, flush

```ts
import { AppenderMode, LogLevel, Xlog } from 'marsrs-react-native-xlog';

const xlog = Xlog.open({
  logDir: `${RNFS.DocumentDirectoryPath}/xlog`,
  namePrefix: 'marsrs',
  level: LogLevel.info,
  mode: AppenderMode.async,
});
xlog.consoleLogEnabled = __DEV__;

xlog.i('startup', 'hello from mars');

xlog.flush(true);   // before the app reads or uploads the files
xlog.close();
```

`logDir` is the one option with no default — the rest are on
[the configuration page](/configuration), under the names the Kotlin of the port
gives them. `Xlog.open` throws when the appender will not take the directory: an
empty `logDir` or `namePrefix`, or one the process cannot write to.

## Nothing answers a `Promise`

The module's method queue is `RCTJSThread`, so a method of it is made on the JS
thread and returns from there: a call that answers a value answers it before the
next line runs, and none of them answers a `Promise`. That is what lets this
`Xlog` be the `Xlog` of `platforms/apple/MarsRSXlog` and of
`platforms/android/marsrs` member for member — `Xlog.open(config)` answers the
appender, and `xlog.i(tag, message)` has landed by the time it returns.

The five settings are properties and not `setLevel` / `getLevel` pairs, because a
JS property is the spelling the Swift and the Kotlin use and a TurboModule setter
is called where it is written:

```ts
xlog.level = LogLevel.warning;      // the appender's own, read back through it
xlog.maxFileSizeBytes = 8 * 1024 * 1024;
```

`level` is the one of the five the C ABI answers a getter for, so `xlog.level`
reads the appender's own; `mode`, `consoleLogEnabled`, `maxFileSizeBytes` and
`maxAliveTimeSeconds` answer what this instance last wrote, which is all the C
ABI leaves to answer — and all the Swift and the Kotlin of the port answer too.

## Writing

The write is `android.util.Log`'s shape — `v`/`d`/`i`/`w`/`e`/`f`, each of them a
tag and a message, and `log(level, tag, message)` when the level is not known
until the call.

```ts
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

```ts
if (xlog.isLoggable(LogLevel.debug)) {
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
| drain the cache | `xlog.flush(true)` |

`close()` drains what is left and closes the appender. Two `Xlog`s of one
`namePrefix` are one appender — the native side is one module holding one
appender per prefix, and every call carries the prefix it is about — and an
`Xlog.open` of a prefix that is already open answers the appender it made
rather than a second one over it, so two names hold one `Xlog` and `close` on
either closes it for both. A part of the app whose logs are read apart from the
rest wants a prefix of its own.

## What is not in it

The file, the function and the line of a record are empty: there is no JS frame
to name, and the C++ writes an empty one too. Where the current file is is not
answered — no `mars_xlog_current_log_path` and no `Xlog.currentLogPath` — because
the path is a thing the app asks of the directory it gave, and
[the log files page](/log-files) is what names it.
