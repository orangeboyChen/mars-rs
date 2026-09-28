#!/usr/bin/env bash
#
# Packages the pair of Flutter plugins — platforms/flutter/marsrs and
# platforms/flutter/marsrs-xlog — as
# <asset-dir>/marsrs-flutter-<version>.tar.gz and
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
# The two are the pair platforms/android/ publishes as `marsrs` and `xlog`, and
# they are two directories rather than one shared tree because a plugin that
# took the other's sources would take all of them, which is what the xlog one
# exists not to do. So what there is to stamp, this script stamps twice:
#
#   * `version:` of pubspec.yaml and `s.version` of the podspec — the version of
#     the release;
#   * the AAR coordinate of platforms/android/build.gradle.kts — `marsrs` for
#     the whole port and `xlog` for the xlog half, at the version of this
#     release and not at the one the file was written against;
#   * the newest entry of CHANGELOG.md, which is the version of the release:
#     pub.dev warns about a package whose changelog says nothing about the
#     version it is, and a changelog entry is the one thing here no one can
#     write in advance, because the version is typed when the release is
#     dispatched;
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

# The framework the two plugins carry: the one the apple job built, named by the
# zip it came in. Both are lower-case, the way every archive of this port's own
# is spelled — what is CamelCase is the module inside, `MarsRSXlogFFI`, because
# that is the name an app writes after `import`.
framework_zip="${XCFRAMEWORK_ZIP:-marsrs-xlog.xcframework.zip}"
framework="${framework_zip%.zip}"
zip="$assets/$framework_zip"
test -f "$zip" || { echo "::error::$zip is missing, nothing to package"; exit 1; }

out="$root/dist"
mkdir -p "$out"

# <directory> <package> <AAR> <archive>: the pair, whole port first. The
# directory is the one in the repository and the one in the tarball; the package
# is the name of pubspec.yaml and of the podspec, which is not the same
# spelling — `platforms/flutter/marsrs` publishes `marsrs_flutter`. The only
# thing the AAR argument changes is which AAR the Android half of the plugin
# resolves.
package_one() {
    local pkg="$1" name="$2" aar="$3" archive="$4"
    # The name the tarball holds the plugin under.
    local dir="${pkg##*/}"

    rm -rf "$pkg/ios/Frameworks" "$pkg/ios/include"
    mkdir -p "$pkg/ios/Frameworks" "$pkg/ios/include"
    unzip -q "$zip" -d "$pkg/ios/Frameworks"
    test -d "$pkg/ios/Frameworks/$framework" || { echo "::error::$zip held no $framework"; exit 1; }
    cp crates/marsrs-ffi/include/mars_xlog.h "$pkg/ios/include/"
    # The licence of the port, which the podspec names as `../LICENSE` and
    # pub.dev takes out of the package's root. It is checked in under
    # platforms/flutter/ too — a checkout publishes without this script — so
    # what this copy buys is that the two cannot drift: the one that is packed
    # is always the one at the root of this repository.
    cp LICENSE "$pkg/LICENSE"

    python3 - "$version" "$pkg" "$name" "$aar" <<'PY'
import re
import sys

version, pkg, name, aar = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
prefix = pkg + "/"

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

# The changelog the package carries, whose newest entry has to be the version
# being published: pub.dev warns about one that says nothing about it. So the
# version is stamped here the way pubspec.yaml's is — as an entry of its own
# above the one the tree carries, and not by rewriting that one's heading, which
# would move the entry of a version already published under a new number. The
# entry says what is true of the plugin of every release: it is the natives of
# that version. What changed in the Dart between two of them is in the notes of
# the release, which is what the link is.
path = prefix + "CHANGELOG.md"
src = open(path).read()
heading = "## %s" % version
if not re.search(r"^%s$" % re.escape(heading), src, flags=re.M):
    entry = (
        "%s\n"
        "\n"
        "* The plugin of [mars-rs v%s]: the `%s` AAR and the\n"
        "  MarsRSXlog.xcframework of that release.\n"
        "\n"
        "[mars-rs v%s]: https://github.com/orangeboyChen/mars-rs/releases/tag/v%s\n"
        "\n" % (heading, version, aar, version, version)
    )
    lines = src.splitlines(True)
    at = next((i for i, line in enumerate(lines) if line.startswith("## ")), len(lines))
    lines[at:at] = [entry]
    open(path, "w").write("".join(lines))
PY

    tar -czf "$out/$archive-$version.tar.gz" -C platforms/flutter "$dir"
    ls -l "$out/$archive-$version.tar.gz"
}

package_one platforms/flutter/marsrs marsrs_flutter marsrs marsrs-flutter
package_one platforms/flutter/marsrs-xlog marsrs_flutter_xlog xlog marsrs-flutter-xlog
