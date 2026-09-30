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

# What Dart asks for: `_invoke<T>('name', …)`, `_send('name', …)`, and the
# `_channel.invokeMethod<T>('name', …)` of the one call that is not a member
# yet — `open`, which every app makes first. The file is read as one line,
# because the call and the name are not always on the same one.
#
# The generic in front of the name is not matched by shape — it holds one of
# its own (`List<dynamic>`), so `>` is not the end of it — only by being the
# thing between the call and the name.
dart_methods() {
    local dir="$1"
    for file in "$dir"/lib/*.dart; do
        tr '\n' ' ' <"$file"
        echo
    done |
        grep -hoE "(_invoke|invokeMethod|_send)[^(]*\( *'[A-Za-z]+'" |
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

# The methods the channel has to carry, written down rather than read out of
# the three files: a name misspelled the same way in all six of them is
# invisible to a check that only ever compares them with each other, and it is
# the mistake a rename makes most easily.
declare -a required=(
    open
    log
    isLoggable
    flush
    requestFlush
    setLevel
    getLevel
    setMode
    setConsoleLogEnabled
    setMaxFileSize
    setMaxAliveTime
    currentLogPath
    logFiles
    logFileNames
    close
)

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

    for method in "${required[@]}"; do
        if ! echo "$dart" | grep -qx "$method" &&
            ! echo "$kotlin" | grep -qx "$method" &&
            ! echo "$objc" | grep -qx "$method"; then
            echo "::error::$name: '$method' is gone from all three halves of the channel"
            status=1
        elif ! echo "$kotlin" | grep -qx "$method"; then
            echo "::error::$name: the Kotlin half has no arm for '$method'"
            status=1
        elif ! echo "$objc" | grep -qx "$method"; then
            echo "::error::$name: the Objective-C half has no branch for '$method'"
            status=1
        fi
    done

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

    # The other direction is only worth saying out loud: a handler no Dart
    # member asks for is a channel method an app may still call by hand —
    # `getLevel` is one today — and not a member this package's `Xlog` lacks.
    for method in $(echo "$kotlin"$'\n'"$objc" | sort -u); do
        if ! echo "$dart" | grep -qx "$method"; then
            echo "::notice::$name: '$method' is on the channel but not asked for by lib/*.dart"
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
