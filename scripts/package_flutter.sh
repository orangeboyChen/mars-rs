#!/usr/bin/env bash
#
# Packages flutter/mars_rs_xlog as <asset-dir>/mars-rs-flutter-<version>.tar.gz:
# the plugin of a release, complete with the two things a consumer cannot
# resolve for itself.
#
#   scripts/package_flutter.sh <version> <asset-dir>
#
# <asset-dir> is where release.yml downloaded the apple job's artifact to — the
# directory MarsRSXlog.xcframework.zip is in. Nothing is built here, and nothing
# Rust is needed: the framework is the apple job's, and the AAR the Android half
# depends on is JitPack's.
#
# What it stamps and what it copies:
#
#   * `version:` of pubspec.yaml and `s.version` of the podspec — the version of
#     the release;
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
pkg="flutter/mars_rs_xlog"
mkdir -p "$out"

rm -rf "$pkg/ios/Frameworks" "$pkg/ios/include"
mkdir -p "$pkg/ios/Frameworks" "$pkg/ios/include"
unzip -q "$zip" -d "$pkg/ios/Frameworks"
test -d "$pkg/ios/Frameworks/$framework" || { echo "::error::$zip held no $framework"; exit 1; }
cp crates/mars-ffi/include/mars_xlog.h "$pkg/ios/include/"
# The licence of the port, which the podspec names.
cp LICENSE "$pkg/LICENSE"

python3 - "$version" <<'PY'
import re
import sys

version = sys.argv[1]
prefix = "flutter/mars_rs_xlog/"

# The version of the package itself: pub's, and the podspec's, which CocoaPods
# reads on its own because there is no pub to ask.
path = prefix + "pubspec.yaml"
src = open(path).read()
src, n = re.subn(r"^version: .*$", "version: %s" % version, src, count=1, flags=re.M)
assert n == 1, "pubspec.yaml has no version to stamp"
open(path, "w").write(src)

path = prefix + "ios/mars_rs_xlog.podspec"
src = open(path).read()
src, n = re.subn(r"s\.version\s+= '[^']*'", "s.version          = '%s'" % version, src, count=1)
assert n == 1, "the podspec has no version to stamp"
open(path, "w").write(src)

# The AAR of this release, which the Android half resolves from JitPack.
path = prefix + "android/build.gradle.kts"
src = open(path).read()
src, n = re.subn(r"mars-rs-xlog:[^\"]*", "mars-rs-xlog:%s" % version, src)
assert n == 1, "android/build.gradle.kts has no AAR coordinate to stamp"
open(path, "w").write(src)
PY

archive="$out/mars-rs-flutter-$version.tar.gz"
rm -f "$archive"
tar -czf "$archive" -C flutter mars_rs_xlog
ls -l "$archive"
