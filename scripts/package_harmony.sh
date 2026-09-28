#!/usr/bin/env bash
#
# Packages platforms/harmonyos/marsrs-xlog as the HAR ohpm publishes:
# <asset-dir>/marsrs-harmonyos-xlog-<version>.har, a release asset beside the
# other platforms'.
#
#   scripts/package_harmony.sh <version> <asset-dir> [napi-dir]
#
# Nothing is built here. `libmarsrs_xlog.so` — one per ABI under
# <napi-dir>/<abi>/, which defaults to what scripts/build_harmony_napi.sh
# wrote — is dropped into `libs/<abi>/`: the directory a HarmonyOS module keeps
# its prebuilt natives in, and the one hvigor packs a HAR out of. An app that
# takes the package compiles ArkTS and resolves nothing else, because the
# staticlib of the C ABI is linked into that `.so` and not shipped beside it.
#
# What this stamps is the version of `oh-package.json5`, which is the number
# ohpm reads and the number the archive is named after.
#
# hvigor is not run, and that is a choice with one consequence worth knowing:
# a HAR DevEco builds carries `compatibleSdkVersion`, `compatibleSdkType`,
# `obfuscated` and `nativeComponents`, which hvigor fills in from the SDK it
# built with, and `ohpm publish` looks for them. Assembled by hand they are
# absent. `compability_log_level` in `.ohpmrc` decides whether that is a
# warning or an error, and the first publish — which is by hand, see
# .github/scripts/publish_ohpm.sh — is where to find out.
#
# To have DevEco build the archive instead: open this module in a DevEco
# project and run `hvigorw --mode module -p product=default assembleHar`. The
# tree it builds is the tree staged here, `libs/` included.

set -euo pipefail

version="${1:-}"
assets="${2:-}"
napi="${3:-}"

if [ -z "$version" ] || [ -z "$assets" ]; then
    echo "usage: $(basename "$0") <version> <asset-dir> [napi-dir]" >&2
    exit 2
fi
version="${version#v}"

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

case "$assets" in
    /*) ;;
    *) assets="$root/$assets" ;;
esac
mkdir -p "$assets"

module="$root/platforms/harmonyos/marsrs-xlog"
napi="${napi:-$root/target/harmony-napi}"
case "$napi" in
    /*) ;;
    *) napi="$root/$napi" ;;
esac

# The three ABIs the package carries, in the spelling a HarmonyOS module uses
# and not the one Rust spells the target with — the same three
# scripts/build_harmony_napi.sh builds, and the same mismatch of names.
abis=(arm64-v8a armeabi-v7a x86_64)

for abi in "${abis[@]}"; do
    test -f "$napi/$abi/libmarsrs_xlog.so" || {
        echo "::error::no $napi/$abi/libmarsrs_xlog.so — run scripts/build_harmony_napi.sh first"
        exit 1
    }
done

name="$(python3 -c 'import re,sys; print(re.search(r"\"name\": \"([^\"]*)\"", open(sys.argv[1]).read()).group(1))' "$module/oh-package.json5")"
test -n "$name" || { echo "::error::$module/oh-package.json5 names no package"; exit 1; }

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
package="$stage/package"
mkdir -p "$package"

# What a HAR of this module is: the entry point and the two manifests at the
# top, `src/` under them, and the natives in `libs/`. `README.md` and
# `CHANGELOG.md` are ohpm's own requirement of a package, which is why they are
# copied and not merely present in the repository — the README of the module is
# the page ohpm renders.
cp "$module/Index.ets" \
   "$module/oh-package.json5" \
   "$module/build-profile.json5" \
   "$module/hvigorfile.ts" \
   "$module/README.md" \
   "$module/CHANGELOG.md" \
   "$package/"
cp -R "$module/src" "$package/src"
cp LICENSE "$package/LICENSE"

for abi in "${abis[@]}"; do
    mkdir -p "$package/libs/$abi"
    cp "$napi/$abi/libmarsrs_xlog.so" "$package/libs/$abi/"
done

python3 - "$version" "$package/oh-package.json5" <<'PY'
import re
import sys

version, path = sys.argv[1], sys.argv[2]
src = open(path).read()
src, n = re.subn(r'"version": "[^"]*"', '"version": "%s"' % version, src, count=1)
assert n == 1, "oh-package.json5 has no version to stamp"
open(path, "w").write(src)
PY

# A HAR is an archive of that tree and nothing else — no directory holding it,
# `oh-package.json5` at the root — which is what makes it the same shape as the
# one `hvigorw assembleHar` writes.
archive="$assets/$name-$version.har"
tar -czf "$archive" -C "$package" .

test -f "$archive" || { echo "::error::no $archive was written"; exit 1; }
ls -l "$archive"
tar -tzf "$archive" | grep -E '\.so$|oh-package\.json5$' | sort
