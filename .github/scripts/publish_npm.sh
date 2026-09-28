#!/usr/bin/env bash
#
# Publishes the two React Native modules to npm, at the version of the release:
# `platforms/react-native/marsrs` and `platforms/react-native/marsrs-xlog`,
# under the `name` of their own package.json — `marsrs-react-native` and
# `marsrs-react-native-xlog`, which is also the name
# scripts/package_react_native.sh's tarballs carry.
#
#   .github/scripts/publish_npm.sh <version>
#
# No credential: the job asks for `id-token: write`, and npm's trusted
# publishing exchanges this run's OIDC token for a token of the registry's own.
# What npm does need is this repository named as a trusted publisher of the
# package — owner, repository, and the file name of the workflow that publishes,
# `release.yml` — which is set on npmjs.com and not in this repository. That is
# also the one thing that has to come first: a package npm has never seen has no
# settings to put a trusted publisher in, so its first version is published by
# hand, once, and this script says so instead of failing a release over it.
#
# Published out of the directories the packaging stamped, and not out of the
# tarballs: what goes up is what the release carries — the version of
# package.json, the xcframework the podspec names and the AAR coordinate of this
# release — and npm reads `files` out of package.json either way.

set -euo pipefail

version="${1:-}"
if [ -z "$version" ]; then
    echo "usage: $(basename "$0") <version>" >&2
    exit 2
fi
version="${version#v}"

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

for pkg in platforms/react-native/marsrs platforms/react-native/marsrs-xlog; do
    name="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["name"])' "$pkg/package.json")"

    # The two questions the crates.io step asks crates.io. The first is the one
    # a trusted publisher cannot answer for: a package npm does not have is a
    # package that has to be published by hand first.
    if ! npm view "$name" version > /dev/null 2>&1; then
        echo "::warning::npm has no $name yet; publish its first version by hand, then name this repository as its trusted publisher"
        continue
    fi
    if npm view "$name@$version" version > /dev/null 2>&1; then
        echo "$name $version is already on npm"
        continue
    fi

    echo "publishing $name $version"
    # `--provenance`: the registry records what built the tarball it was given.
    # No `--access`: an unscoped package is public, and npm says so.
    (cd "$pkg" && npm publish --provenance)
done
