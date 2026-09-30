# Changelog

ohpm shows this file on the package's page, so it is the history of the package
and not the history of the repository: what changed in what an app installs, and
not every commit that got it there.

## 1.0.0

The first version of the package.

* `Xlog.open(config)` and the API every other platform of the port carries,
  from ArkTS: `v`/`d`/`i`/`w`/`e`/`f`, `log`, `level`, `mode`,
  `consoleLogEnabled`, `maxFileSizeBytes`, `maxAliveTimeSeconds`,
  `isLoggable`, `requestFlush`, `flushNow` and `close` — and the three that
  answer where the files are: `namePrefix`, `isOpen`, `currentLogPath`,
  `logFiles` and `logFileNames`. Every one of them is answered out of this
  appender's own prefix and directory.
* No `flush()`: an awaited drain is an async work item and a promise, and every
  method of the NAPI module behind this package is synchronous — so what a
  caller that wants the drain off its own thread gives `flushNow()` is a
  thread of its own.
* `libmarsrs_xlog.so` for `arm64-v8a`, `armeabi-v7a` and `x86_64`, with the
  Rust core linked in, so there is nothing for an app to resolve.
