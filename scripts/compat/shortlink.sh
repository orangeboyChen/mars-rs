#!/bin/sh
# The short-link cross-read test: a request one implementation packs has to be
# the same bytes as the other's, and read back the same way by both.
#
#   sh scripts/compat/shortlink.sh
#
# The long-link half (`stn.sh`) proves the twenty bytes in front of a long-link
# package; this one proves what a short-link task goes out as — a `POST` of the
# cgi, five fields of mars' own, the headers the caller asked for, and the body
# — by putting the C++ of `mars/stn/proto/shortlink_packer.cc` next to the Rust
# of it.
#
# `pack` on either side writes the request. `parse` reads one back with that
# side's own http parser and prints a canonical form of what it found — the
# status, the request line, every field of the head in the order the head holds
# them, and the body — which is the same form on both sides, so the two can be
# diffed. A case therefore asks three things: that the two files are the same
# bytes, that the C++ reads the Rust's file as the Rust reads the C++'s, and
# that the body that comes back out is the body that went in.
#
# The C++ is built by `upstream_build.sh`, which clones Tencent/mars into
# `target/upstream` on first use — nothing is vendored into this repository.
set -e

REPO=$(cd "$(dirname "$0")/../.." && pwd)
OUT=$REPO/target/compat
WORK=$OUT/shortlink
UP=${MARS_UPSTREAM_DIR:-$REPO/target/upstream/Tencent-mars}

cargo build --manifest-path "$REPO/Cargo.toml" -p marsrs-compat --release
RUST=$REPO/target/release/stn-compat
CPP=$OUT/upstream_shortlink

# … or older than the harness it was built out of: an edit to
# `upstream_shortlink.cpp` is otherwise a binary that keeps answering for a
# source it no longer matches, and the C++ column of the table below is then
# not the C++ of this tree.
if [ ! -x "$CPP" ] || [ "$REPO/scripts/compat/upstream_shortlink.cpp" -nt "$CPP" ]; then
    MARS_SRC="$REPO/scripts/compat/upstream_shortlink.cpp" \
    MARS_SRCS="$UP/mars/comm/autobuffer.cc \
               $UP/mars/comm/http.cc \
               $UP/mars/comm/strutil.cc \
               $UP/mars/comm/unix/xlogger_threadinfo.cc \
               $UP/mars/comm/xlogger/xlogger.cc \
               $UP/mars/comm/xlogger/xlogger_category.cc \
               $UP/mars/stn/proto/shortlink_packer.cc" \
    MARS_C_SRCS="$UP/mars/comm/assert/__assert.c \
                 $UP/mars/comm/time_utils.c \
                 $UP/mars/comm/xlogger/xloggerbase.c" \
    MARS_UPSTREAM_DIR="$UP" \
    sh "$REPO/scripts/compat/upstream_build.sh" "$CPP" > /dev/null
fi

rm -rf "$WORK"
mkdir -p "$WORK"

# One row per request. `headers` is `name:value` separated by commas, the order
# the caller gave them in, which is not the order they go out in: the head is
# sorted by name with neither name's case in the way, on either side.
python3 <<'PY' > "$WORK/cases.txt"
rows = []


def case(name, url, body, headers=()):
    # a field of nothing would collapse the two spaces around it, and `read`
    # collapses those into one — shifting every field after it
    rows.append((name, url, body.hex() or "-", ",".join(headers) or "-"))


case("empty-body", "/cgi-bin/mars", b"")
case("one-byte", "/cgi-bin/mars", b"\x7f")
case("hello", "/cgi-bin/mars", b"hello")
case("every-byte", "/cgi-bin/mars", bytes(range(256)))
case("four-k", "/cgi-bin/mars", b"A" * 4096)
case("url-with-query", "/cgi-bin/mars?a=1&b=2", b"\x01")
# the five fields mars writes are the caller's to overwrite
case("host", "/m", b"x", ("Host:example.com",))
case("keep-alive", "/m", b"x", ("Connection:Keep-Alive",))
case("content-length-of-its-own", "/m", b"abc", ("Content-Length:99",))
case("user-agent-of-its-own", "/m", b"x", ("User-Agent:mars-rs",))
# given in no order at all, and one name twice in two cases
case("headers-out-of-order", "/m", b"x", ("X-Zebra:1", "X-Alpha:2", "X-Middle:3"))
case("one-name-two-cases", "/m", b"abc", ("Host:x", "host:y"))

for row in rows:
    print(*row)
PY

FAILED=0
printf '| case | bytes | cross-read | body |\n|---|---|---|---|\n'

while read -r name url body_hex headers; do
    if [ "$body_hex" = "-" ]; then body_hex=""; fi
    if [ "$headers" = "-" ]; then
        HEADERS=""
    else
        HEADERS=$(echo "$headers" | tr ',' '\n' | sed 's/^/--header=/')
    fi

    # `$HEADERS` is split on purpose: it is one `--header=N:V` per field, and
    # `sh` has no list to hold them in.
    RUST_FILE="$WORK/$name-rust.bin"
    CPP_FILE="$WORK/$name-cpp.bin"
    "$RUST" shortlink pack --url="$url" --body="$body_hex" $HEADERS --out="$RUST_FILE"
    "$CPP" pack --url="$url" --body="$body_hex" $HEADERS --out="$CPP_FILE"

    # 1. the two files.
    if cmp -s "$RUST_FILE" "$CPP_FILE"; then
        BYTES=identical
    else
        BYTES=FAILED
        FAILED=$((FAILED + 1))
        # A diagnostic on stdout lands between two rows of the table this
        # script prints, which is why it goes to stderr.
        cmp "$RUST_FILE" "$CPP_FILE" >&2 || true
    fi

    # 2. each side's parser on the other side's file. The files are the same
    # bytes, so any difference here is a difference between the two readings.
    "$CPP" parse --in="$RUST_FILE" > "$WORK/$name-rc.txt"
    "$RUST" shortlink parse --in="$CPP_FILE" > "$WORK/$name-cr.txt"
    if cmp -s "$WORK/$name-rc.txt" "$WORK/$name-cr.txt"; then
        READ=ok
    else
        READ=FAILED
        FAILED=$((FAILED + 1))
        diff "$WORK/$name-rc.txt" "$WORK/$name-cr.txt" >&2 || true
    fi

    # 3. the body that went in is the body that came out. `body ` with nothing
    # after it is an empty one, which is why the two are one string.
    if [ "$body_hex" = "-" ]; then
        BODY_EXP="body "
    else
        BODY_EXP="body $body_hex"
    fi
    BODY=ok
    for file in "$WORK/$name-rc.txt" "$WORK/$name-cr.txt"; do
        if ! grep -qx "$BODY_EXP" "$file"; then
            BODY=FAILED
            FAILED=$((FAILED + 1))
            grep '^body' "$file" || true
        fi
    done

    printf '| %s | %s | %s | %s |\n' "$name" "$BYTES" "$READ" "$BODY"
done < "$WORK/cases.txt"

if [ "$FAILED" -ne 0 ]; then
    echo "$FAILED check(s) failed" >&2
    exit 1
fi
echo "every short-link cross-read check passed; the files are in $WORK"
