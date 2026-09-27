#!/bin/sh
# Builds the C++ half of the cross-read test: upstream's own decoder, and the
# encoder in `upstream_encode.cpp` that mirrors `xlog-compat encode`.
#
#   sh scripts/compat/upstream_compat.sh [out-dir]
#
# Both land in `out-dir` (default `target/compat`). Upstream is the checkout
# `scripts/compat/upstream_build.sh` clones into `target/upstream` — nothing is
# vendored into this repository — and `MARS_UPSTREAM_DIR` overrides it.
#
# The decoder is `mars/xlog/crypt/decode_log_file_c_impl/decode_log_file.c`,
# which is the C++ project's own reader. It carries its ECDH keys as two
# compile-time constants, so the copy that is built here is patched with the
# keys `crates/mars-compat/fixtures/manifest.json` holds: the pair the golden
# files were encrypted with, and the pair the Rust decoder is handed.
set -e

REPO=$(cd "$(dirname "$0")/../.." && pwd)
OUTDIR=${1:-$REPO/target/compat}
UP=${MARS_UPSTREAM_DIR:-$REPO/target/upstream/Tencent-mars}
MANIFEST=$REPO/crates/mars-compat/fixtures/manifest.json
DECODER_DIR=$UP/mars/xlog/crypt/decode_log_file_c_impl

if [ ! -d "$UP/mars/xlog" ]; then
    MARS_UPSTREAM_DIR=$UP sh "$REPO/scripts/compat/upstream_build.sh" /dev/null > /dev/null
fi

mkdir -p "$OUTDIR"

# The two keys out of the manifest, the way `tests/golden.rs` reads them.
PRIVKEY=$(python3 -c "import json,sys;print(json.load(open('$MANIFEST'))['privkey'])")
PUBKEY=$(python3 -c "import json,sys;print(json.load(open('$MANIFEST'))['pubkey'])")

INC="-I$UP -I$UP/mars -I$UP/mars/xlog/crypt/decode_log_file_c_impl"
# `zstd` is a Homebrew prefix away on macOS; zlib is the system's.
if [ -d /opt/homebrew/lib ]; then
    LIBS="-L/opt/homebrew/lib -lz -lzstd"
else
    LIBS="-lz -lzstd"
fi

# A patched copy: `PRIV_KEY` / `PUB_KEY` are `const char*` initialised to `""`
# at file scope, which no `-D` can override, and the upstream tree is not to be
# edited in place.
#
# The second line is not a choice: `zstdDecompress` reads `lastPos` at
# `decode_log_file.c:212` and declares it nowhere, so the file does not compile
# as it stands. It is one `size_t`, and the loop only ever compares the
# position it reached against the one it reached before.
SRC=$OUTDIR/decode_log_file.c
sed -e "s|^const char\* PRIV_KEY = \"\";|const char* PRIV_KEY = \"$PRIVKEY\";|" \
    -e "s|^const char\* PUB_KEY = \"\";|const char* PUB_KEY = \"$PUBKEY\";|" \
    -e "198s|    bool done = false;|    bool done = false;\n    size_t lastPos = 0;|" \
    "$DECODER_DIR/decode_log_file.c" > "$SRC"
grep -q "PRIV_KEY = \"$PRIVKEY\"" "$SRC" || {
    echo "could not patch the decoder's keys — did upstream move them?" >&2
    exit 1
}
grep -q "size_t lastPos = 0;" "$SRC" || {
    echo "upstream declares lastPos now — the line-198 patch can go" >&2
}

OBJ=$(mktemp -d)
trap 'rm -rf "$OBJ"' EXIT

cc -O2 -w $INC -c "$SRC" -o "$OBJ/decode.o"
cc -O2 -w $INC -c "$DECODER_DIR/micro-ecc-master/uECC.c" -o "$OBJ/uecc.o"
cc -o "$OUTDIR/upstream_decode" "$OBJ"/decode.o "$OBJ"/uecc.o $LIBS

# The encoder is built by the same script that clones upstream, so both C++
# binaries come out of the same upstream sources.
sh "$REPO/scripts/compat/upstream_build.sh" "$OUTDIR/upstream_encode" > /dev/null

echo "$OUTDIR/upstream_encode"
echo "$OUTDIR/upstream_decode"
