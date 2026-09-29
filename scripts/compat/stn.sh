#!/bin/sh
# The long-link cross-read test: a package one implementation packs has to be
# read by the other, in both directions, and the two files have to be the same
# bytes.
#
#   sh scripts/compat/stn.sh
#
# `cross.sh` proves the file format of `mars/xlog`; this one proves the wire
# format of `mars/stn/proto/longlink_packer.cc` — the twenty packed bytes of
# `__STNetMsgXpHeader` in front of every long-link package — by putting the C++
# of that file next to the Rust of it and asking both the same questions.
#
# Both halves are driven the same way: `pack` writes a file, `unpack` prints one
# line, `code cmdid seq package_len body-hex`. `code` is the `int` the C++
# answers; the four after it are the ones upstream fills through references and
# the port hands back in one value. Only a package that unpacked is compared on
# more than the code: `longlink_unpack` answers `LONGLINK_UNPACK_CONTINUE` and
# `_FALSE` *after* filling part of those four — `__unpack_test` sets the cmdid
# and the seq before it asks whether the whole package has arrived, and every
# field before it refuses one over a megabyte — while the port answers the code
# alone, so a line whose code is not `0` is a code and nothing more on either
# side.
#
# The C++ is built by `upstream_build.sh`, which clones Tencent/mars into
# `target/upstream` on first use — nothing is vendored into this repository. The
# translation units it is asked for are the ones `longlink_packer.cc` needs and
# nothing else: `autobuffer.cc` and the thread-info of `xlogger`, and not the
# zlib, the zstd and the boost the xlog harness carries.
set -e

REPO=$(cd "$(dirname "$0")/../.." && pwd)
OUT=$REPO/target/compat
WORK=$OUT/stn
UP=${MARS_UPSTREAM_DIR:-$REPO/target/upstream/Tencent-mars}

cargo build --manifest-path "$REPO/Cargo.toml" -p marsrs-compat --release
RUST=$REPO/target/release/stn-compat
CPP=$OUT/upstream_stn

if [ ! -x "$CPP" ]; then
    MARS_SRC="$REPO/scripts/compat/upstream_stn.cpp" \
    MARS_SRCS="$UP/mars/comm/autobuffer.cc \
               $UP/mars/comm/unix/xlogger_threadinfo.cc \
               $UP/mars/libraries/mars_android_sdk/jni/longlink_packer.cc" \
    MARS_C_SRCS="$UP/mars/comm/assert/__assert.c \
                 $UP/mars/comm/time_utils.c \
                 $UP/mars/comm/xlogger/xloggerbase.c" \
    MARS_UPSTREAM_DIR="$UP" \
    sh "$REPO/scripts/compat/upstream_build.sh" "$CPP" > /dev/null
fi

rm -rf "$WORK"
mkdir -p "$WORK"

# One row per question, and the answer asked for in the same breath: `code` is
# the `int` both sides answer, and the four after it are what a package that
# unpacked has to give back. A row of `raw` is a file neither side packed — the
# awkward ones a stream can hand over, which no encoder of ours would write.
python3 <<'PY' > "$WORK/cases.txt"
import struct

HEADER_LEN = 20
MAX_PACKAGE_LEN = 1024 * 1024

rows = []


def hx(body):
    # A field of nothing would leave two spaces together, and `read` collapses
    # those into one — shifting every field after it — so empty prints as `-`.
    return body.hex() or "-"


def header(head_length, client_version, cmdid, seq, body_length):
    return struct.pack(">IIIII", head_length, client_version, cmdid, seq, body_length)


def pack(name, pack_version, unpack_version, cmdid, seq, body, cut=0):
    if cut:
        # not a whole package yet, however the header reads
        code = -2
    elif pack_version != unpack_version:
        code = -1
    else:
        code = 0
    rows.append(("pack", name, pack_version, unpack_version, cmdid, seq, hx(body), cut,
                 code, cmdid, seq, HEADER_LEN + len(body), hx(body)))


def raw(name, unpack_version, data, code, cmdid=0, seq=0, package_len=0, body=b""):
    rows.append(("raw", name, unpack_version, unpack_version, 0, 0, hx(data), 0,
                 code, cmdid, seq, package_len, hx(body)))


pack("empty-body", 0, 0, 0, 0, b"")
pack("one-byte", 0, 0, 1, 2, b"\x7f")
pack("hello", 0, 0, 11, 22, b"hello")
pack("noop-cmdid", 0, 0, 6, 1, b"\x00\x01\x02")
pack("signal-keep-max-seq", 200, 200, 243, 0xFFFFFFFF, b"A" * 4096)
pack("every-byte", 0, 0, 6, 1, bytes(range(256)))
pack("version-200", 200, 200, 1, 1, b"mars")
# a version the reader was not set to: `__unpack_test` compares the header's
# client version with `sg_client_version` before it reads anything else
pack("version-mismatch", 200, 201, 1, 1, b"mars")
# a package the stream has not finished handing over
pack("truncated-body", 0, 0, 1, 1, b"0123456789", cut=1)
pack("truncated-header", 0, 0, 1, 1, b"0123456789", cut=15)

raw("empty-file", 0, b"", -2)
raw("short-header", 0, b"\x00" * 19, -2)
# `head_length` is taken as it comes on both sides: a header that claims `0`
# is a package whose body starts at the header itself
raw("head-length-zero", 0, header(0, 0, 1, 1, 3) + b"abc", 0, 1, 1, 3, b"\x00\x00\x00")
raw("over-a-megabyte", 0, header(HEADER_LEN, 0, 1, 1, MAX_PACKAGE_LEN + 1), -1)

for row in rows:
    print(*row)
PY

FAILED=0
printf '| case | rust -> cpp | cpp -> rust | bytes |\n|---|---|---|---|\n'

while read -r kind name pack_version unpack_version cmdid seq payload cut \
          code_exp cmdid_exp seq_exp len_exp body_exp; do
    if [ "$payload" = "-" ]; then payload=""; fi
    if [ "$body_exp" = "-" ]; then body_exp=""; fi

    if [ "$kind" = pack ]; then
        RUST_FILE="$WORK/$name-rust.bin"
        CPP_FILE="$WORK/$name-cpp.bin"
        "$RUST" pack --client-version="$pack_version" --cmdid="$cmdid" \
            --seq="$seq" --body="$payload" --out="$RUST_FILE"
        "$CPP" pack --client-version="$pack_version" --cmdid="$cmdid" \
            --seq="$seq" --body="$payload" --out="$CPP_FILE"

        # The same cut on both files: a stream that stopped early stopped at the
        # same byte for either encoder, so the two are still the same file.
        if [ "$cut" != 0 ]; then
            python3 -c 'import sys
path, cut = sys.argv[1], int(sys.argv[2])
data = open(path, "rb").read()
open(path, "wb").write(data[:len(data) - cut])' "$RUST_FILE" "$cut"
            python3 -c 'import sys
path, cut = sys.argv[1], int(sys.argv[2])
data = open(path, "rb").read()
open(path, "wb").write(data[:len(data) - cut])' "$CPP_FILE" "$cut"
        fi

        # Nothing in a long-link package is chosen at random, so "the Rust
        # packer writes what the C++ writes" is a claim about bytes here and not
        # only about what the two read back.
        if cmp -s "$RUST_FILE" "$CPP_FILE"; then
            BYTES=identical
        else
            BYTES=FAILED
            FAILED=$((FAILED + 1))
            cmp "$RUST_FILE" "$CPP_FILE" || true
        fi
    else
        RUST_FILE="$WORK/$name.bin"
        CPP_FILE="$RUST_FILE"
        python3 -c 'import sys
open(sys.argv[1], "wb").write(bytes.fromhex(sys.argv[2]))' "$RUST_FILE" "$payload"
        BYTES="one file"
    fi

    # 1. the Rust package, read by the C++.
    "$CPP" unpack --client-version="$unpack_version" --in="$RUST_FILE" \
        > "$WORK/$name-rc.txt"
    read -r code got_cmdid got_seq got_len got_body < "$WORK/$name-rc.txt"
    if [ "$code" = "$code_exp" ] && {
        [ "$code_exp" != 0 ] ||
            [ "$got_cmdid $got_seq $got_len $got_body" = "$cmdid_exp $seq_exp $len_exp $body_exp" ]
    }; then
        RUST_CPP=ok
    else
        RUST_CPP=FAILED
        FAILED=$((FAILED + 1))
        echo "$name (rust -> cpp): $(cat "$WORK/$name-rc.txt")" >&2
        echo "$name (rust -> cpp) expected: $code_exp $cmdid_exp $seq_exp $len_exp $body_exp" >&2
    fi

    # 2. the C++ package, read by the Rust.
    "$RUST" unpack --client-version="$unpack_version" --in="$CPP_FILE" \
        > "$WORK/$name-cr.txt"
    read -r code got_cmdid got_seq got_len got_body < "$WORK/$name-cr.txt"
    if [ "$code" = "$code_exp" ] && {
        [ "$code_exp" != 0 ] ||
            [ "$got_cmdid $got_seq $got_len $got_body" = "$cmdid_exp $seq_exp $len_exp $body_exp" ]
    }; then
        CPP_RUST=ok
    else
        CPP_RUST=FAILED
        FAILED=$((FAILED + 1))
        echo "$name (cpp -> rust): $(cat "$WORK/$name-cr.txt")" >&2
        echo "$name (cpp -> rust) expected: $code_exp $cmdid_exp $seq_exp $len_exp $body_exp" >&2
    fi

    printf '| %s | %s | %s | %s |\n' "$name" "$RUST_CPP" "$CPP_RUST" "$BYTES"
done < "$WORK/cases.txt"

if [ "$FAILED" -ne 0 ]; then
    echo "$FAILED check(s) failed" >&2
    exit 1
fi
echo "every long-link cross-read check passed; the files are in $WORK"
