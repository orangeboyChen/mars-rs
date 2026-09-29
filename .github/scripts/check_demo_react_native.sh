#!/usr/bin/env bash
# Type-checks `demo/react-native/App.tsx` against the React Native module of
# this tree.
#
#   NODE_MODULES=<a node_modules with react, react-native, react-native-fs>
#   .github/scripts/check_demo_react_native.sh
#
# Environment:
#   NODE_MODULES   where the dependencies `App.tsx` imports are installed — the
#                  `npm install --prefix` of the workflow, whose `node_modules`
#                  this is
#   TSC            the `tsc` to run; `tsc` on PATH when it is not set
#
# What it is for is the question nothing else in this repository asks. The demo
# is a complete `App.tsx` and not a complete project — the `android/` and
# `ios/` halves of one are a hundred files `npx @react-native-community/cli
# init` writes — so there is no build here to run and no `tsc` in the demo's
# own directory to ask. A symbol the module stops exporting is a demo a reader
# finds broken, and not a red job.
#
# The module is read out of `platforms/react-native/` and not off npm, because
# what this gate is about is drift *here*: the published package is a snapshot
# of a release, so a check against it stays green until the next release and
# says nothing about the commit that moved the module out from under the demo.
# It is installed under the name `App.tsx` imports it by, which is the whole of
# what npm would have done.

set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
modules="${NODE_MODULES:?NODE_MODULES is not set — npm install react react-native react-native-fs into a prefix and point it at that node_modules}"
tsc="${TSC:-tsc}"

if ! command -v "$tsc" > /dev/null 2>&1; then
    echo "::error::$tsc is not on PATH — npm install typescript, or pass TSC=<path to tsc>"
    exit 1
fi

project="$(dirname "$modules")"
module="$modules/marsrs-react-native-xlog"

mkdir -p "$module"
cp "$repo/platforms/react-native/marsrs-xlog/package.json" "$module/"
cp -R "$repo/platforms/react-native/marsrs-xlog/src" "$module/"

# The demo, as its own project: one file, and the options a React Native app is
# compiled with — JSX without an import of React, and the resolution a bundler
# does and Node never did.
cp "$repo/demo/react-native/App.tsx" "$project/App.tsx"
cat > "$project/tsconfig.json" <<'JSON'
{
  "compilerOptions": {
    "target": "ES2020",
    "lib": ["ES2020", "DOM"],
    "jsx": "react-jsx",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true,
    "esModuleInterop": true,
    "allowSyntheticDefaultImports": true
  },
  "files": ["App.tsx"]
}
JSON

cd "$project"
"$tsc" --noEmit -p tsconfig.json
