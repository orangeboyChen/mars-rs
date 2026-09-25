// The `MarsRS` module: the whole port in one import.
//
// `mars-ffi` is an xlog C ABI today — 21 `mars_xlog_*` symbols and nothing
// else — so what this re-exports right now is `MarsRSXlog` and nothing more.
// `Stn.swift` and `Sdt.swift` land here as the C ABI grows, and an app that
// only logs keeps importing `MarsRSXlog`: the split is the one the Android
// packages make between `mars-rs` and `mars-rs-xlog`.

@_exported import MarsRSXlog
