# The C ABI

`mars-rs-<version>-<host>.tar.gz` (Linux, macOS) and `.zip` (Windows) hold
`include/mars_xlog.h`, `include/mars_sdt.h`, `include/mars_stn.h` and the static
and shared libraries of `marsrs-ffi` — built `--features sdt,stn`, the task
pipeline and the diagnosis included, which is what the Android `.so` carries —
for `x86_64-unknown-linux-gnu`,
`aarch64-apple-darwin` and `x86_64-pc-windows-msvc`.

The headers are checked in next to the crate, at `crates/marsrs-ffi/include`,
which is the directory a consumer points a compiler at — and the one the Kotlin
Multiplatform module's cinterop is written against.
