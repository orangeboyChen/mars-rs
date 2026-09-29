# Changelog

ohpm shows this file on the package's page, so it is the history of the package
and not the history of the repository: what changed in what an app installs, and
not every commit that got it there.

## 1.0.0

The first version of the package.

* `Xlog.open(config)` and the API every other platform of the port carries,
  from ArkTS: `v`/`d`/`i`/`w`/`e`/`f`, `log`, `level`, `mode`,
  `consoleLogEnabled`, `maxFileSizeBytes`, `maxAliveTimeSeconds`,
  `isLoggable`, `requestFlush`, `flushNow` and `close`.
* `libmarsrs_xlog.so` for `arm64-v8a`, `armeabi-v7a` and `x86_64`, with the
  Rust core linked in, so there is nothing for an app to resolve.
