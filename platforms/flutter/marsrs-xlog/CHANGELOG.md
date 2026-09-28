# Changelog

## 0.1.0-alpha.2

* The first version: `Xlog` over the 28 `mars_xlog_*` symbols the C ABI is —
  `Xlog.open`, the six levels `v` to `f`, `isLoggable`, `flush`, `close`, and the
  settings: `setLevel`, `getLevel`, `setMode`, `setConsoleLogEnabled`,
  `setMaxFileSize`, `setMaxAliveTime`. Every call answers a `Future`, because
  every one of them crosses the method channel.

  Android resolves `io.github.orangeboychen.marsrs:xlog:<version>` from JitPack;
  iOS carries the `MarsRSXlog.xcframework` of the same version, which CocoaPods
  cannot take out of `Package.swift`.
