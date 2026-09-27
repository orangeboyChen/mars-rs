// What turns a Swift closure into the C function pointer the diagnosis asks:
// the box behind the context of `mars_sdt_run_checks`, the question it reads and
// the answer it writes.
//
// The probe is the one thing `mars_sdt.h` asks a caller to supply, and it is a
// `void (*)(void*, const MarsSdtQuery*, MarsSdtAnswer*)` — a function pointer,
// which cannot capture the closure an app hands over. So the closure lives in
// `ProbeBox` and the box is handed across as the context, retained for as long
// as the run is and released when the run is over.
//
// The answer is the other half of the seam: the diagnosis reads the addresses a
// resolve found *after* the probe has answered, so they cannot be the pointers
// `withCString` lends for the length of a closure. They are copies, and the box
// holds them (`Held`) until the next question is asked.

import Foundation

/// The Swift probe of a run, behind the context `mars_sdt_run_checks` hands back
/// with every question.
internal final class ProbeBox {
    /// The network the caller answered with.
    private let probe: (MarsSdt.Query) -> MarsSdt.Result

    /// What the answers of this run are made of, thrown away when the next
    /// question is asked: the diagnosis has read this one by then.
    private var held = Held()

    /// Wraps the probe of one run.
    internal init(_ probe: @escaping (MarsSdt.Query) -> MarsSdt.Result) {
        self.probe = probe
    }

    deinit {
        held.clear()
    }

    /// What the caller's probe answered, as the C struct the diagnosis reads.
    internal func result(of query: MarsSdtQuery) -> MarsSdtAnswer {
        held.clear()
        let result = probe(MarsSdt.Query(query))
        // Zeroed first and then filled in for the kind it is: every field of a
        // `MarsSdtAnswer` is read for the `kind` in it and left alone for the
        // others, which is what the header promises the diagnosis does.
        var answer = MarsSdtAnswer()
        switch result {
        case .nothing:
            answer.kind = MarsSdtNothing

        case let .dns(errorCode, rtt, addresses):
            answer.kind = MarsSdtDns
            answer.error_code = errorCode
            answer.rtt = rtt
            answer.ips = held.strings(addresses)
            answer.ip_count = UInt32(addresses.count)

        case let .tcp(errorCode, rtt, noop):
            answer.kind = MarsSdtTcp
            answer.error_code = errorCode
            answer.rtt = rtt
            answer.sent = noop.sent
            answer.received = noop.received
            answer.is_noop_resp = noop.isNoopResponse ? 1 : 0

        case let .http(errorCode, rtt, statusCode):
            answer.kind = MarsSdtHttp
            answer.error_code = errorCode
            answer.rtt = rtt
            answer.status_code = statusCode

        case let .ping(errorCode, rtt, lossRate, averageRTT):
            answer.kind = MarsSdtPing
            answer.error_code = errorCode
            answer.rtt = rtt
            answer.loss_rate = lossRate
            answer.avgrtt = averageRTT
        }
        return answer
    }
}

extension MarsSdt.Query {
    /// What the diagnosis asked, as the Swift reads it.
    internal init(_ query: MarsSdtQuery) {
        self.init(
            probe: MarsSdt.Probe(rawValue: query.kind.rawValue) ?? .nothing,
            host: String(cString: query.host),
            port: query.port,
            timeout: query.timeout
        )
    }
}

/// The [`MarsSdtHosts`] of `links`, valid for the length of the closure: the
/// names, the host strings and the port array they point at are all this
/// function's own.
///
/// Lent and not copied, because a diagnosis is started with them and reads them
/// before it answers: nothing here has to outlive the call.
internal func withHosts<R>(_ links: [MarsSdt.Link], body: ([MarsSdtHosts]) -> R) -> R {
    let ports = links.flatMap(\.ports)
    // The names first and then one string per port, in the order the hosts come:
    // the C structs are built out of this one array of pointers.
    let strings = links.map(\.name) + ports.map(\.host)
    return withCStrings(strings) { pointers in
        // One allocation for every port of every link, so the `ports` of each
        // host is a pointer into one array that outlives the call.
        let buffer = UnsafeMutablePointer<MarsSdtIpPort>.allocate(capacity: ports.count + 1)
        defer { buffer.deallocate() }
        var hosts: [MarsSdtHosts] = []
        var portIndex = 0
        for (index, link) in links.enumerated() {
            let first = portIndex
            for port in link.ports {
                buffer[portIndex] = MarsSdtIpPort(ip: pointers[links.count + portIndex], port: port.port)
                portIndex += 1
            }
            hosts.append(MarsSdtHosts(
                name: pointers[index],
                ports: buffer + first,
                port_count: UInt32(link.ports.count)
            ))
        }
        return body(hosts)
    }
}
