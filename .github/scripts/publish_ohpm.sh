#!/usr/bin/env bash
#
# Publishes the HarmonyOS package to ohpm: the HAR
# scripts/package_harmony.sh wrote, under the `name` of its own
# oh-package.json5 — `marsrs-harmonyos-xlog`, which is also the name of the
# archive.
#
#   .github/scripts/publish_ohpm.sh <version>
#
# ohpm is npm's shape but not npm's trust model: there is no OIDC and no
# trusted publisher, so unlike .github/scripts/publish_npm.sh this one needs a
# credential in the environment, and it is a credential made of two halves:
#
#   OHOS_PUBLIC_TOKEN  the publish code — ohpm's `publish_id` — that it gives
#                      the publisher of the package, and the name the
#                      repository's secret has. OHPM_PUBLISH_ID is read too,
#                      for a hand run that sets it in the environment instead.
#   OHPM_KEY_PATH      the private half of an SSH keypair whose public half is
#                      uploaded to ohpm — or OHPM_KEY, the key itself, which
#                      this script writes to a file of its own
#
# Both are configured in `~/.ohpmrc` for a machine that publishes by hand, and
# `ohpm publish` takes them as `--publish_id` and `--key_path` for one that does
# not — which is what this script does with them, so a runner needs no
# `.ohpmrc` of its own.
#
# Neither exists until somebody has an ohpm account, so the gate is the
# credential and not the registry: with no publish code there is nothing to
# publish with, and the release is not failed over it. What it is not is green:
# a missing credential is `::error::` and `published=false` now, because a
# release that went out with five of its nine crates missing and said nothing
# is what a warning buys. Which is also why the first version goes up by hand
# — an ohpm name is asked for once and kept, and a package ohpm has never seen
# has no publisher and no publish code yet.

set -euo pipefail

# Whether this script put the HAR on ohpm: what a Summary step of the job reads
# before it names a version.
published() {
    printf 'published=%s\n' "$1" >> "${GITHUB_OUTPUT:-/dev/null}"
}

version="${1:-}"
if [ -z "$version" ]; then
    echo "usage: $(basename "$0") <version>" >&2
    exit 2
fi
version="${version#v}"

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

# The name the archive is published under, read out of the module's own manifest
# the way scripts/package_harmony.sh derives it, and not hardcoded: the two are
# one name written in one place, and a rename of the package is a rename in the
# manifest and not in this script.
name="$(python3 -c 'import re,sys; print(re.search(r"\"name\": \"([^\"]*)\"", open(sys.argv[1]).read()).group(1))' \
    "$root/platforms/harmonyos/marsrs-xlog/oh-package.json5")"
test -n "$name" || { echo "::error::platforms/harmonyos/marsrs-xlog/oh-package.json5 names no package"; exit 1; }

har="$root/dist/$name-$version.har"
test -f "$har" || { echo "::error::no $har — run scripts/package_harmony.sh first"; exit 1; }

# The publish code: OHOS_PUBLIC_TOKEN is the secret the repository carries, and
# OHPM_PUBLISH_ID is the same number under the name it is documented by.
publish_id="${OHOS_PUBLIC_TOKEN:-${OHPM_PUBLISH_ID:-}}"
if [ -z "$publish_id" ]; then
    echo "::error::no OHOS_PUBLIC_TOKEN: $har is left unpublished. Publish its first version by hand (ohpm publish $har), then set OHOS_PUBLIC_TOKEN and OHPM_KEY"
    published false
    exit 0
fi
if ! command -v ohpm > /dev/null 2>&1; then
    echo "::error::ohpm is not on PATH — install the DevEco Command Line Tools, or publish $har by hand"
    published false
    exit 0
fi

# The key: either a path to one, or the key itself in `OHPM_KEY`, which is what
# a repository secret can carry and a path cannot. 0600 because ssh and ohpm
# both refuse a private key anyone else can read.
key_path="${OHPM_KEY_PATH:-}"
if [ -z "$key_path" ] && [ -n "${OHPM_KEY:-}" ]; then
    key_path="$(mktemp)"
    printf '%s\n' "$OHPM_KEY" > "$key_path"
    chmod 600 "$key_path"
    trap 'rm -f "$key_path"' EXIT
fi
if [ -z "$key_path" ]; then
    echo "::error::no OHPM_KEY_PATH or OHPM_KEY: $har is left unpublished"
    published false
    exit 0
fi

echo "publishing $(basename "$har")"
# No `--tag`: a release is a release, and `latest` is what an
# `ohpm install marsrs-harmonyos-xlog` asks for. A version ohpm already has is
# refused by ohpm and not overwritten, so a re-run of a release that is already
# up fails here rather than silently publishing a second copy.
ohpm publish "$har" --publish_id "$publish_id" --key_path "$key_path"

published true
