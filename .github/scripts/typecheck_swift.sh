#!/usr/bin/env bash
# Compiles the Swift of `platforms/apple` for every platform Package.swift
# promises, and links nothing.
#
#   .github/scripts/typecheck_swift.sh
#
# `swift build` cannot do it, and that is why this script exists: two of the
# five targets of Package.swift are binary ones, and what a binary target
# holds is a prebuilt xcframework — an iOS and watchOS one, which no macOS
# host links, and which a pull request would have to download first. So
# nothing in a pull request ever compiled the Swift: the first compile of it
# was the release, the run that builds the framework an app downloads. A
# symbol the SDK of one platform does not have — `UIApplication` on watchOS,
# which `canImport(UIKit)` answers `true` for — was a red release rather than
# a red pull request.
#
# What stands in for the two binary targets is a module map over the headers
# the frameworks carry, written out of the crate the headers belong to:
# `mars_xlog.h` as `MarsRSFFI`, and `mars_sdt.h` with `mars_stn.h` as
# `MarsRSNetFFI`. A C module is all a Swift file takes from either target —
# the rest of a binary target is a static library that no compile of Swift
# reads — so this answers the question the release's compile answers.
#
# A module is emitted rather than the sources being `-typecheck`ed, because a
# type-check stops before SILGen, and SILGen is where a use of `self` before
# `super.init()` is caught: the one thing a type-check of a class with an
# implicit `super.init()` cannot see. Emitting a module is still no link — no
# slice of any SDK is linked here — and it is what lets the umbrella module,
# which imports the other two, be compiled after them.
#
# Six triples: one per destination the two frameworks carry a slice for, at
# the deployment target Package.swift declares for its platform. A device
# triple is not enough, because a `#if targetEnvironment(simulator)`, a
# `#if arch(x86_64)` or a symbol the simulator's SDK answers differently are
# compiled by a simulator triple and by nothing else — and the iOS simulator
# is two architectures, arm64 and x86_64, which is why its slice is lipo'd
# out of both. A watchOS simulator is arm64 only, and nothing asks for the
# x86_64 one.

set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The C modules, out of the headers of the crate that generates them.
mkdir -p "$work/module/MarsRSFFI" "$work/module/MarsRSNetFFI"
cp "$repo/crates/marsrs-ffi/include/mars_xlog.h" "$work/module/MarsRSFFI/"
cp "$repo/crates/marsrs-ffi/include/mars_sdt.h" \
   "$repo/crates/marsrs-ffi/include/mars_stn.h" "$work/module/MarsRSNetFFI/"
cat > "$work/module/MarsRSFFI/module.modulemap" <<'MAP'
module MarsRSFFI {
    header "mars_xlog.h"
    export *
}
MAP
cat > "$work/module/MarsRSNetFFI/module.modulemap" <<'MAP'
module MarsRSNetFFI {
    header "mars_sdt.h"
    header "mars_stn.h"
    export *
}
MAP

# The Swift modules, in the order they depend on each other: `MarsRS` is the
# umbrella, and it imports the other two.
modules=(MarsRSXlog MarsRSNet MarsRS)
# <sdk>:<triple>, and the triple carries the deployment target the package
# declares — the version a symbol's availability is read against. The four
# destinations are the `slices` of scripts/build_xcframework.sh, the ones an
# app that takes the xcframework builds for.
platforms=(
  "iphoneos:arm64-apple-ios12.0"
  "iphonesimulator:arm64-apple-ios12.0-simulator"
  "iphonesimulator:x86_64-apple-ios12.0-simulator"
  "watchos:arm64_32-apple-watchos10.0"
  "watchos:arm64-apple-watchos10.0"
  "watchsimulator:arm64-apple-watchos10.0-simulator"
)

for platform in "${platforms[@]}"; do
  sdk="${platform%%:*}"
  triple="${platform##*:}"
  sdk_path="$(xcrun --sdk "$sdk" --show-sdk-path)"
  built="$work/$triple"
  mkdir -p "$built"
  for module in "${modules[@]}"; do
    echo "== $module ($triple)"
    # `-I "$built"`: the modules compiled before this one, which is what the
    # umbrella needs.
    xcrun swiftc -emit-module \
      -module-name "$module" \
      -sdk "$sdk_path" \
      -target "$triple" \
      -I "$work/module" -I "$built" \
      -o "$built/$module.swiftmodule" \
      "$repo/platforms/apple/$module"/*.swift
  done
done
