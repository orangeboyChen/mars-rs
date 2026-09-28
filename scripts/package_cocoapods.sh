#!/usr/bin/env bash
#
# Packages the two CocoaPods pods that carry a binary — MarsRSXlog and MarsRSNet
# — as <asset-dir>/marsrs-cocoapods-<half>-<version>.zip, which is what
# `pod install` downloads for them.
#
#   scripts/package_cocoapods.sh <version> <asset-dir>
#
# <asset-dir> is where release.yml downloaded the apple job's artifact to — the
# directory marsrs-xlog.xcframework.zip and marsrs-net.xcframework.zip are in.
# Nothing is built here and no Rust toolchain is needed: the frameworks are the
# apple job's, and the Swift is the port's own, taken out of Sources/ rather
# than compiled — the app's Xcode is what compiles it.
#
# Why a pod is a zip of a framework *and* the sources, when the Swift package
# takes the first out of a url and the second out of a git tag: a pod has one
# `source`, and it is the only thing `pod install` fetches. SwiftPM reads a
# package's manifest from a tag and its binary targets from urls, so the two can
# come from two places; CocoaPods reads a pod's files from its source and from
# nowhere else, so what a pod is made of has to travel in one archive. The
# framework inside is the release's own — the same file Package.swift names, down
# to the checksum — and beside it goes the Swift of the module the pod is.
#
# What is not packaged here is the third pod: `MarsRS` carries no framework of
# its own, so its source is the git tag and not an archive of this release.

set -euo pipefail

version="${1:-}"
assets="${2:-}"
if [ -z "$version" ] || [ -z "$assets" ]; then
    echo "usage: $(basename "$0") <version> <asset-dir>" >&2
    exit 2
fi
version="${version#v}"

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

# A relative asset dir has to be resolved against the workspace before anything
# else moves the cwd.
case "$assets" in
    /*) ;;
    *) assets="$root/$assets" ;;
esac

out="$root/dist"
mkdir -p "$out"

# One entry per pod that carries a binary: `<xcframework>:<swift module>` — the
# artefact the release built, named the way the crates are, and the module of
# Sources/ whose Swift goes into the archive beside it.
pods=(
    marsrs-xlog.xcframework:MarsRSXlog
    marsrs-net.xcframework:MarsRSNet
)

package_one() {
    local name="$1" module="$2"
    local zip="$assets/$name.zip"
    local half="${name%.xcframework}"
    half="${half#marsrs-}"
    local archive="$out/marsrs-cocoapods-$half-$version.zip"
    local stage="$root/target/cocoapods/$half"

    test -f "$zip" || { echo "::error::$zip is missing, nothing to package"; exit 1; }

    rm -rf "$stage"
    mkdir -p "$stage/Sources/$module"
    unzip -q "$zip" -d "$stage"
    test -d "$stage/$name" || { echo "::error::$zip held no $name"; exit 1; }
    cp Sources/"$module"/*.swift "$stage/Sources/$module/"
    # The licence of the port, which `s.license` of every podspec names.
    cp LICENSE "$stage/LICENSE"

    # Flat, and not under a directory of the version's: CocoaPods extracts an
    # archive and takes its root as the pod's, which is where the podspec's
    # `vendored_frameworks` and `source_files` look for what they name.
    (cd "$stage" && zip -q -r "$archive" "$name" Sources LICENSE)
    test -f "$archive" || { echo "::error::no $archive was written"; exit 1; }
    ls -l "$archive"
}

for pod in "${pods[@]}"; do
    IFS=':' read -r name module <<< "$pod"
    package_one "$name" "$module"
done
