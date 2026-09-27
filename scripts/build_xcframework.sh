#!/usr/bin/env bash
#
# Builds MarsRS.xcframework.zip: `mars-ffi` as a static library for the iOS
# device and simulator and for the watchOS device and simulator, with its
# header and a module map next to it, which is what Package.swift hands to a
# Swift app.

# The framework is named after the project, not after xlog: the C ABI is the
# port's, and everything the port grows into belongs in it. xlog is what it
# carries today (Sources/MarsRSXlog/Xlog.swift is the Swift of it), not what the
# framework is.
#
#   scripts/build_xcframework.sh 0.1.0 [output-dir]
#
# The version is only a name for the zip's directory of origin; the binary is
# the same whatever it says. Run it from anywhere; it finds the workspace from
# its own path. The xcframework is left in <output-dir>/MarsRS.xcframework.zip
# (default: target/xcframework).
#
# Why a static library and not a framework bundle: the C++ project ships the
# same shape (see MarsXlog.xcframework of orangeboyChen/mars) and SwiftPM's
# binary targets take either. `-headers` is what makes `import MarsRSFFI`
# resolve in Swift.

set -euo pipefail

version="${1:-}"
if [ -z "$version" ]; then
    echo "usage: $(basename "$0") <version> [output-dir]" >&2
    exit 2
fi
version="${version#v}"
out="${2:-$(cd "$(dirname "$0")/.." && pwd)/target/xcframework}"

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

# The zip is written from a subshell that cd's elsewhere, so a relative
# output dir has to be resolved against the workspace before that happens.
case "$out" in
    /*) ;;
    *) out="$root/$out" ;;
esac

name=MarsRS.xcframework
header_dir="$root/target/xcframework-build/headers"

# One entry per slice the framework carries, and a slice is one (sdk,
# destination) a consumer can build for: `<name>:<triple>[:<triple>…]`.
# A destination with two architectures — the iOS simulator on an Apple Silicon
# Mac and on an Intel one — is lipo'd into the single library an xcframework
# takes, because that is the shape `xcodebuild -create-xcframework` wants.
#
# watchOS is two device architectures, because a watch is two architectures:
# `arm64_32` — the 32-bit-pointer arm64 of every watch before the arm64 ones,
# which is what a watchOS 10–25 app links — and the arm64 of the watches
# watchOS 26 moved over, which is what `aarch64-apple-watchos` builds. One
# slice carries both, the way the iOS simulator slice carries arm64 and x86_64.
#
# The first of the two is tier 3: no channel ships a std for it, so it is built
# out of the nightly's own sources with `-Z build-std`. The arm64 one ships a
# std, and that std is built for watchOS 26, so its half of the slice asks for
# watchOS 26 whatever `WATCHOS_DEPLOYMENT_TARGET` says. `x86_64-apple-watchos-sim`
# is tier 3 with no std either, and nothing asks for it: a watchOS simulator is
# arm64.
slices=(
    ios-device:aarch64-apple-ios
    ios-simulator:aarch64-apple-ios-sim:x86_64-apple-ios
    watchos-device:arm64_32-apple-watchos:aarch64-apple-watchos
    watchos-simulator:aarch64-apple-watchos-sim
)

# The tier 3 target, which is the one `rustup target add` has nothing to add
# for and the one `cargo` needs a nightly and `-Z build-std` for.
build_std_target=arm64_32-apple-watchos

# The floor `Package.swift` declares for each platform, which is what the
# slices are built at.
ios_min=12.0
watchos_min=10.0

# Every triple of every slice, and then the ones `rustup target add` is asked
# for: the tier 3 one has no std to add, on any channel.
targets=()
std_targets=()
for slice in "${slices[@]}"; do
    IFS=':' read -r -a slice_targets <<< "${slice#*:}"
    targets+=("${slice_targets[@]}")
    for target in "${slice_targets[@]}"; do
        [ "$target" = "$build_std_target" ] || std_targets+=("$target")
    done
done

echo "building mars-ffi for ${targets[*]}"

rustup target add "${std_targets[@]}" > /dev/null
# `-Z build-std` is nightly, and it compiles std out of its sources.
rustup toolchain install nightly --profile minimal --component rust-src > /dev/null
for slice in "${slices[@]}"; do
    IFS=':' read -r -a slice_targets <<< "${slice#*:}"

    # The deployment target of the sdk, which `rustc` and `cc-rs` both read out
    # of the environment and which is what `Package.swift` promises: 12 for iOS,
    # 10 for watchOS. Left unset, the C objects of `zstd-sys` come out at the
    # deployment target of the SDK itself — 26.5 today — and an app whose own
    # target is lower than that is warned about once per object by `ld64`.
    #
    # It does not move every slice: the arm64 watchOS device one is built for
    # watchOS 26 whatever this says, because its std is.
    case "${slice%%:*}" in
        ios-*)
            export IPHONEOS_DEPLOYMENT_TARGET=$ios_min
            unset WATCHOS_DEPLOYMENT_TARGET
            ;;
        watchos-*)
            export WATCHOS_DEPLOYMENT_TARGET=$watchos_min
            unset IPHONEOS_DEPLOYMENT_TARGET
            ;;
    esac

    for target in "${slice_targets[@]}"; do
        if [ "$target" = "$build_std_target" ]; then
            cargo +nightly build --release -p mars-ffi --target "$target" \
                -Z build-std=std,panic_abort
        else
            cargo build --release -p mars-ffi --target "$target"
        fi
    done
done

# The local symbols of an archive are for the linker that built it, not for the
# one that links it into an app: `strip -x` keeps every `mars_xlog_*` and drops
# everything else, which is 31 % of the archive (21.1 MB -> 14.5 MB for
# `aarch64-apple-ios`) and of the zip a consumer downloads, and it does not
# change one byte of what the app ends up linking. Cargo's own `strip` cannot do
# it: that one is a flag for the linker, and a `staticlib` is never linked.
for target in "${targets[@]}"; do
    strip -x -S "$root/target/$target/release/libmars_ffi.a"
done

rm -rf "$root/target/xcframework-build" "$out"
mkdir -p "$out" "$header_dir"

cp crates/mars-ffi/include/mars_xlog.h "$header_dir/"
cat > "$header_dir/module.modulemap" <<'MAP'
module MarsRSFFI {
    umbrella header "mars_xlog.h"
    export *
}
MAP

# One `-library … -headers …` pair per slice. `create-xcframework` works out
# the sdk and the destination of each library from the build version of its
# objects, which is what makes a static library built by cargo recognizable as
# a watchOS one.
args=()
for slice in "${slices[@]}"; do
    label="${slice%%:*}"
    IFS=':' read -r -a slice_targets <<< "${slice#*:}"
    libs=()
    for target in "${slice_targets[@]}"; do
        lib="$root/target/$target/release/libmars_ffi.a"
        test -f "$lib" || { echo "::error::$lib is missing"; exit 1; }
        libs+=("$lib")
    done
    if [ "${#libs[@]}" -eq 1 ]; then
        slice_lib="${libs[0]}"
    else
        slice_lib="$root/target/xcframework-build/$label/libmars_ffi.a"
        mkdir -p "$(dirname "$slice_lib")"
        lipo -create "${libs[@]}" -output "$slice_lib"
    fi
    args+=(-library "$slice_lib" -headers "$header_dir")
done

xcodebuild -create-xcframework "${args[@]}" \
    -output "$root/target/xcframework-build/$name"

# `-allow-warnings` is not an option of `create-xcframework`, so a name clash
# would fail above; what is left to check is that every slice is there, named
# the way a consumer's `xcodebuild` looks for it (ios-arm64,
# ios-arm64_x86_64-simulator, watchos-arm64_arm64_32, watchos-arm64-simulator).
ls "$root/target/xcframework-build/$name"

(cd "$root/target/xcframework-build" && zip -q -r "$out/$name.zip" "$name")

checksum="$(shasum -a 256 "$out/$name.zip" | cut -d ' ' -f 1)"
echo "built $out/$name.zip"
echo "spm checksum: $checksum"
