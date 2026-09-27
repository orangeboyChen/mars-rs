#!/usr/bin/env bash
# Trigger the JitPack build of a release and wait for its answer.
#
# This is the `Trigger the JitPack build` step of
# .github/workflows/release.yml. JitPack builds lazily, so nothing would
# publish the AAR until a consumer asks for it: this kicks the build off once
# the release assets are online and waits for the answer.
#
# Environment:
#   TAG                    the tag the release was published as
#   JITPACK_GROUP          the group the AARs are published under
#   JITPACK_ARTIFACT       mars-rs
#   JITPACK_ARTIFACT_XLOG  mars-rs-xlog

set -euo pipefail

: "${TAG:?}" "${JITPACK_GROUP:?}" "${JITPACK_ARTIFACT:?}" "${JITPACK_ARTIFACT_XLOG:?}"

# the group the AARs are published under, and the one JitPack always
# answers for; both name the same repository, and one build of it
# publishes both modules
TRIGGERED=""
for REPO in "$JITPACK_GROUP.$JITPACK_ARTIFACT" "com.github.orangeboyChen.mars-rs"; do
  URL="https://jitpack.io/api/builds/$REPO/$TAG"
  if curl -sSfL "$URL" > /dev/null; then
    echo "asked jitpack.io to build $REPO $TAG"
    TRIGGERED="$URL"
    break
  fi
done
if [ -z "$TRIGGERED" ]; then
  echo "::error::jitpack.io answered none of the build requests for $TAG"
  exit 1
fi
LOG="https://jitpack.io/$JITPACK_GROUP.$JITPACK_ARTIFACT/$TAG/build.log"
status() {
  # a build that has not started yet answers 404; that is not a
  # failure, it is a build to wait for
  curl -sSfL "$TRIGGERED" 2>/dev/null | python3 -c '
import json, os, sys

try:
    doc = json.load(sys.stdin)
except ValueError:
    sys.exit(0)

# the version as tagged, and as JitPack writes it without the "v"
wanted = {os.environ["TAG"], os.environ["TAG"].lstrip("v")}
found = ""

def walk(node):
    global found
    if isinstance(node, dict):
        for key, value in node.items():
            if key in wanted and isinstance(value, str):
                found = value
            else:
                walk(value)
    elif isinstance(node, list):
        for item in node:
            walk(item)

walk(doc)
print(found)'
}
for i in $(seq 1 40); do
  STATE=$(status || true)
  echo "JitPack $TAG: ${STATE:-unknown}"
  case "$(printf '%s' "$STATE" | tr '[:upper:]' '[:lower:]')" in
    ok)
      echo "JitPack build succeeded"
      # an "ok" build is not proof that both AARs resolve: check each
      # one, over the group spellings and the two versions JitPack
      # answers for. A warning and not an error — the release is
      # published either way, and the URL convention is JitPack's.
      for ARTIFACT in "$JITPACK_ARTIFACT" "$JITPACK_ARTIFACT_XLOG"; do
        FOUND=""
        for GROUP in "$JITPACK_GROUP" "com.github.orangeboyChen.mars-rs"; do
          for VERSION in "$TAG" "${TAG#v}"; do
            URL="https://jitpack.io/$(printf '%s' "$GROUP" | tr '.' '/')/$ARTIFACT/$VERSION/$ARTIFACT-$VERSION.aar"
            if curl -sSfLo /dev/null "$URL"; then
              echo "resolved $GROUP:$ARTIFACT:$VERSION"
              FOUND="$URL"
            fi
          done
        done
        [ -n "$FOUND" ] || echo "::warning::$ARTIFACT did not resolve on jitpack.io for $TAG, see $LOG"
      done
      exit 0 ;;
    error)
      echo "::error::JitPack build failed, see $LOG"
      curl -sSfL "$LOG" | tail -40 || true
      exit 1 ;;
  esac
  sleep 30
done
echo "::warning::JitPack is still building after 20 minutes, see $LOG"
