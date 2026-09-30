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
#     side and decoded by the other, byte for byte. The eight rows nothing
#     encrypts are held to more than that: their two files have to be the same
#     bytes, because upstream's zlib and zstd are the ones the C++ build links.
#     A crypt row cannot be — its salt and its key are new every run.
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
# A manifest of no case is a run that compared nothing: `FAILED` would stay at
# zero over a table of no rows, which is the same answer a run of sixteen
# passing cases gives.
[ -s "$WORK/cases.txt" ] || {
    echo "crates/marsrs-compat/fixtures/manifest.json names no case" >&2
    exit 1
}

FAILED=0
# The rows that compared nothing instead of failing: see the `cpp-decoder` arm.
# Counted so that the run's last line cannot say "every check passed" over a
# table with a row in it that was excused.
CPP_DECODER=0
printf '| case | rust -> cpp | cpp -> rust | bytes |\n|---|---|---|---|\n'

while read -r name mode compress sync crypt flush_every file; do
    # `--pubkey` empty is "no server key", i.e. the no-crypt magics.
    if [ "$crypt" = 1 ]; then
        KEY="--pubkey=$PUBKEY"
    else
        KEY="--pubkey="
    fi

    # 0. The C++ encoder's file of the scenario, written first: it is both what
    # the Rust decoder reads in step 2 and the control of step 1.
    "$OUT/upstream_encode" --mode="$mode" --compress="$compress" \
        --sync="$sync" --flush-every="$flush_every" $KEY \
        --records="$FIX/inputs.bin" --out="$WORK/$name-cpp.xlog" > /dev/null

    # 1. the Rust encoder's file, decoded by upstream's own decoder.
    "$REPO/target/release/xlog-compat" encode --mode="$mode" --compress="$compress" \
        --sync="$sync" --flush-every="$flush_every" $KEY \
        --records="$FIX/inputs.bin" --out="$WORK/$name-rust.xlog" > /dev/null
    "$OUT/upstream_decode" "$WORK/$name-rust.xlog" "$WORK/$name-rust.plain"
    if python3 "$REPO/scripts/compat/check.py" exact "$FIX/expected.bin" \
        "$WORK/$name-rust.plain" > "$WORK/$name-rust.diff"; then
        RUST_CPP=ok
    else
        # The control: the C++ encoder's own file of the same scenario, written
        # in step 0, through the C++ decoder. Two things have to hold before
        # "the decoder cannot read this shape" is a verdict and not an excuse:
        # that file has to fail too, and the two wrong outputs have to be the
        # *same* wrong bytes. Without the second one, a Rust file that decodes
        # to something else — nothing at all, a prefix, differently corrupted
        # bytes — is waved through on the strength of a case the decoder was
        # already known to fail.
        #
        # The golden files of `fixtures/` are not the control: they are what
        # `tests/golden.rs` reads, and they were written by an older upstream
        # build — the zstd ones come out a byte longer than the one this script
        # builds today — so a control built on them would hold the Rust encoder
        # to a build it never had.
        "$OUT/upstream_decode" "$WORK/$name-cpp.xlog" "$WORK/$name-control.plain"
        if python3 "$REPO/scripts/compat/check.py" exact "$FIX/expected.bin" \
            "$WORK/$name-control.plain" > /dev/null; then
            RUST_CPP=FAILED
            FAILED=$((FAILED + 1))
            cat "$WORK/$name-rust.diff"
        elif [ -s "$WORK/$name-control.plain" ] && python3 "$REPO/scripts/compat/check.py" exact \
            "$WORK/$name-control.plain" "$WORK/$name-rust.plain" \
            > "$WORK/$name-control.diff"; then
            # An empty control is not "upstream reads both files the same
            # way", it is upstream's decoder having written nothing at all —
            # and two empty files compare equal, so without the `-s` the row
            # is excused for a comparison that never happened.
            RUST_CPP="cpp-decoder"
            CPP_DECODER=$((CPP_DECODER + 1))
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

    # 3. The two files themselves, for the rows nothing encrypts: upstream's
    # zlib and zstd are the very ones the C++ build links, so "the Rust encoder
    # writes what the C++ writes" is a claim about bytes and not about
    # plaintext. A crypt row is random per run — the salt and the key of every
    # block — so there the two files cannot be the same bytes and the
    # plaintext of step 2 is the whole of the answer.
    if [ "$crypt" = 0 ]; then
        if cmp -s "$WORK/$name-rust.xlog" "$WORK/$name-cpp.xlog"; then
            BYTES=identical
        else
            BYTES=FAILED
            FAILED=$((FAILED + 1))
            cmp "$WORK/$name-rust.xlog" "$WORK/$name-cpp.xlog" || true
        fi
    else
        BYTES="random"
    fi

    printf '| %s | %s | %s | %s |\n' "$name" "$RUST_CPP" "$CPP_RUST" "$BYTES"
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
            # A row that names no file is a row that compared nothing: the
            # loop over an empty list leaves `ok` where it is, and `ok` is a
            # claim about a comparison that ran.
            files="$(cat "$WORK/app-rust-$tag.txt")"
            if [ -z "$files" ]; then
                echo "$tag: the Rust appender named no file to decode" >&2
                RUST_CPP=FAILED
            else
                RUST_CPP=ok
            fi
            for file in $files; do
                "$OUT/upstream_decode" "$file" "$file.plain"
                python3 "$REPO/scripts/compat/check.py" lines \
                    "$FIX/inputs.bin" "$file.plain" || RUST_CPP=FAILED
            done

            # Upstream writes, the Rust decoder reads.
            "$OUT/upstream_encode" --appender=1 --mode="$mode" --sync="$sync" \
                --pubkey="$KEY" --records="$FIX/inputs.bin" \
                --out="$CPP_DIR" > "$WORK/app-cpp-$tag.txt"
            files="$(cat "$WORK/app-cpp-$tag.txt")"
            if [ -z "$files" ]; then
                echo "$tag: upstream's appender named no file to decode" >&2
                CPP_RUST=FAILED
            else
                CPP_RUST=ok
            fi
            for file in $files; do
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
# Not an `exit 1` — a row upstream's own decoder excuses is not a failure of
# the port — but not silence either: "every cross-read check passed" over a
# table with an excused row in it reads as though the rust -> cpp direction had
# been compared on every case, and it has not.
if [ "$CPP_DECODER" -ne 0 ]; then
    echo "$CPP_DECODER row(s) compared nothing: upstream's decoder read its own file the way it read ours" >&2
fi
echo "every cross-read check passed; the files are in $WORK"
