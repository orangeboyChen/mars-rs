#!/usr/bin/env bash
#
# Publishes the nine crates of a release to crates.io, at the version of it.
#
#   .github/scripts/publish_crates.sh <version>
#
# Run from the root of a checkout: the manifest is stamped with <version> and
# then `cargo publish` is asked for one crate at a time. Which checkout is the
# caller's choice — release.yml publishes off the branch it ran on, publish.yml
# off the tag of a release that is already out — and it is the version argument
# that decides what goes up either way, because the version a checkout carries
# is the one it was committed at and not the one of the release.
#
# Nine and not two: crates.io resolves a dependency out of the registry and not
# out of a path, so a published crate cannot depend on a crate that is not
# published. `marsrs` and `marsrs-xlog` are the two a Rust caller takes — the
# pair the C++ project publishes as `mars-core` and `mars-xlog` — and the seven
# they are built out of go with them. `marsrs-ffi`, `marsrs-jni` and
# `marsrs-compat` are the three that stay off it, each with `publish = false`
# and a note saying why: what their callers need is a `libmars_ffi.a`, a
# `libmarsrsxlog.so` and a CLI for the differential test, and none of those is
# in what `cargo publish` packs.
#
# The one credential of the release. crates.io has no trusted publishing, so
# unlike the npm and the pub.dev half there is a token here to keep — and a
# release without one is a release that says so and publishes nothing, rather
# than one that fails.

set -euo pipefail

version="${1:-}"
if [ -z "$version" ]; then
    echo "usage: $(basename "$0") <version>" >&2
    exit 2
fi
version="${version#v}"

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

# crates.io answers 403 to the user agent `curl` sends by default.
: "${CRATES_IO_UA:=mars-rs release (https://github.com/orangeboyChen/mars-rs)}"

if [ -z "${CARGO_REGISTRY_TOKEN:-}" ]; then
    echo "::warning::CARGO_REGISTRY_TOKEN is not set; publish $version to crates.io by hand"
    exit 0
fi

python3 - "$version" <<'PY'
import re, sys

version = sys.argv[1]
src = open('Cargo.toml').read()

# Both spellings of the workspace version: the one in `[workspace.package]`,
# which every crate inherits, and the one in each `[workspace.dependencies]`
# entry of a crate of ours, which is the version requirement a published crate
# resolves its siblings by. A release that shipped 0.2.0 while its crates still
# asked for `^0.1.0` would not resolve for anybody. Only the two places are
# touched, so an external dependency that happens to be at the same version as
# the workspace keeps the version it asks for.
ours = re.search(r'\[workspace\.package\]\n(.*?)\n\n', src, re.S).group(1)
current = re.search(r'version = "([^"]+)"', ours).group(1)

out = []
in_deps = False
for line in src.splitlines(keepends=True):
    if line.startswith('[workspace.dependencies]'):
        in_deps = True
    elif line.startswith('['):
        in_deps = False
    if line.startswith('version = ') or (in_deps and line.startswith('marsrs-')):
        line = line.replace('version = "%s"' % current, 'version = "%s"' % version)
    out.append(line)

open('Cargo.toml', 'w').write(''.join(out))
print('crates.io version: %s (was %s)' % (version, current))
PY

# The status of a version of a crate on crates.io: 200 is "it is there", 404 is
# "it is not", and both are answers about the version. `curl -f` folds every
# other status into the same false branch as a 404 — a rate limit, an outage, a
# token that expired or lost its scope — and 404 is the branch that publishes,
# which is not a thing crates.io lets anyone do twice over one version. So the
# status is what is read here, and those two are the only two acted on.
crate_status() {
    curl -sS -o /dev/null -w '%{http_code}' -A "$CRATES_IO_UA" \
        "https://crates.io/api/v1/crates/$1/$version"
}

# Dependency order, and the two crates a caller takes last: crates.io resolves a
# dependency out of the registry, so a crate is only publishable once the crates
# it names are on it.
for CRATE in marsrs-core marsrs-comm marsrs-crypt marsrs-buffer \
             marsrs-appender marsrs-sdt marsrs-stn marsrs-xlog marsrs; do
    # Asked up to five times, because a 429 or a 5xx is crates.io being busy
    # and not a fact about the version: ten seconds, then again.
    for _ in 1 2 3 4 5; do
        status="$(crate_status "$CRATE")"
        case "$status" in
            429|5[0-9][0-9]) sleep 10; continue ;;
        esac
        break
    done

    case "$status" in
        200)
            echo "$CRATE $version is already on crates.io"
            continue
            ;;
        404)
            ;;
        *)
            echo "::error::crates.io answered $status for $CRATE $version; whether it is up is unknown, so it is not published over"
            exit 1
            ;;
    esac

    # The version is the release's and the checkout is the tag's, so the tree
    # carries an edit `cargo publish` would otherwise refuse.
    cargo publish -p "$CRATE" --allow-dirty

    # crates.io's index is not its database: a publish is visible to the next
    # `cargo publish` only once the index has caught up, and a dependency that
    # has not is "no matching package". Anything but a 200 is "not yet" here —
    # the index that answers 404, and an answer this job could not read — and
    # either way the publish that follows is what decides, and says so.
    for _ in $(seq 1 60); do
        [ "$(crate_status "$CRATE")" = 200 ] && break
        sleep 5
    done
done
