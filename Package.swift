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
//      config.namePrefix = "marsrs"
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
//  A product is not a module — the product is what a dependency is declared
//  against, the module is what `import` names — and SwiftPM asks for no
//  agreement between the two. These three give them the same name anyway,
//  which is what an app writes in its own manifest:
//
//      .product(name: "MarsRSXlog", package: "mars-rs")
//
//  A product is what Xcode lists to tick and what a manifest names, so one
//  spelled differently from the module behind it is a name a consumer has to
//  translate before they can write the import. Every package Apple ships
//  answers it the same way: `swift-log`'s product is `Logging`, and all twelve
//  of `swift-collections`' are their module names. What is lower-case is the
//  package, `mars-rs`, after the URL it is fetched from, and the two files a
//  consumer downloads — `marsrs-xlog.xcframework.zip` and
//  `marsrs-net.xcframework.zip` — the way the crates are spelled, because a
//  file name asks for nothing of a compiler.
//
//  Three products over two prebuilt libraries, and the first library is xlog's:
//  the C ABI is 28 `mars_xlog_*` symbols and nothing else, which is why its
//  artifact is named after xlog and not after the port — orangeboyChen/mars publishes
//  `MarsXlog.xcframework` for the same reason, and taking xlog alone is the only
//  thing its package offers. `MarsRSXlog` is the Swift over it.
//
//  The net half — `Stn.swift` and `Sdt.swift` — is the second artifact,
//  `marsrs-net.xcframework`, built `--no-default-features --features sdt,stn`:
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
//  `marsrs-xlog.xcframework.zip`, and the workflow rewrites the name along with
//  the tag. The C module inside keeps the port's name, `MarsRSFFI`, for the
//  same reason: renaming it would break the asset every consumer resolves
//  until a release carries the renamed one. A binary target is named after the
//  module it carries and not after the zip it downloads, and SwiftPM asks for
//  no match between the two — the zip is extracted under the target's name and
//  the module is the one the module map inside declares.
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
        // One name for two things, and the way Apple's own packages do it —
        // `swift-log` is a package whose product is `Logging`: a product is the
        // name a manifest depends on and the module behind it is the name an
        // `import` spells, so there is nothing to translate between the two.
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
            url: "https://github.com/orangeboyChen/mars-rs/releases/download/v0.1.0-alpha.3/marsrs-xlog.xcframework.zip",
            checksum: "bf80759153364056ec70461017d967f59a682701b0a74c4a53f42e78ca31bc31"
        ),
        // A thin Swift face of the C ABI: a binary target is a module of C
        // symbols only, so this is where the strings and the enums of
        // `mars_xlog.h` become something Swift can call. It re-exports the C
        // module too, so `mars_xlog_*` stays available for the callers who
        // want it.
        //
        // `MarsRSXlog` and not `marsrs-xlog`: the product above is the name a
        // consumer depends on, and this is the name their `import` spells —
        // the same name, so the two ask for no translating. It is also the
        // name every artifact of the build carries: `MarsRSXlog.swiftmodule`,
        // `MarsRSXlog-Swift.h`, and the product never appears in one of them.
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
            path: "platforms/apple/MarsRSXlog",
            linkerSettings: [
                .linkedFramework("CoreFoundation"),
            ]
        ),
        // The umbrella: everything the port exposes, so that one import is
        // enough. It carries no binary of its own — it re-exports the xlog one
        // and the net one, which is what `MarsRS` is made of, and an app that
        // only logs keeps importing `MarsRSXlog` and nothing more.
        //
        // The three Swift modules are `platforms/apple/`, one directory each:
        // every platform of the port has a directory of its own under
        // `platforms/` — `android`, `flutter`, `kmp` and `react-native` are
        // the packaging of the others — and a target that names no `path` is
        // a target SwiftPM looks for at the root.
        .target(
            name: "MarsRS",
            dependencies: ["MarsRSXlog", "MarsRSNet"],
            path: "platforms/apple/MarsRS"
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
            url: "https://github.com/orangeboyChen/mars-rs/releases/download/v0.1.0-alpha.3/marsrs-net.xcframework.zip",
            checksum: "8910a092bad8317109bb72e6ef2626bcedb680983534c91ce9aced0da043dcfa"
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
            path: "platforms/apple/MarsRSNet"
        )
    ]
)
