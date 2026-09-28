#!/usr/bin/env bash
#
# Builds `libmars_ffi.so` (crate `marsrs-ffi`, the C ABI) for every HarmonyOS ABI
# the port can target, and lays them out as <output-dir>/<abi>/libmars_ffi.so —
# the shape a HarmonyOS module wants its natives in (`libs/arm64-v8a/…`), and so
# the shape a HAR is packed from.
#
#   scripts/build_harmony.sh [output-dir]
#
# Needs the OHOS native SDK, and a Rust that ships a std for the targets:
#
#   * the SDK is `$OHOS_SDK_HOME` when that is set — a DevEco Studio
#     installation's `…/sdk/default/openharmony`, whose `native/` is what is
#     used here — and otherwise the public OpenHarmony SDK, downloaded from the
#     Huawei Cloud mirror, which needs no account, into
#     `${OHOS_SDK_CACHE:-target/ohos-sdk}`. The mirror's SDK is a Linux one in
#     both senses: the components offered are `linux` and `windows`, and the
#     toolchain in the linux one is Linux ELF. So the download is a Linux step,
#     and `OHOS_SDK_HOME` is the way in anywhere else.
#   * the three targets are tier 2 with host tools, so every channel ships a std
#     for them: `rustup target add` is enough and stable is enough, unlike
#     `arm64_32-apple-watchos` of `scripts/build_xcframework.sh`, which is tier 3
#     and needs a nightly and `-Z build-std`.
#
# `zstd-sys` compiles C, so both cc-rs and rustc need the SDK clang — see the
# comment above the exports. The SDK ships one clang wrapper per target, named
# after the Rust target, and the wrapper carries `-target …`, `--sysroot=…` and
# the ARM flags itself, which is why little else has to be said to the linker.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
out="${1:-$root/target/harmony}"

sdk_version="${OHOS_SDK_VERSION:-6.1-Release}"
sdk_mirror="${OHOS_SDK_MIRROR:-https://repo.huaweicloud.com/openharmony/os}"
sdk_cache="${OHOS_SDK_CACHE:-$root/target/ohos-sdk}"

if [ -z "${OHOS_SDK_HOME:-}" ]; then
    # The SDK is a tarball of zips, one per component, and the only component a
    # Rust build needs is the linux native one: its clang, the clang headers,
    # the sysroot of the three targets and the unwinder. Its debugger and its
    # cmake are left in the archive and out of the cache, because the download
    # is 2.7 GB and the runner's disk is not.
    OHOS_SDK_HOME="$sdk_cache/ohos-sdk/linux"
    if [ ! -d "$OHOS_SDK_HOME/native" ]; then
        command -v unzip > /dev/null || {
            echo "::error::unzip is needed to unpack the OHOS SDK"
            exit 1
        }
        mkdir -p "$sdk_cache"
        name=ohos-sdk-windows_linux-public.tar.gz
        echo "downloading the OpenHarmony $sdk_version SDK"
        curl -sSfL -o "$sdk_cache/$name" "$sdk_mirror/$sdk_version/$name"
        # The mirror publishes the hash on its own, with no filename beside it,
        # so it is compared here instead of being handed to `sha256sum -c`.
        expected="$(curl -sSfL "$sdk_mirror/$sdk_version/$name.sha256" | tr -d '[:space:]')"
        if command -v sha256sum > /dev/null; then
            actual="$(sha256sum "$sdk_cache/$name" | cut -d ' ' -f 1)"
        else
            actual="$(shasum -a 256 "$sdk_cache/$name" | cut -d ' ' -f 1)"
        fi
        [ "$expected" = "$actual" ] || {
            echo "::error::the OHOS SDK does not match its published checksum"
            exit 1
        }
        if tar --version 2>&1 | grep -q GNU; then
            # GNU tar only globs with --wildcards; bsdtar globs by itself and
            # rejects the flag.
            tar -xzf "$sdk_cache/$name" -C "$sdk_cache" \
                --wildcards 'linux/native-linux-x64-*.zip'
        else
            tar -xzf "$sdk_cache/$name" -C "$sdk_cache" \
                'linux/native-linux-x64-*.zip'
        fi
        # `unzip -d` creates one directory, not a path: the three levels it is
        # given here have to exist before it is asked for them.
        mkdir -p "$OHOS_SDK_HOME"
        # What is kept: clang and its headers, the sysroot of the three targets,
        # the static unwinder of each of them, and the one library `lld` cannot
        # start without — the linker is a Linux binary built against the libxml2
        # of the machine it was built on, and the SDK ships that very one beside
        # it (`$ORIGIN/../lib` is its runpath), so without it every link ends in
        # `libxml2.so.16: cannot open shared object file`. Not the debugger, not
        # the cmake, not liblldb.
        unzip -q "$sdk_cache"/linux/native-linux-x64-*.zip -d "$OHOS_SDK_HOME" \
            'native/sysroot/*' 'native/llvm/bin/*' 'native/llvm/lib/clang/*' \
            'native/llvm/lib/libxml2.so*' \
            'native/llvm/lib/aarch64-linux-ohos/libunwind.a' \
            'native/llvm/lib/arm-linux-ohos/libunwind.a' \
            'native/llvm/lib/x86_64-linux-ohos/libunwind.a'
        # The tarball and the zip are each bigger than what was taken out of
        # them.
        rm -rf "${sdk_cache:?}/$name" "${sdk_cache:?}/linux"
    fi
fi

native="$OHOS_SDK_HOME/native"
test -x "$native/llvm/bin/clang" || {
    echo "::error::no OHOS native SDK under $OHOS_SDK_HOME"
    exit 1
}

# abi:rust-target:name-the-SDK-spells-the-target-with. They differ: the clang
# wrapper is `armv7-unknown-linux-ohos-clang` while the sysroot and the
# runtime libraries are under `arm-linux-ohos`.
abis=(
    "arm64-v8a:aarch64-unknown-linux-ohos:aarch64-linux-ohos"
    "armeabi-v7a:armv7-unknown-linux-ohos:arm-linux-ohos"
    "x86_64:x86_64-unknown-linux-ohos:x86_64-linux-ohos"
)

targets=()
for entry in "${abis[@]}"; do
    targets+=("$(cut -d: -f2 <<< "$entry")")
done
rustup target add "${targets[@]}" > /dev/null

# What the caller asked for (`-D warnings` in CI), kept apart so that it is not
# grown once per ABI.
base_rustflags="${RUSTFLAGS:-}"

rm -rf "$out"
mkdir -p "$out"

for entry in "${abis[@]}"; do
    abi="$(cut -d: -f1 <<< "$entry")"
    target="$(cut -d: -f2 <<< "$entry")"
    sdk_target="$(cut -d: -f3 <<< "$entry")"
    clang="$native/llvm/bin/$target-clang"
    test -x "$clang" || { echo "::error::$clang is not executable"; exit 1; }

    flat="$(printf '%s' "$target" | tr '-' '_')"
    upper="$(printf '%s' "$flat" | tr '[:lower:]' '[:upper:]')"
    # cargo reads the upper-cased CARGO_TARGET_<TRIPLE>_LINKER, cc-rs reads
    # CC_<triple> verbatim: the upper-cased CC_<TRIPLE> is silently ignored.
    export "CC_${flat}=$clang"
    export "AR_${flat}=$native/llvm/bin/llvm-ar"
    export "CARGO_TARGET_${upper}_LINKER=$clang"
    export PATH="$native/llvm/bin:$PATH"

    # `-lunwind` is what rustc asks the linker for on ohos — the unwinder every
    # `catch_unwind` of the C ABI crosses — and the SDK keeps it as a static
    # library beside clang, one directory per target, rather than in the sysroot
    # where the linker would look: it is put on the path here, and kept out of
    # the library's dynamic table, so that nothing resolves an unwind through us
    # and the app's own unwinder stays the app's.
    export RUSTFLAGS="$base_rustflags \
-C link-arg=-L$native/llvm/lib/$sdk_target \
-C link-arg=-Wl,--exclude-libs,libunwind.a"

    echo "building marsrs-ffi for $target ($abi)"
    # `--features sdt,stn` on top of the default `xlog`, the way the host
    # archive of the release builds it: a HarmonyOS app reaches the C ABI
    # through NAPI of its own, and the three `mars_*.h` it is given name the
    # task pipeline and the diagnosis too. A `.so` without them would be the
    # one archive of the C ABI in the release that cannot do what its own
    # headers declare.
    cargo build --release -p marsrs-ffi --target "$target" --features sdt,stn

    mkdir -p "$out/$abi"
    cp "target/$target/release/libmars_ffi.so" "$out/$abi/libmars_ffi.so"
done

find "$out" -name '*.so' | sort
