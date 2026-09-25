#!/usr/bin/env bash
#
# Builds `libmarsxlog.so` (crate `mars-jni`) for every Android ABI the release
# ships, and lays them out as <output-dir>/<abi>/libmarsxlog.so — the shape of
# mars-android-native.zip and of android/mars-xlog/libs/, which is what the AAR
# is assembled from.
#
#   scripts/build_android.sh [output-dir]
#
# Needs the Android NDK (ANDROID_HOME / ANDROID_SDK_ROOT, default
# ~/Library/Android/sdk on macOS, $ANDROID_HOME in CI) and the three Rust
# targets; `rustup target add` is run for them. The 16 KiB page size comes from
# .cargo/config.toml, which is why RUSTFLAGS must not be exported here.
#
# `zstd-sys` compiles C, so both cc-rs and rustc need the NDK clang, and they
# spell the env var prefixes differently — see the comment above the exports.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
out="${1:-$root/target/android}"

ndk_version="${NDK_VERSION:-27.1.12297006}"
android_home="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
ndk_dir="$android_home/ndk/$ndk_version"
ndk_bin="$ndk_dir/toolchains/llvm/prebuilt/linux-x86_64/bin"
[ -d "$ndk_bin" ] || ndk_bin="$ndk_dir/toolchains/llvm/prebuilt/darwin-x86_64/bin"
[ -d "$ndk_bin" ] || { echo "::error::no NDK $ndk_version under $android_home"; exit 1; }

# abi:rust-target:clang
abis=(
    "arm64-v8a:aarch64-linux-android:aarch64-linux-android24-clang"
    "armeabi-v7a:armv7-linux-androideabi:armv7a-linux-androideabi24-clang"
    "x86_64:x86_64-linux-android:x86_64-linux-android24-clang"
)

targets=()
for entry in "${abis[@]}"; do
    targets+=("$(cut -d: -f2 <<< "$entry")")
done
rustup target add "${targets[@]}" > /dev/null

rm -rf "$out"
mkdir -p "$out"

for entry in "${abis[@]}"; do
    abi="$(cut -d: -f1 <<< "$entry")"
    target="$(cut -d: -f2 <<< "$entry")"
    clang="$ndk_bin/$(cut -d: -f3 <<< "$entry")"
    test -x "$clang" || { echo "::error::$clang is not executable"; exit 1; }

    flat="$(printf '%s' "$target" | tr '-' '_')"
    upper="$(printf '%s' "$flat" | tr '[:lower:]' '[:upper:]')"
    # cargo reads the upper-cased CARGO_TARGET_<TRIPLE>_LINKER, cc-rs reads
    # CC_<triple> verbatim: the upper-cased CC_<TRIPLE> is silently ignored.
    export "CC_${flat}=$clang"
    export "AR_${flat}=$ndk_bin/llvm-ar"
    export "CARGO_TARGET_${upper}_LINKER=$clang"
    export PATH="$ndk_bin:$PATH"

    echo "building mars-jni for $target ($abi)"
    cargo build --release -p mars-jni --target "$target"

    mkdir -p "$out/$abi"
    cp "target/$target/release/libmarsxlog.so" "$out/$abi/libmarsxlog.so"
done

find "$out" -name '*.so' | sort
