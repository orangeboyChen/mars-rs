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
# The directory is a cache and not an install: the run that finds an hvigor in
# it — the same version, from the same URL, pruned the same way, which is what
# the identity file written at the end of this script records — downloads
# nothing, which is the point — scripts/assemble_harmony_har.sh takes
# `DEVECO_HOME` of its own for a DevEco Studio that is already on the machine,
# and CI points that at this directory.
#
# What is unpacked is the whole archive minus the innards of two components of
# the SDK, and it is the same subtraction scripts/build_harmony.sh makes on the
# OHOS SDK:
#
#   sdk/default/*/native      1.5 GB of clang, sysroots and unwinders
#   sdk/default/*/previewer   0.3 GB of the IDE's previewer
#
# The two are the difference between 2.1 GB and 0.4 GB, which is the difference
# between a cache entry that is most of a repository's budget and one that is a
# thirtieth of it. A HAR build of this module runs neither: it has no CMake
# project, because its natives are built by scripts/build_harmony_napi.sh and
# arrive already built, and what hvigor is asked for here is an ArkTS compile.
#
# What is kept of the two is the one file each that names it — `native`'s
# `oh-uni-package.json`, `previewer`'s — because an SDK is complete to hvigor's
# loader component by component, and a component it cannot find at all is
# `SDK component missing` before a task runs, whether or not the build would
# have opened a file inside it. Give this module a CMake project and the whole
# `native` component has to come back; set DEVECO_CLI_PRUNE=0 to keep all of it.

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
#
# Nor is `hvigorw` on its own enough: what is in the directory is a function of
# three things apart from the version — the URL it came from and whether it was
# pruned — and the cache key a job writes is the version. So the identity of
# what was unpacked is written beside `hvigorw` and read back: a directory
# fetched by a job that asked for the pruned tools is a directory that cannot
# answer a job that needs the C++ toolchain, and one fetched from a URL that
# has moved is not the toolchain the checksum was published for, though both
# look like a cache hit to `test -x`.
identity="$dir/command-line-tools/.deveco-cli-identity"
want="$version $url $prune"
if [ -x "$dir/command-line-tools/bin/hvigorw" ] \
    && [ "$(cat "$identity" 2>/dev/null)" = "$want" ]; then
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
        'command-line-tools/sdk/default/hms/native/uni-package.json' \
        'command-line-tools/sdk/default/hms/previewer/uni-package.json' \
        'command-line-tools/sdk/default/openharmony/ets/*' \
        'command-line-tools/sdk/default/openharmony/js/*' \
        'command-line-tools/sdk/default/openharmony/toolchains/*' \
        'command-line-tools/sdk/default/openharmony/native/oh-uni-package.json' \
        'command-line-tools/sdk/default/openharmony/previewer/oh-uni-package.json'
fi
# Larger than what was taken out of it, and a cache entry nobody wants.
rm -f "$zip"

test -x "$dir/command-line-tools/bin/hvigorw" || {
    echo "::error::no hvigorw under $dir — the archive is not the one this script expects"
    exit 1
}
# Written last, and only for a directory that has an hvigor in it: a run that
# died between `unzip` and here leaves no identity behind, so the next one
# downloads again instead of trusting a half-unpacked tree.
printf '%s\n' "$want" > "$identity"
du -sh "$dir/command-line-tools"
