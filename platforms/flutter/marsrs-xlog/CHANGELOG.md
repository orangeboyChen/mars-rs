# Changelog

## 0.1.0-alpha.3

* The plugin of [mars-rs v0.1.0-alpha.3]: the `xlog` AAR and the
  `marsrs-xlog.xcframework` of that release.

[mars-rs v0.1.0-alpha.3]: https://github.com/orangeboyChen/mars-rs/releases/tag/v0.1.0-alpha.3

## 0.1.0-alpha.2

* The first version: `Xlog` over the 16 `mars_xlog_*` symbols the C ABI is —
  `Xlog.open`, the six levels `v` to `f`, `isLoggable`, `flush`, `close`, and the
  settings: `setLevel`, `getLevel`, `setMode`, `setConsoleLogEnabled`,
  `setMaxFileSize`, `setMaxAliveTime`. Four of the calls answer a `Future` —
  `open`, `flush`, `close` and `isLoggable`, the four with something to answer;
  a write and a setting cross the channel without the caller waiting for it, and
  `requestFlush()` is one of those.

  Android resolves `io.github.orangeboychen.marsrs:xlog:<version>` from JitPack;
  iOS carries the `marsrs-xlog.xcframework` of the same version, which CocoaPods
  cannot take out of `Package.swift`.
