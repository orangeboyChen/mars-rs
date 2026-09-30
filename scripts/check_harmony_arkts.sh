#!/usr/bin/env bash
#
# Type-checks the ArkTS of platforms/harmonyos/marsrs-xlog — `Index.ets` and
# `src/main/ets/xlog/` — which is the half of the package no compiler in this
# repository sees.
#
#   scripts/check_harmony_arkts.sh
#
# Environment:
#   TSC   the `tsc` to run; `tsc` on PATH when it is not set
#
# ArkTS is TypeScript's syntax under a stricter set of rules, so `tsc` reads
# these files as they stand: copied into a project of their own with `.ets`
# renamed to `.ts`, which is the whole of what it needs told, because the
# extension is the only difference. What that catches is what a compiler
# catches, and nothing else in this repository catches it:
#
#   * a method of `libmarsrs_xlog.so` that `Xlog.ets` calls and
#     `index.d.ts` does not declare — a name in two of the three places is a
#     call that answers `undefined` in the app;
#   * a call of the wrong arity, or of a wrong type, across the NAPI boundary;
#   * a field of `XlogConfig` that is not the one the NAPI module reads.
#
# What it does not catch is the rest of ArkTS's rules — no `any`, no `unknown`,
# no `var`, no structural typing — which is why scripts/check_harmony.sh reads
# the same files as text and says so out loud. DevEco's `codelinter` is the one
# tool that would catch all of it, and it arrives inside a 2 GB command-line
# toolchain that wants a JDK and a DevEco project around this module.
#
# The NAPI module is reached the way the ArkTS reaches it: `import xlogNapi
# from 'libmarsrs_xlog.so'` names a module no TypeScript resolver has, so this
# script writes the shim that answers it — out of `index.d.ts` and nothing else,
# so a method the declarations do not have is a method the ArkTS cannot call.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

module="$root/platforms/harmonyos/marsrs-xlog"
types="src/main/cpp/types/libmarsrs_xlog"
tsc="${TSC:-tsc}"

if ! command -v "$tsc" > /dev/null 2>&1; then
    echo "::error::$tsc is not on PATH — npm install typescript, or pass TSC=<path to tsc>"
    exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The module, as a TypeScript project: the tree as it is, with `.ets` renamed,
# so the imports of `Index.ets` (`./src/main/ets/xlog/LogLevel`) still resolve
# and nothing has to be rewritten for the checker's sake.
cp -R "$module"/. "$work/"
while IFS= read -r -d '' file; do
    mv "$file" "${file%.ets}.ts"
done < <(find "$work" -name '*.ets' -print0)

# The one file that is not the module's: the default import an ArkTS
# `import xlogNapi from 'libmarsrs_xlog.so'` answers, which in ArkTS is the
# whole of what the library exports.
cat > "$work/napi_shim.ts" <<'TS'
// Written by scripts/check_harmony_arkts.sh: what `import xlogNapi from
// 'libmarsrs_xlog.so'` answers. Not a file of the package.
import * as xlogNapi from './src/main/cpp/types/libmarsrs_xlog';

export default xlogNapi;
TS

cat > "$work/tsconfig.json" <<'JSON'
{
  "compilerOptions": {
    "target": "ES2020",
    "lib": ["ES2020"],
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "noEmit": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noImplicitReturns": true,
    "noFallthroughCasesInSwitch": true,
    "forceConsistentCasingInFileNames": true,
    "skipLibCheck": true,
    "types": [],
    "baseUrl": ".",
    "paths": {
      "libmarsrs_xlog.so": ["./napi_shim"]
    }
  },
  "include": ["**/*.ts"],
  // `hvigorfile.ts` is hvigor's own build script and not the package's ArkTS:
  // it imports `@ohos/hvigor-ohos-plugin`, which a runner does not have —
  // `ohpm install` is what fetches it, and that is a registry call this check
  // does not make — and what it says is the five lines every HAR's says.
  "exclude": ["hvigorfile.ts"]
}
JSON

"$tsc" --version
# `--pretty false`: a red box of colour escapes is not what a run log wants.
"$tsc" --pretty false -p "$work/tsconfig.json"

# Counted as the program counts them and not as the directory holds them: the
# five `.ets` of the package — the ones under `src/main/ets/xlog/` — plus the
# `index.d.ts` of the NAPI module, which is the file the shim above is written
# out of. Left out are the two that are not in the program: `napi_shim.ts`,
# which this script wrote, and `hvigorfile.ts`, which tsconfig excludes —
# hvigor's own build script, whose import of `@ohos/hvigor-ohos-plugin` a
# runner does not have. Both are `.ts` in the same tree, so a `find` for the
# extension counted eight files for a program of six.
echo "type-checked $(find "$work" -name '*.ts' -not -name 'napi_shim.ts' -not -name 'hvigorfile.ts' | wc -l | tr -d ' ') ArkTS file(s) of $types as TypeScript"
