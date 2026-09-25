#!/usr/bin/env bash
#
# Builds MarsRS.xcframework.zip: `mars-ffi` as a static library for the iOS
# device and for the iOS simulator, with its header and a module map next to
# it, which is what Package.swift hands to a Swift app.

# The framework is named after the project, not after xlog: the C ABI is the
# port's, and everything the port grows into belongs in it. xlog is what it
# carries today (Sources/MarsRS/Xlog.swift is the Swift of it), not what the
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

# `aarch64-apple-ios` is the device; the two others are the simulator on an
# Apple Silicon Mac and on an Intel one, and `lipo` makes one library of both.
device_target=aarch64-apple-ios
sim_targets=(aarch64-apple-ios-sim x86_64-apple-ios)

echo "building mars-ffi for $device_target and ${sim_targets[*]}"

rustup target add "$device_target" "${sim_targets[@]}" > /dev/null
cargo build --release -p mars-ffi --target "$device_target"
for target in "${sim_targets[@]}"; do
    cargo build --release -p mars-ffi --target "$target"
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

device_lib="$root/target/$device_target/release/libmars_ffi.a"
test -f "$device_lib" || { echo "::error::$device_lib is missing"; exit 1; }

sim_lib="$root/target/xcframework-build/simulator/libmars_ffi.a"
mkdir -p "$(dirname "$sim_lib")"
sim_libs=()
for target in "${sim_targets[@]}"; do
    sim_libs+=("$root/target/$target/release/libmars_ffi.a")
done
lipo -create "${sim_libs[@]}" -output "$sim_lib"

xcodebuild -create-xcframework \
    -library "$device_lib" -headers "$header_dir" \
    -library "$sim_lib" -headers "$header_dir" \
    -output "$root/target/xcframework-build/$name"

# `-allow-warnings` is not an option of `create-xcframework`, so a name clash
# would fail above; what is left to check is that both slices are there.
ls "$root/target/xcframework-build/$name"

(cd "$root/target/xcframework-build" && zip -q -r "$out/$name.zip" "$name")

checksum="$(shasum -a 256 "$out/$name.zip" | cut -d ' ' -f 1)"
echo "built $out/$name.zip"
echo "spm checksum: $checksum"
