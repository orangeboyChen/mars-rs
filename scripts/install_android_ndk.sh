#!/usr/bin/env bash
# Install the Android NDK a `cargo build --target <android triple>` needs.
#
#   scripts/install_android_ndk.sh ["platforms;android-34" ...]
#
# Every package named on the command line is installed next to the NDK: the
# cross build of the .so wants nothing but the NDK, while the release's AAR
# wants the SDK platform and the build tools with it.
#
# `android-actions/setup-android` is deliberately not used: it installs the
# long-removed `tools` SDK package, so its `sdkmanager` call exits non-zero and
# the job fails before any Rust code is compiled. What the runner image ships
# is enough — the reason orangeboyChen/mars installs the SDK by hand too.
#
# Environment:
#   ANDROID_HOME  the SDK the runner image already provides
#   NDK_VERSION   the NDK, pinned

set -euo pipefail

: "${ANDROID_HOME:?}" "${NDK_VERSION:?}"

sdkmanager=$(command -v sdkmanager || true)
if [ -z "$sdkmanager" ]; then
  sdkmanager=$(ls -d "$ANDROID_HOME"/cmdline-tools/*/bin/sdkmanager 2>/dev/null | head -1 || true)
fi
if [ -z "$sdkmanager" ]; then
  echo "::error::sdkmanager not found under $ANDROID_HOME"
  exit 1
fi
echo "sdkmanager: $sdkmanager"

# `yes` is killed by SIGPIPE as soon as sdkmanager stops reading, and under
# `pipefail` that 141 would fail the job although sdkmanager itself succeeded.
{ yes || true; } | "$sdkmanager" --licenses > /dev/null

"$sdkmanager" --install "ndk;${NDK_VERSION}" "$@"
test -d "$ANDROID_HOME/ndk/${NDK_VERSION}"
