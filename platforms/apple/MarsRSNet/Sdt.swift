// The Swift face of the C ABI in `mars_sdt.h` (crate `marsrs-ffi`, the `sdt`
// feature).
//
// This is the net half of the port: `MarsRSNet` carries this file and
// `Stn.swift`, and `MarsRS` re-exports the module, so `import MarsRS` is all an
// app that runs a diagnosis writes. `MarsRSNet` is not a product of its own:
// it is the half that is not xlog, because xlog is the half an app that only
// logs takes — and the framework it is built from carries no `mars_xlog_*`
// symbol, which is what lets an app that takes both link xlog's exactly once.
//
// The binary target of Package.swift is a static library plus a module map, so
// what `import MarsRSNetFFI` gives a caller is the C surface itself: pointers
// to C strings, arrays with a count beside them and tagged structs whose fields
// are read for the one tag they carry. This file is that surface in Swift —
// `String`s, enums with the answers and a network the caller hands over as a
// closure — and re-exports the C module, so `mars_sdt_start_active_check` and
// friends are still reachable from here for whoever prefers them.
//
// Every call is a straight translation of a symbol in the header; nothing here
// adds behaviour the C ABI does not have.

import Foundation

// Re-exported so that `import MarsRSNet` also gives the C symbols.
@_exported import MarsRSNetFFI

/// `SdtLogic` of `mars/sdt/sdt.h`: the diagnosis of the two links' hosts.
public enum MarsSdt {
    /// `CheckIPPort` of `mars/sdt/src/sdt.h` — one host and port of a link, which
    /// is one of the pairs a diagnosis is started with.
    public struct HostPort {
        /// The host, as an IP or a name.
        public var host: String
        /// The port, which is the one a check dials on that host.
        public var port: UInt16

        /// The only public initializer: the memberwise one a struct gets is
        /// internal, so without this one the type is one an app can read and
        /// never write.
        public init(host: String, port: UInt16) {
            self.host = host
            self.port = port
        }
    }

    /// One entry of the `CheckIPPorts` a diagnosis is started with: the hosts of
    /// one link, under the name that link is known by.
    public struct Link {
        /// The name the link is known by, which is the one the `_ext` calls ask
        /// with.
        public var name: String
        /// The hosts of that link, in the order a check tries them.
        public var ports: [HostPort]

        /// The only public initializer: the memberwise one a struct gets is
        /// internal, so without this one the type is one an app can read and
        /// never write.
        public init(name: String, ports: [HostPort]) {
            self.name = name
            self.ports = ports
        }
    }

    /// `NetCheckType` — one check of a request, which is also the `detectType` of
    /// every entry of the report, so a plan and a report name the same checks
    /// with the same integers.
    public enum Check: UInt32 {
        /// `kPingCheck`.
        case ping = 0
        /// `kDnsCheck`.
        case dns = 1
        /// `kNewDnsCheck` — the resolve against another server, to compare with.
        case newDns = 2
        /// `kTcpCheck`.
        case tcp = 3
        /// `kHttpCheck`.
        case http = 4
        /// `kTracerouteCheck` — planned, but no probe is asked for it yet.
        case traceroute = 5
        /// `kReqBufCheck` — planned, but no probe is asked for it yet.
        case reqBuf = 6
    }

    /// Which probe is being asked: the four of `mars/sdt/src/checkimpl/`, plus
    /// `nothing` for a probe nobody answered.
    public enum Probe: UInt32 {
        /// Nobody answered — a host with no network to probe with, which the
        /// check that asked reads as a failure, and a failed check ends the
        /// run. A ping is the one exception: a ping nobody sent is a check that
        /// did not run, and the plan goes on behind it.
        case nothing = 0
        /// `socket_gethostbyname` — the resolve of one host name.
        case dns = 1
        /// `TcpQuery` — a long-link noop out, and whatever comes back.
        case tcp = 2
        /// `SendHttpQuery` — the request that tells the net-check CGI somebody is
        /// looking for it.
        case http = 3
        /// `PingQuery::RunPingQuery`.
        case ping = 4
    }

    /// What one probe is asked.
    public struct Query {
        /// Which probe it is: one of the four of `mars/sdt/src/checkimpl/`, or
        /// `nothing` for one nobody answered.
        public var probe: Probe
        /// The domain for a resolve, the ip for a noop, the URL for the HTTP
        /// request and the host for a ping.
        public var host: String
        /// `TcpQuery`'s port; `0` for the other three.
        public var port: UInt16
        /// Milliseconds, or seconds for a ping.
        public var timeout: UInt32

        /// One probe to answer, which is the only way an app builds a
        /// [`Query`] of its own: a public struct's memberwise initializer is
        /// internal, so without this one the type is one an app can read and
        /// never write — the same reason [`Noop`] carries an initializer.
        ///
        /// A run hands the app its [`Query`]s, so what this one is for is a
        /// test, or a caller that replays a probe it recorded.
        public init(probe: Probe, host: String, port: UInt16, timeout: UInt32) {
            self.probe = probe
            self.host = host
            self.port = port
            self.timeout = timeout
        }
    }

    /// What a noop round trip recorded: how much went out, how much came back
    /// and whether what came back was the answer to *this* noop.
    public struct Noop {
        /// `tcp_send` — `0` and above is a noop that went out.
        public var sent: Int32
        /// `tcp_receive` — `0` and above is an answer that came back.
        public var received: Int32
        /// Whether what came back was the answer to *this* noop: one that was
        /// not is `kTcpRespErr`, and not a failure of the socket.
        public var isNoopResponse: Bool

        /// The readings of one noop, which is the only way an app answers
        /// `Result.tcp`: a public struct's memberwise initializer is internal,
        /// so without this one the case is one an app can read and never
        /// write.
        public init(sent: Int32, received: Int32, isNoopResponse: Bool) {
            self.sent = sent
            self.received = received
            self.isNoopResponse = isNoopResponse
        }
    }

    /// What one probe answered. Every case carries the readings a run of that
    /// probe leaves behind, and nothing else: a resolve answers addresses, a
    /// noop a round trip, the CGI a status and a ping a loss rate.
    public enum Result {
        /// Nobody answered, which the check that asked reads as a failure — and
        /// a failed check ends the run. A ping is the one exception: a ping
        /// nobody sent is one the C++ never made, so it is left out of the
        /// report and the run goes on to the check behind it.
        case nothing
        /// The resolve of one host name.
        case dns(errorCode: Int32, rtt: UInt64, addresses: [String])
        /// A noop out, and whatever came back.
        case tcp(errorCode: Int32, rtt: UInt64, noop: Noop)
        /// The answer of the net-check CGI.
        case http(errorCode: Int32, rtt: UInt64, statusCode: Int32)
        /// `PingStatus`: how many of the pings of the run were lost, and how long
        /// the ones that came back took.
        case ping(errorCode: Int32, rtt: UInt64, lossRate: Float, averageRTT: Float)
    }

    /// `Reset` — a diagnosis made again from nothing: the check in flight, the
    /// plan it was running and every result waiting to be taken.
    public static func reset() {
        mars_sdt_reset()
    }

    /// `SetHttpNetcheckCGI` — the URL the HTTP check asks for.
    public static func setHTTPNetCheckCGI(_ cgi: String) {
        cgi.withCString { mars_sdt_set_http_netcheck_cgi($0) }
    }

    /// `HttpNetcheckCGI` — the URL the HTTP check asks for, or `nil` when the
    /// buffer was too small, which `cgiBufferSize` bytes never is.
    public static var httpNetCheckCGI: String? {
        var buffer = [CChar](repeating: 0, count: cgiBufferSize)
        let written = mars_sdt_http_netcheck_cgi(&buffer, UInt32(buffer.count))
        guard written >= 0 else {
            return nil
        }
        return String(cString: buffer)
    }

    /// `StartActiveCheck` — a diagnosis of the two links' hosts.
    ///
    /// - Returns: `MARS_SDT_OK`, or `MARS_SDT_ERR_BUSY` when a check is already
    ///   in flight, or `MARS_SDT_ERR_PANIC`.
    @discardableResult
    public static func startActiveCheck(
        longLink: [Link],
        shortLink: [Link],
        mode: Int32,
        timeout: UInt32
    ) -> Int32 {
        withHosts(longLink) { longHosts in
            withHosts(shortLink) { shortHosts in
                longHosts.withUnsafeBufferPointer { longPointer in
                    shortHosts.withUnsafeBufferPointer { shortPointer in
                        mars_sdt_start_active_check(
                            longPointer.baseAddress,
                            UInt32(longHosts.count),
                            shortPointer.baseAddress,
                            UInt32(shortHosts.count),
                            mode,
                            timeout
                        )
                    }
                }
            }
        }
    }

    /// `CancelActiveCheck` — the check in flight is asked to stop.
    public static func cancelActiveCheck() {
        mars_sdt_cancel_active_check()
    }

    /// `IsChecking` — whether a check is in flight.
    public static var isChecking: Bool {
        mars_sdt_is_checking() != 0
    }

    /// `GetSdtingPlan` — the checks the request is going to make, in order.
    public static var plan: [Check] {
        let count = Int(mars_sdt_plan(nil, 0))
        var checks = [MarsSdtCheck](repeating: MarsSdtCheck(rawValue: 0), count: count)
        let written = checks.withUnsafeMutableBufferPointer { buffer in
            mars_sdt_plan(buffer.baseAddress, UInt32(count))
        }
        return checks.prefix(Int(written)).compactMap { Check(rawValue: $0.rawValue) }
    }

    /// `RunCheck` — the planned checks, one probe per check, over the network
    /// `probe` answers with. This is the `__RunOn` thread of the C++, driven by
    /// the caller: the port has no sockets of its own.
    ///
    /// `networkType` is the `comm::getNetInfo()` every check writes into its
    /// profiles, which is the platform's to answer — on Android it is
    /// `PlatformComm.getNetInfo`, on iOS the caller's own.
    ///
    /// - Returns: `MARS_SDT_OK`, or `MARS_SDT_ERR_NO_CHECK` when nothing was in
    ///   flight, or `MARS_SDT_ERR_PANIC`.
    ///
    /// A probe is asked while the process-wide diagnosis is held, so it must not
    /// call another `MarsSdt`: everything it needs is in the query it is given.
    @discardableResult
    public static func runChecks(networkType: Int32, probe: @escaping (Query) -> Result) -> Int32 {
        let runner = ProbeBox(probe)
        let context = Unmanaged.passRetained(runner).toOpaque()
        // The run is over when the call answers, and nothing has kept a pointer
        // into the box: the diagnosis reads every answer before it asks again.
        defer { Unmanaged<ProbeBox>.fromOpaque(context).release() }
        return mars_sdt_run_checks(
            context,
            { ctx, query, answer in
                guard let ctx, let query, let answer else {
                    return
                }
                let asked = Unmanaged<ProbeBox>.fromOpaque(ctx).takeUnretainedValue()
                answer.pointee = asked.result(of: query.pointee)
            },
            networkType
        )
    }

    /// `SdtLogic.reportSignalDetectResults(String)` — the JSON of everything the
    /// checks have reported since the last call, or `nil` when there was nothing
    /// to take, or when a report that does not fit `reportBufferLimit` bytes is
    /// the one the checks produced.
    ///
    /// Taking it empties it: the next call reports what happened since. A report
    /// that did not fit is not taken — `mars_sdt_take_report` answers
    /// `MARS_SDT_ERR_NO_SPACE` and keeps the results, which is what lets the
    /// retry below ask again with a buffer twice the size and get the diagnosis
    /// instead of an empty one.
    public static func takeReport() -> String? {
        var size = reportBufferSize
        while size <= reportBufferLimit {
            var buffer = [CChar](repeating: 0, count: size)
            let written = mars_sdt_take_report(&buffer, UInt32(size))
            if written >= 0 {
                return String(cString: buffer)
            }
            // The C ABI answers "no room" and not the size it needs, so the only
            // way to ask for more is to ask again with more. It keeps the
            // results it could not hand over, so this retry is not one that
            // takes an empty report.
            if written != MARS_SDT_ERR_NO_SPACE {
                return nil
            }
            size += size
        }
        return nil
    }

    /// The buffer `httpNetCheckCGI` writes into; a URL never fills it.
    private static let cgiBufferSize = 1_024

    /// Where `takeReport` starts: a run of two links' hosts is a few hundred
    /// bytes of JSON.
    private static let reportBufferSize = 4_096

    /// Where it stops: a diagnosis that reports more than a megabyte is not one
    /// an app parses.
    private static let reportBufferLimit = 1_048_576
}
