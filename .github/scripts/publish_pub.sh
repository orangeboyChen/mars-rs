#!/usr/bin/env bash
#
# Publishes the two Flutter plugins to pub.dev, at the version of the release.
#
#   .github/scripts/publish_pub.sh <version> <plugin-dir>...
#
# A `<plugin-dir>` is a stamped plugin, unpacked from the release's
# marsrs-flutter-<version>.tar.gz: pubspec.yaml and the podspec carry the
# version of the release and ios/Frameworks carries the xcframework. It is not
# the directory of a checkout, which has neither — hence pub.yml taking the
# plugins off the release and not out of the tree.
#
# No credential. pub.dev takes an OIDC token the way npm does, and this script
# mints the run's own and hands it to `dart pub token add`: GitHub's endpoint is
# what `id-token: write` buys, and `https://pub.dev` is the audience it is asked
# for. Two things have to be true before it can work, and neither is in this
# repository:
#
#   * the package is on pub.dev already — automation cannot create one, so the
#     first version of `marsrs_flutter` and `marsrs_flutter_xlog` is published
#     by hand, once;
#   * "Enable publishing from GitHub Actions" has been clicked in the package's
#     Admin tab, with this repository and the tag-pattern `v{{version}}`.
#
# A package pub.dev does not have is a warning and not a failure, for the same
# reason it is one on the npm side: only a human can publish a first version.

set -euo pipefail

version="${1:-}"
if [ -z "$version" ]; then
    echo "usage: $(basename "$0") <version> <plugin-dir>..." >&2
    exit 2
fi
shift
version="${version#v}"

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

# GitHub's OIDC token, for pub.dev: the exchange npm's trusted publishing makes
# too, asked for here rather than left to a client that may not make it.
token="$(curl -sS -f -H "Authorization: bearer $ACTIONS_ID_TOKEN_REQUEST_TOKEN" \
    "${ACTIONS_ID_TOKEN_REQUEST_URL}&audience=https://pub.dev" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["value"])')"
# `pub` reads the token off stdin, and keeps it in the pub cache of this job and
# nowhere else.
dart pub token add https://pub.dev <<< "$token"

ua="mars-rs release (https://github.com/orangeboyChen/mars-rs)"

for pkg in "$@"; do
    name="$(python3 -c 'import re,sys; print(re.search(r"^name:\s*(\S+)", open(sys.argv[1]).read(), re.M).group(1))' "$pkg/pubspec.yaml")"

    # 200 is "pub.dev has the package", 404 is "it does not" — the first is the
    # one that can be published to, and the second is the one that has to be
    # published by hand first.
    status="$(curl -sS -o /dev/null -w '%{http_code}' -A "$ua" \
        "https://pub.dev/api/packages/$name")"
    case "$status" in
        200)
            ;;
        404)
            echo "::warning::pub.dev has no $name yet; publish its first version by hand, then enable publishing from GitHub Actions in its Admin tab"
            continue
            ;;
        *)
            echo "::error::pub.dev answered $status for $name; whether it is there is unknown, so it is not published to"
            exit 1
            ;;
    esac

    # and then the same question about the version: pub.dev refuses a version it
    # has, so a release published twice over is not a release that fails.
    status="$(curl -sS -o /dev/null -w '%{http_code}' -A "$ua" \
        "https://pub.dev/api/packages/$name/$version")"
    case "$status" in
        200)
            echo "$name $version is already on pub.dev"
            continue
            ;;
        404)
            ;;
        *)
            echo "::error::pub.dev answered $status for $name $version; whether it is up is unknown, so it is not published over"
            exit 1
            ;;
    esac

    echo "publishing $name $version"
    (cd "$pkg" && dart pub publish --force)
done
