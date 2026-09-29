// The path symbols of the C ABI, read back as a `String`.
//
// `mars_xlog_current_log_path` and `mars_xlog_current_log_cache_path` answer no
// pointer: they write into a buffer the caller owns and answer how many bytes
// they wrote, which is what [path(of:)] takes and turns into a string. The
// buffer is ours and 1,024 bytes long because a path never fills it — a symbol
// that answers a length rather than a pointer is the C ABI's way of saying the
// caller decides how much it can hold.
//
// `mars_xlog_getfilepath_from_timespan` and `mars_xlog_make_logfile_name` are
// the same two arguments over again, but they stand for a list the C++ fills a
// `std::vector` with and a C caller asks for one index of at a time, so
// [paths(of:)] is that walk: index after index until a symbol answers that
// there is nothing at that index.
//
// It is here and not in [Xlog] because `platforms/apple/MarsRSXlog/Xlog.swift`
// class an app is given, and [path(of:)] is the seam it reads a path through —
// the same reason [withCStrings] is in `CStrings.swift` and not beside the call
// that needs it.

import Foundation

/// The buffer the path symbols write into.
internal enum PathBuffer {
    /// How many bytes it is; a path never fills it.
    internal static let size = 1_024
}

/// What `body` wrote, as a `String`, or `nil` when it wrote nothing: `0` is
/// what a path symbol answers for an appender with no open file.
internal func path(of body: (UnsafeMutablePointer<CChar>, UInt32) -> Int32) -> String? {
    var buffer = [CChar](repeating: 0, count: PathBuffer.size)
    let written = body(&buffer, UInt32(buffer.count))
    guard written > 0 else {
        return nil
    }
    return String(cString: buffer)
}

/// What `body` answers for index after index, starting at `0` and stopping at
/// the first index it answers nothing for — the list the C++ fills a
/// `std::vector` with, walked on the C ABI's side of the seam.
///
/// A negative code is an index with nothing in it too, so an error ends the
/// walk the way the end of the list does: the app is given the paths that
/// were there, and not a reason the rest were not.
internal func paths(of body: (UInt32, UnsafeMutablePointer<CChar>, UInt32) -> Int32) -> [String] {
    var index: UInt32 = 0
    var walked: [String] = []
    while let found = path(of: { buffer, len in body(index, buffer, len) }) {
        walked.append(found)
        index += 1
    }
    return walked
}
