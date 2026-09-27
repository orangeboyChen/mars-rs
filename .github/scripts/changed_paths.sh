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
#   GITHUB_OUTPUT       where `needed=true|false` is written
#
# The one way this must never fail is by concluding "nothing changed": an
# answer it cannot derive, an event that is not a diff, a first push to a
# branch that had no before — all of them run the gate.

set -euo pipefail

needed() {
  printf 'needed=%s\n' "${1:-true}" >> "${GITHUB_OUTPUT:-/dev/stdout}"
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
# against, which a fork's clone does not carry: ask for it before giving up.
git fetch --no-tags --depth=1 origin "$BASE_SHA" 2>/dev/null || true

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
