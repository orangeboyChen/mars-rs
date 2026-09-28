// What turns a Swift closure into the C function pointer the pipeline asks:
// the box behind the context of `mars_stn_set_app`, and the answer it writes.
//
// The app is the one thing `mars_stn.h` asks a caller to supply, and it is a
// `void (*)(void*, const MarsStnQuestion*, MarsStnAnswer*)` — a function
// pointer, which cannot capture the closure an app hands over. So the closure
// lives in `AppBox`, the box is the context every question is handed back with,
// and `MarsStn.setApp` is what keeps it alive.
//
// The answer is the other half of the seam: the pipeline reads the bytes a task
// sends and the addresses a resolve found *after* the app has answered, so they
// cannot be the pointers Swift lends for the length of a closure. They are
// copies, and the box holds them (`Held`) until the next question is asked.

import Foundation

/// The app STN asks, behind the context `mars_stn_set_app` hands back with every
/// question.
///
/// `@unchecked Sendable`, because the C ABI asks on whatever thread it likes:
/// the box holds the closure the app handed over and the memory of one answer,
/// and nothing of either is touched from two questions at once.
internal final class AppBox: @unchecked Sendable {
    /// The app.
    private let ask: (StnQuestion) -> StnAnswer

    /// What the answers of this app are made of, thrown away when the next
    /// question is asked: the pipeline has read this one by then.
    private var held = Held()

    /// Wraps the app of one process.
    internal init(_ ask: @escaping (StnQuestion) -> StnAnswer) {
        self.ask = ask
    }

    deinit {
        held.clear()
    }

    /// What the app answered, as the C struct the pipeline reads.
    internal func answer(to question: MarsStnQuestion) -> MarsStnAnswer {
        held.clear()
        let answer = ask(StnQuestion(question))
        // Zeroed first and then filled in for the kind it is: every field of a
        // `MarsStnAnswer` is read for the `kind` in it and left alone for the
        // others, which is what the header promises the pipeline does.
        var value = MarsStnAnswer()
        switch answer {
        case .nothing:
            value.kind = MarsStnAnswerNothing

        case let .yes(yes):
            value.kind = MarsStnAnswerYes
            value.yes = yes ? 1 : 0

        case let .addresses(addresses):
            value.kind = MarsStnAnswerIps
            value.ips = held.strings(addresses)
            value.ip_count = UInt32(addresses.count)

        case let .encoded(bytes):
            value.kind = MarsStnAnswerEncoded
            value.bytes = held.bytes(bytes)
            value.byte_count = UInt32(bytes.count)

        case let .failed(errorCode):
            value.kind = MarsStnAnswerFailed
            value.error_code = errorCode

        case let .decoded(errorCode, handle):
            value.kind = MarsStnAnswerDecoded
            value.error_code = errorCode
            value.handle = handle.rawValue

        case let .ended(errorCode):
            value.kind = MarsStnAnswerEnded
            value.error_code = errorCode

        case let .identified(mode, bytes, hash, cmdid):
            value.kind = MarsStnAnswerIdentified
            value.mode = mode.rawValue
            value.bytes = held.bytes(bytes)
            value.byte_count = UInt32(bytes.count)
            value.hash = held.bytes(hash)
            value.hash_count = UInt32(hash.count)
            value.cmdid = cmdid

        case let .limit(limit):
            value.kind = MarsStnAnswerLimit
            value.limit = limit
        }
        return value
    }
}
