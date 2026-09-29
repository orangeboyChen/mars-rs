#!/usr/bin/env bash
#
# Publishes the two React Native modules to npm, at the version of the release:
# `platforms/react-native/marsrs` and `platforms/react-native/marsrs-xlog`,
# under the `name` of their own package.json — `marsrs-react-native` and
# `marsrs-react-native-xlog`, which is also the name
# scripts/package_react_native.sh's tarballs carry.
#
#   .github/scripts/publish_npm.sh <version> [<dir>...]
#
# No credential: the job asks for `id-token: write`, and npm's trusted
# publishing exchanges this run's OIDC token for a token of the registry's own.
# What npm does need is this repository named as a trusted publisher of the
# package — owner, repository, and the file name of the workflow that publishes,
# `release.yml` — which is set on npmjs.com and not in this repository. That is
# also the one thing that has to come first: a package npm has never seen has no
# settings to put a trusted publisher in, so its first version is published by
# hand, once, and this script says so instead of failing a release over it.
# What it says it with is `::error::` and not a warning: a green job that
# published nothing is a release that looks complete and is not, which is how a
# release can go out without the crates anybody could have seen were missing.
#
# Published out of the directories the packaging stamped, and not out of the
# tarballs: what goes up is what the release carries — the version of
# package.json, the xcframework the podspec names and the AAR coordinate of this
# release — and npm reads `files` out of package.json either way.
#
# A `<dir>` is asked for by publish.yml, which publishes a release that is
# already out: it takes the two `npm pack` tarballs off the release and unpacks
# them, so what it hands here is the module that tarball held and not the tree
# of a checkout. With no argument the pair of the tree is published, which is
# what release.yml wants — it has just stamped them itself.

set -euo pipefail

version="${1:-}"
if [ -z "$version" ]; then
    echo "usage: $(basename "$0") <version> [<dir>...]" >&2
    exit 2
fi
# The version is not a directory, and what is left is the pair to publish.
shift
version="${version#v}"

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

# Whether this script put the modules on npm: what a Summary step of the job
# reads before it names a version. `false` on every path that leaves one
# unpublished, so a summary can never quote a version npm does not have.
published() {
    printf 'published=%s\n' "$1" >> "${GITHUB_OUTPUT:-/dev/null}"
}

# The pair of the tree, unless the caller named the directories to publish.
if [ "$#" -gt 0 ]; then
    pkgs=("$@")
else
    pkgs=(platforms/react-native/marsrs platforms/react-native/marsrs-xlog)
fi

every_published=true

# The status of a package or of a version of one on npm: 200 is "npm has it",
# 404 is "it does not", and both are answers about the package. Read off the
# registry and not out of the exit of an `npm view`, which says the same thing
# about a registry it never reached as about a package that is not there — and
# what the second of those does to a run is skip both modules with a warning,
# which is a release that published nothing of either and ended green.
registry_status() {
    curl -sS -o /dev/null -w '%{http_code}' --max-time 30 \
        "https://registry.npmjs.org/$1"
}

for pkg in "${pkgs[@]}"; do
    name="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["name"])' "$pkg/package.json")"

    # The two questions the crates.io step asks crates.io. The first is the one
    # a trusted publisher cannot answer for: a package npm does not have is a
    # package that has to be published by hand first.
    status="$(registry_status "$name")"
    case "$status" in
        404)
            echo "::error::npm has no $name yet; publish its first version by hand, then name this repository as its trusted publisher"
            every_published=false
            continue
            ;;
        200)
            ;;
        *)
            echo "::error::the npm registry answered $status for $name; whether it is there is unknown, so it is not published to"
            exit 1
            ;;
    esac

    status="$(registry_status "$name/$version")"
    case "$status" in
        200)
            echo "$name $version is already on npm"
            continue
            ;;
        404)
            ;;
        *)
            echo "::error::the npm registry answered $status for $name $version; whether it is up is unknown, so it is not published over"
            exit 1
            ;;
    esac

    # The dist-tag, named only for a pre-release: `alpha` for 0.1.0-alpha.2,
    # `beta` for 0.1.0-beta.10. npm asks for one then — under the tag it
    # defaults to it refuses the publish outright, "You must specify a tag
    # using --tag when publishing a prerelease version" — and a tag that is
    # named is the only tag the publish moves, so `latest` stays where the
    # last stable version left it, which is what the `npm install <name>` of
    # the docs and of the README installs.
    #
    # A stable version is published with the tag left implicit: that is what
    # keeps npm's own check on `latest`, the one that refuses a version lower
    # than a version already published — a 1.5.1 asked for after a 2.0.0 —
    # and it is a check npm makes of a tag it picked itself.
    tag=""
    if [ "${version#*-}" != "$version" ]; then
        tag="${version#*-}"
        tag="${tag%%.*}"
    fi
    args=(--provenance)
    if [ -n "$tag" ]; then
        args+=(--tag "$tag")
    fi

    echo "publishing $name $version${tag:+ as $tag}"
    # `--provenance`: the registry records what built the tarball it was given.
    # No `--access`: an unscoped package is public, and npm says so.
    (cd "$pkg" && npm publish "${args[@]}")
done

published "$every_published"
