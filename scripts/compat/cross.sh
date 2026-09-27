#!/bin/sh
# The cross-read test: every `.xlog` one implementation writes has to be read,
# decrypted and used by the other, in both directions.
#
#   sh scripts/compat/cross.sh
#
# Two layers, because they prove different things:
#
#   * the record layer — the 16 combinations of `fixtures/manifest.json`
#     (zlib/zstd x sync/async x crypt on/off x flush policy), encoded by one
#     side and decoded by the other, byte for byte.
#   * the appender layer — the same matrix through the real `XloggerAppender`
#     on the C++ side and `appender_open`/`appender_write` on the Rust one, so
#     what is decoded is a `<prefix>_YYYYMMDD.xlog` and not just block bytes.
#
# Both halves of the C++ side are built by `upstream_compat.sh`, which clones
# Tencent/mars into `target/upstream` on first use — nothing is vendored into
# this repository. The decoder is upstream's own
# `mars/xlog/crypt/decode_log_file_c_impl/decode_log_file.c`.
set -e

REPO=$(cd "$(dirname "$0")/../.." && pwd)
FIX=$REPO/crates/marsrs-compat/fixtures
OUT=$REPO/target/compat
WORK=$OUT/cross
MANIFEST=$FIX/manifest.json

PRIVKEY=$(python3 -c "import json;print(json.load(open('$MANIFEST'))['privkey'])")
PUBKEY=$(python3 -c "import json;print(json.load(open('$MANIFEST'))['pubkey'])")

cargo build --manifest-path "$REPO/Cargo.toml" -p marsrs-compat --release
cargo build --manifest-path "$REPO/Cargo.toml" -p marsrs-appender --release \
    --example xlog_file

if [ ! -x "$OUT/upstream_encode" ] || [ ! -x "$OUT/upstream_decode" ]; then
    sh "$REPO/scripts/compat/upstream_compat.sh" "$OUT" > /dev/null
fi

rm -rf "$WORK"
mkdir -p "$WORK"
python3 -c "
import json
manifest = json.load(open('$MANIFEST'))
for case in manifest['cases']:
    print(case['name'], case['mode'], case['compress'], case['sync'],
          case['crypt'], case['flush_every'], case['file'])" > "$WORK/cases.txt"

FAILED=0
printf '| case | rust -> cpp | cpp -> rust |\n|---|---|---|\n'

while read -r name mode compress sync crypt flush_every file; do
    # `--pubkey` empty is "no server key", i.e. the no-crypt magics.
    if [ "$crypt" = 1 ]; then
        KEY="--pubkey=$PUBKEY"
    else
        KEY="--pubkey="
    fi

    # 1. the Rust encoder's file, decoded by upstream's own decoder.
    "$REPO/target/release/xlog-compat" encode --mode="$mode" --compress="$compress" \
        --sync="$sync" --flush-every="$flush_every" $KEY \
        --records="$FIX/inputs.bin" --out="$WORK/$name-rust.xlog" > /dev/null
    "$OUT/upstream_decode" "$WORK/$name-rust.xlog" "$WORK/$name-rust.plain"
    if python3 "$REPO/scripts/compat/check.py" exact "$FIX/expected.bin" \
        "$WORK/$name-rust.plain" > "$WORK/$name-rust.diff"; then
        RUST_CPP=ok
    else
        # The control: the same scenario's golden file — written by the C++
        # encoders — through the C++ decoder. Two things have to hold before
        # "the decoder cannot read this shape" is a verdict and not an excuse:
        # the golden file has to fail too, and the two wrong outputs have to be
        # the *same* wrong bytes. Without the second one, a Rust file that
        # decodes to something else — nothing at all, a prefix, differently
        # corrupted bytes — is waved through on the strength of a case the
        # decoder was already known to fail.
        "$OUT/upstream_decode" "$FIX/$file" "$WORK/$name-control.plain"
        if python3 "$REPO/scripts/compat/check.py" exact "$FIX/expected.bin" \
            "$WORK/$name-control.plain" > /dev/null; then
            RUST_CPP=FAILED
            FAILED=$((FAILED + 1))
            cat "$WORK/$name-rust.diff"
        elif python3 "$REPO/scripts/compat/check.py" exact \
            "$WORK/$name-control.plain" "$WORK/$name-rust.plain" \
            > "$WORK/$name-control.diff"; then
            RUST_CPP="cpp-decoder"
            echo "$name: upstream's decoder reads both files the same wrong" \
                "way, so this is not a difference between the encoders" >&2
        else
            RUST_CPP=FAILED
            FAILED=$((FAILED + 1))
            cat "$WORK/$name-rust.diff"
            cat "$WORK/$name-control.diff"
        fi
    fi

    # 2. upstream's file, decoded by the Rust decoder.
    "$OUT/upstream_encode" --mode="$mode" --compress="$compress" --sync="$sync" \
        --flush-every="$flush_every" $KEY \
        --records="$FIX/inputs.bin" --out="$WORK/$name-cpp.xlog" > /dev/null
    "$REPO/target/release/xlog-compat" decode --privkey="$PRIVKEY" \
        --in="$WORK/$name-cpp.xlog" --out="$WORK/$name-cpp.plain" > /dev/null
    if python3 "$REPO/scripts/compat/check.py" exact "$FIX/expected.bin" \
        "$WORK/$name-cpp.plain" > "$WORK/$name-cpp.diff"; then
        CPP_RUST=ok
    else
        CPP_RUST=FAILED
        FAILED=$((FAILED + 1))
        cat "$WORK/$name-cpp.diff"
    fi

    printf '| %s | %s | %s |\n' "$name" "$RUST_CPP" "$CPP_RUST"
done < "$WORK/cases.txt"

# The appender layer: the real thing, both directions.
printf '\n| appender | rust -> cpp | cpp -> rust |\n|---|---|---|\n'
for mode in zlib zstd; do
    for sync in 1 0; do
        for crypt in 0 1; do
            if [ "$crypt" = 1 ]; then
                KEY="$PUBKEY"
            else
                KEY=""
            fi
            tag="$mode-sync$sync-crypt$crypt"

            RUST_DIR="$WORK/app-rust-$tag"
            CPP_DIR="$WORK/app-cpp-$tag"
            mkdir -p "$RUST_DIR" "$CPP_DIR"

            # Rust writes, upstream's decoder reads. `--mode` and `--sync` are
            # what make this row the row it says it is: the example defaults to
            # a synchronous zlib appender otherwise.
            "$REPO/target/release/examples/xlog_file" --mode="$mode" \
                --sync="$sync" "$RUST_DIR" "$FIX/inputs.bin" "$KEY" \
                > "$WORK/app-rust-$tag.txt"
            RUST_CPP=ok
            for file in $(cat "$WORK/app-rust-$tag.txt"); do
                "$OUT/upstream_decode" "$file" "$file.plain"
                python3 "$REPO/scripts/compat/check.py" lines \
                    "$FIX/inputs.bin" "$file.plain" || RUST_CPP=FAILED
            done

            # Upstream writes, the Rust decoder reads.
            "$OUT/upstream_encode" --appender=1 --mode="$mode" --sync="$sync" \
                --pubkey="$KEY" --records="$FIX/inputs.bin" \
                --out="$CPP_DIR" > "$WORK/app-cpp-$tag.txt"
            CPP_RUST=ok
            for file in $(cat "$WORK/app-cpp-$tag.txt"); do
                "$REPO/target/release/xlog-compat" decode --privkey="$PRIVKEY" \
                    --in="$file" --out="$file.plain" > /dev/null
                python3 "$REPO/scripts/compat/check.py" lines \
                    "$FIX/inputs.bin" "$file.plain" || CPP_RUST=FAILED
            done

            [ "$RUST_CPP" = ok ] || FAILED=$((FAILED + 1))
            [ "$CPP_RUST" = ok ] || FAILED=$((FAILED + 1))
            printf '| %s | %s | %s |\n' "$tag" "$RUST_CPP" "$CPP_RUST"
        done
    done
done

if [ "$FAILED" -ne 0 ]; then
    echo "$FAILED check(s) failed" >&2
    exit 1
fi
echo "every cross-read check passed; the files are in $WORK"
