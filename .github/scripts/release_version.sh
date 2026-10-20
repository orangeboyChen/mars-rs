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
#   XCFRAMEWORK_ZIP, XCFRAMEWORK_NET_ZIP, NATIVE_ZIP, MARSRS_AAR, MARSRS_XLOG_AAR,
#   KMP_NATIVE_ZIP, KMP_MAVEN_ZIP
#                              the assets a complete release carries. Each is
#                              defaulted below to the name release.yml gives it,
#                              so the invocation above works with none of them
#                              set — it was the one thing standing between the
#                              documented example and `unbound variable`.
#   CRATES_IO_UA               the user agent crates.io is asked with: it
#                              answers 403 to the one `curl` sends by default
#   GH_TOKEN                   what `gh release view` reads
#   GITHUB_OUTPUT              appended to: tag, version, prerelease, is_new,
#                              skip
#   GITHUB_STEP_SUMMARY        appended to: the table of the same five
#
# "Complete" is asked of crates.io as well as of the release: a release whose
# crates.io job went red carries every asset above and none of the nine crates,
# and a version that looks complete is a version a rerun skips.

set -euo pipefail

# Set by the workflow, defaulted here so that the script also runs outside it.
GITHUB_OUTPUT="${GITHUB_OUTPUT:-/dev/null}"
GITHUB_STEP_SUMMARY="${GITHUB_STEP_SUMMARY:-/dev/null}"
CRATES_IO_UA="${CRATES_IO_UA:-mars-rs release (https://github.com/orangeboyChen/mars-rs)}"
# The names the workflow sets them to, which are the names the assets of every
# release carry; see the `env:` block of release.yml.
XCFRAMEWORK_ZIP="${XCFRAMEWORK_ZIP:-marsrs-xlog.xcframework.zip}"
XCFRAMEWORK_NET_ZIP="${XCFRAMEWORK_NET_ZIP:-marsrs-net.xcframework.zip}"
NATIVE_ZIP="${NATIVE_ZIP:-marsrs-android-native.zip}"
MARSRS_AAR="${MARSRS_AAR:-marsrs.aar}"
MARSRS_XLOG_AAR="${MARSRS_XLOG_AAR:-xlog.aar}"
KMP_NATIVE_ZIP="${KMP_NATIVE_ZIP:-marsrs-kmp-native.zip}"
KMP_MAVEN_ZIP="${KMP_MAVEN_ZIP:-marsrs-kmp-maven.zip}"

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
ASSETS="$XCFRAMEWORK_ZIP $XCFRAMEWORK_NET_ZIP $NATIVE_ZIP $MARSRS_AAR $MARSRS_XLOG_AAR"
# the Kotlin Multiplatform pair: the archives the module is built out of, and
# the maven repository it is published as. Both are one asset whatever the
# version is, unlike the host archives below.
ASSETS="$ASSETS $KMP_NATIVE_ZIP $KMP_MAVEN_ZIP"
# the two pods that carry a binary, named by the version the way the host
# archives below are: a pod is downloaded under the version it is, so an
# archive of another version is not the one its podspec names
ASSETS="$ASSETS marsrs-cocoapods-xlog-$VERSION.zip marsrs-cocoapods-net-$VERSION.zip"
# the four packages of the two plugin ecosystems, one archive each, and named
# by the package: `package_flutter.sh` and `package_react_native.sh` write
# them into `dist` under the name and the version npm and pub read
ASSETS="$ASSETS marsrs-flutter-$VERSION.tar.gz marsrs-flutter-xlog-$VERSION.tar.gz"
ASSETS="$ASSETS marsrs-react-native-$VERSION.tgz marsrs-react-native-xlog-$VERSION.tgz"
# HarmonyOS, two ways in: the three `.so` and the headers an app writes a NAPI
# module of its own against, and the HAR of the ArkTS and the same Rust core
ASSETS="$ASSETS marsrs-harmony-$VERSION.tar.gz marsrs-harmonyos-xlog-$VERSION.har"
# one archive per host of the matrix in release.yml, in the shape that host
# packages it in — the C ABI and then the `xlog` CLI, of the same three hosts
for HOST in x86_64-unknown-linux-gnu:tar.gz aarch64-apple-darwin:tar.gz \
            x86_64-pc-windows-msvc:zip; do
  ASSETS="$ASSETS marsrs-$VERSION-${HOST%%:*}.${HOST##*:}"
  ASSETS="$ASSETS marsrs-xlog-cli-$VERSION-${HOST%%:*}.${HOST##*:}"
done
if gh release view "$TAG" >/dev/null 2>&1; then
  PUBLISHED=$(gh release view "$TAG" --json assets -q '.assets[].name')
  COMPLETE=true
  for ASSET in $ASSETS; do
    grep -qx "$ASSET" <<< "$PUBLISHED" || COMPLETE=false
  done

  # The nine crates, asked of crates.io the way publish_crates.sh asks it. A
  # release can carry every asset above and still be a release that put four of
  # the nine on crates.io — which is what v0.1.0-alpha.3 was, and what this
  # test used to miss, because it only ever counted files on the release. A
  # version crates.io does not have is one a rerun has to publish, so a crate
  # that is missing is a `skip=false` and not a green light to do nothing.
  for CRATE in marsrs-core marsrs-comm marsrs-crypt marsrs-buffer \
               marsrs-appender marsrs-sdt marsrs-stn marsrs-xlog marsrs; do
    # Three asks and not one: a 429 or a 5xx is crates.io being busy and not a
    # fact about the version. What is left after three is unknown, and unknown
    # is treated as missing — the other way round, a release whose five crates
    # are absent is skipped because crates.io did not answer about them.
    status=000
    for _ in 1 2 3; do
      status="$(curl -sS -o /dev/null -w '%{http_code}' -A "$CRATES_IO_UA" \
        --max-time 30 "https://crates.io/api/v1/crates/$CRATE/$VERSION")" || status=000
      case "$status" in
        429|5[0-9][0-9]) sleep 5; continue ;;
      esac
      break
    done
    case "$status" in
      200) ;;
      404)
        echo "::error::$CRATE $VERSION is not on crates.io, and the release that should carry it is already out"
        COMPLETE=false
        ;;
      *)
        echo "::error::crates.io answered $status for $CRATE $VERSION; whether it is up is unknown, so $TAG is treated as incomplete"
        COMPLETE=false
        ;;
    esac
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
