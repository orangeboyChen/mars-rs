#!/usr/bin/env bash
#
# Builds the native libraries the Kotlin Multiplatform packaging needs:
# `marsrs-ffi` as a static library for every Kotlin/Native target the release
# ships, laid out as <output-dir>/<kotlin-target>/libmars_ffi.a — the shape
# `platforms/kmp/*/build.gradle.kts` asks for and the shape cinterop embeds
# into a klib, which is how an app of that target links `marsrs-ffi` without
# being told where it is.
#
#   scripts/build_kmp_native.sh [output-dir]
#
# Which targets it builds is what the host can build: every Apple one on macOS,
# and Linux and Windows on Linux, where `apt` has the cross toolchains.
# .github/workflows/release.yml runs it once per host — Apple on `macos-15`, the
# rest on `ubuntu-latest` — and uploads what it wrote with the release.
#
# It builds no Android library: `scripts/build_android.sh` does, and the AAR and
# the Kotlin Multiplatform module share that one `libmarsrsxlog.so`.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
out="${1:-$root/target/kmp-native}"

# <kotlin-target>:<rust-triple>[:build-std]
#
# The Kotlin name is the one `platforms/kmp/*/build.gradle.kts` declares the
# target under and the one cinterop is configured for; the triple is what the
# archive of that Kotlin target is built for. `build-std` marks the tier 3
# triples no channel ships a std for, which `cargo` can only build out of a
# nightly's own sources.
#
# Two Kotlin targets are not in the list, because Rust has no triple for either:
# `watchosX64` and `tvosX64`, the x86_64 simulators. A watchOS simulator is
# arm64 on every Mac that can run one, and `x86_64-apple-tvos` is not a target
# `rustup` knows.
apple_targets=(
    iosArm64:aarch64-apple-ios
    iosX64:x86_64-apple-ios
    iosSimulatorArm64:aarch64-apple-ios-sim
    macosX64:x86_64-apple-darwin
    macosArm64:aarch64-apple-darwin
    watchosArm64:arm64_32-apple-watchos:build-std
    watchosDeviceArm64:aarch64-apple-watchos
    watchosSimulatorArm64:aarch64-apple-watchos-sim
    tvosArm64:aarch64-apple-tvos
    tvosSimulatorArm64:aarch64-apple-tvos-sim
)

# Linux builds its own target and cross-compiles the other two, with the
# toolchains `apt install gcc-aarch64-linux-gnu mingw-w64` puts there.
linux_targets=(
    linuxX64:x86_64-unknown-linux-gnu
    linuxArm64:aarch64-unknown-linux-gnu
    mingwX64:x86_64-pc-windows-gnu
)

case "$(uname -s)" in
    Darwin) targets=("${apple_targets[@]}") ;;
    Linux) targets=("${linux_targets[@]}") ;;
    *)
        echo "::error::$(uname -s) builds none of the Kotlin Multiplatform targets; run this on macOS or Linux"
        exit 1
        ;;
esac

# Every triple of the list, and then the ones `rustup target add` is asked for:
# the `build-std` one has no std to add, on any channel.
triples=()
std_triples=()
for entry in "${targets[@]}"; do
    IFS=':' read -r _ triple build_std <<< "$entry"
    triples+=("$triple")
    [ "$build_std" = build-std ] || std_triples+=("$triple")
done

echo "building marsrs-ffi for ${triples[*]}"

rustup target add "${std_triples[@]}" > /dev/null
# `-Z build-std` is nightly, and it compiles std out of its sources.
if [ "${#std_triples[@]}" -ne "${#triples[@]}" ]; then
    rustup toolchain install nightly --profile minimal --component rust-src > /dev/null
fi

rm -rf "$out"

for entry in "${targets[@]}"; do
    IFS=':' read -r kotlin triple build_std <<< "$entry"

    # The deployment target of the sdk, which `rustc` and `cc-rs` both read out
    # of the environment and which is what makes the archive linkable into an app
    # of the floor the Kotlin side promises. Left unset, the C objects of
    # `zstd-sys` come out at the deployment target of the SDK itself — 26.5
    # today — and an app whose own target is lower than that is warned about once
    # per object by `ld64`.
    case "$triple" in
        *-apple-ios | *-apple-ios-sim)
            export IPHONEOS_DEPLOYMENT_TARGET=12.0
            unset WATCHOS_DEPLOYMENT_TARGET TVOS_DEPLOYMENT_TARGET
            ;;
        *-apple-watchos | *-apple-watchos-sim)
            export WATCHOS_DEPLOYMENT_TARGET=10.0
            unset IPHONEOS_DEPLOYMENT_TARGET TVOS_DEPLOYMENT_TARGET
            ;;
        *-apple-tvos | *-apple-tvos-sim)
            export TVOS_DEPLOYMENT_TARGET=12.0
            unset IPHONEOS_DEPLOYMENT_TARGET WATCHOS_DEPLOYMENT_TARGET
            ;;
        *-apple-darwin)
            export MACOSX_DEPLOYMENT_TARGET=11.0
            unset IPHONEOS_DEPLOYMENT_TARGET WATCHOS_DEPLOYMENT_TARGET TVOS_DEPLOYMENT_TARGET
            ;;
    esac

    # The cross toolchains, which cargo reads out of the environment: `CC_<triple>`
    # verbatim for cc-rs, and the upper-cased `CARGO_TARGET_<TRIPLE>_LINKER` for
    # itself — the upper-cased `CC_<TRIPLE>` is silently ignored.
    case "$triple" in
        aarch64-unknown-linux-gnu)
            export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
            export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
            ;;
        x86_64-pc-windows-gnu)
            export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc
            export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
            export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
            ;;
    esac

    if [ "$build_std" = build-std ]; then
        cargo +nightly build --release -p marsrs-ffi --target "$triple" \
            -Z build-std=std,panic_abort
    else
        cargo build --release -p marsrs-ffi --target "$triple"
    fi

    lib="$root/target/$triple/release/libmars_ffi.a"
    test -f "$lib" || { echo "::error::$lib is missing"; exit 1; }

    # The local symbols of an archive are for the linker that built it, not for
    # the one that links it into an app, and this archive ends up inside a klib a
    # consumer downloads: `scripts/build_xcframework.sh` strips the same 31 % out
    # of the slices for the same reason, and it changes nothing an app links.
    case "$triple" in
        *-apple-*) strip -x -S "$lib" ;;
        *-pc-windows-gnu) x86_64-w64-mingw32-strip --strip-debug "$lib" ;;
        *) strip --strip-debug "$lib" ;;
    esac

    mkdir -p "$out/$kotlin"
    cp "$lib" "$out/$kotlin/libmars_ffi.a"
    echo "$kotlin: $out/$kotlin/libmars_ffi.a"
done

find "$out" -name '*.a' | sort
