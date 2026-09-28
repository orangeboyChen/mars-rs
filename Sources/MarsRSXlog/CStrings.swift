// The C strings an `Xlog` call needs, lent for the length of the call.
//
// `mars_xlog_new_instance` and `mars_xlog_write_instance` take four `const
// char*` — the directory and the prefix, the key and the cache; the tag, the
// file, the function and the message — and the C ABI copies what it needs out
// of them before it answers, so a pointer `String.withCString` lends is enough.
// It is good for the length of its closure and not a moment longer, which is
// what makes this a function over a closure rather than a function over four
// strings.
//
// `Sources/MarsRSNet/CStrings.swift` is the same seam for the net half, and is
// separate because the two are modules that do not import each other.

import Foundation

/// The four C strings a call needs, valid for the length of `body`.
///
/// The nesting is what it takes: `withCString` lends one pointer per string and
/// only for the length of its own closure, so `body` runs at the bottom of four
/// of them, where all four are good at once.
internal func withCStrings<R>(
    first: String,
    second: String,
    third: String,
    fourth: String,
    body: (UnsafePointer<CChar>, UnsafePointer<CChar>, UnsafePointer<CChar>, UnsafePointer<CChar>) -> R
) -> R {
    first.withCString { firstPointer in
        second.withCString { secondPointer in
            third.withCString { thirdPointer in
                fourth.withCString { fourthPointer in
                    body(firstPointer, secondPointer, thirdPointer, fourthPointer)
                }
            }
        }
    }
}
