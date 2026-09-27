// swift-tools-version: 5.9
//
//  mars-rs — Swift Package Manager distribution (iOS, watchOS)
//
//      .package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")
//
//  and then
//
//      import MarsRS          // the whole port
//
//      var config = XlogConfig(logDirectory: logDir)
//      config.namePrefix = "Ham"
//      config.publicKey = "..."
//      let log = try Xlog(config)
//
//      log.info(message: "hello", tag: "Net")
//      log.flush(sync: true)
//
//  or, for an app that only logs:
//
//      import MarsRSXlog      // xlog alone, and the same API
//
//  or, for an app that runs tasks and does not log:
//
//      import MarsRSNet       // the diagnosis and the task pipeline
//
//  Two products over one prebuilt library, and the library is xlog's: the C ABI
//  is 28 `mars_xlog_*` symbols and nothing else, which is why the artifact is
//  named after xlog and not after the port — orangeboyChen/mars publishes
//  `MarsXlog.xcframework` for the same reason, and taking xlog alone is the only
//  thing its package offers. `MarsRSXlog` is the Swift over it.
//
//  The net half — `Stn.swift` and `Sdt.swift` — is the second artifact,
//  `MarsRSNet.xcframework`, built `--no-default-features --features sdt,stn`:
//  it carries `mars_sdt_*` and `mars_stn_*` and no `mars_xlog_*` at all, so an
//  app that only logs keeps downloading xlog alone and an app that takes both
//  links xlog's symbols exactly once. Two Rust static libraries out of one
//  crate do link side by side, which is not obvious: the symbols they share are
//  the crate's own, and rustc emits those private extern, so the second copy is
//  not a duplicate definition. `MarsRS` is the umbrella over both.
//
//  The package ships prebuilt xcframeworks built by
//  .github/workflows/release.yml (scripts/build_xcframework.sh), so consumers
//  need neither a Rust toolchain nor an NDK. The one the xlog binary target
//  points at is still `MarsRS.xcframework.zip`, because that is the name the tag
//  it resolves was published under; the release after it publishes
//  `MarsRSXlog.xcframework.zip`, and the workflow rewrites the name along with
//  the tag. The C module inside keeps the port's name, `MarsRSFFI`, for the
//  same reason: renaming it would break the asset every consumer resolves
//  until a release carries the renamed one.
//
//  `Package.swift` is rewritten by that workflow for every release: each binary
//  target's url and checksum are the ones of the tag being published — one pair
//  per artifact — and they arrive as a pull request because the default branch
//  refuses direct pushes.
//
import PackageDescription

let package = Package(
    name: "mars-rs",
    platforms: [
        .iOS(.v12),
        // watchOS 10, and the framework is 10 on the device too: what a watch
        // that runs watchOS 10–25 links is the `arm64_32` half of the device
        // slice — the 32-bit-pointer arm64 of every watch before the arm64
        // ones. That architecture is a tier 3 target no channel ships a std
        // for, so `scripts/build_xcframework.sh` builds one out of a nightly's
        // sources with `-Z build-std`. The arm64 half next to it is the
        // watchOS 26 one, and `.v10` is as new as `Platform.watchOS` gets in
        // swift-tools-version 5.9, which is where the manifest stays so that a
        // Swift 5 project can still depend on the package.
        .watchOS(.v10)
    ],
    products: [
        .library(name: "MarsRS", targets: ["MarsRS"]),
        // for a consumer who logs and nothing else
        .library(name: "MarsRSXlog", targets: ["MarsRSXlog"]),
        // for a consumer who runs tasks and logs nothing: the net half, which
        // is the half xlog is not
        .library(name: "MarsRSNet", targets: ["MarsRSNet"]),
    ],
    targets: [
        // Prebuilt binary: ios-arm64 + ios-arm64_x86_64-simulator +
        // watchos-arm64_arm64_32 + watchos-arm64-simulator, each with
        // `mars_xlog.h` and the module map that names it `MarsRSFFI`.
        //
        // What it holds is xlog and nothing else, and that is checked rather
        // than promised: `scripts/build_xcframework.sh` fails if a slice ever
        // exports a `mars_stn_*` or a `mars_sdt_*`, because the day it does the
        // answer is a second framework for `MarsRS` — not a wider promise from
        // this one, which is what shipping STN and SDT to an app that only logs
        // would be.
        .binaryTarget(
            name: "MarsRSFFI",
            url: "https://github.com/orangeboyChen/mars-rs/releases/download/v0.1.0-alpha.2/MarsRS.xcframework.zip",
            checksum: "076428c37d9532449048eddac02a69b0a0d7a227a7874ca22d981fd5622afa37"
        ),
        // A thin Swift face of the C ABI: a binary target is a module of C
        // symbols only, so this is where the strings and the enums of
        // `mars_xlog.h` become something Swift can call. It re-exports the C
        // module too, so `mars_xlog_*` stays available for the callers who
        // want it.
        //
        // The framework the static library needs and cannot name for itself:
        // a Rust static library carries no link flags, and the time zone
        // lookup of `iana-time-zone` calls `CFTimeZone*`. `import Foundation`
        // would bring it in for this module's own sake; naming it is what
        // keeps a caller who takes the C surface, or drops Foundation,
        // linking. (The C++ project names libc++ and libz here; the port
        // needs neither — it is Rust, and its zlib is `zlib-rs`.)
        .target(
            name: "MarsRSXlog",
            dependencies: ["MarsRSFFI"],
            path: "Sources/MarsRSXlog",
            linkerSettings: [
                .linkedFramework("CoreFoundation"),
            ]
        ),
        // The umbrella: everything the port exposes, so that one import is
        // enough. It carries no binary of its own — it re-exports the xlog one
        // and the net one, which is what `MarsRS` is made of, and an app that
        // only logs keeps importing `MarsRSXlog` and nothing more.
        .target(
            name: "MarsRS",
            dependencies: ["MarsRSXlog", "MarsRSNet"]
        ),
        // The net half's binary: the same slices, built `--no-default-features
        // --features sdt,stn`, with `mars_sdt.h` and `mars_stn.h` under one
        // umbrella and a module map naming the module `MarsRSNetFFI`.
        //
        // Carrying no `mars_xlog_*` is what it is for — an app that takes both
        // frameworks links xlog's symbols once — and it is checked rather than
        // promised: `scripts/build_xcframework.sh` fails the build the day a
        // slice of either framework exports a symbol that is not its own.
        //
        // No release carries this asset yet, so the url and the checksum are
        // the placeholders the release workflow rewrites: it points each binary
        // target at the tag it is publishing, one asset and one checksum per
        // artifact.
        .binaryTarget(
            name: "MarsRSNetFFI",
            url: "https://github.com/orangeboyChen/mars-rs/releases/download/v0.1.0-alpha.3/MarsRSNet.xcframework.zip",
            checksum: "0000000000000000000000000000000000000000000000000000000000000000"
        ),
        // The Swift over the net half's C ABI: `MarsSdt`, `MarsStn`, and the
        // `StnTask` / `StnQuestion` / `StnAnswer` they are made of.
        //
        // No `linkerSettings`, unlike the xlog target: the net archive
        // references nothing outside libSystem, which every app links. The
        // xlog one names `CoreFoundation` because `iana-time-zone` asks
        // `CFTimeZone*` what the time zone is, and a Rust static library
        // carries no link flags of its own.
        .target(
            name: "MarsRSNet",
            dependencies: ["MarsRSNetFFI"],
            path: "Sources/MarsRSNet"
        )
    ]
)
