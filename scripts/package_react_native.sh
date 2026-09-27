#!/usr/bin/env bash
#
# Packages react-native as <asset-dir>/<name>-<version>.tgz with `npm pack`: the
# module of a release, complete with the two things a consumer cannot resolve for
# itself.
#
#   scripts/package_react_native.sh <version> <asset-dir>
#
# <asset-dir> is where release.yml downloaded the apple job's artifact to — the
# directory MarsRSXlog.xcframework.zip is in. Nothing is built here, and nothing
# Rust is needed: the framework is the apple job's, and the AAR the Android half
# depends on is JitPack's.
#
# What it stamps and what it copies:
#
#   * `version` of package.json, which the podspec reads on its own, so one
#     number is stamped and not two;
#   * the `mars-rs-xlog` coordinate of android/build.gradle.kts — the AAR of
#     this release, and not the one the file was written against;
#   * MarsRSXlog.xcframework into ios/Frameworks, because CocoaPods cannot
#     resolve the SwiftPM binary target of Package.swift;
#   * mars_xlog.h into ios/include, out of crates/mars-ffi/include rather than
#     from a copy kept here, so it cannot drift from the C ABI.

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

framework=MarsRSXlog.xcframework
zip="$assets/$framework.zip"
test -f "$zip" || { echo "::error::$zip is missing, nothing to package"; exit 1; }

out="$root/dist"
pkg="react-native"

case "$out" in
    /*) ;;
    *) out="$root/$out" ;;
esac
mkdir -p "$out"

rm -rf "$pkg/ios/Frameworks" "$pkg/ios/include"
mkdir -p "$pkg/ios/Frameworks" "$pkg/ios/include"
unzip -q "$zip" -d "$pkg/ios/Frameworks"
test -d "$pkg/ios/Frameworks/$framework" || { echo "::error::$zip held no $framework"; exit 1; }
cp crates/mars-ffi/include/mars_xlog.h "$pkg/ios/include/"
# The licence of the port, which package.json's `files` names.
cp LICENSE "$pkg/LICENSE"

python3 - "$version" <<'PY'
import json
import re
import sys

version = sys.argv[1]

# The version of the package: npm's, and the podspec's, which reads this file
# rather than carry a number of its own.
path = "react-native/package.json"
src = open(path).read()
src, n = re.subn(r'"version": "[^"]*"', '"version": "%s"' % version, src, count=1)
assert n == 1, "package.json has no version to stamp"
open(path, "w").write(src)

# The AAR of this release, which the Android half resolves from JitPack.
path = "react-native/android/build.gradle.kts"
src = open(path).read()
src, n = re.subn(r"mars-rs-xlog:[^\"]*", "mars-rs-xlog:%s" % version, src)
assert n == 1, "android/build.gradle.kts has no AAR coordinate to stamp"
open(path, "w").write(src)
PY

cd "$pkg"
name="$(python3 -c 'import json; print(json.load(open("package.json"))["name"])')"
archive="$out/$name-$version.tgz"
rm -f "$archive"
npm pack --pack-destination "$out" > /dev/null
# `npm pack` names the tarball after the name and the version it read, so the
# file being there is the check that the stamp above took.
test -f "$archive" || { echo "::error::npm pack wrote no $archive"; exit 1; }
ls -l "$archive"
