#!/usr/bin/env bash
#
# Builds platforms/harmonyos/marsrs-xlog the way HarmonyOS builds it: DevEco's
# own `hvigorw assembleHar`, over a project this script writes around the
# module.
#
#   scripts/assemble_harmony_har.sh [output-dir]
#
# Environment:
#   DEVECO_HOME   where the DevEco Command Line Tools are unpacked — the
#                 `command-line-tools/` of the zip (default:
#                 <repo>/.deveco-cli/command-line-tools)
#
# This is the one check of the HarmonyOS module that is a build and not a read:
# scripts/check_harmony_arkts.sh reads the ArkTS as TypeScript and
# scripts/check_harmony.sh reads it as text, and neither of them is an ArkTS
# compiler. This one is — `hvigorw` compiles the module with the SDK's own
# ArkTS toolchain, and what comes out is a HAR of the shape ohpm publishes:
# with `compatibleSdkVersion`, `compatibleSdkType`, `obfuscated` and
# `nativeComponents` in it, which are the four fields hvigor fills in from the
# SDK it built with and the four a hand-assembled archive does not have.
#
# The project is written here and not kept in the repository, because the
# module is not a DevEco project: what ships is built by
# scripts/build_harmony_napi.sh and packed by scripts/package_harmony.sh, and
# a root `oh-package.json5` beside the module would say otherwise. Hvigor will
# not build a module that has no project around it — `--mode module` or not —
# so this is the smallest one that builds this module and nothing else.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

out="${1:-$root/dist}"
case "$out" in
    /*) ;;
    *) out="$root/$out" ;;
esac

deveco="${DEVECO_HOME:-$root/.deveco-cli/command-line-tools}"
hvigorw="$deveco/bin/hvigorw"
if [ ! -f "$hvigorw" ]; then
    echo "::error::no hvigorw under $deveco — unpack the DevEco Command Line Tools there, or set DEVECO_HOME"
    exit 1
fi

module="$root/platforms/harmonyos/marsrs-xlog"
name="$(python3 -c 'import re,sys; print(re.search(r"\"name\": \"([^\"]*)\"", open(sys.argv[1]).read()).group(1))' \
    "$module/oh-package.json5")"
version="$(python3 -c 'import re,sys; print(re.search(r"\"version\": \"([^\"]*)\"", open(sys.argv[1]).read()).group(1))' \
    "$module/oh-package.json5")"
# What hvigor calls the module: `module.name` of src/main/module.json5, and not
# the ohpm name in oh-package.json5 and not the directory it is in.
modname="$(python3 -c 'import re,sys; print(re.search(r"\"name\": \"([^\"]*)\"", open(sys.argv[1]).read()).group(1))' \
    "$module/src/main/module.json5")"

# What the product is built for, read out of the SDK and not written here: the
# `compatibleSdkVersion` hvigor asks a product for is
# `<platformVersion>(<apiVersion>)` of the SDK it is about, and a number that
# disagrees with the SDK beside it is a build that fails on a field nobody
# reads. The pinned Command Line Tools carry HarmonyOS 5.1.0 Release, API 18.
sdk_version="$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1]))["data"]; print("%s(%s)" % (d["platformVersion"], d["apiVersion"]))' \
    "$deveco/sdk/default/sdk-pkg.json")"
# The same number again, without the API in it: `modelVersion`, which hvigor
# wants in `hvigor/hvigor-config.json5` and in the project's own
# `oh-package.json5` at once, and equal in both — a model it does not recognise,
# or one the two files disagree on, is refused before a task runs. It is the
# SDK's platform version and not a number of ours: `CURRENT_MODEL_VERSION` of
# the plugin is the newest platform version it supports, and a model older than
# 5.0.0 is one it tells you to migrate away from.
model_version="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["data"]["platformVersion"])' \
    "$deveco/sdk/default/sdk-pkg.json")"

project="$(mktemp -d)"
trap 'rm -rf "$project"' EXIT

cp -R "$module"/. "$project/marsrs-xlog/"

# The project root. A project-level `oh-package.json5` is a smaller thing than
# a module's: ohpm's schema for it requires `modelVersion` and nothing else, and
# the `name`, `version` and `main` a HAR carries are fields of the module below,
# not of the project around it.
cat > "$project/oh-package.json5" <<JSON5
{
  "modelVersion": "$model_version",
  "description": "The project hvigor needs around the module. Written by scripts/assemble_harmony_har.sh, and not a file of the repository.",
  "dependencies": {},
  "devDependencies": {}
}
JSON5

# `AppScope/app.json5`: hvigor's model of a project reads it whether or not the
# build is of an app, and there is no way to tell it that this project has no
# entry — a `--mode module` build of one module is refused without it. The
# bundle name is never installed under, and never signed into a HAP: it is the
# name of a project that exists for one `assembleHar` and is then deleted.
mkdir -p "$project/AppScope"
cat > "$project/AppScope/app.json5" <<JSON5
{
  "app": {
    "bundleName": "io.github.orangeboychen.marsrs.assemble",
    "vendor": "mars-rs",
    "versionCode": 1000000,
    "versionName": "1.0.0"
  }
}
JSON5

cat > "$project/build-profile.json5" <<JSON5
{
  "app": {
    "signingConfigs": [],
    "products": [
      {
        "name": "default",
        "runtimeOS": "HarmonyOS",
        "compatibleSdkVersion": "$sdk_version",
        "buildOption": {}
      }
    ],
    "buildModeSet": [
      { "name": "debug" },
      { "name": "release" }
    ]
  },
  "modules": [
    {
      "name": "marsrs_xlog",
      "srcPath": "./marsrs-xlog",
      "targets": [
        { "name": "default", "applyToProducts": ["default"] }
      ]
    }
  ]
}
JSON5

cat > "$project/hvigorfile.ts" <<'TS'
import { appTasks } from '@ohos/hvigor-ohos-plugin';

export default {
    system: appTasks,  /* Built-in plugin of Hvigor. It cannot be modified. */
    plugins: [],       /* Custom plugin to extend the functionality of Hvigor. */
}
TS

mkdir -p "$project/hvigor"
cat > "$project/hvigor/hvigor-config.json5" <<JSON5
{
  "modelVersion": "$model_version",
  "dependencies": {},
  "plugins": []
}
JSON5

export DEVECO_SDK_HOME="${DEVECO_SDK_HOME:-$deveco/sdk}"
export PATH="$deveco/bin:$deveco/tool/node/bin:$PATH"
export CI=true
export HOME="${HOME:-/root}"

mkdir -p "$out"
(
    cd "$project"
    # `--version` is not a flag every hvigorw answers; the log is better with
    # the number in it and no worse without.
    "$hvigorw" --version || true
    "$hvigorw" assembleHar --mode module -p "module=$modname@default" \
        -p product=default --no-daemon
)

# Where hvigor leaves a HAR of one module: build/<product>/outputs/<target>/.
har="$(find "$project/marsrs-xlog/build" -name '*.har' -print -quit)"
test -n "$har" || { echo "::error::hvigor wrote no .har under marsrs-xlog/build"; exit 1; }

cp "$har" "$out/$name-$version.har"
ls -l "$out/$name-$version.har"
# A HAR is a tar.gz of the module's tree, which is what makes it the same
# archive scripts/package_harmony.sh writes by hand.
tar -tzf "$out/$name-$version.har" | grep -E 'oh-package\.json5$|\.ets$|\.so$' | sort || true
