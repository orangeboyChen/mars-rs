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
//      var config = MarsXlogConfiguration(logDirectory: logDir)
//      config.namePrefix = "Ham"
//      config.publicKey = "..."
//      MarsXlog.open(config)
//
//      MarsXlog.write(.info, tag: "Net", message: "hello")
//      MarsXlog.flush(sync: true)
//
//  or, for an app that only logs:
//
//      import MarsRSXlog      // xlog alone, and the same API
//
//  Two products over one prebuilt library, and the library is xlog's: the C ABI
//  is 28 `mars_xlog_*` symbols and nothing else, which is why the artifact is
//  named after xlog and not after the port — orangeboyChen/mars publishes
//  `MarsXlog.xcframework` for the same reason, and taking xlog alone is the only
//  thing its package offers. `MarsRSXlog` is the Swift over it; `MarsRS` is the
//  umbrella, and `Stn.swift` / `Sdt.swift` land there — with a framework of
//  their own, so that an app that only logs keeps downloading xlog alone.
//
//  The package ships a prebuilt xcframework built by
//  .github/workflows/release.yml (scripts/build_xcframework.sh), so consumers
//  need neither a Rust toolchain nor an NDK. The one this points at is still
//  `MarsRS.xcframework.zip`, because that is the name the tag it resolves was
//  published under; the release after it publishes
//  `MarsRSXlog.xcframework.zip`, and the workflow rewrites the name along with
//  the tag. The C module inside keeps the port's name, `MarsRSFFI`, for the
//  same reason: renaming it would break the asset every consumer resolves
//  until a release carries the renamed one.
//
//  `Package.swift` is rewritten by that workflow for every release: the
//  binary target's url and its checksum are the ones of the tag being
//  published, and they arrive as a pull request because the default branch
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
        // enough. It carries no binary of its own while `mars-ffi` is xlog
        // only — it re-exports the xlog one — and `Stn.swift` / `Sdt.swift`
        // land here with a `MarsRS.xcframework` of their own, so that an app
        // that only logs keeps importing `MarsRSXlog` and nothing more.
        .target(
            name: "MarsRS",
            dependencies: ["MarsRSXlog"]
        )
    ]
)
