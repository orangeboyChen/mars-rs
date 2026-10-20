#!/bin/sh
# The comm cross-read test: the wire formats of `mars/comm`, read by both sides.
#
#   sh scripts/compat/comm.sh
#
# `stn.sh` proves the long link's wire format, `shortlink.sh` the short link's
# and `sdt.sh` the URL of an HTTP check. This one proves the five that sit under
# all of them: `mars/comm/basepacker.cc` — the package the long link spoke
# before `longlink_packer.cc` took it over, and the `Simple*` pair of a length
# and a body — `mars/comm/adler32.c`, the hash the first one puts over its URL
# and its body, `mars/comm/strutil.cc`, the string helpers every one of them
# spells its URL with, `mars/comm/socket/socket_address.cc`, the address a
# caller connects to, and `mars/comm/http.cc`, the request and the answer a
# short link writes on one.
#
# A case asks three things:
#
#   bytes      — the two sides write the same package, byte for byte
#   cross-read — the Rust reads the bytes the C++ wrote, and the other way round
#   expected   — what either side wrote or read is what the case says it should
#                be; that is the answer which keeps the other two honest, since
#                two implementations agreeing is not proof on its own
#
# The expectations come out of Python, a third implementation of the same
# format, so a case is a package the two sides have to agree about and be right
# about.
#
# Two answers of the port's differ from the C++'s, both on purpose, and both
# are handled here rather than hidden:
#
# - A package that is refused. The C++ answers the `__LINE__` of the check that
#   failed, a positive number that means nothing outside `basepacker.cc`, and
#   the port answers `1`. A positive code is therefore compared on its sign and
#   not on its number: `normalize` prints `refused` for either.
# - A `Simple*` package whose length is shorter than the length in front of it.
#   The C++ hands the caller `_packlen - sizeof(T)` bytes, which has wrapped
#   around, and the port refuses. No case feeds one in: there is nothing for the
#   C++ to read and still be alive to answer.
#
# Three more belong to `strutil`, and no case asks for any of them either:
#
# - `Str2Hex` of text that is not hex. The C++ reads every two characters with
#   `strtol`, which answers `0` for a pair it cannot read, and the port answers
#   "there are no bytes" — which is the answer a caller can act on.
# - `ci_find_substr` with a `pos` past the end of the haystack. The C++ starts
#   its `std::search` at `str.begin() + pos`, which is not an iterator of the
#   string at all, and the port answers "not found".
# - `Str2Hex` of more than 1024 characters, which the C++ asserts on.
#
# Two more belong to `socket_address`, and they are why its table is in two:
#
# - The three strings of a NAT64 address. The C++ writes the well-known prefix
#   and then `inet_ntop`s the address into `ip_ + 9` — but the address it hands
#   over is the v6 one and not the IPv4 one it has just taken out of it, so its
#   `ip_` is `64:ff9b::64:ff9b::c000:201`, its `ip()` is `64:ff9b::c000:201`
#   and its `url()` is `[64:ff9b::64:ff9b::c000:201]:80`. The port answers
#   `[64:ff9b::192.0.2.1]:80` and `192.0.2.1`. The address behind them, which
#   is what the table of the address compares, is the same 16 bytes on both
#   sides.
# - The ip text `socket_address(const char*)` will take. `inet_pton` reads a
#   zero-padded octet as decimal and the port's parse refuses one, so
#   `010.1.1.1` is a valid `10.1.1.1` in the C++ and `AF_UNSPEC` here. No case
#   feeds one in; the address table is about the bytes and not the parse.
#
# Five more belong to `http`, and the first two of them are cases that are
# asked for:
#
# - The body of a `Connection: close` answer that names no `Content-Length`.
#   The C++ ends it the moment its head is whole and nothing came behind it —
#   `bodyreceiver_->Length()` is `0`, which is also what `ContentLength()`
#   answers for a field that is not there — so the bytes that arrive in the
#   next read sit in the buffer for the rest of the connection, and the answer
#   is an empty one. The port waits for the socket, which is what the length
#   of such a body is.
# - A `Content-Length: 0` on a socket the peer closes, which the C++ reads as
#   a body the socket is the length of — it asks `0 == contentLength` and not
#   whether the field is there — and the port ends at the head.
#
# - A chunk whose size line never ends. The C++ waits for the `CRLF` for as
#   long as the peer writes — `recvbuf_` grows by every read — and the port
#   refuses a line longer than the bound a line of a head has.
#
# Both are asked for, and both answers of both are pinned: a row like that
# prints `diverges` where the others print `ok`, and it fails when either side
# changes what it answers — which is what keeps a divergence from being a case
# the table quietly stops asking about.
#
# One more of the same belongs to `http`, and no case asks for it:
#
# - A body of chunks that add up to more than 4g. The C++ bounds each chunk on
#   its own and nothing else, so a peer that announces one after another of
#   them is waved through, and the port bounds the body they add up to. Four
#   gibibytes is what a case would have to carry.
# - `KeepAliveTimeout` of a `Keep-Alive` whose `timeout=` is not at the front of
#   a token. The C++ gets past `timeout=` by skipping `sizeof(const char*)` — 8
#   — characters, and not the 7 the string is long, so `max=100, timeout=7`
#   reads `=7`, which is not a timeout, and answers the default of 5. The port
#   takes what is behind `timeout=` and answers 7. Every `Keep-Alive` in the
#   tables below has it at the front, where the two agree.
# - A status code that is not one. The C++ casts the `long` `strtol` read to an
#   `int`, which is a different number again for anything past `INT_MAX`, and
#   the port answers `0` for a code it cannot read.
# - The first line a `Parser` refuses. The C++ keeps what it read out of it
#   before it gave up — so `HTTP/3 200 OK` leaves a `version_unknown` behind —
#   and the port keeps none of it. No case feeds one in: what a caller reads
#   after a refusal is not a line anyone should be reading.
#
# A first line of fewer tokens than the C++ asks for aborts it — `xassert2` is
# on, on every build that is not `NDEBUG` — so a request is always three tokens
# here and an answer always two, which is what the wire carries anyway.
#
# The two `socket_address` calls that ask the platform which network it is on —
# `v4tonat64_address` and `fix_current_nat64_addr`, which look the NAT64 prefix
# up over DNS — are not driven either: a harness has to answer the same line on
# every run, on a machine with no NAT64 in front of it. `upstream_comm.cpp`
# stands one in for each of them so that the rest of the class links.
#
# `adler32` has one case it cannot have, and it is why the seeded ones below
# start from a real checksum: the C++ reduces a seed whose two halves are not
# already below 65521 and the port hands it back unchanged. No caller in mars
# can produce such a seed — `basepacker.cc` passes either `0` or the answer of
# a previous call — so what is compared is every seed a caller can produce. A
# `NULL` buffer with a length of `0` is a different answer again, `1`, and that
# one is a convention about the pointer rather than about the bytes: the C++
# harness hands a pointer to no bytes over, which is what `--data=` means, and
# not `NULL`.
#
# The C++ is built by `upstream_build.sh`, which clones Tencent/mars into
# `target/upstream` on first use — nothing is vendored into this repository.
set -e

REPO=$(cd "$(dirname "$0")/../.." && pwd)
OUT=$REPO/target/compat
WORK=$OUT/comm
UP=${MARS_UPSTREAM_DIR:-$REPO/target/upstream/Tencent-mars}

cargo build --manifest-path "$REPO/Cargo.toml" -p marsrs-compat --release
RUST=$REPO/target/release/comm-compat
CPP=$OUT/upstream_comm

# … or older than the harness it was built out of: an edit to
# `upstream_comm.cpp` is otherwise a binary that keeps answering for a source
# it no longer matches, and the C++ column of the table below is then not
# the C++ of this tree.
if [ ! -x "$CPP" ] || [ "$REPO/scripts/compat/upstream_comm.cpp" -nt "$CPP" ]; then
    MARS_SRC="$REPO/scripts/compat/upstream_comm.cpp" \
    MARS_SRCS="$UP/mars/comm/socket/socket_address.cc \
               $UP/mars/comm/basepacker.cc \
               $UP/mars/comm/autobuffer.cc \
               $UP/mars/comm/ptrbuffer.cc \
               $UP/mars/comm/http.cc \
               $UP/mars/comm/crypt/ibase64.cc \
               $UP/mars/comm/strutil.cc \
               $UP/mars/comm/unix/xlogger_threadinfo.cc \
               $UP/mars/comm/xlogger/xlogger.cc \
               $UP/mars/comm/xlogger/xlogger_category.cc" \
    MARS_C_SRCS="$UP/mars/comm/adler32.c \
                 $UP/mars/comm/assert/__assert.c \
                 $UP/mars/comm/time_utils.c \
                 $UP/mars/comm/xlogger/xloggerbase.c" \
    MARS_UPSTREAM_DIR="$UP" \
    sh "$REPO/scripts/compat/upstream_build.sh" "$CPP" > /dev/null
fi

rm -rf "$WORK"
mkdir -p "$WORK"
FAILED=0

# A code of `0` is `*_OK`, a negative one is "read on", and a positive one is a
# package the C++ refused. Two things follow from that, and both are why a
# reading goes through here before it is compared:
#
# - Which line of `basepacker.cc` refused the package is the C++'s business
#   alone — it answers that line's `__LINE__` and the port answers `1` — so what
#   is compared is the refusal, printed as `refused` by either.
# - The fields behind it are not. `Packer_Unpack` has already written the URL,
#   the sequence and the length into the caller's variables by the time it comes
#   to the hash, so the one refusal the C++ answers with those filled in is the
#   hash's; the port's [`Refused`] carries why and nothing else. A refusal is
#   therefore compared on the refusal alone.
#
# Empty fields collapse here too, which is what lets an empty URL or an empty
# body compare: `awk` reads a run of blanks as one separator on either side.
normalize() {
    awk '{ if ($1 + 0 > 0) print "refused"; else { $1 = $1; print } }' "$1"
}

# One artifact, read by both sides: `compare_readings NAME SUBCMD BIN...`.
# `SUBCMD` is the words before `--in=`, the same on both sides. `CROSS` ends as
# `ok` when every reading of every artifact is one reading, and `EXPECTED` when
# that reading is the line in `exp-NAME.txt`.
#
# A case whose two readings are not one — `exp-rust-NAME.txt` and
# `exp-cpp-NAME.txt` are written for it instead — is pinned on either side and
# not diffed: `CROSS` ends as `diverges`, and a side that answers something
# else than the line that is its own fails the case all the same.
compare_readings() {
    _name=$1
    _subcmd=$2
    shift 2
    CROSS=ok
    EXPECTED=ok
    _diverges=no
    if [ -f "$WORK/exp-rust-$_name.txt" ]; then
        _diverges=yes
        normalize "$WORK/exp-rust-$_name.txt" > "$WORK/$_name-expected-rust.txt"
        normalize "$WORK/exp-cpp-$_name.txt" > "$WORK/$_name-expected-cpp.txt"
    else
        normalize "$WORK/exp-$_name.txt" > "$WORK/$_name-expected.txt"
    fi
    _at=0
    for _bin in "$@"; do
        _at=$((_at + 1))
        for _side in rust cpp; do
            _bin_of_side=$RUST
            if [ "$_side" = cpp ]; then _bin_of_side=$CPP; fi
            # shellcheck disable=SC2086
            # `_subcmd` is two words on purpose: the subcommand and its
            # `--kind=`, both unquoted so that the shell splits them.
            $_bin_of_side $_subcmd --in="$_bin" > "$WORK/$_name-$_side-$_at.txt"
            normalize "$WORK/$_name-$_side-$_at.txt" > "$WORK/$_name-$_side-$_at-norm.txt"
            _expected=$WORK/$_name-expected.txt
            if [ "$_diverges" = yes ]; then _expected=$WORK/$_name-expected-$_side.txt; fi
            if [ "$_diverges" = no ] && [ "$_side$_at" != "rust1" ] && ! cmp -s "$WORK/$_name-$_side-$_at-norm.txt" "$WORK/$_name-rust-1-norm.txt"; then
                CROSS=FAILED
                FAILED=$((FAILED + 1))
                diff "$WORK/$_name-rust-1-norm.txt" "$WORK/$_name-$_side-$_at-norm.txt" || true
            fi
            if ! cmp -s "$WORK/$_name-$_side-$_at-norm.txt" "$_expected"; then
                EXPECTED=FAILED
                FAILED=$((FAILED + 1))
                echo "$_name $_side: read $(cat "$WORK/$_name-$_side-$_at-norm.txt")" >&2
                echo "$_name $_side: want $(cat "$_expected")" >&2
            fi
        done
    done
    if [ "$CROSS" = ok ] && [ "$_diverges" = yes ]; then CROSS=diverges; fi
}

# One line of two halves: `compare_line NAME FIELDS`, where `FIELDS` is a `cut`
# range. `CROSS` ends as `ok` when both halves answer those fields the same way,
# and `EXPECTED` when that is what Python wrote — so a table can be about the
# end of a line the two sides agree on and not about the whole of it.
compare_line() {
    _name=$1
    _fields=$2
    cut -d' ' -f"$_fields" "$WORK/exp-$_name.txt" > "$WORK/$_name-expected.txt"
    CROSS=ok
    EXPECTED=ok
    for _side in rust cpp; do
        cut -d' ' -f"$_fields" "$WORK/$_name-$_side.txt" > "$WORK/$_name-$_side-cut.txt"
        if [ "$_side" = cpp ] && ! cmp -s "$WORK/$_name-cpp-cut.txt" "$WORK/$_name-rust-cut.txt"; then
            CROSS=FAILED
            FAILED=$((FAILED + 1))
            diff "$WORK/$_name-rust-cut.txt" "$WORK/$_name-cpp-cut.txt" || true
        fi
        if ! cmp -s "$WORK/$_name-$_side-cut.txt" "$WORK/$_name-expected.txt"; then
            EXPECTED=FAILED
            FAILED=$((FAILED + 1))
            echo "$_name: $_side read $(cat "$WORK/$_name-$_side-cut.txt")" >&2
            echo "$_name: want $(cat "$WORK/$_name-expected.txt")" >&2
        fi
    done
}

# One line each side answered, against the other's and against the one Python
# wrote: `compare_answer NAME KIND`, where the two are
# `<name>-<side>-<kind>.txt` and the expectation is `exp-<kind>-<name>.txt`.
compare_answer() {
    _name=$1
    _kind=$2
    CROSS=ok
    EXPECTED=ok
    for _side in rust cpp; do
        if [ "$_side" = cpp ] \
            && ! cmp -s "$WORK/$_name-cpp-$_kind.txt" "$WORK/$_name-rust-$_kind.txt"; then
            CROSS=FAILED
            FAILED=$((FAILED + 1))
            diff "$WORK/$_name-rust-$_kind.txt" "$WORK/$_name-cpp-$_kind.txt" || true
        fi
        if ! cmp -s "$WORK/$_name-$_side-$_kind.txt" "$WORK/exp-$_kind-$_name.txt"; then
            EXPECTED=FAILED
            FAILED=$((FAILED + 1))
            echo "$_name: $_side answered $(cat "$WORK/$_name-$_side-$_kind.txt")" >&2
            echo "$_name: want $(cat "$WORK/exp-$_kind-$_name.txt")" >&2
        fi
    done
}

# The case tables, and the expectation of every case in them. A row holds
# what the case is made of and nothing else; what has to come out of it is in
# `exp-<name>.txt` next to it, and the bytes a `raw` case reads are in
# `in-<name>.bin`.
python3 - "$WORK" <<'PY'
import hashlib
import os
import socket
import struct
import sys

work = sys.argv[1]
BASE = 65521


# The third implementation of the same format: nothing here is what either side
# runs, so a case is a package the two of them have to agree about and be right
# about.
def adler32(seed, data):
    a = seed & 0xFFFF
    b = (seed >> 16) & 0xFFFF
    for byte in data:
        a = (a + byte) % BASE
        b = (b + a) % BASE
    return (b << 16) | a


def magic(head_len, url_len, total):
    return (head_len + url_len + total) & 0xFF


def pack(url, seq, data, do_hash=True):
    url = url.encode()[:128]
    total = 16 + len(url) + len(data)
    hash_ = 0
    if do_hash:
        hash_ = adler32(0, url)
        if data:
            hash_ = adler32(hash_, data)
    return bytes([magic(16, len(url), total), 1, 16, len(url)]) + \
        struct.pack(">III", total, seq, hash_) + url + data


def header(magic_, url_len, total, seq, hash_):
    return bytes([magic_, 1, 16, url_len]) + struct.pack(">III", total, seq, hash_)


def row(name, *fields):
    rows.append(" ".join([name] + ["-" if f == "" else str(f) for f in fields]))


def expect(name, line):
    with open(os.path.join(work, "exp-%s.txt" % name), "w") as f:
        f.write(line + "\n")


def expect_of(kind, name, line):
    # the same, for a case that answers more than one line — the line a head
    # reads as, and the line the answer built out of it reads back as
    with open(os.path.join(work, "exp-%s-%s.txt" % (kind, name)), "w") as f:
        f.write(line + "\n")


def expect_side(side, name, line):
    # the answer one side has to give, for a case the two answer differently:
    # both of them are pinned, so a side that changes what it answers fails
    with open(os.path.join(work, "exp-%s-%s.txt" % (side, name)), "w") as f:
        f.write(line + "\n")


def artifact(name, data):
    with open(os.path.join(work, "in-%s.bin" % name), "wb") as f:
        f.write(data)


def bytes_of(name, data):
    with open(os.path.join(work, "exp-%s.bin" % name), "wb") as f:
        f.write(data)


def packer_line(code, url, seq, pack_len, data):
    return "%s %s %d %d %s" % (code, url, seq, pack_len, data.hex())


def simple_line(code, pack_len, data):
    return "%s %d %s" % (code, pack_len, data.hex())


rows = []

# --- adler32: the hash, over the same bytes on both sides. --------------------
# A body of nothing, one of one byte, the two lengths either side of the one the
# C++ takes its fast path for, one long enough to be read in `NMAX` blocks, and
# one of the highest bytes there are — the last of those is where a sum that is
# not reduced often enough would show up.
for name, data, seed in [
    ("nothing", b"", 0),
    ("one-byte", b"a", 0),
    ("fifteen-bytes", bytes(range(0x41, 0x50)), 0),
    ("sixteen-bytes", bytes(range(0x41, 0x51)), 0),
    ("high-bytes", bytes([0xFF]) * 20, 0),
    ("nmax-blocks", bytes([0x5A]) * 5552, 0),
    ("nmax-and-a-bit", bytes([0x5A]) * 5553, 0),
]:
    row(name, data.hex(), seed)
    expect(name, str(adler32(seed, data)))

# The seed a caller really hands over: the answer of a previous call, which is
# what `basepacker.cc` does with the hash over the URL before it reads the body.
row("seeded-with-a-checksum", "6d617273", adler32(0, b"/cgi-bin"))
expect("seeded-with-a-checksum", str(adler32(adler32(0, b"/cgi-bin"), b"mars")))

with open(os.path.join(work, "adler32.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")

# --- base64: the account a proxy is logged in with. ---------------------------
# `EncodeBase64`, which is the one of the two mars calls: `username:password`
# into the `Basic` of a `Proxy-Authorization`. `DecodeBase64` is the other and
# nothing in mars asks for it, so the port does not have one.
ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"


def base64_of(data):
    # three bytes into four characters, and a last group of one or two is
    # padded out to four with `=`
    out = ""
    for at in range(0, len(data), 3):
        group = data[at:at + 3]
        first = group[0]
        second = group[1] if len(group) > 1 else 0
        third = group[2] if len(group) > 2 else 0
        out += ALPHABET[first >> 2]
        out += ALPHABET[((first & 0x03) << 4) | (second >> 4)]
        if len(group) > 1:
            out += ALPHABET[((second & 0x0F) << 2) | (third >> 6)]
        else:
            out += "="
        out += ALPHABET[third & 0x3F] if len(group) > 2 else "="
    return out


rows = []
for name, data in [
    ("nothing", b""),
    ("one-byte", b"f"),
    ("two-bytes", b"fo"),
    ("three-bytes", b"foo"),
    ("four-bytes", b"foob"),
    ("a-byte-of-every-value", bytes(range(256))),
    ("the-highest-bytes", bytes([0xFF]) * 5),
    ("the-account-of-a-proxy", b"mars:secret"),
    ("an-empty-password", b"mars:"),
]:
    row(name, data.hex())
    encoded = base64_of(data)
    expect_of("base64", name, "%d %s" % (len(encoded), encoded or "-"))

with open(os.path.join(work, "base64.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")

# --- packer pack: a whole package, written by both sides. ---------------------
rows = []
for name, url, seq, data, do_hash in [
    ("url-and-body", "/cgi", 7, bytes.fromhex("0102"), True),
    ("no-body", "/cgi", 0, b"", True),
    ("no-url", "", 1, bytes.fromhex("0a0b0c"), True),
    # a hash of `0` is a hash `packer_unpack` does not check
    ("hash-off", "/longlink", 42, bytes.fromhex("00ff"), False),
    ("url-of-128", "a" * 128, 3, bytes.fromhex("01"), True),
    # `strnlen(_url, 128)`: the tail of a URL of 129 is not packed
    ("url-of-129", "a" * 129, 3, bytes.fromhex("01"), True),
    ("sequence-of-the-last", "/cgi", 0xFFFFFFFF, bytes.fromhex("07"), True),
    ("body-of-a-thousand", "/cgi", 1, (bytes(range(256)) * 4)[:1000], True),
]:
    row(name, url, seq, data.hex(), "yes" if do_hash else "no")
    packed = pack(url, seq, data, do_hash)
    bytes_of(name, packed)
    expect(name, packer_line(0, url[:128], seq, 16 + len(url[:128]) + len(data), data))

with open(os.path.join(work, "packer-pack.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")

# --- packer unpack: bytes neither side wrote, read by both. ------------------
# What a stream does to a package: too few bytes for the header, a header with
# no URL behind it, a URL with no body behind it, a `magic` that does not add
# up, a URL longer than the package that carries it, a package of more than a
# megabyte, a hash that does not match, a hash of `0` — which is not checked —
# and a whole package with the next one's bytes behind it.
whole = pack("/cgi", 7, bytes.fromhex("0102"))
cases = [
    ("no-bytes", b"", packer_line(-3, "", 0, 0, b"")),
    ("one-byte-short-of-a-header", whole[:15], packer_line(-3, "", 0, 0, b"")),
    ("header-with-no-url", whole[:16], packer_line(-2, "", 0, 0, b"")),
    ("url-with-no-body", whole[:-1], packer_line(-1, "/cgi", 7, 22, b"")),
    ("magic-that-does-not-add-up", bytes([whole[0] ^ 0xFF]) + whole[1:], "refused"),
    ("url-longer-than-the-package", header(magic(16, 200, 20), 200, 20, 7, 0), "refused"),
    ("package-of-more-than-a-megabyte",
     header(magic(16, 4, 0x100001), 4, 0x100001, 7, 0) + b"/cgi" + bytes(4), "refused"),
    ("hash-that-does-not-match",
     whole[:15] + bytes([whole[15] ^ 0xFF]) + whole[16:], "refused"),
    ("hash-of-zero-is-not-checked", whole[:12] + bytes(4) + whole[16:],
     packer_line(0, "/cgi", 7, 22, bytes.fromhex("0102"))),
    ("bytes-behind-a-whole-package", whole + b"behind",
     packer_line(0, "/cgi", 7, 22, bytes.fromhex("0102"))),
]

rows = []
for name, data, line in cases:
    row(name)
    artifact(name, data)
    expect(name, line)

with open(os.path.join(work, "packer-unpack.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")

# --- simple pack: a length and a body, written by both sides. ----------------
rows = []
for name, kind, data in [
    ("short-of-nothing", "short", b""),
    ("short-of-one", "short", b"a"),
    ("short-of-254", "short", bytes(254)),
    ("int-of-nothing", "int", b""),
    ("int-of-three", "int", bytes.fromhex("010203")),
    ("int-of-300", "int", (bytes(range(256)) * 2)[:300]),
]:
    row(name, kind, data.hex())
    head_len = 2 if kind == "short" else 4
    fmt = ">H" if kind == "short" else ">I"
    bytes_of(name, struct.pack(fmt, len(data) + head_len) + data)
    expect(name, simple_line(0, len(data) + head_len, data))

with open(os.path.join(work, "simple-pack.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")

# --- simple unpack: bytes neither side wrote, read by both. ------------------
rows = []
for name, kind, data, line in [
    ("short-with-no-length", "short", bytes(1), simple_line(-2, 0, b"")),
    ("short-with-a-length-but-no-body", "short", bytes.fromhex("000501"),
     simple_line(-1, 5, b"")),
    ("short-with-a-body-of-nothing", "short", bytes.fromhex("0002"),
     simple_line(0, 2, b"")),
    ("short-of-the-longest-length", "short", bytes.fromhex("ffff"),
     simple_line(-1, 0xFFFF, b"")),
    ("int-with-no-length", "int", bytes(3), simple_line(-2, 0, b"")),
    ("int-with-a-length-but-no-body", "int", bytes.fromhex("0000000601"),
     simple_line(-1, 6, b"")),
    ("int-with-a-body-of-nothing", "int", bytes.fromhex("00000004"),
     simple_line(0, 4, b"")),
    ("int-of-the-longest-length", "int", bytes.fromhex("ffffffff"),
     simple_line(-1, 0xFFFFFFFF, b"")),
]:
    row(name, kind)
    artifact(name, data)
    expect(name, line)

with open(os.path.join(work, "simple-unpack.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")

# --- strutil: the string helpers, one line each. ---------------------------
# A row is `name fn data arg pos`: `--data` is the bytes of the string and
# `--arg` the second string, both hex so that a byte the shell would eat — a
# tab, a newline, a backslash — is still one a case can name.
def h(text):
    return text.encode().hex()


def url_encode(text):
    out = []
    for byte in text.encode():
        ch = chr(byte)
        if ch.isascii() and (ch.isalnum() or ch in ".-_*"):
            out.append(ch)
        elif byte == 0x20:
            out.append("+")
        else:
            out.append("%%%02X" % byte)
    return "".join(out)


SPACE = " \t\n\x0b\x0c\r"


def trim(text):
    return text.strip(SPACE)


def split_token(text, delimiters):
    tokens = []
    for token in "".join(" " if c in delimiters else c for c in text).split(" "):
        if token:
            tokens.append(token)
    return "|".join(tokens)


def file_name_from_path(path):
    pos = path.rfind("\\")
    if pos < 0:
        pos = path.rfind("/")
    if pos < 0 or pos + 1 >= len(path):
        return path
    return path[pos + 1:]


def ci_find_substr(haystack, needle, pos):
    at = haystack.lower().find(needle.lower(), pos)
    return str(at) if 0 <= at else "-1"


rows = []
for name, fn, data, arg, pos in [
    ("a-url-of-nothing", "url_encode", "", None, None),
    ("a-url-and-its-query", "url_encode", "a b?c=d&e", None, None),
    ("the-bytes-that-stay", "url_encode", "AZaz09.-_*", None, None),
    ("a-url-that-is-not-ascii", "url_encode", "é", None, None),
    ("whitespace-on-both-ends", "trim", " \t\n\x0b\x0c\rhello\r\n", None, None),
    ("nothing-but-whitespace", "trim", "    ", None, None),
    ("no-whitespace-to-trim", "trim", "hello", None, None),
    ("the-case-of-a-mixed-word", "lower", "MiXeD-123", None, None),
    ("the-case-of-a-mixed-word-upper", "upper", "MiXeD-123", None, None),
    ("a-prefix-that-is-one", "starts_with", "mars-rs", "mars", None),
    ("a-prefix-that-is-not", "starts_with", "mars-rs", "rs", None),
    ("a-suffix-that-is-one", "ends_with", "mars-rs", "rs", None),
    ("a-suffix-that-is-not", "ends_with", "mars-rs", "mars", None),
    ("the-tokens-of-a-request-line", "split_token", "GET /cgi HTTP/1.1", " ", None),
    ("runs-of-delimiters-are-one", "split_token", "a,,b;c", ",;", None),
    ("every-default-delimiter", "split_token", "a:b,c;d e", " \t\n\r;:,.?", None),
    # `--data` of a `hex2str` is bytes and not text, so the case is the three
    # bytes `00 ff 10` and the answer is the hex of them.
    ("bytes-become-hex", "hex2str", None, None, None),
    ("hex-becomes-bytes", "str2hex", "00ff10", None, None),
    ("a-path-of-two-slashes", "file_name_from_path", "a/b/c", None, None),
    ("a-path-of-a-backslash", "file_name_from_path", "a\\b/c", None, None),
    # a path that ends in a separator is one with no name behind it, and the
    # answer is the path itself on both sides
    ("a-path-that-ends-in-one", "file_name_from_path", "a/b/", None, None),
    ("a-needle-of-another-case", "ci_find_substr", "MarsRS", "rs", 0),
    ("a-needle-that-is-not-there", "ci_find_substr", "MarsRS", "zz", 0),
    ("a-search-that-starts-later", "ci_find_substr", "MarsRS", "rs", 3),
    ("the-md5-of-nothing", "md5", "", None, None),
    ("the-md5-of-mars", "md5", "mars", None, None),
]:
    if fn == "hex2str":
        row_data, line = "00ff10", "00ff10"
    elif fn == "str2hex":
        row_data, line = h(data), bytes.fromhex(data).hex()
    elif fn == "url_encode":
        row_data, line = h(data), url_encode(data)
    elif fn == "trim":
        row_data, line = h(data), trim(data)
    elif fn == "lower":
        row_data, line = h(data), data.lower()
    elif fn == "upper":
        row_data, line = h(data), data.upper()
    elif fn == "starts_with":
        row_data, line = h(data), "1" if data.startswith(arg) else "0"
    elif fn == "ends_with":
        row_data, line = h(data), "1" if data.endswith(arg) else "0"
    elif fn == "split_token":
        row_data, line = h(data), split_token(data, arg)
    elif fn == "file_name_from_path":
        row_data, line = h(data), file_name_from_path(data)
    elif fn == "ci_find_substr":
        row_data, line = h(data), ci_find_substr(data, arg, pos)
    else:
        row_data, line = h(data), hashlib.md5(data.encode()).hexdigest()

    row(name, fn, row_data, "" if arg is None else h(arg), "" if pos is None else pos)
    expect(name, line)

with open(os.path.join(work, "strutil.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")

# --- socket: an address, and every answer a caller reads off it. -------------
# A row is `name key value port map text`: `key` is `ip`, `v4` or `v6`, and
# `value` is the text of the address or its bytes in hex. What comes back is one
# line of fifteen fields, and the two tables read different ends of it — see
# `socket_line`.
V4_MAPPED = bytes.fromhex("00000000000000000000ffff")
NAT64 = bytes.fromhex("0064ff9b0000000000000000")
WELL_KNOWN = "64:ff9b::"


def socket_line(family, packed, port):
    if family == "unspec":
        return "unspec - 0 0 0 0 0 0 0 0 0 0 - - -"

    if family == "v4":
        host = int.from_bytes(packed, "big")
        ip = socket.inet_ntop(socket.AF_INET, packed)
        ipv6 = ip
        length = 16
        mapped = False
    else:
        mapped = packed[:12] == V4_MAPPED
        host = int.from_bytes(packed[12:], "big")
        length = 28
        ipv6 = socket.inet_ntop(socket.AF_INET6, packed)
        if mapped or packed[:8] == NAT64[:8]:
            # a prefix `ip()` strips: a v4-mapped address on both sides, and a
            # NAT64 one on the port's
            ip = socket.inet_ntop(socket.AF_INET, packed[12:])
            if not mapped:
                ipv6 = WELL_KNOWN + ip
        else:
            ip = ipv6
    url = ("%s:%d" % (ip, port)) if family == "v4" else ("[%s]:%d" % (ipv6, port))

    # `valid_server_address(_allowloopback, _ignore_port)`, both false
    if family == "v4" or mapped:
        server = port != 0 and host not in (0, 0xFFFFFFFF, 0x7F000001)
    else:
        server = True  # the `// TODO` branch: a real v6 address is taken as it comes
    one = 1 if server else 0
    return "%s %s %d %d %d %d %d %d %d %d %d %d %s %s %s" % (
        family,
        packed.hex(),
        port,
        1,
        1 if (family == "v4" or mapped) else 0,
        1 if (family == "v6" and not mapped) else 0,
        1 if mapped else 0,
        length,
        one,
        1 if (family == "v4" and host == 0x7F000001) else 0,
        1 if (family == "v4" and host == 0xFFFFFFFF) else 0,
        1 if (family == "v4" and host == 0xFFFFFFFF and port != 0) else 0,
        ip,
        ipv6,
        url,
    )


def pton(text):
    return socket.inet_pton(socket.AF_INET6, text)


rows = []
for name, key, value, port, mapped, family, packed, text in [
    ("a-v4-address", "ip", "8.8.8.8", 53, "no", "v4", bytes([8, 8, 8, 8]), "yes"),
    ("the-loopback-address", "ip", "127.0.0.1", 80, "no", "v4", bytes([127, 0, 0, 1]), "yes"),
    ("the-broadcast-address", "ip", "255.255.255.255", 7, "no", "v4", bytes([255] * 4), "yes"),
    ("the-any-address", "ip", "0.0.0.0", 80, "no", "v4", bytes(4), "yes"),
    ("a-port-of-nothing", "ip", "1.2.3.4", 0, "no", "v4", bytes([1, 2, 3, 4]), "yes"),
    ("a-v6-address", "ip", "2001:db8::1", 443, "no", "v6", pton("2001:db8::1"), "yes"),
    ("the-v6-loopback-address", "ip", "::1", 80, "no", "v6", pton("::1"), "yes"),
    ("a-v4-address-written-as-a-mapped-one", "ip", "::ffff:1.2.3.4", 80, "no", "v6",
     V4_MAPPED + bytes([1, 2, 3, 4]), "yes"),
    ("bytes-of-a-v4-address", "v4", "01020304", 8080, "no", "v4", bytes([1, 2, 3, 4]), "yes"),
    ("bytes-of-a-mapped-address", "v6", (V4_MAPPED + bytes([1, 2, 3, 4])).hex(), 80, "no", "v6",
     V4_MAPPED + bytes([1, 2, 3, 4]), "yes"),
    ("a-v4-address-mapped", "ip", "1.2.3.4", 80, "yes", "v6", V4_MAPPED + bytes([1, 2, 3, 4]),
     "yes"),
    # `ip`, `ipv6` and `url` of a NAT64 address are the port's own reading, so
    # this case is in the table of the address and not in the one of the text
    ("a-nat64-address", "v6", (NAT64 + bytes([192, 0, 2, 1])).hex(), 80, "no", "v6",
     NAT64 + bytes([192, 0, 2, 1]), "no"),
    ("an-ip-that-is-neither", "ip", "300.1.1.1", 80, "no", "unspec", b"", "yes"),
]:
    row(name, key, value, port, mapped, text)
    expect(name, socket_line(family, packed, port))

with open(os.path.join(work, "socket.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")

# --- http: the request or the answer a caller writes and reads. ---------------
# The third implementation of the same head: a head is one entry per name, held
# in the order the C++'s `std::map` holds its entries — by name, and neither
# name's case in the way — and a name that is written twice keeps the case of
# the first time and the value of the last.
VERSIONS = ["HTTP/0.9", "HTTP/1.0", "HTTP/1.1", "HTTP/2"]
METHODS = ["GET", "POST", "OPTIONS", "HEAD", "PUT", "DELETE", "TRACE", "CONNECT"]
DIGITS = "0123456789"


def tokens(text):
    # `strutil::SplitToken` with " ": a run of the delimiter is one, and a
    # token of no characters does not come out of it.
    return [token for token in text.split(" ") if token]


def merge(fields):
    # one entry per name, and a name that is written twice keeps the case of
    # the first time and the value of the last
    order = []
    names = {}
    values = {}
    for name, value in fields:
        key = name.lower()
        if key not in values:
            order.append(key)
            names[key] = name
        values[key] = value
    return [(names[key], values[key]) for key in order]


def head_of(fields):
    return "".join("%s: %s\r\n" % it for it in sorted(merge(fields), key=lambda it: it[0].lower()))


def to_int(text):
    # what `strtol` reads: the digits at the front of a token
    sign, rest = (1, text)
    if rest[:1] in ("+", "-"):
        sign, rest = (-1 if rest[0] == "-" else 1), rest[1:]
    digits = ""
    for char in rest:
        if char not in DIGITS:
            break
        digits += char
    return sign * int(digits) if digits else 0


def to_uint(text):
    # what `strtoull` reads: the digits at the front of a token, `0` when
    # there are none, `UINT64_MAX` when they do not fit, and the wrap-around
    # of a sign in front of them — which is why a `Content-Length` of `-1` is
    # a body that cannot be read and not one of nothing
    rest = text.lstrip()
    negative = rest.startswith("-")
    rest = rest[1:] if negative else rest.lstrip("+")
    digits = ""
    for char in rest:
        if char not in DIGITS:
            break
        digits += char
    if not digits:
        return 0
    value = min(int(digits), 2**64 - 1)
    return (-value) % 2**64 if negative else value


def to_hex(text):
    # what `strtoull(text, NULL, 16)` reads out of the size line of a chunk:
    # the digits of base 16 at the front of it, and `0x` in front of them
    rest = text.strip()
    if rest[:2].lower() == "0x":
        rest = rest[2:]
    digits = ""
    for char in rest:
        if char not in "0123456789abcdefABCDEF":
            break
        digits += char
    return int(digits, 16) if digits else 0


def version_of(text):
    return text if text in VERSIONS else "version_unknown"


def method_of(text):
    return text if text in METHODS else "UNKNOWN"


def request_line(method, url, version):
    return "%s %s %s\r\n" % (method_of(method), url, version_of(version))


def status_line(version, code, reason):
    return "%s %d %s\r\n" % (version_of(version), code, reason)


def request_from(text):
    if "\r\n" not in text:
        return "refused"
    parts = tokens(text.split("\r\n")[0])
    # a line of fewer than three tokens is one the C++ asserts on, so no case
    # asks for one
    if len(parts) < 3 or parts[0] not in METHODS or parts[2] not in VERSIONS:
        return "refused"
    return "%s %s %s" % (parts[0], parts[1], parts[2])


def status_from(text):
    if "\r\n" not in text:
        return "refused"
    parts = tokens(text.split("\r\n")[0])
    if len(parts) < 2 or parts[0] not in VERSIONS:
        return "refused"
    reason = parts[2] if len(parts) == 3 else ""
    return "%s %d %s" % (parts[0], to_int(parts[1]), reason if reason else "-")


def fields_of(text):
    # `--set`, `--manipulate` and `--update`: a `|`-separated list of
    # `name:value`, and the first colon is the one that parts them.
    out = []
    if not text:
        return out
    for field in text.split("|"):
        name, colon, value = field.partition(":")
        if colon:
            out.append((name, value))
    return out


def manipulated(fields, text):
    # `Manipulate`: a value that is nothing but whitespace takes the field away
    out = list(fields)
    for name, value in fields_of(text):
        if value.strip() == "":
            out = [it for it in out if it[0].lower() != name.lower()]
        else:
            out.append((name, value))
    return out


def reads_of(text):
    values = {name.lower(): value for name, value in fields_of(text)}

    def field(name):
        return values.get(name)

    timeout = 5
    if field("connection") is not None:
        alive = field("keep-alive")
        if alive and "timeout=" in alive:
            for token in alive.split(","):
                if "timeout=" in token:
                    timeout = to_int(token.split("timeout=")[1])
                    break
            timeout = timeout if 0 < timeout < 60 else 5

    range_ = "-"
    whole = field("range")
    if whole is not None and whole.startswith("bytes="):
        rest = whole[6:].strip()
        if "-" in rest:
            start, _, end = rest.partition("-")
            range_ = "%d,%d" % (to_int(start), to_int(end))

    content_range = "-"
    whole = field("content-range")
    if whole is not None and whole.startswith("bytes "):
        rest = whole[6:].strip()
        if "-" in rest and "/" in rest:
            start, _, rest = rest.partition("-")
            end, _, total = rest.partition("/")
            content_range = "%d,%d,%d" % (to_uint(start), to_uint(end), to_uint(total))

    return "%d %d %d %d %d %s %s" % (to_uint(field("content-length") or ""),
                                     timeout,
                                     (field("transfer-encoding") or "").lower() == "chunked",
                                     (field("connection") or "").lower() == "close",
                                     (field("connection") or "").lower() == "keep-alive",
                                     range_,
                                     content_range)


def looks_of(text, name):
    for field_name, value in fields_of(text):
        if field_name.lower() == name.lower():
            return value if value else "-"
    return "-"


def build_of(mode, first, text, kind, body):
    fields = fields_of(text)
    if kind == "block":
        # a block body of no bytes writes nothing at all, not even the head
        if not body:
            return "ok", b""
        fields.append(("Content-Length", str(len(body))))
    elif kind == "chunks":
        fields.append(("Transfer-Encoding", "chunked"))
    if not fields:
        return "none", b""
    out = first.encode() + head_of(fields).encode() + b"\r\n" + body
    return "ok", out


# The bound a line of a head has, which is also the bound the port puts on the
# size line of a chunk: `MAX_CHUNK_SIZE_LINE` is `MAX_FIRST_LINE`, 8k.
MAX_LINE = 8 * 1024


def parse_of(raw, waits_for_the_socket=False, bounds_the_chunk_line=False):
    # what one `Recv` of a whole answer gives: the head is in it, and so is the
    # body the head told it how long to expect
    #
    # `waits_for_the_socket` is the port's answer and not the C++'s, and it is
    # about one thing: whose length a body with no `Content-Length` on a socket
    # the peer closes has. `bounds_the_chunk_line` is the same for the size
    # line of a chunk that never ends. It is a third implementation of both,
    # so that a case that asks for either is pinned on both sides and not just
    # diffed.
    default = status_line("HTTP/1.0", 0, "").encode().hex()
    if not raw:
        return "start 0 0 0 respond %s - -" % default

    crlf = raw.find(b"\r\n")
    if crlf < 0:
        # a first line that is not whole yet: nothing has been read out of it,
        # not even the mode, so what a caller asking for the line gets is the
        # one an answer starts with
        return "first-line 0 0 0 respond %s - -" % default

    crlf_crlf = raw.find(b"\r\n\r\n")
    first = raw[:crlf + 2].decode("latin-1")
    first_len = crlf + 2
    parts = tokens(first[:-2])
    if first.startswith("HTTP/"):
        mode = "respond"
        shown = status_line(parts[0], to_int(parts[1]), parts[2] if len(parts) == 3 else "")
    else:
        mode = "request"
        shown = request_line(parts[0], parts[1], parts[2])

    if crlf_crlf < 0:
        return "header-fields %d 0 0 %s %s - -" % (first_len, mode, shown.encode().hex())

    if crlf_crlf == crlf:
        # the first line is the whole head, so there is no field block to
        # measure and the body starts behind the line that ends it
        fields, header_len, rest = [], 0, raw[crlf + 4:]
    else:
        header_len = crlf_crlf + 4 - first_len
        fields = fields_of_blocks(raw[first_len:crlf_crlf + 4].decode("latin-1"))
        rest = raw[crlf_crlf + 4:]

    values = {name.lower(): value for name, value in fields}
    close = (values.get("connection") or "").lower() == "close"
    has_length = "content-length" in values
    length = to_uint(values.get("content-length") or "")

    if (values.get("transfer-encoding") or "").lower() == "chunked":
        body = b""
        status = "body"
        while True:
            size_end = rest.find(b"\r\n")
            if size_end < 0:
                # the size line is not whole yet. The C++ waits for the
                # `CRLF`, and `recvbuf_` is as long as the peer wrote; the
                # port refuses a line past the bound and ends the answer in
                # an error.
                if bounds_the_chunk_line and len(rest) > MAX_LINE:
                    status = "body-error"
                break
            size = to_hex(rest[:size_end].decode("latin-1"))
            begin = size_end + 2
            if size == 0:
                # the last chunk: a size of nothing, and then the trailer it
                # takes a `CRLF` of its own to end
                if len(rest) >= begin + 2 and rest.find(b"\r\n", begin) >= 0:
                    status = "end"
                break
            if len(rest) < begin + size + 2:
                break
            if rest[begin + size:begin + size + 2] != b"\r\n":
                # a chunk that is not the size its own line says it is
                status = "body-error"
                break
            body += rest[begin:begin + size]
            rest = rest[begin + size + 2:]
    elif close and not has_length:
        # a body the peer closes the socket at the end of: no field said how
        # long, so its length is the socket's. The C++ compares the body it has
        # to the `0` a field that is not there reads as, and ends the moment
        # its head is whole — with the bytes of the body still to come, which
        # is every head a socket hands over on its own. The port waits.
        body, status = rest, ("body" if rest or waits_for_the_socket else "end")
    elif close and length == 0:
        # a `Content-Length: 0` on a socket the peer closes, which the C++ asks
        # as `0 == contentLength` and reads as the branch above — a body the
        # socket is the length of — while the port ends it at the head. No case
        # feeds one in.
        body, status = (b"", "end") if waits_for_the_socket else (rest, "body")
    elif len(rest) <= length:
        body, status = rest, ("end" if len(rest) == length else "body")
    else:
        body, status = rest[:length], "end"

    return "%s %d %d %d %s %s %s %s" % (status,
                                        first_len,
                                        header_len,
                                        len(body),
                                        mode,
                                        shown.encode().hex(),
                                        head_of(fields).encode().hex() or "-",
                                        body.hex() or "-")


def fields_of_blocks(block):
    # `__ParserHeaders`: a line with no colon in it is a field of itself, a
    # line of nothing but colons is skipped, and so is the empty line the head
    # ends with.
    out = []
    for line in block.split("\r\n"):
        if line == "" or all(char == ":" for char in line):
            continue
        name, colon, value = line.partition(":")
        if not colon:
            out.append((line, line))
        elif len(line) > len(name) + 1:
            out.append((name.strip(), value.strip()))
    return out


rows = []
for name, kind, method, url, version, code, reason, data in [
    ("a-get-request", "request-to", "GET", "/", "HTTP/1.1", "-", "-", "-"),
    ("a-post-request", "request-to", "POST", "/cgi-bin/mars", "HTTP/1.1", "-", "-", "-"),
    ("a-request-of-no-url", "request-to", "GET", "-", "HTTP/1.0", "-", "-", "-"),
    ("a-request-of-http-2", "request-to", "GET", "/", "HTTP/2", "-", "-", "-"),
    ("a-version-that-is-not-one", "request-to", "GET", "/", "HTTP/3", "-", "-", "-"),
    ("a-method-that-is-not-one", "request-to", "FROB", "/", "HTTP/1.1", "-", "-", "-"),
    ("a-request-read-back", "request-from", "-", "-", "-", "-", "-", "GET /cgi HTTP/1.1\r\n"),
    ("a-line-with-no-crlf", "request-from", "-", "-", "-", "-", "-", "GET / HTTP/1.1"),
    ("a-line-of-an-unknown-method", "request-from", "-", "-", "-", "-", "-", "FROB / HTTP/1.1\r\n"),
    ("a-line-of-an-unknown-version", "request-from", "-", "-", "-", "-", "-", "GET / HTTP/3\r\n"),
    ("a-line-of-a-fourth-word", "request-from", "-", "-", "-", "-", "-", "GET /a b HTTP/1.1\r\n"),
    ("a-line-that-starts-blank", "request-from", "-", "-", "-", "-", "-", " GET / HTTP/1.1\r\n"),
    ("a-status-line", "status-to", "-", "-", "HTTP/1.1", "200", "OK", "-"),
    ("a-status-line-of-no-reason", "status-to", "-", "-", "HTTP/1.1", "404", "-", "-"),
    ("a-status-read-back", "status-from", "-", "-", "-", "-", "-", "HTTP/1.1 200 OK\r\n"),
    ("a-status-of-two-words", "status-from", "-", "-", "-", "-", "-", "HTTP/1.1 404 Not Found\r\n"),
    ("a-status-of-no-reason", "status-from", "-", "-", "-", "-", "-", "HTTP/1.1 204\r\n"),
    ("a-status-of-no-code", "status-from", "-", "-", "-", "-", "-", "HTTP/1.1 abc OK\r\n"),
    ("a-status-that-is-not-one", "status-from", "-", "-", "-", "-", "-", "GET / HTTP/1.1\r\n"),
]:
    row(name, kind, method, url, version, code, reason,
        # a line has a `CRLF` at the end of it, so the row carries it as hex
        "" if data == "-" else data.encode("latin-1").hex())
    if kind == "request-to":
        expect_of("first", name,
                  request_line(method, "" if url == "-" else url, version).encode().hex())
    elif kind == "status-to":
        expect_of("first", name,
                  status_line(version, int(code),
                              "" if reason == "-" else reason).encode().hex())
    elif kind == "request-from":
        expect_of("first", name, request_from(data))
    else:
        expect_of("first", name, status_from(data))

with open(os.path.join(work, "http-first.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")

rows = []
for name, set_, manipulate, update in [
    ("one-field", "Host:mars", "-", "-"),
    ("two-fields-out-of-order", "Host:mars|Accept:*/*", "-", "-"),
    ("a-name-of-another-case", "host:mars|Host:other", "-", "-"),
    ("a-value-that-is-changed", "Host:mars|Host:other", "-", "-"),
    ("a-value-of-nothing", "Host:", "-", "-"),
    ("a-manipulated-field", "Host:mars", "Host:other", "-"),
    ("a-field-manipulated-away", "Host:mars", "Host:", "-"),
    ("an-updated-field", "Host:mars", "-", "Host:other"),
    ("a-value-with-a-colon-in-it", "Location:http://mars/x", "-", "-"),
    ("a-field-of-every-kind", "Content-Length:12|Connection:keep-alive|Host:mars", "-", "-"),
    ("no-fields-at-all", "-", "-", "-"),
]:
    row(name, set_, manipulate, update)
    # `--set`, then `--manipulate`, then `--update`, and the head that comes
    # out of the three of them
    fields = merge(manipulated(fields_of(set_), manipulate) + fields_of(update))
    expect_of("fields", name,
              "%d %s" % (len(fields), head_of(fields).encode().hex() or "-"))

with open(os.path.join(work, "http-fields.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")

# --- http reads ---------------------------------------------------------------
rows = []
for name, set_ in [
    ("no-fields-at-all", "-"),
    ("a-content-length", "Content-Length:12"),
    ("a-content-length-that-is-not-one", "Content-Length:abc"),
    ("a-content-length-that-is-negative", "Content-Length:-1"),
    ("a-keep-alive-connection", "Connection:keep-alive"),
    ("a-keep-alive-of-a-timeout", "Connection:keep-alive|Keep-Alive:timeout=7"),
    ("a-timeout-behind-another", "Connection:keep-alive|Keep-Alive:max=100,timeout=7"),
    ("a-timeout-of-nothing", "Connection:keep-alive|Keep-Alive:timeout=0"),
    ("a-timeout-of-a-minute", "Connection:keep-alive|Keep-Alive:timeout=60"),
    ("a-timeout-that-is-not-one", "Connection:keep-alive|Keep-Alive:timeout=abc"),
    ("a-closed-connection", "Connection:close"),
    ("a-chunked-body", "Transfer-Encoding:chunked"),
    ("a-range", "Range:bytes=0-99"),
    ("a-range-that-is-not-one", "Range:bytes=0"),
    ("a-content-range", "Content-Range:bytes 0-99/100"),
    ("a-head-of-every-kind",
     "Connection:keep-alive|Content-Length:12|Keep-Alive:timeout=7|Range:bytes=0-99|Transfer-Encoding:chunked"),
]:
    row(name, set_)
    expect_of("reads", name, reads_of(set_))

with open(os.path.join(work, "http-reads.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")

# --- http looks ---------------------------------------------------------------
rows = []
for name, set_, key in [
    ("the-host", "Host:mars", "Host"),
    ("the-host-by-another-case", "Host:mars", "host"),
    ("a-field-that-is-not-there", "Host:mars", "Accept"),
    ("a-value-of-nothing", "Host:", "Host"),
    ("a-value-with-a-colon-in-it", "Location:http://mars/x", "Location"),
    ("the-connection", "Connection:keep-alive|Content-Length:12", "connection"),
    ("no-fields-at-all", "-", "Host"),
]:
    row(name, set_, key)
    expect_of("looks", name, looks_of(set_, key))

with open(os.path.join(work, "http-looks.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")

# --- http build: a request or an answer, written by both sides. ---------------
# `kind` is `none`, `block` or `chunks`, and `body` is the hex of the body — or
# of the framed chunks — which is `-` for one of no bytes.
CHUNKS = "340d0a6d6172730d0a300d0a0d0a"  # "4\r\nmars\r\n0\r\n\r\n"

rows = []
for name, mode, method, url, version, code, reason, set_, kind, body in [
    ("a-get-request", "request", "GET", "/", "HTTP/1.1", "-", "-", "Host:mars", "none", "-"),
    ("a-request-of-a-body", "request", "POST", "/cgi", "HTTP/1.1", "-", "-", "Host:mars",
     "block", "6d617273"),
    ("a-body-of-nothing", "request", "POST", "/cgi", "HTTP/1.1", "-", "-", "Host:mars",
     "block", "-"),
    ("a-request-of-chunks", "request", "POST", "/cgi", "HTTP/1.1", "-", "-", "Host:mars",
     "chunks", CHUNKS),
    ("a-request-of-no-fields", "request", "GET", "/", "HTTP/1.1", "-", "-", "-", "none", "-"),
    ("an-answer", "respond", "-", "-", "HTTP/1.1", "200", "OK", "Host:mars", "none", "-"),
    ("an-answer-of-a-body", "respond", "-", "-", "HTTP/1.1", "200", "OK", "Host:mars",
     "block", "6d617273"),
    ("an-answer-of-chunks", "respond", "-", "-", "HTTP/1.1", "404", "Not-Found", "Host:mars",
     "chunks", CHUNKS),
    ("an-answer-of-no-fields", "respond", "-", "-", "HTTP/1.0", "204", "-", "-", "none", "-"),
]:
    row(name, mode, method, url, version, code, reason, set_, kind, body)
    if mode == "request":
        first = request_line(method, "" if url == "-" else url, version)
    else:
        first = status_line(version, int(code), "" if reason == "-" else reason)
    payload = b"" if body == "-" else bytes.fromhex(body)
    answer, out = build_of(mode, first, "" if set_ == "-" else set_,
                           None if kind == "none" else kind, payload)
    expect_of("build", name, "%s %s" % (answer, out.hex() or "-"))
    bytes_of(name, out)
    # and the same bytes, read back by both sides
    expect(name, parse_of(out))

with open(os.path.join(work, "http-build.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")

# --- http parse: bytes neither side wrote, read by both. ----------------------
# `DIVERGENT` is a case the two sides answer differently, and it is pinned on
# each of them — see the two `http` answers at the top of this file.
DIVERGENT = {"a-closed-connection-of-nothing",
             "a-closed-connection-of-a-length-of-nothing",
             "a-chunk-size-line-that-never-ends"}
rows = []
for name, data in [
    # a name a case of another table has is not a name this one may have: the
    # two share one `exp-<name>.txt`
    ("an-answer-of-no-bytes", b""),
    ("a-first-line-that-is-not-whole", b"HTTP/1.1 200 O"),
    ("a-request-line-that-is-not-whole", b"GET / HTTP/1.1"),
    ("a-head-that-is-not-whole", b"HTTP/1.1 200 OK\r\nHost: mar"),
    ("a-whole-answer", b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nmars"),
    ("an-answer-of-a-body-of-nothing", b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"),
    ("a-body-that-is-short", b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nmars"),
    ("a-body-that-is-long", b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nmars"),
    ("a-closed-connection", b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nmars"),
    # the two answers the port does not share with the C++: how long a body on
    # a socket the peer closes is
    ("a-closed-connection-of-nothing", b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n"),
    ("a-closed-connection-of-a-length-of-nothing",
     b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 0\r\n\r\nmars"),
    ("a-chunked-body",
     b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nmars\r\n0\r\n\r\n"),
    ("a-chunk-that-is-not-the-size-it-says",
     b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nmar\r\n0\r\n\r\n"),
    ("a-chunked-body-that-is-not-whole",
     b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nmars"),
    # the third answer the port does not share with the C++: a size line the
    # peer writes and never ends, which is a line the C++ waits on for as
    # long as it is written and the port refuses past `MAX_LINE`
    ("a-chunk-size-line-that-never-ends",
     b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n" + b"0" * (MAX_LINE + 1)),
    ("a-request", b"GET / HTTP/1.1\r\nHost: mars\r\n\r\n"),
    ("a-field-of-no-colon", b"HTTP/1.1 200 OK\r\nmars\r\n\r\n"),
    ("a-field-of-colons", b"HTTP/1.1 200 OK\r\n:::\r\n\r\n"),
    ("two-fields-of-one-name", b"HTTP/1.1 200 OK\r\nHost: a\r\nhost: b\r\n\r\n"),
]:
    row(name)
    artifact(name, data)
    if name in DIVERGENT:
        # a case the two sides answer differently, pinned on both of them
        # rather than hidden: `compare_readings` asks each side for the answer
        # that is its own, and either one changing fails the case. Each flag
        # is the port's answer to one case and is asked of no other — the
        # chunked one never reaches the socket, and the two on a closed
        # socket never reach a chunk.
        expect_side("cpp", name, parse_of(data))
        expect_side("rust", name,
                    parse_of(data,
                             waits_for_the_socket=True,
                             bounds_the_chunk_line=True))
    else:
        expect(name, parse_of(data))

with open(os.path.join(work, "http-parse.txt"), "w") as f:
    f.write("\n".join(rows) + "\n")
PY

# --- adler32 -----------------------------------------------------------------
printf '### adler32\n\n| case | cross-read | expected |\n|---|---|---|\n'
while read -r name data seed; do
    if [ "$data" = "-" ]; then data=""; fi
    "$RUST" adler32 --data="$data" --seed="$seed" > "$WORK/$name-rust.txt"
    "$CPP" adler32 --data="$data" --seed="$seed" > "$WORK/$name-cpp.txt"

    if cmp -s "$WORK/$name-rust.txt" "$WORK/$name-cpp.txt"; then
        CROSS=ok
    else
        CROSS=FAILED
        FAILED=$((FAILED + 1))
        diff "$WORK/$name-rust.txt" "$WORK/$name-cpp.txt" || true
    fi

    EXPECTED=ok
    for side in rust cpp; do
        if ! cmp -s "$WORK/$name-$side.txt" "$WORK/exp-$name.txt"; then
            EXPECTED=FAILED
            FAILED=$((FAILED + 1))
            echo "$name: $side read $(cat "$WORK/$name-$side.txt")" >&2
            echo "$name: want $(cat "$WORK/exp-$name.txt")" >&2
        fi
    done

    printf '| %s | %s | %s |\n' "$name" "$CROSS" "$EXPECTED"
done < "$WORK/adler32.txt"

# --- base64 -------------------------------------------------------------------
printf '\n### base64\n\n| case | cross-read | expected |\n|---|---|---|\n'
while read -r name data; do
    if [ "$data" = "-" ]; then data=""; fi

    "$RUST" base64 --data="$data" > "$WORK/$name-rust-base64.txt"
    "$CPP" base64 --data="$data" > "$WORK/$name-cpp-base64.txt"
    compare_answer "$name" base64
    printf '| %s | %s | %s |\n' "$name" "$CROSS" "$EXPECTED"
done < "$WORK/base64.txt"

# --- packer pack -------------------------------------------------------------
printf '\n### packer pack\n\n| case | bytes | cross-read | expected |\n|---|---|---|---|\n'
while read -r name url seq data hash; do
    if [ "$url" = "-" ]; then url=""; fi
    if [ "$data" = "-" ]; then data=""; fi

    "$RUST" packer pack --url="$url" --seq="$seq" --data="$data" --hash="$hash" \
        --out="$WORK/$name-rust.bin"
    "$CPP" packer pack --url="$url" --seq="$seq" --data="$data" --hash="$hash" \
        --out="$WORK/$name-cpp.bin"

    BYTES=ok
    if ! cmp -s "$WORK/$name-rust.bin" "$WORK/$name-cpp.bin"; then
        BYTES=FAILED
        FAILED=$((FAILED + 1))
        cmp "$WORK/$name-rust.bin" "$WORK/$name-cpp.bin" || true
    fi
    for side in rust cpp; do
        if ! cmp -s "$WORK/$name-$side.bin" "$WORK/exp-$name.bin"; then
            BYTES="$BYTES/expected"
            FAILED=$((FAILED + 1))
            cmp "$WORK/exp-$name.bin" "$WORK/$name-$side.bin" || true
        fi
    done

    # Each side reads the package the other wrote, and its own.
    compare_readings "$name" "packer unpack" \
        "$WORK/$name-rust.bin" "$WORK/$name-cpp.bin"
    printf '| %s | %s | %s | %s |\n' "$name" "$BYTES" "$CROSS" "$EXPECTED"
done < "$WORK/packer-pack.txt"

# --- packer unpack -----------------------------------------------------------
printf '\n### packer unpack\n\n| case | cross-read | expected |\n|---|---|---|\n'
while read -r name; do
    compare_readings "$name" "packer unpack" "$WORK/in-$name.bin"
    printf '| %s | %s | %s |\n' "$name" "$CROSS" "$EXPECTED"
done < "$WORK/packer-unpack.txt"

# --- simple pack -------------------------------------------------------------
printf '\n### simple pack\n\n| case | bytes | cross-read | expected |\n|---|---|---|---|\n'
while read -r name kind data; do
    if [ "$data" = "-" ]; then data=""; fi

    "$RUST" simple pack --kind="$kind" --data="$data" --out="$WORK/$name-rust.bin"
    "$CPP" simple pack --kind="$kind" --data="$data" --out="$WORK/$name-cpp.bin"

    BYTES=ok
    if ! cmp -s "$WORK/$name-rust.bin" "$WORK/$name-cpp.bin"; then
        BYTES=FAILED
        FAILED=$((FAILED + 1))
        cmp "$WORK/$name-rust.bin" "$WORK/$name-cpp.bin" || true
    fi
    for side in rust cpp; do
        if ! cmp -s "$WORK/$name-$side.bin" "$WORK/exp-$name.bin"; then
            BYTES="$BYTES/expected"
            FAILED=$((FAILED + 1))
            cmp "$WORK/exp-$name.bin" "$WORK/$name-$side.bin" || true
        fi
    done

    compare_readings "$name" "simple unpack --kind=$kind" \
        "$WORK/$name-rust.bin" "$WORK/$name-cpp.bin"
    printf '| %s | %s | %s | %s |\n' "$name" "$BYTES" "$CROSS" "$EXPECTED"
done < "$WORK/simple-pack.txt"

# --- simple unpack -----------------------------------------------------------
printf '\n### simple unpack\n\n| case | cross-read | expected |\n|---|---|---|\n'
while read -r name kind; do
    compare_readings "$name" "simple unpack --kind=$kind" "$WORK/in-$name.bin"
    printf '| %s | %s | %s |\n' "$name" "$CROSS" "$EXPECTED"
done < "$WORK/simple-unpack.txt"

# --- strutil -----------------------------------------------------------------
# Nothing is written here: what a case compares is the one line each side
# answers, against the other's and against the one Python wrote. `--data` and
# `--arg` are hex, so a byte the shell would eat — a tab, a newline, a
# backslash — is still one a case can name.
printf '\n### strutil\n\n| case | cross-read | expected |\n|---|---|---|\n'
while read -r name fn data arg pos; do
    if [ "$data" = "-" ]; then data=""; fi
    if [ "$arg" = "-" ]; then arg=""; fi
    if [ "$pos" = "-" ]; then pos=0; fi

    "$RUST" strutil "$fn" --data="$data" --arg="$arg" --pos="$pos" > "$WORK/$name-rust.txt"
    "$CPP" strutil "$fn" --data="$data" --arg="$arg" --pos="$pos" > "$WORK/$name-cpp.txt"

    if cmp -s "$WORK/$name-rust.txt" "$WORK/$name-cpp.txt"; then
        CROSS=ok
    else
        CROSS=FAILED
        FAILED=$((FAILED + 1))
        diff "$WORK/$name-rust.txt" "$WORK/$name-cpp.txt" || true
    fi

    EXPECTED=ok
    for side in rust cpp; do
        if ! cmp -s "$WORK/$name-$side.txt" "$WORK/exp-$name.txt"; then
            EXPECTED=FAILED
            FAILED=$((FAILED + 1))
            echo "$name: $side answered $(cat "$WORK/$name-$side.txt")" >&2
            echo "$name: want $(cat "$WORK/exp-$name.txt")" >&2
        fi
    done

    printf '| %s | %s | %s |\n' "$name" "$CROSS" "$EXPECTED"
done < "$WORK/strutil.txt"

# --- socket ------------------------------------------------------------------
# One address per case, and the two tables read the two ends of the line it
# answers with: the address itself — the family, the bytes, the port, the four
# `valid_*`, the three `is*` and the length — and then the three strings. The
# second table is the one a NAT64 address is not in, because there the strings
# are the port's own reading and not the C++'s (see the head of this file).
printf '\n### socket address\n\n| case | cross-read | expected |\n|---|---|---|\n'
while read -r name key value port map text; do
    set -- "--$key=$value" "--port=$port"
    if [ "$map" = yes ]; then set -- "$@" --map=yes; fi

    "$RUST" socket "$@" > "$WORK/$name-rust.txt"
    "$CPP" socket "$@" > "$WORK/$name-cpp.txt"
    compare_line "$name" 1-12
    printf '| %s | %s | %s |\n' "$name" "$CROSS" "$EXPECTED"
done < "$WORK/socket.txt"

printf '\n### socket text\n\n| case | cross-read | expected |\n|---|---|---|\n'
while read -r name key value port map text; do
    if [ "$text" = no ]; then continue; fi
    compare_line "$name" 13-15
    printf '| %s | %s | %s |\n' "$name" "$CROSS" "$EXPECTED"
done < "$WORK/socket.txt"

# --- http ---------------------------------------------------------------------
# The head of a request or of an answer, and then the whole of one: the first
# line, the fields, what a caller reads out of a head, one field, the bytes a
# `Builder` writes, and the reading a `Parser` makes of them. The last two
# tables are the cross-read proper — each side parses the answer the other
# wrote — and a `### http parse` case is a wire neither side wrote at all.
printf '\n### http first line\n\n| case | cross-read | expected |\n|---|---|---|\n'
while read -r name kind method url version code reason data; do
    if [ "$method" = "-" ]; then method=""; fi
    if [ "$url" = "-" ]; then url=""; fi
    if [ "$reason" = "-" ]; then reason=""; fi
    if [ "$code" = "-" ]; then code=0; fi
    if [ "$data" = "-" ]; then data=""; fi

    # a case is either a line the two sides write or one they read, and the
    # action of a case that reads one is `--data`, which no other case passes
    case $kind in
        request-to) set -- http request-line --method="$method" --url="$url" --version="$version" ;;
        status-to) set -- http status-line --version="$version" --code="$code" --reason="$reason" ;;
        request-from) set -- http request-line --data="$data" ;;
        *) set -- http status-line --data="$data" ;;
    esac

    "$RUST" "$@" > "$WORK/$name-rust-first.txt"
    "$CPP" "$@" > "$WORK/$name-cpp-first.txt"
    compare_answer "$name" first
    printf '| %s | %s | %s |\n' "$name" "$CROSS" "$EXPECTED"
done < "$WORK/http-first.txt"

printf '\n### http fields\n\n| case | cross-read | expected |\n|---|---|---|\n'
while read -r name set manipulate update; do
    if [ "$set" = "-" ]; then set=""; fi
    if [ "$manipulate" = "-" ]; then manipulate=""; fi
    if [ "$update" = "-" ]; then update=""; fi

    "$RUST" http fields --set="$set" --manipulate="$manipulate" --update="$update" \
        > "$WORK/$name-rust-fields.txt"
    "$CPP" http fields --set="$set" --manipulate="$manipulate" --update="$update" \
        > "$WORK/$name-cpp-fields.txt"
    compare_answer "$name" fields
    printf '| %s | %s | %s |\n' "$name" "$CROSS" "$EXPECTED"
done < "$WORK/http-fields.txt"

printf '\n### http reads\n\n| case | cross-read | expected |\n|---|---|---|\n'
while read -r name set; do
    if [ "$set" = "-" ]; then set=""; fi

    "$RUST" http reads --set="$set" > "$WORK/$name-rust-reads.txt"
    "$CPP" http reads --set="$set" > "$WORK/$name-cpp-reads.txt"
    compare_answer "$name" reads
    printf '| %s | %s | %s |\n' "$name" "$CROSS" "$EXPECTED"
done < "$WORK/http-reads.txt"

printf '\n### http looks\n\n| case | cross-read | expected |\n|---|---|---|\n'
while read -r name set key; do
    if [ "$set" = "-" ]; then set=""; fi

    "$RUST" http looks --set="$set" --name="$key" > "$WORK/$name-rust-looks.txt"
    "$CPP" http looks --set="$set" --name="$key" > "$WORK/$name-cpp-looks.txt"
    compare_answer "$name" looks
    printf '| %s | %s | %s |\n' "$name" "$CROSS" "$EXPECTED"
done < "$WORK/http-looks.txt"

printf '\n### http build\n\n| case | bytes | cross-read | expected |\n|---|---|---|---|\n'
while read -r name mode method url version code reason set kind body; do
    if [ "$method" = "-" ]; then method=""; fi
    if [ "$url" = "-" ]; then url=""; fi
    if [ "$reason" = "-" ]; then reason=""; fi
    if [ "$code" = "-" ]; then code=0; fi
    if [ "$set" = "-" ]; then set=""; fi
    if [ "$body" = "-" ]; then body=""; fi

    set -- --mode="$mode" --method="$method" --url="$url" --version="$version" \
        --code="$code" --reason="$reason" --set="$set"
    # a body of no bytes is still a body of its kind, so `--body` and
    # `--chunks` are passed empty rather than not at all
    if [ "$kind" = block ]; then set -- "$@" --body="$body"; fi
    if [ "$kind" = chunks ]; then set -- "$@" --chunks="$body"; fi

    "$RUST" http build "$@" --out="$WORK/$name-rust.bin" > "$WORK/$name-rust-build.txt"
    "$CPP" http build "$@" --out="$WORK/$name-cpp.bin" > "$WORK/$name-cpp-build.txt"

    BYTES=ok
    if ! cmp -s "$WORK/$name-rust.bin" "$WORK/$name-cpp.bin"; then
        BYTES=FAILED
        FAILED=$((FAILED + 1))
        cmp "$WORK/$name-rust.bin" "$WORK/$name-cpp.bin" || true
    fi
    for side in rust cpp; do
        if ! cmp -s "$WORK/$name-$side.bin" "$WORK/exp-$name.bin"; then
            BYTES="$BYTES/expected"
            FAILED=$((FAILED + 1))
            cmp "$WORK/exp-$name.bin" "$WORK/$name-$side.bin" || true
        fi
    done

    compare_answer "$name" build
    printf '| %s | %s | %s | %s |\n' "$name" "$BYTES" "$CROSS" "$EXPECTED"
done < "$WORK/http-build.txt"

# The same answers, read back: each side parses the bytes the other wrote, and
# its own.
printf '\n### http build, read back\n\n| case | cross-read | expected |\n|---|---|---|\n'
while read -r name rest; do
    compare_readings "$name" "http parse" \
        "$WORK/$name-rust.bin" "$WORK/$name-cpp.bin"
    printf '| %s | %s | %s |\n' "$name" "$CROSS" "$EXPECTED"
done < "$WORK/http-build.txt"

printf '\n### http parse\n\n| case | cross-read | expected |\n|---|---|---|\n'
while read -r name; do
    compare_readings "$name" "http parse" "$WORK/in-$name.bin"
    printf '| %s | %s | %s |\n' "$name" "$CROSS" "$EXPECTED"
done < "$WORK/http-parse.txt"

if [ "$FAILED" -ne 0 ]; then
    echo "$FAILED check(s) failed" >&2
    exit 1
fi
echo "every comm cross-read check passed; the files are in $WORK"
