#!/usr/bin/env bash
# Compiles the Swift of `demo/apple` for iOS, and links nothing.
#
#   .github/scripts/typecheck_demo_swift.sh
#
# The Apple demo is sources and not a project — an iOS app is a bundle, and a
# bundle is not a thing a repository can carry as text — so nothing in this
# repository ever compiled it. What that left unanswered is the same question
# the release used to be the first to answer about `platforms/apple`: a symbol
# the app calls and the port no longer exports is a demo a reader finds broken,
# not a red job.
#
# How it is compiled is how typecheck_swift.sh compiles the port's own Swift,
# and for the same reason: `MarsRSXlog` is a binary target of Package.swift
# holding a prebuilt xcframework, which no macOS host links. A module map over
# `mars_xlog.h` stands in for it — a C module is all the Swift takes from the
# framework — and the port's Swift is emitted first, because `LogStore.swift`
# imports `MarsRSXlog` and what it imports has to exist before the demo is
# compiled against it.
#
# A module is emitted rather than the sources being `-typecheck`ed, for the
# reason typecheck_swift.sh gives: a type-check stops before SILGen, and SILGen
# is where a use of `self` before the `super.init()` Swift writes for an
# `NSObject` subclass is caught.
#
# Two destinations and not one. A `#if targetEnvironment(simulator)`, an
# `#if arch(x86_64)` or a symbol the two SDKs answer differently is compiled by
# one triple and by nothing else; arm64 is the architecture an iOS app is built
# for on both.
#
# The deployment target is the demo's own and not the package's: `MarsRSXlog`
# compiles for iOS 12, but this app is SwiftUI, which starts at iOS 13, and
# 17.0 is below every SDK a current Xcode carries.

set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The C module, out of the header of the crate that generates it.
mkdir -p "$work/module/MarsRSFFI"
cp "$repo/crates/marsrs-ffi/include/mars_xlog.h" "$work/module/MarsRSFFI/"
cat > "$work/module/MarsRSFFI/module.modulemap" <<'MAP'
module MarsRSFFI {
    header "mars_xlog.h"
    export *
}
MAP

platforms=(
  "iphoneos:arm64-apple-ios17.0"
  "iphonesimulator:arm64-apple-ios17.0-simulator"
)

for platform in "${platforms[@]}"; do
  sdk="${platform%%:*}"
  triple="${platform##*:}"
  sdk_path="$(xcrun --sdk "$sdk" --show-sdk-path)"
  built="$work/$triple"
  mkdir -p "$built"

  # `-I "$built"`: the module compiled just above, which is what the demo's
  # `import MarsRSXlog` resolves against.
  echo "== MarsRSXlog ($triple)"
  xcrun swiftc -emit-module \
    -module-name MarsRSXlog \
    -sdk "$sdk_path" \
    -target "$triple" \
    -I "$work/module" \
    -o "$built/MarsRSXlog.swiftmodule" \
    "$repo/platforms/apple/MarsRSXlog"/*.swift

  echo "== demo/apple ($triple)"
  xcrun swiftc -emit-module \
    -module-name MarsRSDemo \
    -sdk "$sdk_path" \
    -target "$triple" \
    -I "$work/module" -I "$built" \
    -o "$built/MarsRSDemo.swiftmodule" \
    "$repo/demo/apple/Sources"/*.swift
done
