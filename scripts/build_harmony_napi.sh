#!/usr/bin/env bash
#
# Builds `libmarsrs_xlog.so` — the NAPI module of
# platforms/harmonyos/marsrs-xlog — for every HarmonyOS ABI the port can target,
# and lays them out as <output-dir>/<abi>/libmarsrs_xlog.so: the shape a
# HarmonyOS module wants its natives in (`libs/arm64-v8a/…`), and so the shape a
# HAR is packed from.
#
#   scripts/build_harmony_napi.sh [output-dir]
#
# This is the whole native half of the package, and it is one `.so` per ABI and
# not two: `libmars_ffi.a` — the staticlib of `marsrs-ffi` — is linked into it,
# so an app that takes the HAR resolves nothing else and the appender is the
# only thing in the process that owns a copy of the Rust core. `libmars_ffi.so`,
# which scripts/build_harmony.sh builds beside it, is the other way in: an app
# that would rather write its own NAPI module takes that one and links against
# it. Both are built from the same crate; this one adds the NAPI shim of
# `src/main/cpp/napi_init.cpp` that an app of the package does not have to.
#
# Needs the OHOS native SDK — the same one scripts/build_harmony.sh uses, and
# the same two ways of having it: `$OHOS_SDK_HOME`, or the cache that script
# downloaded into `${OHOS_SDK_CACHE:-target/ohos-sdk}`. Unlike that script this
# one does not download it: an SDK that is not there yet is a
# `scripts/build_harmony.sh` that has not run, and saying so is better than
# spending the 2.7 GB twice.
#
# `hvigor` is not involved, and that is the point: the SDK's clang is what links
# this, so a Linux runner can build it without DevEco Studio, and what the HAR
# carries is a binary and not a CMake project an app's build has to run.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
out="${1:-$root/target/harmony-napi}"

sdk_cache="${OHOS_SDK_CACHE:-$root/target/ohos-sdk}"
if [ -z "${OHOS_SDK_HOME:-}" ]; then
    OHOS_SDK_HOME="$sdk_cache/ohos-sdk/linux"
fi

native="$OHOS_SDK_HOME/native"
test -x "$native/llvm/bin/clang" || {
    echo "::error::no OHOS native SDK under $OHOS_SDK_HOME — run scripts/build_harmony.sh first, or set OHOS_SDK_HOME"
    exit 1
}

# The napi headers of the SDK: `napi/native_api.h` is what
# src/main/cpp/napi_init.cpp includes, and an SDK without them is an SDK this
# cannot compile against — a native-only SDK, or one too old to carry them.
test -f "$native/sysroot/usr/include/napi/native_api.h" || {
    echo "::error::no napi headers under $native/sysroot/usr/include/napi — this SDK cannot build a NAPI module"
    exit 1
}

# abi:rust-target:name-the-SDK-spells-the-target-with — the same three
# scripts/build_harmony.sh builds for, and the same mismatch of names.
abis=(
    "arm64-v8a:aarch64-unknown-linux-ohos:aarch64-linux-ohos"
    "armeabi-v7a:armv7-unknown-linux-ohos:arm-linux-ohos"
    "x86_64:x86_64-unknown-linux-ohos:x86_64-linux-ohos"
)

source="$root/platforms/harmonyos/marsrs-xlog/src/main/cpp/napi_init.cpp"
include="$root/crates/marsrs-ffi/include"
test -f "$source" || { echo "::error::$source is missing"; exit 1; }

rm -rf "$out"
mkdir -p "$out"

for entry in "${abis[@]}"; do
    abi="$(cut -d: -f1 <<< "$entry")"
    target="$(cut -d: -f2 <<< "$entry")"
    sdk_target="$(cut -d: -f3 <<< "$entry")"

    # The staticlib of the C ABI, which is what is linked in. `cargo build` is
    # idempotent, so this is cheap when scripts/build_harmony.sh ran first, and
    # it is what makes this script runnable on its own — but it is not free:
    # that one builds `--features sdt,stn` for a `.so` and this one the default
    # set for a staticlib, so the crate's own unit is compiled again either
    # way.
    #
    # What is deliberately *not* carried over is that script's `-C link-arg`.
    # A staticlib is not linked, so the two it sets — the `-L` that finds
    # libunwind and the one that keeps it out of the dynamic table — do nothing
    # here; and a `RUSTFLAGS` that differs from the build that script just did
    # fingerprints every unit of the graph away from it, which is the whole
    # dependency graph compiled a second time for all three targets. The `-L`
    # goes to the `clang` below instead, where there is a link for it to do
    # something to.
    echo "building marsrs-ffi for $target ($abi)"
    clang="$native/llvm/bin/$target-clang"
    flat="$(printf '%s' "$target" | tr '-' '_')"
    upper="$(printf '%s' "$flat" | tr '[:lower:]' '[:upper:]')"
    export "CC_${flat}=$clang"
    export "AR_${flat}=$native/llvm/bin/llvm-ar"
    export "CARGO_TARGET_${upper}_LINKER=$clang"
    export PATH="$native/llvm/bin:$PATH"
    cargo build --release -p marsrs-ffi --target "$target"

    archive="$root/target/$target/release/libmars_ffi.a"
    test -f "$archive" || { echo "::error::no $archive to link"; exit 1; }

    echo "linking libmarsrs_xlog.so for $abi"
    mkdir -p "$out/$abi"
    # `--allow-shlib-undefined`: the napi symbols this calls are answered by the
    # process that loads the library — an app's own, through `libace_napi.z.so` —
    # and not by anything on this link line. Without it the link fails on every
    # `napi_*` it does not resolve, which is how a NAPI module of the SDK's own
    # is built as well.
    #
    # `-lunwind`, and the `-L` that finds it, are scripts/build_harmony.sh's:
    # the unwinder every `catch_unwind` of the C ABI crosses, kept static and
    # kept out of the dynamic table so that nothing resolves an unwind through
    # this library and the app's own unwinder stays the app's.
    "$clang" \
        -shared -fPIC -O2 \
        -I "$include" \
        -I "$native/sysroot/usr/include" \
        -I "$native/sysroot/usr/include/$sdk_target" \
        -o "$out/$abi/libmarsrs_xlog.so" \
        "$source" \
        "$archive" \
        -L "$native/llvm/lib/$sdk_target" \
        -lunwind -ldl -lm -lc \
        -Wl,--allow-shlib-undefined \
        -Wl,--exclude-libs,libunwind.a

    test -f "$out/$abi/libmarsrs_xlog.so" || {
        echo "::error::the link wrote no $out/$abi/libmarsrs_xlog.so"
        exit 1
    }
done

# `find` answers 0 for a tree with no `.so` in it and so does `sort`, so an
# output directory the loop above never wrote to is a script that passed. What
# says the three are there is the count and not the exit of either.
count="$(find "$out" -name '*.so' | wc -l | tr -d ' ')"
[ "$count" -eq "${#abis[@]}" ] || {
    echo "::error::$out holds $count .so and not ${#abis[@]}"
    exit 1
}
find "$out" -name '*.so' | sort
