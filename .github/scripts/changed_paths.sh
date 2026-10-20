#!/usr/bin/env bash
# Does this run's diff touch any of the globs it is given?
#
#   .github/scripts/changed_paths.sh <glob> ...
#
# The filter that answers that used to sit in a workflow's `on:`, and there it
# stopped the whole run: a run that never starts reports no status at all, so
# the ten checks the ruleset asks for stayed "expected" and the pull request
# stayed BLOCKED — #108 waited two days for statuses that were not coming.
# What a skipped *job* reports is a status, and a skipped one satisfies the
# requirement, which is the difference that matters here: every run starts,
# every job starts, and a job with nothing to read says so and stops. The
# minutes are not spent and the check is still answered.
#
# Environment:
#   BASE_SHA, HEAD_SHA  the two ends of the diff, taken from the event
#   GITHUB_OUTPUT       where `<key>=true|false` is written
#   OUTPUT_KEY          the name it is written under; `needed` when it is not
#                       set, which is the one every caller already reads. A
#                       caller asking more than one question of one diff —
#                       demo.yml, of eight demos with eight different upstreams
#                       — sets it per call and reads one output per question.
#
# The one way this must never fail is by concluding "nothing changed": an
# answer it cannot derive, an event that is not a diff, a first push to a
# branch that had no before — all of them run the gate.

set -euo pipefail

needed() {
  printf '%s=%s\n' "${OUTPUT_KEY:-needed}" "${1:-true}" >> "${GITHUB_OUTPUT:-/dev/stdout}"
  exit 0
}

if [ -z "${BASE_SHA:-}" ] || [ -z "${HEAD_SHA:-}" ]; then
  needed true
fi
case "$BASE_SHA$HEAD_SHA" in
  *[!0-9a-fA-F]*) needed true ;;
esac
# A base of all zeroes is a branch that did not exist before, so every file of
# the commit is a file that changed.
case "$BASE_SHA" in
  *[!0]*) ;;
  *) needed true ;;
esac

# The base of a pull request is a commit of the repository it was opened
# against, which a fork's clone does not carry: ask for it, but only when it is
# missing — a `--depth=1` fetch of a commit the clone already has would draw a
# shallow boundary the merge base of the diff below may sit behind.
if ! git cat-file -e "$BASE_SHA^{commit}" 2>/dev/null; then
  # A fetch that cannot get it is answered by what comes after and not by
  # this line: `git diff` of a commit the clone does not have fails too, and
  # a diff it cannot compute is one of the answers this script must never
  # conclude "nothing changed" from. So the `|| true` is the fetch's and the
  # gate's answer is the `needed true` below.
  git fetch --no-tags --depth=1 origin "$BASE_SHA" 2>/dev/null || true
fi

changed=$(mktemp)
trap 'rm -f "$changed"' EXIT
git diff --name-only "$BASE_SHA...$HEAD_SHA" > "$changed" 2>/dev/null || needed true

while IFS= read -r file; do
  for glob in "$@"; do
    # shellcheck disable=SC2254 # an unquoted pattern is the point of it
    case $file in
      $glob) needed true ;;
    esac
  done
done < "$changed"

needed false
