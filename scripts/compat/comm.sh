#!/bin/sh
# The comm cross-read test: the wire formats of `mars/comm`, read by both sides.
#
#   sh scripts/compat/comm.sh
#
# `stn.sh` proves the long link's wire format, `shortlink.sh` the short link's
# and `sdt.sh` the URL of an HTTP check. This one proves the four that sit under
# all of them: `mars/comm/basepacker.cc` — the package the long link spoke
# before `longlink_packer.cc` took it over, and the `Simple*` pair of a length
# and a body — `mars/comm/adler32.c`, the hash the first one puts over its URL
# and its body, `mars/comm/strutil.cc`, the string helpers every one of them
# spells its URL with, and `mars/comm/socket/socket_address.cc`, the address a
# caller connects to.
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

if [ ! -x "$CPP" ]; then
    MARS_SRC="$REPO/scripts/compat/upstream_comm.cpp" \
    MARS_SRCS="$UP/mars/comm/socket/socket_address.cc \
               $UP/mars/comm/basepacker.cc \
               $UP/mars/comm/autobuffer.cc \
               $UP/mars/comm/ptrbuffer.cc \
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
compare_readings() {
    _name=$1
    _subcmd=$2
    shift 2
    CROSS=ok
    EXPECTED=ok
    normalize "$WORK/exp-$_name.txt" > "$WORK/$_name-expected.txt"
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
            if [ "$_side$_at" != "rust1" ] && ! cmp -s "$WORK/$_name-$_side-$_at-norm.txt" "$WORK/$_name-rust-1-norm.txt"; then
                CROSS=FAILED
                FAILED=$((FAILED + 1))
                diff "$WORK/$_name-rust-1-norm.txt" "$WORK/$_name-$_side-$_at-norm.txt" || true
            fi
            if ! cmp -s "$WORK/$_name-$_side-$_at-norm.txt" "$WORK/$_name-expected.txt"; then
                EXPECTED=FAILED
                FAILED=$((FAILED + 1))
                echo "$_name $_side: read $(cat "$WORK/$_name-$_side-$_at-norm.txt")" >&2
                echo "$_name $_side: want $(cat "$WORK/$_name-expected.txt")" >&2
            fi
        done
    done
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

if [ "$FAILED" -ne 0 ]; then
    echo "$FAILED check(s) failed" >&2
    exit 1
fi
echo "every comm cross-read check passed; the files are in $WORK"
