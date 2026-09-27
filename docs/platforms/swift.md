# SwiftPM

```swift
// Package.swift
.package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")
// and, in the target that takes it:
.product(name: "marsrs-xlog", package: "mars-rs")
```

Three products, spelled the way the crates are: `marsrs` is the whole port,
`marsrs-xlog` is the logging half of it and `marsrs-net` is the half that is not
logging — the diagnosis and the task pipeline — for an app that runs tasks and
does not log. A product is what a manifest depends on and a module is what
`import` names, and the two do not have to agree: the modules behind those three
are `MarsRS`, `MarsRSXlog` and `MarsRSNet`, so taking `marsrs-xlog` is
`import MarsRSXlog`. The two halves are two artifacts and not one library split
at the Swift layer: `MarsRSXlog` is Swift over the `MarsRSFFI` binary target,
`MarsRSNet` over `MarsRSNetFFI`, and each is a static xcframework published with
the release — `marsrs-xlog.xcframework` and `marsrs-net.xcframework`.

## What either one holds

What either one holds is the feature set it is named for.
`scripts/build_xcframework.sh` builds `-p marsrs-ffi --no-default-features
--features xlog` for the first and `--no-default-features --features sdt,stn`
for the second — SDT and STN reach the C ABI behind features of their own, and a
set that is spelled out does not inherit them — and then fails if a slice of
either one exports a symbol of the port's that is not its own. That is what makes
the split worth having: an app that only logs downloads xlog's bytes and nothing
else, and an app that takes both links xlog's symbols exactly once. Two Rust
static libraries out of one crate do link side by side, which is not obvious —
the symbols they share are the crate's own, and rustc emits those private extern,
so the second copy is not a duplicate definition.

That is also how the C++ project answers it — orangeboyChen/mars publishes one
SPM product, `MarsXlog`, over a `MarsXlog.xcframework` built from the xlog subset
of its sources, and its Android build has an `--xlog-only` mode for the same
reason — and it is why the xlog artifact is named after xlog here and not after
the port. The tag and the SPM checksums are written into `Package.swift` by the
release workflow on a `chore/package-swift-<tag>` branch, proposed as a pull
request; there is one checksum per binary target, because a checksum is bound to
the zip it was computed from. A module is what a consumer imports and a zip is
what a consumer downloads, so the two are named by different rules: the
artifacts take the lower-case pair — `marsrs-xlog.xcframework.zip` and
`marsrs-net.xcframework.zip` — with the next release, while the C module inside
keeps the name `MarsRSFFI`, because a module rename travels only with the
release that publishes the renamed one. A zip's name asks for nothing of the
kind: SwiftPM extracts an artifact under its binary target's name and reads the
module map inside it, which is why the file can be lower-case while the module
it carries is not.

## The four slices

The framework carries four slices — `ios-arm64`, `ios-arm64_x86_64-simulator`,
`watchos-arm64_arm64_32` and `watchos-arm64-simulator` — so an app target of
either platform resolves it. The watchOS device slice holds two architectures:
`arm64_32`, which is what a watch running watchOS 10 to 25 links, and `arm64`,
which is what watchOS 26 moved its watches onto. `arm64_32` is a tier 3 target
no channel ships a std for, so it is built out of a nightly's sources with
`-Z build-std`; the arm64 half needs no such thing, and is built for
watchOS 26 because its std is. `x86_64-apple-watchos-sim` is left out — a
watchOS simulator is arm64.

What an app pays for the xlog framework is about 1.0 MB of `__TEXT` on arm64,
measured by linking a slice into an otherwise empty executable with
`-dead_strip`: the 21 MB archive of a slice is the shelf the linker picks from,
not what lands in the app. Each archive is stripped of its local symbols before
it is packaged — 31 % off the zip a consumer downloads, and nothing off the
link, because every symbol the artifact is named for is an external symbol and
stays.

## The Swift over the C ABI

`Sources/MarsRSXlog/Xlog.swift` and `Sources/MarsRSNet/` are what the port
exposes: the Swift over the 28 `mars_xlog_*` symbols, and the Swift over the
`mars_sdt_*` and `mars_stn_*` of the diagnosis and the task pipeline —
`MarsSdt`, `MarsStn` and the `StnTask`, `StnQuestion` and `StnAnswer` they ask
and answer with. `MarsRS` is the module that re-exports both halves, which is
why it exists as a module of its own and not just as a name for xlog.
orangeboyChen/mars ships a single `MarsXlog` product because xlog is all its
package has; here the xlog-only import is `import MarsRSXlog` and the net-only
one is `import MarsRSNet`.

An app writes through an `Xlog` of its own — `let log = try Xlog(XlogConfig(
logDirectory: dir))`, then `log.info(message: "hello", tag: "Net")` — the two
steps the Android `Xlog` is, with `XlogConfig`, `LogLevel`, `AppenderMode` and
`CompressMode` spelled the way the Kotlin API spells them, so one app reads the
same either way. `file`, `function` and `line` of a record come from the call
site: `#file` costs Swift nothing, and the C ABI carries them anyway. Nothing
here is deprecated, because there is no older Swift API to keep — the
process-wide appender is a set of C symbols, and `@_exported import MarsRSFFI`
reaches them.
