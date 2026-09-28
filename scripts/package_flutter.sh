#!/usr/bin/env bash
#
# Packages the pair of Flutter plugins — flutter/marsrs_flutter and
# flutter/marsrs_flutter_xlog — as <asset-dir>/marsrs-flutter-<version>.tar.gz and
# <asset-dir>/marsrs-flutter-xlog-<version>.tar.gz: the plugins of a release,
# complete with the two things a consumer cannot resolve for itself.
#
#   scripts/package_flutter.sh <version> <asset-dir>
#
# <asset-dir> is where release.yml downloaded the apple job's artifact to — the
# directory MarsRSXlog.xcframework.zip is in. Nothing is built here, and nothing
# Rust is needed: the framework is the apple job's, and the AAR the Android half
# of each plugin depends on is JitPack's.
#
# The two are the pair android/ publishes as `marsrs` and `xlog`, and
# they are two directories rather than one shared tree because a plugin that
# took the other's sources would take all of them, which is what the xlog one
# exists not to do. So what there is to stamp, this script stamps twice:
#
#   * `version:` of pubspec.yaml and `s.version` of the podspec — the version of
#     the release;
#   * the AAR coordinate of android/build.gradle.kts — `marsrs` for the whole
#     port and `xlog` for the xlog half, at the version of this release
#     and not at the one the file was written against;
#   * MarsRSXlog.xcframework into ios/Frameworks, because CocoaPods cannot
#     resolve the SwiftPM binary target of Package.swift;
#   * mars_xlog.h into ios/include, out of crates/marsrs-ffi/include rather than
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
mkdir -p "$out"

# <directory> <AAR> <archive>: the pair, whole port first. The only thing either
# argument changes is which AAR the Android half of the plugin resolves.
package_one() {
    local pkg="$1" aar="$2" archive="$3"
    # The plugin's own directory, and the name the archive holds it under.
    local name="${pkg##*/}"

    rm -rf "$pkg/ios/Frameworks" "$pkg/ios/include"
    mkdir -p "$pkg/ios/Frameworks" "$pkg/ios/include"
    unzip -q "$zip" -d "$pkg/ios/Frameworks"
    test -d "$pkg/ios/Frameworks/$framework" || { echo "::error::$zip held no $framework"; exit 1; }
    cp crates/marsrs-ffi/include/mars_xlog.h "$pkg/ios/include/"
    # The licence of the port, which the podspec names.
    cp LICENSE "$pkg/LICENSE"

    python3 - "$version" "$pkg" "$aar" <<'PY'
import re
import sys

version, pkg, aar = sys.argv[1], sys.argv[2], sys.argv[3]
prefix = pkg + "/"
name = pkg.rsplit("/", 1)[-1]

# The version of the package itself: pub's, and the podspec's, which CocoaPods
# reads on its own because there is no pub to ask.
path = prefix + "pubspec.yaml"
src = open(path).read()
src, n = re.subn(r"^version: .*$", "version: %s" % version, src, count=1, flags=re.M)
assert n == 1, "pubspec.yaml has no version to stamp"
open(path, "w").write(src)

path = prefix + "ios/%s.podspec" % name
src = open(path).read()
src, n = re.subn(r"s\.version\s+= '[^']*'", "s.version          = '%s'" % version, src, count=1)
assert n == 1, "the podspec has no version to stamp"
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

    tar -czf "$out/$archive-$version.tar.gz" -C flutter "$name"
    ls -l "$out/$archive-$version.tar.gz"
}

package_one flutter/marsrs_flutter marsrs marsrs-flutter
package_one flutter/marsrs_flutter_xlog xlog marsrs-flutter-xlog
