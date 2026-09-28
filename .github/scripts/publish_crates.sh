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
#
# `000` for a `curl` that came back with no answer at all, and not a failure of
# the script: this is called inside a command substitution, where a command that
# fails is a `set -e` that ends the run — and a run ended by one is a release
# red over a connection that would have come back on the second ask. `--max-time`
# is what keeps a request that never answers from being one that holds the job
# until the workflow's own timeout does.
crate_status() {
    local status
    # `%{http_code}` is `000` for a `curl` that got no answer, which is the
    # status wanted of it here; what the `||` is for is its exit status, which
    # `set -e` would end the run on from inside the substitution this is called
    # in. Assigned and not echoed beside it: a `curl` that fails writes its
    # `000` and *then* fails, so an `echo` would put two of them there.
    status="$(curl -sS -o /dev/null -w '%{http_code}' -A "$CRATES_IO_UA" \
        --max-time 30 "https://crates.io/api/v1/crates/$1/$version")" || status=000
    printf '%s' "$status"
}

# Where a crate sits in the sparse index: the name lower-cased, under the
# first two letters of it over the next two — `marsrs-crypt` is
# `ma/rs/marsrs-crypt`. A name of one, two or three letters gets a directory
# of its own; none of this port's is that short, but the rule is the index's
# and not this script's.
index_path() {
    local name
    # The index lower-cases a name: a crate can be published with capitals in
    # it, and the file it is found under is the lower-cased one. `tr` and not
    # `${name,,}`, which is a bash 4 expansion and a bad substitution on the
    # 3.2 a macOS still ships.
    name="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')"
    case "${#name}" in
        1) echo "1/$name" ;;
        2) echo "2/$name" ;;
        3) echo "3/${name:0:1}/$name" ;;
        *) echo "${name:0:2}/${name:2:2}/$name" ;;
    esac
}

# Whether `cargo` can resolve <crate> <version> — the question the publish of
# the crate that depends on this one asks. `cargo` reads the sparse index and
# not the api above, and the index is a copy of crates.io's database that lags
# behind it: a version the api answers 200 for and the index has not is a
# "no matching package named <crate> found, location searched: crates.io
# index" in the next `cargo publish` of the run.
index_has() {
    local body
    # Fetched whole, and not piped into a `grep`: one that matches stops
    # reading at the line it matched, `curl` is left writing into a pipe
    # nobody is reading, and the SIGPIPE it dies of is — under the `pipefail`
    # this script asks for — a "this version is not in the index" that ends
    # the run over a version that is in it. The index of a crate of nine
    # versions is small enough for `curl` to finish first; one of a hundred
    # is not, and that is what a crate becomes.
    body="$(curl -sS -A "$CRATES_IO_UA" --max-time 30 \
        "https://index.crates.io/$(index_path "$1")")" || return 1
    # `case` and not a `grep`: a glob is literal in everything but `*?[]`,
    # none of which a version holds, and the quotes either side of the field
    # are what keep the match to the `vers` of the crate — the requirement of
    # a dependency on that same version sits under `"req"`.
    case "$body" in
        *"\"vers\":\"$version\""*) return 0 ;;
        *) return 1 ;;
    esac
}

# Dependency order, and the two crates a caller takes last: crates.io resolves a
# dependency out of the registry, so a crate is only publishable once the crates
# it names are on it.
for CRATE in marsrs-core marsrs-comm marsrs-crypt marsrs-buffer \
             marsrs-appender marsrs-sdt marsrs-stn marsrs-xlog marsrs; do
    # Asked up to five times, because a 429 or a 5xx is crates.io being busy
    # and not a fact about the version: ten seconds, then again. A `000` is
    # asked again for the same reason — it is no answer at all, and an answer
    # is what a publish is decided on.
    for _ in 1 2 3 4 5; do
        status="$(crate_status "$CRATE")"
        case "$status" in
            000|429|5[0-9][0-9]) sleep 10; continue ;;
        esac
        break
    done

    case "$status" in
        200)
            echo "$CRATE $version is already on crates.io"
            ;;
        404)
            # The version is the release's and the checkout is the tag's, so the
            # tree carries an edit `cargo publish` would otherwise refuse.
            cargo publish -p "$CRATE" --allow-dirty
            ;;
        000)
            echo "::error::crates.io answered none of the five asks about $CRATE $version; whether it is up is unknown, so it is not published over"
            exit 1
            ;;
        *)
            echo "::error::crates.io answered $status for $CRATE $version; whether it is up is unknown, so it is not published over"
            exit 1
            ;;
    esac

    # What is waited for is the index and not the api: crates.io's index is a
    # second, cached copy of its database, and it is the one `cargo` resolves
    # out of, so an api that answers 200 while the index still answers 404 is a
    # wait that ends early and a "no matching package" in the crate published
    # next — which is what a release of these nine ended red on.
    #
    # Ten minutes, and not five: a crate the index has never seen is the
    # slowest case it has, and the release this job is the last step of is out
    # and tagged by now, so waiting is cheaper than a red step that publish.yml
    # has to be asked to run again.
    #
    # What is asked again at the end is the answer the loop reached and not the
    # index: a probe that fails is a transient one — a DNS, a TLS, a 5xx — and
    # the loop has 119 more of them to spend, whereas one that fails here would
    # end the run over a version that was already seen.
    in_index=false
    for _ in $(seq 1 120); do
        if index_has "$CRATE"; then in_index=true; break; fi
        sleep 5
    done
    if [ "$in_index" != true ]; then
        echo "::error::$CRATE $version is on crates.io but not in its index after ten minutes; the crates that depend on it cannot resolve it, so they are not published"
        exit 1
    fi
done
