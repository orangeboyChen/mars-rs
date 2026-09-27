#!/usr/bin/env bash
# Resolve the version a release is published as, and whether it is already out.
#
# This is the `Resolve the version` step of .github/workflows/release.yml,
# lifted out of the YAML so that it can be read — and run — on its own:
#
#   INPUT_BUMP=patch INPUT_CHANNEL=alpha GITHUB_OUTPUT=/dev/null \
#     .github/scripts/release_version.sh
#
# with `git` and `gh` on $PATH pointing at a stubbed origin if the answer is
# wanted without a network.
#
# Environment:
#   INPUT_VERSION              the version, typed out (overrides the bump)
#   INPUT_BUMP                 major | minor | patch
#   INPUT_CHANNEL              stable | alpha | beta
#   INPUT_PRERELEASE           the number of the pre-release, when typed
#   XCFRAMEWORK_ZIP, XCFRAMEWORK_NET_ZIP, NATIVE_ZIP, CORE_AAR, XLOG_AAR,
#   KMP_NATIVE_ZIP, KMP_MAVEN_ZIP
#                              the assets a complete release carries
#   GH_TOKEN                   what `gh release view` reads
#   GITHUB_OUTPUT              appended to: tag, version, prerelease, is_new,
#                              skip
#   GITHUB_STEP_SUMMARY        appended to: the table of the same five

set -euo pipefail

# Set by the workflow, defaulted here so that the script also runs outside it.
GITHUB_OUTPUT="${GITHUB_OUTPUT:-/dev/null}"
GITHUB_STEP_SUMMARY="${GITHUB_STEP_SUMMARY:-/dev/null}"

tags() {
  # every tag of the shape vX.Y.Z or vX.Y.Z-<suffix>, oldest first.
  # awk does the filtering so that a repository with no matching tag
  # yet is an empty list and not a failed pipeline.
  git ls-remote --tags --refs origin \
    | awk -F/ '{ tag = $NF
                 if (tag ~ /^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$/) print tag }' \
    | sort -V
}

if [ -n "${INPUT_VERSION:-}" ]; then
  VERSION="${INPUT_VERSION#v}"
else
  TAGS="$(tags)"
  LAST_STABLE="$(awk '!/[-]/' <<< "$TAGS" | tail -1)"
  LAST_PRE="$(awk '/[-]/' <<< "$TAGS" | tail -1)"
  [ -n "$LAST_STABLE" ] || LAST_STABLE=v0.0.0

  # the greater of two tags, and the version one bump past another
  newest() { printf '%s\n%s\n' "$1" "$2" | sort -V | tail -1; }
  bump_of() {
    IFS='.' read -r MAJOR MINOR PATCH <<< "${1#v}"
    case "${INPUT_BUMP:-patch}" in
      major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
      minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
      patch) PATCH=$((PATCH + 1)) ;;
    esac
    echo "$MAJOR.$MINOR.$PATCH"
  }

  # the version being worked towards: what the last pre-release is
  # of, as long as no stable release has caught up with it yet
  PRE_BASE=""
  if [ -n "$LAST_PRE" ] \
      && [ "$(newest "${LAST_PRE%%-*}" "$LAST_STABLE")" = "${LAST_PRE%%-*}" ] \
      && [ "${LAST_PRE%%-*}" != "$LAST_STABLE" ]; then
    PRE_BASE="${LAST_PRE#v}"
    PRE_BASE="${PRE_BASE%%-*}"
  fi

  if [ "${INPUT_CHANNEL:-stable}" = stable ]; then
    # a pre-release that is out already is promoted to its version:
    # v1.2.4-alpha.2 -> v1.2.4, and no bump is applied
    if [ -n "$PRE_BASE" ]; then
      VERSION="$PRE_BASE"
    else
      VERSION="$(bump_of "$LAST_STABLE")"
    fi
  else
    if [ -n "$PRE_BASE" ]; then
      VERSION="$PRE_BASE"
    else
      VERSION="$(bump_of "$LAST_STABLE")"
    fi
    # alpha.1, alpha.2 …: what was typed, else one past the last one
    # of this channel, else the first.
    #
    # Counting on is only right while it is the same line being continued,
    # which is what a non-empty `$PRE_BASE` means: `v1.1.1-alpha.9` is followed
    # by `v1.1.1-alpha.10`. An empty one means the version moved — a v1.1.1
    # that is out already sends a patch bump to v1.1.2 — and the first alpha of
    # v1.1.2 is `v1.1.2-alpha.1`, not the tenth: the number belongs to the
    # version, not to the repository.
    if [ -n "${INPUT_PRERELEASE:-}" ]; then
      # A number that was typed is taken as it is: an operator who asked for
      # `alpha.10` and got `alpha.1` has to be told, not published, so a bad
      # one is left for the check below to reject instead of being made a
      # harmless-looking first.
      NUMBER="$INPUT_PRERELEASE"
    else
      if [ -n "$PRE_BASE" ]; then
        case "$LAST_PRE" in
          *-${INPUT_CHANNEL}.[0-9]*) NUMBER=$((${LAST_PRE##*-${INPUT_CHANNEL}.} + 1)) ;;
          *)                         NUMBER=1 ;;
        esac
      else
        NUMBER=1
      fi
      # Only what the script derived itself: a tag whose suffix is not a
      # plain number — `beta9`, which sorts before `beta2` — is not a line
      # that can be counted on, so it starts one.
      case "$NUMBER" in '' | *[!0-9]*) NUMBER=1 ;; esac
    fi
    VERSION="$VERSION-$INPUT_CHANNEL.$NUMBER"
  fi
fi

case "$VERSION" in
  *-*) PRERELEASE=true ;;
  *)   PRERELEASE=false ;;
esac
if ! printf '%s' "$VERSION" \
    | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta)(\.[0-9]+)?)?$'; then
  echo "::error::'$VERSION' is not X.Y.Z or X.Y.Z-{alpha,beta}[.N]"
  exit 1
fi

TAG="v$VERSION"
if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
  IS_NEW=false
else
  IS_NEW=true
fi

# never republish a release that is already out: the SPM checksum in
# Package.swift is bound to the zip that was published for the tag.
# A release that is missing an asset stays repairable, though, so
# "complete" means every one of them.
SKIP=false
ASSETS="$XCFRAMEWORK_ZIP $XCFRAMEWORK_NET_ZIP $NATIVE_ZIP $CORE_AAR $XLOG_AAR"
# the Kotlin Multiplatform pair: the archives the module is built out of, and
# the maven repository it is published as. Both are one asset whatever the
# version is, unlike the host archives below.
ASSETS="$ASSETS $KMP_NATIVE_ZIP $KMP_MAVEN_ZIP"
# one archive per host of the matrix in release.yml, in the shape that host
# packages it in
for HOST in x86_64-unknown-linux-gnu:tar.gz aarch64-apple-darwin:tar.gz \
            x86_64-pc-windows-msvc:zip; do
  ASSETS="$ASSETS mars-rs-$VERSION-${HOST%%:*}.${HOST##*:}"
done
if gh release view "$TAG" >/dev/null 2>&1; then
  PUBLISHED=$(gh release view "$TAG" --json assets -q '.assets[].name')
  COMPLETE=true
  for ASSET in $ASSETS; do
    grep -qx "$ASSET" <<< "$PUBLISHED" || COMPLETE=false
  done
  [ "$COMPLETE" = false ] || SKIP=true
fi

echo "tag=$TAG" >> "$GITHUB_OUTPUT"
echo "version=$VERSION" >> "$GITHUB_OUTPUT"
echo "prerelease=$PRERELEASE" >> "$GITHUB_OUTPUT"
echo "is_new=$IS_NEW" >> "$GITHUB_OUTPUT"
echo "skip=$SKIP" >> "$GITHUB_OUTPUT"
{
  echo "### Release $TAG"
  echo ""
  echo "| | |"
  echo "|---|---|"
  echo "| version | \`$VERSION\` |"
  echo "| tag | \`$TAG\` |"
  echo "| pre-release | \`$PRERELEASE\` |"
  echo "| new tag | \`$IS_NEW\` |"
  echo "| already published | \`$SKIP\` |"
} >> "$GITHUB_STEP_SUMMARY"
