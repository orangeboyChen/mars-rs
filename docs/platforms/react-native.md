# React Native

Two packages, one API: `marsrs-react-native-xlog` for an app that only logs, and
`marsrs-react-native` for the whole port — which today is the same thing, because
the C ABI is 28 `mars_xlog_*` symbols and nothing else. STN and SDT land in
`marsrs-react-native` and in nothing else. Take one of the two, not both: both
carry the same native library, and an app with two of it does not build.

## Install

```bash
npm install marsrs-react-native-xlog        # or marsrs-react-native, for the whole port
cd ios && pod install
```

The package is published to npm — publishing is not switched on yet, and until it
is, `scripts/package_react_native.sh <version> <asset-dir>` writes the same module
as `marsrs-react-native-xlog-<version>.tgz`: `npm pack`, the version stamped into
`package.json`, the apple job's `MarsRSXlog.xcframework` copied in. `npm install
./marsrs-react-native-xlog-<version>.tgz` installs that one.

What the package carries that a checkout does not is that framework: CocoaPods
cannot resolve the SwiftPM binary target of `Package.swift`, so iOS needs it
inside the package, and the repository keeps none. Android needs nothing extra: it
resolves `io.github.orangeboychen.marsrs:xlog:<version>` from JitPack, the same
coordinate an app that takes the AAR directly does.

Nothing in the app has to name the module: autolinking finds the `ReactPackage`
in `android/` and the pod in `ios/`, which is what puts `Xlog` in
`NativeModules`.

## Open, write, flush

```ts
import { AppenderMode, LogLevel, Xlog } from 'marsrs-react-native-xlog';

const xlog = await Xlog.open({
  logDir: `${RNFS.DocumentDirectoryPath}/xlog`,
  namePrefix: 'marsrs',
  level: LogLevel.info,
  mode: AppenderMode.async,
});
await xlog.setConsoleLogEnabled(__DEV__);

await xlog.i('startup', 'hello from mars');

await xlog.flush(true);   // before the app reads or uploads the files
await xlog.close();
```

`logDir` is the one option with no default — the rest are on
[the configuration page](/configuration), under the names the Kotlin of the port
gives them.

## Every call is a `Promise`

The module is a native module and not a JSI binding: the Apple binary is a static
library inside `MarsRSXlog.xcframework`, and there is nothing to `dlopen` for
one. So every call that reaches the appender crosses to the platform thread and
answers a `Promise` — `await` it, or let it go when the caller does not want to
wait for a record:

```ts
void xlog.i('startup', 'cold start');
```

That is the one thing this surface cannot share with the Swift and the Kotlin of
the port, where a record costs a call and nothing else, and it is why the
settings are `setLevel(…)` and not `level = …`. What an app reads back —
`level`, `mode`, `maxFileSizeBytes` … — is a getter over the value this instance
holds, and needs no round trip.

## Writing

The write is `android.util.Log`'s shape — `v`/`d`/`i`/`w`/`e`/`f`, each of them a
tag and a message, and `log(level, tag, message)` when the level is not known
until the call.

```ts
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

```ts
if (await xlog.isLoggable(LogLevel.debug)) {
  await xlog.d('net', expensiveDescription());
}
```

## While it is open

| what | how |
|---|---|
| move the level | `await xlog.setLevel(LogLevel.warning)` |
| read the level back | `xlog.level` |
| switch async / sync | `await xlog.setMode(AppenderMode.sync)` |
| mirror records to the console | `await xlog.setConsoleLogEnabled(true)` |
| close a file at a size | `await xlog.setMaxFileSize(8 * 1024 * 1024)` |
| drop a file at an age | `await xlog.setMaxAliveTime(10 * 24 * 3600)` |
| is it still open | `xlog.isOpen` |
| drain the cache | `await xlog.flush(true)` |

`close()` drains what is left and closes the appender. Two `Xlog`s of one
`namePrefix` are one appender — the native side is one module holding one
appender per prefix, and every call carries the prefix it is about — so give a
part of the app whose logs are read apart from the rest a prefix of its own.

## What is not in it

The file, the function and the line of a record are empty: there is no JS frame
to name, and the C++ writes an empty one too. Where the current file is is not
answered — no `mars_xlog_current_log_path` and no `Xlog.currentLogPath` — because
the path is a thing the app asks of the directory it gave, and
[the log files page](/log-files) is what names it.
