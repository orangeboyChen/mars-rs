#!/usr/bin/env bash
# Keeps the three halves of each Flutter plugin on one channel.
#
# A Flutter plugin is a method channel and three files that have to agree on
# what crosses it: `lib/<package>.dart` names a method and awaits an answer,
# the Kotlin half dispatches it in one `when (call.method)`, and the
# Objective-C half in one chain of `isEqualToString:`. Nothing compiles all
# three together — the Dart is analyzed against pub.dev's copy of the package
# and not against this tree — so a method one of them never heard of is a
# `MissingPluginException` at runtime and a green build.
#
# What is checked is the direction that hurts: every method Dart asks for must
# have a handler on both platform halves, and the two packages must ask for the
# same set, because they are one package under two names.

set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

declare -a packages=(
    "marsrs:platforms/flutter/marsrs"
    "marsrs_xlog:platforms/flutter/marsrs-xlog"
)

# What Dart asks for: `_invoke<T>('name', …)` and `_send('name', …)`. The
# file is read one line — the call and the name are not always on the same
# one — so a folded copy is what the pattern is matched against.
dart_methods() {
    local dir="$1"
    for file in "$dir"/lib/*.dart; do
        tr '\n' ' ' <"$file"
        echo
    done |
        # The generic in front of the name is not matched by shape — it holds
        # one of its own (`List<dynamic>`), so `>` is not the end of it —
        # only by being the thing between the call and the name.
        grep -hoE "_invoke[^']{0,40}'[A-Za-z]+'|_send\( *'[A-Za-z]+'" |
        sed -nE "s/.*'([A-Za-z]+)'/\1/p" | sort -u
}

# What the Kotlin half dispatches: `"name" -> handler(call, result)`.
kotlin_methods() {
    local dir="$1"
    find "$dir"/android/src/main/kotlin -name '*.kt' -exec cat {} + |
        grep -hoE '"[A-Za-z]+" -> [a-z][A-Za-z]*\(call, result\)' |
        grep -oE '^"[A-Za-z]+"' | tr -d '"' | sort -u
}

# What the Objective-C half dispatches: `[call.method isEqualToString:@"name"]`.
objc_methods() {
    local dir="$1"
    grep -hoE 'isEqualToString:@"[A-Za-z]+"' "$dir"/ios/Classes/*.m |
        grep -oE '"[A-Za-z]+"' | tr -d '"' | sort -u
}

status=0
previous=""

for entry in "${packages[@]}"; do
    name="${entry%%:*}"
    dir="$root/${entry#*:}"

    dart="$(dart_methods "$dir")"
    kotlin="$(kotlin_methods "$dir")"
    objc="$(objc_methods "$dir")"

    if [ -z "$dart" ]; then
        echo "::error::$name: no method found in lib/*.dart — the pattern this check reads is gone"
        status=1
        continue
    fi
    echo "$name: $(echo "$dart" | wc -l | tr -d ' ') methods in Dart, $(echo "$kotlin" | wc -l | tr -d ' ') in Kotlin, $(echo "$objc" | wc -l | tr -d ' ') in Objective-C"

    for method in $dart; do
        if ! echo "$kotlin" | grep -qx "$method"; then
            echo "::error::$name: '$method' has no arm in the Kotlin 'when (call.method)'"
            status=1
        fi
        if ! echo "$objc" | grep -qx "$method"; then
            echo "::error::$name: '$method' has no branch in the Objective-C handleMethodCall"
            status=1
        fi
    done

    # The two are one package under two names, so a method one of them
    # carries and the other does not is a member an app on the other package
    # cannot reach.
    if [ -n "$previous" ]; then
        for method in $dart; do
            if ! echo "$previous" | grep -qx "$method"; then
                echo "::error::$name: '$method' is not in the other Flutter package"
                status=1
            fi
        done
        for method in $previous; do
            if ! echo "$dart" | grep -qx "$method"; then
                echo "::error::$name: the other Flutter package carries '$method' and this one does not"
                status=1
            fi
        done
    fi
    previous="$dart"
done

if [ "$status" -eq 0 ]; then
    echo "the Flutter plugins agree: one channel, three halves, two names"
fi
exit "$status"
