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
#   OHPM_PUBLISH_ID   the publish code ohpm gives the package's publisher
#   OHPM_KEY_PATH     the private half of an SSH keypair whose public half is
#                     uploaded to ohpm — or OHPM_KEY, the key itself, which
#                     this script writes to a file of its own
#
# Both are configured in `~/.ohpmrc` for a machine that publishes by hand, and
# `ohpm publish` takes them as `--publish_id` and `--key_path` for one that does
# not — which is what this script does with them, so a runner needs no
# `.ohpmrc` of its own.
#
# Neither exists until somebody has an ohpm account, so the gate is the
# credential and not the registry: with no publish code there is nothing to
# publish with, and the release is not failed over it. Which is also why the
# first version goes up by hand — an ohpm name is asked for once and kept, and
# a package ohpm has never seen has no publisher and no publish code yet.

set -euo pipefail

version="${1:-}"
if [ -z "$version" ]; then
    echo "usage: $(basename "$0") <version>" >&2
    exit 2
fi
version="${version#v}"

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

har="$root/dist/marsrs-harmonyos-xlog-$version.har"
test -f "$har" || { echo "::error::no $har — run scripts/package_harmony.sh first"; exit 1; }

if [ -z "${OHPM_PUBLISH_ID:-}" ]; then
    echo "::warning::no OHPM_PUBLISH_ID: $har is left unpublished. Publish its first version by hand (ohpm publish $har), then set OHPM_PUBLISH_ID and OHPM_KEY_PATH"
    exit 0
fi
if ! command -v ohpm > /dev/null 2>&1; then
    echo "::warning::ohpm is not on PATH — install the DevEco Command Line Tools, or publish $har by hand"
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
    echo "::warning::no OHPM_KEY_PATH or OHPM_KEY: $har is left unpublished"
    exit 0
fi

echo "publishing $(basename "$har")"
# No `--tag`: a release is a release, and `latest` is what an
# `ohpm install marsrs-harmonyos-xlog` asks for. A version ohpm already has is
# refused by ohpm and not overwritten, so a re-run of a release that is already
# up fails here rather than silently publishing a second copy.
ohpm publish "$har" --publish_id "$OHPM_PUBLISH_ID" --key_path "$key_path"
