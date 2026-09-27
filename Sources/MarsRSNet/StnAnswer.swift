// What the caller answered. One answer per question, and an answer of another
// kind than the question asked for is no answer: STN takes its own instead,
// which is what a host with no app gets.

import Foundation

/// What the app answered, which `MarsStn` names `MarsStn.Answer`.
public enum StnAnswer {
    /// Nobody answered — the app's own for every question.
    case nothing
    /// `makesureAuthed`, `identifyResponse`.
    case yes(Bool)
    /// `onNewDns`, `netCheckShortLinkHosts`.
    case addresses([String])
    /// `req2Buf` — what the task sends.
    case encoded([UInt8])
    /// `req2Buf` — the code the task ends with, i.e. the C++'s `false`.
    case failed(errorCode: Int32)
    /// `buf2Resp` — the code the task is remembered with, and what STN does with
    /// one that did not go well.
    case decoded(errorCode: Int32, handle: MarsStn.FailHandle)
    /// `onTaskEnd` — the code the task is remembered with.
    case ended(errorCode: Int32)
    /// `identifyCheckBuffer` — the check a new link is used with: the buffer and
    /// the hash of the answer it is judged against.
    case identified(mode: MarsStn.IdentifyMode, bytes: [UInt8], hash: [UInt8], cmdid: UInt32)
    /// `reportTaskLimited` — what the limit is; `0` is "go ahead".
    case limit(UInt32)
}
