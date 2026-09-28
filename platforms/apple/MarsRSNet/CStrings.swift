// The two ways this module hands a C string, or an array of them, across: the
// ones it lends for the length of a call, and the ones it copies because the C
// side reads them after the answer has come back.
//
// Both seams of the net half ask the caller something and read what it answered
// *after* the call that answered has returned: `mars_sdt_run_checks` copies the
// addresses a resolve found, `mars_stn_*` the body of a task and the buffer of a
// check. A pointer `String.withCString` lends is good for the length of its
// closure and not a moment longer, so an answer cannot be made of those: it is
// made of the copies `Held` keeps.
//
// A question, on the other hand, is answered before it is over — the hosts a
// diagnosis is started with, the strings of a task — and those are lent, because
// nothing has to outlive the call they were handed to.

import Foundation

/// The C strings of `values`, lent for the length of `body`.
///
/// The recursion is what it takes: `withCString` lends one pointer per string
/// and only for the length of its own closure, so every string of `values` adds
/// one more closure to the nest and `body` runs at the bottom of it, where all
/// of them are good at once.
internal func withCStrings<R>(_ values: [String], body: ([UnsafePointer<CChar>?]) -> R) -> R {
    var pointers: [UnsafePointer<CChar>?] = []
    func next(_ index: Int) -> R {
        guard index < values.count else {
            return body(pointers)
        }
        return values[index].withCString { pointer in
            pointers.append(pointer)
            return next(index + 1)
        }
    }
    return next(0)
}

/// The C strings of `values`, as the array of pointers a `const char* const*`
/// wants; `nil` for an empty list, which is what a C caller writes for one.
///
/// Lent for the length of `body`, like [`withCStrings`], and counted here rather
/// than by the caller because every list of the C surface is a pointer and a
/// count that have to agree.
internal func withStrings<R>(
    _ values: [String],
    body: (UnsafePointer<UnsafePointer<CChar>?>?) -> R
) -> R {
    guard !values.isEmpty else {
        return body(nil)
    }
    let list = UnsafeMutablePointer<UnsafePointer<CChar>?>.allocate(capacity: values.count)
    defer { list.deallocate() }
    func next(_ index: Int) -> R {
        guard index < values.count else {
            return body(UnsafePointer(list))
        }
        return values[index].withCString { pointer in
            list[index] = pointer
            return next(index + 1)
        }
    }
    return next(0)
}

/// The C strings of every list of `lists`, as one array of pointers a
/// `MarsStnStrings` is made of; `nil` for a list with nothing in it.
internal func withStringLists<R>(
    _ lists: [[String]],
    body: ([UnsafePointer<UnsafePointer<CChar>?>?]) -> R
) -> R {
    var done: [UnsafePointer<UnsafePointer<CChar>?>?] = []
    func next(_ index: Int) -> R {
        guard index < lists.count else {
            return body(done)
        }
        return withStrings(lists[index]) { list in
            done.append(list)
            return next(index + 1)
        }
    }
    return next(0)
}

/// What `pointer` says; empty when there is no pointer, which is what the C ABI
/// hands for a string it does not have.
internal func string(_ pointer: UnsafePointer<CChar>?) -> String {
    guard let pointer else {
        return ""
    }
    return String(cString: pointer)
}

/// The strings of `list`, in order.
internal func strings(from list: MarsStnStrings) -> [String] {
    guard let items = list.items else {
        return []
    }
    return (0 ..< Int(list.count)).map { index in string(items[index]) }
}

/// The bytes of `pointer` and `count`; nothing when there is no pointer, which
/// is what the C ABI hands for a body it does not have.
internal func bytes(from pointer: UnsafePointer<UInt8>?, count: UInt32) -> [UInt8] {
    guard let pointer else {
        return []
    }
    return [UInt8](UnsafeBufferPointer(start: pointer, count: Int(count)))
}

/// One answer's worth of C memory, owned here and thrown away when the next
/// answer is made: the strings and bytes a Swift answer points at.
///
/// Both boxes of this module — the diagnosis' probe and the pipeline's app —
/// hold one of these and clear it before they answer the next question, which is
/// when the C side has finished reading the one before.
internal struct Held {
    /// What `strdup` made, which `free` takes back.
    private var copies: [UnsafeMutablePointer<CChar>] = []
    /// What `allocate` made — the strings of one list, and the bytes of one
    /// answer — which `deallocate` takes back.
    private var arrays: [UnsafeMutableRawPointer] = []

    /// The NUL-terminated copies of `values`, as the array of pointers a
    /// `const char* const*` wants; `nil` for an empty list, which is what a C
    /// caller writes for one.
    internal mutating func strings(_ values: [String]) -> UnsafePointer<UnsafePointer<CChar>?>? {
        guard !values.isEmpty else {
            return nil
        }
        let list = UnsafeMutablePointer<UnsafePointer<CChar>?>.allocate(capacity: values.count)
        arrays.append(UnsafeMutableRawPointer(list))
        for (index, value) in values.enumerated() {
            list[index] = copy(of: value)
        }
        return UnsafePointer(list)
    }

    /// The copy of `value`, as the pointer and count a `const unsigned char*`
    /// wants; `nil` for nothing, which is an empty body and not one that is read.
    internal mutating func bytes(_ value: [UInt8]) -> UnsafePointer<UInt8>? {
        guard !value.isEmpty else {
            return nil
        }
        let copy = UnsafeMutablePointer<UInt8>.allocate(capacity: value.count)
        _ = UnsafeMutableBufferPointer(start: copy, count: value.count).initialize(from: value)
        arrays.append(UnsafeMutableRawPointer(copy))
        return UnsafePointer(copy)
    }

    /// Throws away everything the answer before this one was made of.
    internal mutating func clear() {
        for pointer in copies {
            free(UnsafeMutableRawPointer(pointer))
        }
        for array in arrays {
            array.deallocate()
        }
        copies = []
        arrays = []
    }

    /// A copy of `value` in memory `free` takes back; `nil` when there is none
    /// to be had, which the list of `strings` leaves empty rather than shorter.
    private mutating func copy(of value: String) -> UnsafePointer<CChar>? {
        value.withCString { source in
            // The C function's own name: the call is one Swift reads as the copy
            // it is, and `copies` is what takes it back.
            guard let copy = strdup(source) else {
                return nil
            }
            copies.append(copy)
            return UnsafePointer(copy)
        }
    }
}
