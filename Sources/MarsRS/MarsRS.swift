// The `MarsRS` module: the whole port in one import.
//
// Two halves, and the split is the one the Android packages make between
// `mars-rs` and `mars-rs-xlog`: `MarsRSXlog` is the 28 `mars_xlog_*` symbols of
// the C ABI, and `MarsRSNet` is the `mars_sdt_*` and `mars_stn_*` of the
// diagnosis and the task pipeline. Each comes from a framework of its own —
// neither carries a symbol of the other — so an app that only logs imports
// `MarsRSXlog` and downloads xlog alone, and an app that imports this one gets
// both, with xlog's symbols linked exactly once.

@_exported import MarsRSNet
@_exported import MarsRSXlog
