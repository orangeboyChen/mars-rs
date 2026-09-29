#!/bin/sh
# The SDT cross-read test: the URL of an HTTP check, read by both sides.
#
#   sh scripts/compat/sdt.sh
#
# `stn.sh` proves the long link's wire format and `shortlink.sh` the short
# link's; this one proves what `mars/sdt` reads before it opens a socket —
# `mars/sdt/src/checkimpl/http_url_parser.h`, which splits the URL of an HTTP
# check into the host, the port and the path — by putting the C++ of that header
# next to the Rust of it.
#
# `url` on either side prints a canonical form of what the parser read: one line
# per thing, the same three lines on both sides, so the two can be diffed. A
# case therefore asks two things: that the C++ reads the URL as the Rust reads
# it, and that what either of them read is what the case says it should be —
# which is the answer that keeps the first honest, since two parsers agreeing is
# not proof on its own.
#
# The C++ is built by `upstream_build.sh`, which clones Tencent/mars into
# `target/upstream` on first use — nothing is vendored into this repository.
set -e

REPO=$(cd "$(dirname "$0")/../.." && pwd)
OUT=$REPO/target/compat
WORK=$OUT/sdt
UP=${MARS_UPSTREAM_DIR:-$REPO/target/upstream/Tencent-mars}

cargo build --manifest-path "$REPO/Cargo.toml" -p marsrs-compat --release
RUST=$REPO/target/release/sdt-compat
CPP=$OUT/upstream_sdt

if [ ! -x "$CPP" ]; then
    MARS_SRC="$REPO/scripts/compat/upstream_sdt.cpp" \
    MARS_SRCS="$UP/mars/comm/strutil.cc \
               $UP/mars/comm/unix/xlogger_threadinfo.cc \
               $UP/mars/comm/xlogger/xlogger.cc \
               $UP/mars/comm/xlogger/xlogger_category.cc" \
    MARS_C_SRCS="$UP/mars/comm/assert/__assert.c \
                 $UP/mars/comm/time_utils.c \
                 $UP/mars/comm/xlogger/xloggerbase.c" \
    MARS_UPSTREAM_DIR="$UP" \
    sh "$REPO/scripts/compat/upstream_build.sh" "$CPP" > /dev/null
fi

rm -rf "$WORK"
mkdir -p "$WORK"

# One row per URL, and the answer asked for in the same breath: the host, the
# port and the path the parser has to come back with. The URL is hex, because a
# row is split on whitespace and two of the cases are URLs whose whitespace is
# what they are about — `strutil::Trim` is called on the URL and on the host
# read out of it. A field of nothing prints as `-`, since two spaces together
# would collapse into one and shift every field after it.
python3 <<'PY' > "$WORK/cases.txt"
rows = []


def case(name, url, host, port, path):
    rows.append((name, url.encode().hex() or "-", host or "-", port, path or "-"))


case("host-only", "http://www.qq.com", "www.qq.com", 80, "/")
case("host-and-port", "http://www.qq.com:8080", "www.qq.com", 8080, "/")
case("port-and-path", "http://www.qq.com:8080/cgi", "www.qq.com", 8080, "/cgi")
case("port-and-a-slash", "http://www.qq.com:8080/", "www.qq.com", 8080, "/")
# the scheme is looked for without either case in the way
case("uppercase-scheme", "HTTP://WWW.QQ.COM/cgi", "WWW.QQ.COM", 80, "/cgi")
# `user:pwd@host`, and the host is what comes after the `@`
case("user-and-password", "http://user:pwd@www.qq.com", "www.qq.com", 80, "/")
case("host-behind-an-at", "http://www.qq.com@evil.com/x", "evil.com", 80, "/x")
# a URL that names a port and stops: the colon is the last character, so there
# is no port to read
case("trailing-colon", "http://www.qq.com:", "www.qq.com", 80, "/")
# `atoi` and the C++'s `(uint16_t)` cast: `0` is a port that did not read, and
# `65535` is what a port of `-1` casts to
case("port-zero", "http://www.qq.com:0/x", "www.qq.com", 80, "/x")
case("port-that-is-not-a-number", "http://www.qq.com:abc/x", "www.qq.com", 80, "/x")
case("port-minus-one", "http://www.qq.com:-1/x", "www.qq.com", 65535, "/x")
case("port-of-sixty-five-five-three-six", "http://www.qq.com:65536/x", "www.qq.com", 80, "/x")
# the number at the front of the port, and the path behind it
case("port-with-a-path-behind-it", "http://www.qq.com:80abc/x", "www.qq.com", 80, "/x")
# an ipv6 host, which the parser does not know about: the first colon of the
# address ends the host, on either side
case("ipv6-host", "http://[::1]:8080/x", "[", 80, "/x")
# `Trim`, on the URL and on the host: the whitespace around either is not part
# of it
case("whitespace-around-the-url", "  http://www.qq.com/cgi  ", "www.qq.com", 80, "/cgi")
case("whitespace-behind-the-scheme", "http:// www.qq.com", "www.qq.com", 80, "/")
# a path of nothing at all is the root, and a host of nothing is a URL the
# parser could not read
case("nothing-after-the-scheme", "http://", "", 80, "")
case("not-an-http-url", "https://www.qq.com", "", 80, "")
case("nothing-at-all", "", "", 80, "")
# the scheme and a path with no host in between: nothing between the two ends
# the host, so what it reads as one is the path
case("empty-host", "http:///cgi", "/cgi", 80, "/")

for row in rows:
    print(*row)
PY

FAILED=0
printf '| case | cross-read | expected |\n|---|---|---|\n'

# What the row holds as hex, which is the URL as the parser takes it.
decode() {
    python3 -c 'import sys; sys.stdout.write(bytes.fromhex(sys.argv[1]).decode("utf-8"))' "$1"
}

while read -r name url_hex host_exp port_exp path_exp; do
    if [ "$url_hex" = "-" ]; then url_hex=""; fi
    if [ "$host_exp" = "-" ]; then host_exp=""; fi
    if [ "$path_exp" = "-" ]; then path_exp=""; fi
    URL=$(decode "$url_hex")

    "$RUST" url --url="$URL" > "$WORK/$name-rust.txt"
    "$CPP" url --url="$URL" > "$WORK/$name-cpp.txt"

    # 1. the two readings of the same URL.
    if cmp -s "$WORK/$name-rust.txt" "$WORK/$name-cpp.txt"; then
        READ=ok
    else
        READ=FAILED
        FAILED=$((FAILED + 1))
        diff "$WORK/$name-rust.txt" "$WORK/$name-cpp.txt" || true
    fi

    # 2. the reading against the answer: `host ` with nothing after it is a URL
    # that did not parse, which is why the three are one string each. `-F`,
    # because a host is a string and not a pattern — one of the cases reads `[`
    # as its host, and that is an unbalanced bracket expression to `grep`.
    EXPECTED=ok
    for file in "$WORK/$name-rust.txt" "$WORK/$name-cpp.txt"; do
        for line in "host $host_exp" "port $port_exp" "path $path_exp"; do
            if ! grep -Fxq "$line" "$file"; then
                EXPECTED=FAILED
                FAILED=$((FAILED + 1))
                echo "$name: $(cat "$file")" >&2
                echo "$name expected: $host_exp $port_exp $path_exp" >&2
                break
            fi
        done
    done

    printf '| %s | %s | %s |\n' "$name" "$READ" "$EXPECTED"
done < "$WORK/cases.txt"

if [ "$FAILED" -ne 0 ]; then
    echo "$FAILED check(s) failed" >&2
    exit 1
fi
echo "every sdt cross-read check passed; the files are in $WORK"
