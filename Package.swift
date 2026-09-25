// swift-tools-version: 5.9
//
//  mars-rs — Swift Package Manager distribution (iOS)
//
//      .package(url: "https://github.com/orangeboyChen/mars-rs", from: "0.1.0")
//
//  and then
//
//      import MarsRS
//
//      var config = MarsXlogConfiguration(logDirectory: logDir)
//      config.namePrefix = "Ham"
//      config.publicKey = "..."
//      MarsXlog.open(config)
//
//      MarsXlog.write(.info, tag: "Net", message: "hello")
//      MarsXlog.flush(sync: true)
//
//  One module for the whole project, named after it: `MarsRS`. What the C ABI
//  carries today is xlog, so Xlog.swift is the only file in Sources/MarsRS —
//  Stn.swift and Sdt.swift join it as mars-ffi grows past the logging half.
//
//  The package ships a prebuilt MarsRS.xcframework built by
//  .github/workflows/release.yml (scripts/build_xcframework.sh), so consumers
//  need neither a Rust toolchain nor an NDK.
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
        .iOS(.v12)
    ],
    products: [
        .library(name: "MarsRS", targets: ["MarsRS"])
    ],
    targets: [
        // Prebuilt binary: ios-arm64 + ios-arm64_x86_64-simulator, each with
        // `mars_xlog.h` and the module map that names it `MarsRSFFI`.
        .binaryTarget(
            name: "MarsRSFFI",
            url: "https://github.com/orangeboyChen/mars-rs/releases/download/v0.0.0/MarsRS.xcframework.zip",
            checksum: "0000000000000000000000000000000000000000000000000000000000000000"
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
            name: "MarsRS",
            dependencies: ["MarsRSFFI"],
            path: "Sources/MarsRS",
            linkerSettings: [
                .linkedFramework("CoreFoundation"),
            ]
        )
    ]
)
