#!/usr/bin/env bash
#
# Fetches the DevEco Command Line Tools — `hvigorw`, which is the only ArkTS
# compiler this repository can run — into a directory that survives between
# runs: the archive is 2.1 GB on a mirror whose fast day and slow day are a
# minute and ten apart, and what is taken out of it is a third of a gigabyte.
#
#   scripts/fetch_deveco_cli.sh [dir]        (default: <repo>/.deveco-cli)
#
# Environment:
#   DEVECO_CLI_VERSION   the version, which is also the cache key
#   DEVECO_CLI_URL       the archive, on the Huawei Cloud mirror
#   DEVECO_CLI_SHA256    its checksum
#   DEVECO_CLI_PRUNE     set to 0 to unpack all of it
#
# The directory is a cache and not an install: the run that finds
# `command-line-tools/bin/hvigorw` in it downloads nothing, which is the point —
# scripts/assemble_harmony_har.sh takes `DEVECO_HOME` of its own for a DevEco
# Studio that is already on the machine, and CI points that at this directory.
#
# What is unpacked is the whole archive minus two components of the SDK, and it
# is the same subtraction scripts/build_harmony.sh makes on the OHOS SDK:
#
#   sdk/default/*/native      1.5 GB of clang, sysroots and unwinders, which
#                             build a C++ CMake project — and this module has
#                             no CMake project: its natives are built by
#                             scripts/build_harmony_napi.sh and arrive in the
#                             module already built
#   sdk/default/*/previewer   0.3 GB of the IDE's previewer
#
# The two are the difference between 2.1 GB and 0.4 GB, which is the difference
# between a cache entry that is most of a repository's budget and one that is a
# thirtieth of it. Set DEVECO_CLI_PRUNE=0 to keep them.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

version="${DEVECO_CLI_VERSION:-5.1.0.840}"
url="${DEVECO_CLI_URL:-https://repo.huaweicloud.com/openharmony/ohpm/5.1.0/commandline-tools-linux-x64-${version}.zip}"
sha256="${DEVECO_CLI_SHA256:-}"
prune="${DEVECO_CLI_PRUNE:-1}"

dir="${1:-$root/.deveco-cli}"
case "$dir" in
    /*) ;;
    *) dir="$root/$dir" ;;
esac

# The cache's own test, and not `test -d`: a run interrupted between `mkdir`
# and `unzip` leaves a directory that has no hvigor in it, and a download
# skipped because of it is a build that cannot start.
if [ -x "$dir/command-line-tools/bin/hvigorw" ]; then
    echo "the DevEco Command Line Tools $version are already in $dir"
    exit 0
fi

command -v unzip > /dev/null || { echo "::error::unzip is needed to unpack the DevEco Command Line Tools"; exit 1; }

mkdir -p "$dir"
zip="$dir/commandline-tools-linux-x64-$version.zip"
echo "downloading the DevEco Command Line Tools $version"
curl -sSfL -o "$zip" "$url"

if [ -n "$sha256" ]; then
    # Pinned twice: the version is in the URL and the archive is checked against
    # the hash the mirror published beside it, so a re-upload under the same
    # name is a failed run and not a silent change of toolchain.
    echo "$sha256  $zip" | sha256sum -c -
else
    echo "::warning::no DEVECO_CLI_SHA256: $zip is unpacked unchecked"
fi

if [ "$prune" = 0 ]; then
    unzip -q "$zip" -d "$dir"
else
    # `unzip -d` creates one directory and not a path, which is why every
    # pattern below names the levels it wants and not a prefix of them.
    unzip -q "$zip" -d "$dir" \
        'command-line-tools/version.txt' \
        'command-line-tools/bin/*' \
        'command-line-tools/hvigor/*' \
        'command-line-tools/ohpm/*' \
        'command-line-tools/codelinter/*' \
        'command-line-tools/hstack/*' \
        'command-line-tools/tool/*' \
        'command-line-tools/sdk/default/sdk-pkg.json' \
        'command-line-tools/sdk/default/hms/ets/*' \
        'command-line-tools/sdk/default/hms/toolchains/*' \
        'command-line-tools/sdk/default/openharmony/ets/*' \
        'command-line-tools/sdk/default/openharmony/js/*' \
        'command-line-tools/sdk/default/openharmony/toolchains/*'
fi
# Larger than what was taken out of it, and a cache entry nobody wants.
rm -f "$zip"

test -x "$dir/command-line-tools/bin/hvigorw" || {
    echo "::error::no hvigorw under $dir — the archive is not the one this script expects"
    exit 1
}
du -sh "$dir/command-line-tools"
