// The `MarsRS` module: the whole port in one import.
//
// Two halves, and the split is the one the Android packages make between
// `marsrs` and `xlog`: `MarsRSXlog` is the `mars_xlog_*` half of the C ABI —
// the sixteen of `mars_xlog.h` an appender of an app's own is opened and
// written through — and `MarsRSNet` is the `mars_sdt_*` and `mars_stn_*` of
// the diagnosis and the task pipeline. Each comes from a framework of its own —
// neither carries a symbol of the other — so an app that only logs imports
// `MarsRSXlog` and downloads xlog alone, and an app that imports this one gets
// both, with one `@_exported import` per half.
//
// The two are not one framework, which is why this module is two imports and
// nothing else: `MarsRS` carries no source of its own, and what it exports is
// what the two modules below it declare.
@_exported import MarsRSNet
@_exported import MarsRSXlog
