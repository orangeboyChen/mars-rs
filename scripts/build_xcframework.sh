#!/usr/bin/env bash
#
# Builds marsrs-xlog.xcframework.zip and marsrs-net.xcframework.zip: `marsrs-ffi` as
# a static library for the iOS device and simulator and for the watchOS device
# and simulator, with its headers and a module map next to them, which is what
# Package.swift hands to a Swift app.
#
# Two artifacts and not one, because the feature set the library is built with
# is what an app links and not what the crate happens to hold: `marsrs-xlog` is
# `--no-default-features --features xlog`, so it is xlog's 17 `mars_xlog_*`
# symbols and nothing else, and `marsrs-net` is `--no-default-features --features
# sdt,stn`, so it carries no `mars_xlog_*` at all. That is what lets an app that
# only logs take the first and stop there, and an app that takes both link
# xlog's symbols exactly once instead of twice. The check at the bottom of each
# build fails the day either one exports a symbol that is not its own.
#
#   scripts/build_xcframework.sh 0.1.0 [output-dir]
#
# The version is only a name for the zip's directory of origin; the binaries are
# the same whatever it says. Run it from anywhere; it finds the workspace from
# its own path. The xcframeworks are left in
# <output-dir>/marsrs-xlog.xcframework.zip and
# <output-dir>/marsrs-net.xcframework.zip (default: target/xcframework), and each
# prints the checksum Package.swift names for it.
#
# Why a static library and not a framework bundle: the C++ project ships the
# same shape (see MarsXlog.xcframework of orangeboyChen/mars) and SwiftPM's
# binary targets take either. `-headers` is what makes `import MarsRSFFI` — or
# `import MarsRSNetFFI` — resolve in Swift: an artifact is named for the feature
# set it holds and the module inside it for the C surface a consumer imports, and
# a module rename is a rename of what every consumer imports, so it can only
# travel with the release that publishes it. A zip's name asks for nothing of
# the kind — SwiftPM extracts it under the binary target's name and reads the
# module map inside — which is why these two can be lower-case, the way the
# crates are, while the modules they carry are not.

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

# One entry per artifact: `<xcframework>:<c module>:<feature set>`. The headers
# and the symbol prefix each one is checked against follow from its name below.
artifacts=(
    marsrs-xlog.xcframework:MarsRSFFI:xlog
    marsrs-net.xcframework:MarsRSNetFFI:sdt,stn
)

# One entry per slice every artifact carries, and a slice is one (sdk,
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

rustup target add "${std_targets[@]}" > /dev/null
# `-Z build-std` is nightly, and it compiles std out of its sources.
rustup toolchain install nightly --profile minimal --component rust-src > /dev/null

rm -rf "$root/target/xcframework-build" "$out"
mkdir -p "$out"

for artifact in "${artifacts[@]}"; do
    IFS=':' read -r name module features <<< "$artifact"
    stem="${name%.xcframework}"
    build="$root/target/xcframework-build/$stem"
    header_dir="$build/headers"

    # What the artifact is named for, spelled out twice: the headers the module
    # map hands a caller, and the symbols the library is allowed to export. The
    # net half has no header of its own — it is the diagnosis' and the task
    # pipeline's, two of them, and a module map takes one umbrella — so it gets
    # one here, out of the two it is made of.
    case "$stem" in
        marsrs-xlog)
            headers=(mars_xlog.h)
            umbrella=mars_xlog.h
            owns='_mars_xlog_'
            ;;
        marsrs-net)
            headers=(mars_sdt.h mars_stn.h)
            umbrella=mars_net.h
            owns='_mars_(sdt|stn)_'
            ;;
        *)
            echo "::error::no headers or symbols known for $name"
            exit 1
            ;;
    esac

    echo "building $name (marsrs-ffi --no-default-features --features $features)"

    # The feature set is spelled out here and never inherited from `default`,
    # which is the whole point of building two artifacts: a feature added to
    # `marsrs-ffi` later is in neither of them until it is named here. What the
    # crate carries by default is irrelevant; this is the list. The check below
    # then proves the list is what the name says.
    for slice in "${slices[@]}"; do
        IFS=':' read -r -a slice_targets <<< "${slice#*:}"

        # The deployment target of the sdk, which `rustc` and `cc-rs` both read
        # out of the environment and which is what `Package.swift` promises: 12
        # for iOS, 10 for watchOS. Left unset, the C objects of `zstd-sys` come
        # out at the deployment target of the SDK itself — 26.5 today — and an
        # app whose own target is lower than that is warned about once per
        # object by `ld64`.
        #
        # It does not move every slice: the arm64 watchOS device one is built
        # for watchOS 26 whatever this says, because its std is.
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
                cargo +nightly build --release -p marsrs-ffi --target "$target" \
                    --no-default-features --features "$features" \
                    -Z build-std=std,panic_abort
            else
                cargo build --release -p marsrs-ffi --target "$target" \
                    --no-default-features --features "$features"
            fi
        done
    done

    # The local symbols of an archive are for the linker that built it, not for
    # the one that links it into an app: `strip -x` keeps every symbol the
    # artifact is named for and drops everything else, which is 31 % of the
    # archive (21.1 MB -> 14.5 MB for the xlog `aarch64-apple-ios`) and of the
    # zip a consumer downloads, and it does not change one byte of what the app
    # ends up linking. Cargo's own `strip` cannot do it: that one is a flag for
    # the linker, and a `staticlib` is never linked.
    for target in "${targets[@]}"; do
        strip -x -S "$root/target/$target/release/libmars_ffi.a"
    done

    mkdir -p "$header_dir"
    for header in "${headers[@]}"; do
        cp "crates/marsrs-ffi/include/$header" "$header_dir/"
    done
    # The net half's umbrella is written here; xlog's is the one header the
    # crate owns, copied above.
    if [ "$umbrella" = mars_net.h ]; then
        cat > "$header_dir/$umbrella" <<'HDR'
// The C ABI of the net half of the port: the diagnosis' `mars_sdt_*` and the
// task pipeline's `mars_stn_*`, which is every symbol of the two halves and
// nothing of xlog's.
//
// It is written by scripts/build_xcframework.sh and not kept next to the two
// headers it includes, because a module map takes one umbrella and the net half
// is two seams: this is the umbrella, and `mars_sdt.h` and `mars_stn.h` are the
// headers the crate owns and the sync tests check.
#ifndef MARS_NET_H
#define MARS_NET_H

#include "mars_sdt.h"
#include "mars_stn.h"

#endif
HDR
    fi
    cat > "$header_dir/module.modulemap" <<MAP
module $module {
    umbrella header "$umbrella"
    export *
}
MAP

    # One `-library … -headers …` pair per slice. `create-xcframework` works out
    # the sdk and the destination of each library from the build version of its
    # objects, which is what makes a static library built by cargo recognizable
    # as a watchOS one.
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
            slice_lib="$build/libraries/$label/libmars_ffi.a"
            mkdir -p "$(dirname "$slice_lib")"
            lipo -create "${libs[@]}" -output "$slice_lib"
        fi
        # The artifact is called by the feature set it is, so the binary has to
        # be: every name the port exports is one of the symbols it is named for,
        # and this is what keeps that true. The day the xlog build exports a
        # `mars_sdt_*` or a `mars_stn_*` — or the net one a `mars_xlog_*` —
        # this fails, and the answer is another artifact: not a wider promise
        # from one of these, which is what shipping STN and SDT to an app that
        # only logs, or xlog twice to an app that takes both, would be. std and
        # the crates the archive carries export plenty of symbols of their own;
        # the question asked here is only about the port's.
        foreign="$(nm -gU "$slice_lib" 2>/dev/null |
            grep -E ' T _mars' | grep -Ev " T $owns" || true)"
        if [ -n "$foreign" ]; then
            echo "::error::$label exports symbols $name is not named for:"
            echo "$foreign"
            exit 1
        fi
        # The other half of the same check, and the reason the two are not one:
        # `nm` prints what it can parse and an error for every object it cannot,
        # so an archive it read nothing from would come out clean above. A slice
        # of either framework exports symbols of its own — 17 of them for xlog,
        # 9 and 33 for the diagnosis and the pipeline — so none at all means the
        # check above asked nothing rather than that the answer is yes.
        own="$(nm -gU "$slice_lib" 2>/dev/null | grep -Ec " T $owns" || true)"
        if [ "$own" -eq 0 ]; then
            echo "::error::$label exports no symbol $name is named for: no $owns"
            exit 1
        fi

        args+=(-library "$slice_lib" -headers "$header_dir")
    done

    xcodebuild -create-xcframework "${args[@]}" \
        -output "$build/$name"

    # `-allow-warnings` is not an option of `create-xcframework`, so a name
    # clash would fail above; what is left to check is that every slice is
    # there, named the way a consumer's `xcodebuild` looks for it (ios-arm64,
    # ios-arm64_x86_64-simulator, watchos-arm64_arm64_32,
    # watchos-arm64-simulator).
    #
    # What is asserted is the count and not the four names: an xcframework is
    # one directory per slice and an `Info.plist`, so the directories are the
    # slices, and `${#slices[@]}` is the number the array above asks for — the
    # day a slice is added or one is dropped, this moves with it. `ls` alone is
    # not the check it reads as: it prints, it answers 0, and the directory it
    # is given is one the command above just made, so it cannot be missing.
    slices_built="$(find "$build/$name" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')"
    test "$slices_built" -eq "${#slices[@]}" || {
        echo "::error::$name carries $slices_built slices and not ${#slices[@]}"
        exit 1
    }
    ls "$build/$name"

    (cd "$build" && zip -q -r "$out/$name.zip" "$name")

    checksum="$(shasum -a 256 "$out/$name.zip" | cut -d ' ' -f 1)"
    echo "built $out/$name.zip"
    echo "spm checksum ($module): $checksum"
done
