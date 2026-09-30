// What the caller answered. One answer per question, and an answer of another
// kind than the question asked for is no answer: STN takes its own instead,
// which is what a host with no app gets.

import Foundation

/// What the app answered, which `MarsStn` names `MarsStn.Answer`.
///
/// One answer is a kind and the readings that kind carries, and the rest are the
/// zeros they start out as — which is what `mars_stn.h` promises the pipeline
/// does with a struct it is handed.
///
/// A class and not an `enum` with associated values, because an app that writes
/// Objective-C answers too: an Objective-C file has no `switch` over a case
/// that carries a value, and no way to spell one. So the nine answers are the
/// nine class methods below — `StnAnswer.encoded(body)`, and `[StnAnswer
/// encoded:body]` — and what either language reads out of one is `kind` and the
/// fields it names.
@objc
public final class StnAnswer: NSObject {
    /// Which of the nine answers it is: the `MarsStnAnswer*` integers of
    /// `mars_stn.h`, in the order the header declares them.
    @objc(StnAnswerKind)
    public enum Kind: UInt32 {
        /// Nobody answered — the app's own for every question.
        case nothing = 0
        /// `makesureAuthed`, `identifyResponse`.
        case yes = 1
        /// `onNewDns`, `netCheckShortLinkHosts`.
        case addresses = 2
        /// `req2Buf` — what the task sends.
        case encoded = 3
        /// `req2Buf` — the code the task ends with, i.e. the C++'s `false`.
        case failed = 4
        /// `buf2Resp` — the code the task is remembered with, and what STN does
        /// with one that did not go well.
        case decoded = 5
        /// `onTaskEnd` — the code the task is remembered with.
        case ended = 6
        /// `identifyCheckBuffer` — the check a new link is used with.
        case identified = 7
        /// `reportTaskLimited` — the number the gate weighed the
        /// task against, answered back: a report and not an override,
        /// so a task the gates refused stays refused.
        case limit = 8
    }

    /// Which of the nine it is: what the pipeline reads first, and the only
    /// field of the nine that says what the rest mean.
    @objc public var kind: Kind = .nothing
    /// `yes` — what `makesureAuthed` and `identifyResponse` answered.
    @objc public var isYes: Bool = false
    /// `addresses` — the ips the app knows for a host.
    @objc public var addresses: [String] = []
    /// `encoded`, `identified` — the bytes of the request, or of the check.
    @objc public var body = Data()
    /// `failed`, `decoded`, `ended` — the code the task is remembered with.
    @objc public var errorCode: Int32 = 0
    /// `decoded` — the `handle` of the C ABI: what STN does with an answer that
    /// did not go well.
    @objc public var failHandle: MarsStn.FailHandle = .normal
    /// `identified` — the `mode` of the C ABI: when the check goes out.
    @objc public var identifyMode: MarsStn.IdentifyMode = .now
    /// `identified` — the `hash` of the C ABI: the hash the answer is judged
    /// against. `Hash` is not a name this class can give it, because
    /// `NSObject` has a `hash` of its own.
    @objc public var hashBytes = Data()
    /// `identified` — the command the check is made for.
    @objc public var cmdid: UInt32 = 0
    /// `limit` — the number the gate weighed the task against,
    /// answered back: a report and not an override, so a task the
    /// gates refused stays refused.
    @objc public var limit: UInt32 = 0

    /// Nobody answered: the answer for every question the app has nothing to say
    /// to, and the one STN takes for a host with no app at all.
    @objc
    public static func nothing() -> StnAnswer {
        StnAnswer()
    }

    /// `makesureAuthed`, `identifyResponse` — yes or no.
    ///
    /// - Parameter isYes: whether the app is logged in, or whether the answer is
    ///   the one the check asked for.
    @objc
    public static func yes(_ isYes: Bool) -> StnAnswer {
        let answer = StnAnswer()
        answer.kind = .yes
        answer.isYes = isYes
        return answer
    }

    /// `onNewDns`, `netCheckShortLinkHosts` — the ips the app knows for a host.
    @objc
    public static func addresses(_ addresses: [String]) -> StnAnswer {
        let answer = StnAnswer()
        answer.kind = .addresses
        answer.addresses = addresses
        return answer
    }

    /// `req2Buf` — what the task sends.
    @objc
    public static func encoded(_ body: Data) -> StnAnswer {
        let answer = StnAnswer()
        answer.kind = .encoded
        answer.body = body
        return answer
    }

    /// `req2Buf` — the code the task ends with, which is the C++'s `false`: a
    /// request the app could not write.
    @objc
    public static func failed(errorCode: Int32) -> StnAnswer {
        let answer = StnAnswer()
        answer.kind = .failed
        answer.errorCode = errorCode
        return answer
    }

    /// `buf2Resp` — the code the task is remembered with, and what STN is to do
    /// with an answer that did not go well.
    @objc
    public static func decoded(errorCode: Int32, handle: MarsStn.FailHandle) -> StnAnswer {
        let answer = StnAnswer()
        answer.kind = .decoded
        answer.errorCode = errorCode
        answer.failHandle = handle
        return answer
    }

    /// `onTaskEnd` — the code the task is remembered with.
    @objc
    public static func ended(errorCode: Int32) -> StnAnswer {
        let answer = StnAnswer()
        answer.kind = .ended
        answer.errorCode = errorCode
        return answer
    }

    /// `identifyCheckBuffer` — the check a new link is used with: the buffer and
    /// the hash of the answer it is judged against.
    @objc
    public static func identified(
        mode: MarsStn.IdentifyMode,
        bytes: Data,
        hash: Data,
        cmdid: UInt32
    ) -> StnAnswer {
        let answer = StnAnswer()
        answer.kind = .identified
        answer.identifyMode = mode
        answer.body = bytes
        answer.hashBytes = hash
        answer.cmdid = cmdid
        return answer
    }

    /// `reportTaskLimited` — the number the gate weighed the task
    /// against, answered back: a report and not an override, so a
    /// task the gates refused stays refused.
    @objc
    public static func limit(_ limit: UInt32) -> StnAnswer {
        let answer = StnAnswer()
        answer.kind = .limit
        answer.limit = limit
        return answer
    }

    deinit {
        // Nothing to release: the strings and the bytes are all this object
        // holds, and they go with it. The declaration is what `required_deinit`
        // asks a class for.
    }

    /// An answer of no kind, which is what the class methods above fill in: an
    /// app answers with one of them and never with this.
    override private init() {
        super.init()
    }
}
