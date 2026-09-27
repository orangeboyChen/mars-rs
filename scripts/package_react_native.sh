#!/usr/bin/env bash
#
# Packages the pair of React Native modules — react-native/marsrs-react-native
# and react-native/marsrs-react-native-xlog — with `npm pack`, as
# <asset-dir>/<name>-<version>.tgz each: the modules of a release, complete with
# the two things a consumer cannot resolve for itself.
#
#   scripts/package_react_native.sh <version> <asset-dir>
#
# <asset-dir> is where release.yml downloaded the apple job's artifact to — the
# directory MarsRSXlog.xcframework.zip is in. Nothing is built here, and nothing
# Rust is needed: the framework is the apple job's, and the AAR the Android half
# of each module depends on is JitPack's.
#
# The two are the pair android/ publishes as `marsrs` and `xlog`, and
# they are two directories rather than one shared tree because a module that
# took the other's sources would take all of them, which is what the xlog one
# exists not to do. So what there is to stamp, this script stamps twice:
#
#   * `version` of package.json, which the podspec reads on its own, so one
#     number is stamped and not two;
#   * the AAR coordinate of android/build.gradle.kts — `marsrs` for the whole
#     port and `xlog` for the xlog half, at the version of this release
#     and not at the one the file was written against;
#   * MarsRSXlog.xcframework into ios/Frameworks, because CocoaPods cannot
#     resolve the SwiftPM binary target of Package.swift;
#   * mars_xlog.h into ios/include, out of crates/marsrs-ffi/include rather than
#     from a copy kept here, so it cannot drift from the C ABI.
#
# `npm pack` names each tarball after the `name` and `version` it read, which is
# why the name of a package is also the name of its asset.

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
case "$out" in
    /*) ;;
    *) out="$root/$out" ;;
esac
mkdir -p "$out"

# <directory> <AAR>: the pair, whole port first. The only thing either argument
# changes is which AAR the Android half of the module resolves; the tarball is
# named by `npm pack`, after the name of package.json.
package_one() {
    local pkg="$1" aar="$2"
    local name="${pkg##*/}"

    rm -rf "$pkg/ios/Frameworks" "$pkg/ios/include"
    mkdir -p "$pkg/ios/Frameworks" "$pkg/ios/include"
    unzip -q "$zip" -d "$pkg/ios/Frameworks"
    test -d "$pkg/ios/Frameworks/$framework" || { echo "::error::$zip held no $framework"; exit 1; }
    cp crates/marsrs-ffi/include/mars_xlog.h "$pkg/ios/include/"
    # The licence of the port, which package.json's `files` names.
    cp LICENSE "$pkg/LICENSE"

    python3 - "$version" "$pkg" "$aar" <<'PY'
import re
import sys

version, pkg, aar = sys.argv[1], sys.argv[2], sys.argv[3]
prefix = pkg + "/"

# The version of the package: npm's, and the podspec's, which reads this file
# rather than carry a number of its own.
path = prefix + "package.json"
src = open(path).read()
src, n = re.subn(r'"version": "[^"]*"', '"version": "%s"' % version, src, count=1)
assert n == 1, "package.json has no version to stamp"
open(path, "w").write(src)

# The AAR of this release, which the Android half resolves from JitPack.
path = prefix + "android/build.gradle.kts"
src = open(path).read()
src, n = re.subn(
    r'"io\.github\.orangeboychen\.marsrs:(?:marsrs|xlog):[^"]*"',
    '"io.github.orangeboychen.marsrs:%s:%s"' % (aar, version),
    src,
    count=1,
)
assert n == 1, "android/build.gradle.kts has no AAR coordinate to stamp"
open(path, "w").write(src)
PY

    (cd "$pkg" && npm pack --pack-destination "$out" > /dev/null)
    # `npm pack` names the tarball after the name and the version it read, so
    # the file being there is the check that the stamp above took.
    local archive="$out/$name-$version.tgz"
    test -f "$archive" || { echo "::error::npm pack wrote no $archive"; exit 1; }
    ls -l "$archive"
}

package_one react-native/marsrs-react-native marsrs
package_one react-native/marsrs-react-native-xlog xlog
