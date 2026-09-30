// The Swift face of the C ABI in `mars_sdt.h` (crate `marsrs-ffi`, the `sdt`
// feature).
//
// This is the net half of the port: `MarsRSNet` carries this file and
// `Stn.swift`, and `MarsRS` re-exports the module, so `import MarsRS` is all an
// app that runs a diagnosis writes. `MarsRSNet` is not a product of its own:
// it is the half that is not xlog, because xlog is the half an app that only logs
// takes — and the framework it is built from carries no `mars_xlog_*` symbol,
// which is what lets an app that takes both link xlog's exactly once.
//
// The binary target of Package.swift is a static library plus a module map, so
// what `import MarsRSNetFFI` gives a caller is the C surface itself: pointers to
// C strings, arrays with a count beside them and tagged structs whose fields are
// read for the one tag they carry. This file is that surface in Swift —
// `String`s, enums with the answers and a network the caller hands over as a
// closure — and re-exports the C module, so `mars_sdt_start_active_check` and
// friends are still reachable from here for whoever prefers them.
//
// Every call is a straight translation of a symbol in the header; nothing here
// adds behaviour the C ABI does not have.
//
// `MarsSdt` is a class and not an `enum` of statics, and every value of it is a
// class or an `@objc` enum, as `MarsStn` is: an app written in Objective-C runs
// a diagnosis too. The names the compiler gives them in `MarsRSNet-Swift.h` are
// not the C ones — `MarsSdtProbeQuery`, and not `MarsSdtQuery` — because a Swift
// type and the C struct it is translated from cannot carry the same name in a
// language with no namespaces.

import Foundation

// Re-exported so that `import MarsRSNet` also gives the C symbols.
@_exported import MarsRSNetFFI

/// `SdtLogic` of `mars/sdt/sdt.h`: the diagnosis of the two links' hosts.
@objc
public final class MarsSdt: NSObject {
    /// `CheckIPPort` of `mars/sdt/src/sdt.h` — one host and port of a link, which
    /// is one of the pairs a diagnosis is started with.
    ///
    /// A class and not a struct, because an app that writes Objective-C starts a
    /// diagnosis too — the reason `XlogConfig` is a class and not a struct.
    @objc(MarsSdtHostPort)
    public final class HostPort: NSObject {
        /// The host, as an IP or a name.
        @objc public var host: String = ""
        /// The port, which is the one a check dials on that host.
        @objc public var port: UInt16 = 0

        /// The only way in: neither field has a default of its own.
        @objc
        public init(host: String, port: UInt16) {
            self.host = host
            self.port = port
            super.init()
        }

        deinit {
            // Nothing to release: the host is all this object holds, and it
            // goes with it. The declaration is what `required_deinit` asks a
            // class for.
        }
    }

    /// One entry of the `CheckIPPorts` a diagnosis is started with: the hosts of
    /// one link, under the name that link is known by.
    @objc(MarsSdtLink)
    public final class Link: NSObject {
        /// The name the link is known by, which is the one the `_ext` calls ask
        /// with.
        @objc public var name: String = ""
        /// The hosts of that link, in the order a check tries them.
        @objc public var ports: [HostPort] = []

        /// The only way in: neither field has a default of its own.
        @objc
        public init(name: String, ports: [HostPort]) {
            self.name = name
            self.ports = ports
            super.init()
        }

        deinit {
            // Nothing to release: the name and the hosts are all this object
            // holds, and they go with it. The declaration is what
            // `required_deinit` asks a class for.
        }
    }

    /// `NetCheckType` — one check of a request, which is also the `detectType` of
    /// every entry of the report, so a plan and a report name the same checks
    /// with the same integers.
    @objc(MarsSdtCheckKind)
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

    /// The `NET_CHECK_*` bits a diagnosis is started with, as a set — which is
    /// what the bits are: `[.basic, .long]` is a run of three checks, and `[]`
    /// is a run of none at all, which is the one `MARS_SDT_ERR_BAD_ARG`
    /// answers for.
    ///
    /// `rawValue` is the integer the C ABI asks for, so a caller that names
    /// its own — `1 | 2` — reaches the same entry point with it.
    public struct Mode: OptionSet {
        /// `NET_CHECK_BASIC` — the ping and the DNS check.
        public static let basic = Self(rawValue: NET_CHECK_BASIC)
        /// `NET_CHECK_LONG` — the TCP check, against the long link's hosts.
        public static let long = Self(rawValue: NET_CHECK_LONG)
        /// `NET_CHECK_SHORT` — the HTTP check, against the net-check CGI and the
        /// short link's hosts.
        public static let short = Self(rawValue: NET_CHECK_SHORT)
        /// Every check there is: the three of them together.
        public static let all = Self(rawValue: NET_CHECK_BASIC | NET_CHECK_LONG | NET_CHECK_SHORT)

        /// The bits of one mode, in the integer the C ABI takes.
        public let rawValue: Int32

        /// The only way in: a set of the three above, or an integer of a
        /// caller's own.
        public init(rawValue: Int32) {
            self.rawValue = rawValue
        }
    }

    /// Which probe is being asked: the four of `mars/sdt/src/checkimpl/`, plus
    /// `nothing` for a probe nobody answered.
    ///
    /// The same integers name what a probe *answered*, which is what [`Result`]
    /// carries: one enum for the question and for the answer, as the C ABI's
    /// `MarsSdtKind` is.
    @objc(MarsSdtProbeKind)
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
    @objc(MarsSdtProbeQuery)
    public final class Query: NSObject {
        /// Which probe it is: one of the four of `mars/sdt/src/checkimpl/`, or
        /// `nothing` for one nobody answered.
        @objc public var probe: Probe = .nothing
        /// The domain for a resolve, the ip for a noop, the URL for the HTTP
        /// request and the host for a ping.
        @objc public var host: String = ""
        /// `TcpQuery`'s port; `0` for the other three.
        @objc public var port: UInt16 = 0
        /// Milliseconds, or seconds for a ping.
        @objc public var timeout: UInt32 = 0

        deinit {
            // Nothing to release: the host is all this object holds, and it
            // goes with it. The declaration is what `required_deinit` asks a
            // class for.
        }

        /// One probe to answer, which is the only way an app builds a [`Query`]
        /// of its own: the memberwise initializer a struct would have carried is
        /// internal, so without this one the type is one an app can read and
        /// never write — the same reason [`Noop`] carries an initializer.
        ///
        /// A run hands the app its [`Query`]s, so what this one is for is a
        /// test, or a caller that replays a probe it recorded.
        @objc
        public init(probe: Probe, host: String, port: UInt16, timeout: UInt32) {
            self.probe = probe
            self.host = host
            self.port = port
            self.timeout = timeout
            super.init()
        }
    }

    /// What a noop round trip recorded: how much went out, how much came back
    /// and whether what came back was the answer to *this* noop.
    @objc(MarsSdtNoop)
    public final class Noop: NSObject {
        /// `tcp_send` — `0` and above is a noop that went out.
        @objc public var sent: Int32 = 0
        /// `tcp_receive` — `0` and above is an answer that came back.
        @objc public var received: Int32 = 0
        /// Whether what came back was the answer to *this* noop: one that was
        /// not is `kTcpRespErr`, and not a failure of the socket.
        @objc public var isNoopResponse: Bool = false

        deinit {
            // Nothing to release: the three readings are all this object holds,
            // and they go with it. The declaration is what `required_deinit`
            // asks a class for.
        }

        /// The readings of one noop, which is the only way an app answers
        /// `Result.tcp`: the memberwise initializer a struct would have carried
        /// is internal, so without this one the answer is one an app can read
        /// and never write.
        @objc
        public init(sent: Int32, received: Int32, isNoopResponse: Bool) {
            self.sent = sent
            self.received = received
            self.isNoopResponse = isNoopResponse
            super.init()
        }
    }

    /// What one probe answered: which probe it was, and the readings a run of
    /// that probe leaves behind — a resolve answers addresses, a noop a round
    /// trip, the CGI a status and a ping a loss rate.
    ///
    /// A class and not an `enum` with associated values, for the reason
    /// `StnAnswer` is one: an app that writes Objective-C answers probes too, and
    /// an Objective-C file has no way to spell a case that carries a value. So
    /// the five answers are the five class methods below —
    /// `MarsSdt.Result.dns(errorCode:rtt:addresses:)`, and `[MarsSdtResult
    /// dnsWithErrorCode:rtt:addresses:]` — and what either language reads is
    /// `probe` and the fields it names.
    @objc(MarsSdtResult)
    public final class Result: NSObject {
        /// Which probe answered; `.nothing` is one nobody answered, which the
        /// check that asked reads as a failure — and a failed check ends the
        /// run. A ping is the one exception: a ping nobody sent is one the C++
        /// never made, so it is left out of the report and the run goes on to
        /// the check behind it.
        @objc public var probe: Probe = .nothing
        /// The probe's return value; `0` and above is one that worked.
        @objc public var errorCode: Int32 = 0
        /// How long the probe took.
        @objc public var rtt: UInt64 = 0
        /// `dns` — the addresses a resolve found.
        @objc public var addresses: [String] = []
        /// `tcp` — the round trip of a noop; `nil` for the other three.
        @objc public var noop: Noop?
        /// `http` — the status of the net-check CGI's answer.
        @objc public var statusCode: Int32 = 0
        /// `ping` — how many of the pings of the run were lost.
        @objc public var lossRate: Float = 0
        /// `ping` — how long the ones that came back took.
        @objc public var averageRTT: Float = 0

        /// Nobody answered, which the check that asked reads as a failure.
        @objc
        public static func nothing() -> Result {
            Result()
        }

        /// The resolve of one host name.
        @objc
        public static func dns(errorCode: Int32, rtt: UInt64, addresses: [String]) -> Result {
            let result = Result()
            result.probe = .dns
            result.errorCode = errorCode
            result.rtt = rtt
            result.addresses = addresses
            return result
        }

        /// A noop out, and whatever came back.
        @objc
        public static func tcp(errorCode: Int32, rtt: UInt64, noop: Noop) -> Result {
            let result = Result()
            result.probe = .tcp
            result.errorCode = errorCode
            result.rtt = rtt
            result.noop = noop
            return result
        }

        /// The answer of the net-check CGI.
        @objc
        public static func http(errorCode: Int32, rtt: UInt64, statusCode: Int32) -> Result {
            let result = Result()
            result.probe = .http
            result.errorCode = errorCode
            result.rtt = rtt
            result.statusCode = statusCode
            return result
        }

        /// `PingStatus`: how many of the pings of the run were lost, and how long
        /// the ones that came back took.
        @objc
        public static func ping(
            errorCode: Int32,
            rtt: UInt64,
            lossRate: Float,
            averageRTT: Float
        ) -> Result {
            let result = Result()
            result.probe = .ping
            result.errorCode = errorCode
            result.rtt = rtt
            result.lossRate = lossRate
            result.averageRTT = averageRTT
            return result
        }

        deinit {
            // Nothing to release: the readings, the addresses and the one noop
            // are all this object holds, and they go with it. The declaration is
            // what `required_deinit` asks a class for.
        }

        /// An answer of no probe, which is what the class methods above fill in:
        /// an app answers with one of them and never with this.
        override private init() {
            super.init()
        }
    }

    /// `Reset` — a diagnosis made again from nothing: the check in flight, the
    /// plan it was running and every result waiting to be taken.
    @objc
    public static func reset() {
        mars_sdt_reset()
    }

    /// `SetHttpNetcheckCGI` — the URL the HTTP check asks for.
    @objc
    public static func setHTTPNetCheckCGI(_ cgi: String) {
        cgi.withCString { mars_sdt_set_http_netcheck_cgi($0) }
    }

    /// `HttpNetcheckCGI` — the URL the HTTP check asks for, or `nil` when the
    /// buffer was too small, which `cgiBufferSize` bytes never is.
    @objc public static var httpNetCheckCGI: String? {
        var buffer = [CChar](repeating: 0, count: cgiBufferSize)
        let written = mars_sdt_http_netcheck_cgi(&buffer, UInt32(buffer.count))
        guard written >= 0 else {
            return nil
        }
        return String(cString: buffer)
    }

    /// `StartActiveCheck` — a diagnosis of the two links' hosts.
    ///
    /// The Swift way in: a [`Mode`] rather than the integer the C ABI asks for,
    /// which is what the overload below takes and what Objective-C reaches.
    ///
    /// - Returns: `MARS_SDT_OK`, or `MARS_SDT_ERR_BUSY` when a check is already
    ///   in flight, or `MARS_SDT_ERR_PANIC`.
    @discardableResult
    public static func startActiveCheck(
        longLink: [Link],
        shortLink: [Link],
        mode: Mode,
        timeout: UInt32
    ) -> Int32 {
        return startActiveCheck(
            longLink: longLink,
            shortLink: shortLink,
            mode: mode.rawValue,
            timeout: timeout
        )
    }

    /// `StartActiveCheck` — the same diagnosis, with the mode as the integer
    /// the C ABI asks for: the one Objective-C reaches, because a set of
    /// [`Mode`] has no Objective-C type, and the one a caller that spells its
    /// own bits — `NET_CHECK_BASIC | NET_CHECK_LONG` — takes.
    ///
    /// - Returns: `MARS_SDT_OK`, or `MARS_SDT_ERR_BUSY` when a check is already
    ///   in flight, or `MARS_SDT_ERR_BAD_ARG` when these arguments cannot start
    ///   a check — a mode of no checks at all among them — or
    ///   `MARS_SDT_ERR_PANIC`.
    @objc
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
    @objc
    public static func cancelActiveCheck() {
        mars_sdt_cancel_active_check()
    }

    /// `IsChecking` — whether a check is in flight.
    @objc public static var isChecking: Bool {
        mars_sdt_is_checking() != 0
    }

    /// `GetSdtingPlan` — the checks the request is going to make, in order.
    ///
    /// The one call of this type Objective-C does not reach: an array of enums
    /// has no Objective-C type — `NSArray` holds objects, and a `Check` is an
    /// integer — so the plan stays Swift's, the way the console sink and the
    /// `async` flush of `Xlog` do.
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
    /// Objective-C hands the same closure over as a block, and answers with one
    /// of the [`Result`] the class methods there make.
    @objc
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
    @objc
    public static func takeReport() -> String? {
        var size = reportBufferSize
        while size <= reportBufferLimit {
            var buffer = [CChar](repeating: 0, count: size)
            let written = mars_sdt_take_report(&buffer, UInt32(size))
            if written >= 0 {
                let report = String(cString: buffer)
                // `noResults` is what the C ABI hands over when there was
                // nothing to take: `report_json` of no checks at all. The ABI
                // has no answer but a document for it — no count comes with
                // one — and a diagnosis of nothing is the `nil` this promises
                // and not a report an app has to parse to find out.
                return report == noResults ? nil : report
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

    deinit {
        // Nothing to release: every member of this type is a `static`, and an
        // app never makes one. The declaration is what `required_deinit` asks a
        // class for.
    }

    /// There is nothing an app makes: every member of this type is a `static`,
    /// and the class is what lets Objective-C write them.
    override private init() {
        super.init()
    }

    /// The buffer `httpNetCheckCGI` writes into; a URL never fills it.
    private static let cgiBufferSize = 1_024

    /// Where `takeReport` starts: a run of two links' hosts is a few hundred
    /// bytes of JSON.
    private static let reportBufferSize = 4_096

    /// Where it stops: a diagnosis that reports more than a megabyte is not one
    /// an app parses.
    private static let reportBufferLimit = 1_048_576

    /// The document `report_json` writes for no results: `{"details":[]}`, and
    /// nothing else — the one the C ABI answers when there was nothing to take.
    private static let noResults = "{\"details\":[]}"
}
