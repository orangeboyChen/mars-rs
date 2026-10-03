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
#   JITPACK_ARTIFACT       marsrs
#   JITPACK_ARTIFACT_XLOG  xlog

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
# The log of the build that was triggered, and not of a second spelling of the
# repository: TRIGGERED is the one jitpack.io answered for, and a link built
# out of JITPACK_GROUP alone points at a build nobody asked for.
LOG="https://jitpack.io/${TRIGGERED#https://jitpack.io/api/builds/}/build.log"
poll="$(mktemp)"
trap 'rm -f "$poll"' EXIT

# What jitpack.io said, and whether it said anything at all — which curl and
# python3 between them cannot tell apart. A build that has not started yet
# answers 404, a connection that was refused answers nothing, and a body that
# is not JSON is swallowed by the parse: every one of the three leaves an empty
# line, and an empty line was what this script read as "still building" and
# waited out, so an unreachable registry, a 401 and an unrecognised body were
# all twenty minutes of green. The HTTP status is taken for itself, and only a
# body that came back as an answer is read as one.
status() {
  # `-o` and `-w`, and not a pipe: `pipefail` makes a 404 a failed pipeline,
  # and there would be nothing left to read the answer out of.
  code="$(curl -sS -o "$poll" -w '%{http_code}' "$TRIGGERED" 2>/dev/null)" || code=000
  case "$code" in
    2??) ;;
    # a build that has not started yet answers 404; that is not a
    # failure, it is a build to wait for
    404) echo "not started"; return 0 ;;
    *)
      echo "::error::jitpack.io answered $code for $TRIGGERED" >&2
      return 1 ;;
  esac
  # A 200 that is not JSON is an HTML page and not a status: it says nothing
  # about the build, and waiting it out is what made it look like a slow one.
  state="$(python3 -c '
import json, os, sys

try:
    doc = json.load(open(sys.argv[1]))
except ValueError:
    sys.exit(1)

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
# Nothing about this tag yet is a build jitpack.io has not taken up, which is
# a wait and not an answer; the loop is what decides how long a wait is one.
print(found or "not started")' "$poll")" || return 1
  echo "$state"
}
took_up=0
for i in $(seq 1 40); do
  # No `|| true` now: a poll jitpack.io did not answer is a failure of its own,
  # and not a build that is merely slow.
  if ! STATE="$(status)"; then
    echo "::error::no status for $TAG on jitpack.io, see $LOG"
    exit 1
  fi
  echo "JitPack $TAG: $STATE"
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
    *)
      # Neither `ok` nor `error`: a build jitpack.io has taken up and not
      # finished — "Building" — or a status this script was not written for.
      [ "$STATE" = "not started" ] || took_up=1 ;;
  esac
  sleep 30
done
# Two ways for the twenty minutes to run out, and only one of them is a slow
# build: jitpack.io took the build up and has not finished it, which
# publish.yml can ask about again, or it never took it up at all — forty
# answers with no build of this tag in any of them, which is not a wait.
if [ "$took_up" = 1 ]; then
  echo "::warning::JitPack is still building after 20 minutes, see $LOG"
  exit 0
fi
echo "::error::JitPack never took up the build of $TAG, see $LOG"
exit 1
