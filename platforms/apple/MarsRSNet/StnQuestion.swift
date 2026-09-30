// The eighteen questions STN asks, in the arguments the C++ hands the app: one
// question is one tagged struct — every field is read for the `kind` in it and
// left alone for the others.
//
// The two profiles a question carries are nested here, because they are what a
// report is made of and nothing else reads them; `StnAnswer`, what the app
// answers with, is a file of its own.

import Foundation

/// One of the eighteen questions, in the arguments the C++ hands the app.
///
/// Every field is read for the [`Kind`] in `kind` and left alone for the others,
/// so an app switches on the kind and reads what it names: `host` is the host of
/// `makesureAuthed`, `onNewDns` and `shortLinkNetworkError`; `ipAddress` and
/// `port` the pair of the two network errors; `channelID` the link of `onPush`,
/// `identifyCheckBuffer` and `identifyResponse`; `body` what was pushed, what
/// came back and what the server answered.
///
/// A class and not a struct, because an app that writes Objective-C is asked too
/// — the reason `XlogConfig` is a class and not a struct. What it is made of is
/// the same readings either way.
@objc
public final class StnQuestion: NSObject {
    /// Which of the eighteen questions STN asked.
    ///
    /// The integers are the C ABI's own: they name a question, the way the
    /// C++'s eighteen virtuals do by name.
    @objc(StnQuestionKind)
    public enum Kind: UInt32 {
        /// Nothing was asked: what a question starts out as, and what no
        /// question is ever asked as.
        case nothing = 0
        /// `MakesureAuthed` — is the app logged in for this host and user?
        case makesureAuthed = 1
        /// `TrafficData` — how much went out and came in.
        case trafficData = 2
        /// `OnNewDns` — the ips the app knows for a host.
        case onNewDns = 3
        /// `OnPush` — something the server sent that no task asked for.
        case onPush = 4
        /// `Req2Buf` — what a task is to send.
        case req2Buf = 5
        /// `Buf2Resp` — how the app reads an answer that came back.
        case buf2Resp = 6
        /// `OnTaskEnd` — a task that is over.
        case onTaskEnd = 7
        /// `ReportConnectStatus` — the connection as the app is asked to see it.
        case reportConnectStatus = 8
        /// `OnLongLinkNetworkError` — the main long link's errors.
        case longLinkNetworkError = 9
        /// `OnShortLinkNetworkError`.
        case shortLinkNetworkError = 10
        /// `OnLongLinkStatusChange` — what the default long link is in.
        case longLinkStatusChange = 11
        /// `GetLonglinkIdentifyCheckBuffer` — the check a new link is used with.
        case identifyCheckBuffer = 12
        /// `OnLonglinkIdentifyResponse` — whether the answer is the one the
        /// check asked for.
        case identifyResponse = 13
        /// `RequestSync` — the app is asked to sync.
        case requestSync = 14
        /// `RequestNetCheckShortLinkHosts` — the hosts the network check may
        /// probe.
        case netCheckShortLinkHosts = 15
        /// `ReportTaskProfile` — everything a task left behind.
        case reportTaskProfile = 16
        /// `ReportTaskLimited` — a task the app asked to have limited.
        case reportTaskLimited = 17
        /// `ReportDnsProfile` — how a dns question went.
        case reportDnsProfile = 18
    }

    /// The connect a task ran on, as the app's report wants it: every reading is
    /// a `gettickcount()`.
    @objc(StnQuestionCgiProfile)
    public final class CgiProfile: NSObject {
        /// When the run began.
        @objc public let startTime: UInt64
        /// When the connect began.
        @objc public let startConnectTime: UInt64
        /// When it came back, socket or no socket.
        @objc public let connectSuccessfulTime: UInt64
        /// When the request went out.
        @objc public let startSendPacketTime: UInt64
        /// When the write was done.
        @objc public let sendPacketFinishedTime: UInt64
        /// When the read of the answer began.
        @objc public let startReadPacketTime: UInt64
        /// When the last of it came back.
        @objc public let readPacketFinishedTime: UInt64
        /// When the app was handed the task to write its request.
        @objc public let startEncodePacketTime: UInt64
        /// When the request came back.
        @objc public let encodePacketFinishedTime: UInt64
        /// When the app was handed the answer.
        @objc public let startDecodePacketTime: UInt64
        /// When it was done reading it.
        @objc public let decodePacketFinishedTime: UInt64
        /// The link the task went out on.
        @objc public let channelType: MarsStn.Channel
        /// How the task went out.
        @objc public let transportProtocol: MarsStn.TransportProtocol
        /// How long the pair that won took to answer.
        @objc public let rtt: UInt32
        /// The network the connect was made on.
        @objc public let netType: String

        deinit {
            // Nothing to release: the readings and the one string are all this
            // object holds, and they go with it. The declaration is what
            // `required_deinit` asks a class for.
        }

        /// The connect a question carries, as the Swift reads it.
        internal init(_ profile: MarsStnCgiProfile) {
            startTime = profile.start_time
            startConnectTime = profile.start_connect_time
            connectSuccessfulTime = profile.connect_successful_time
            startSendPacketTime = profile.start_send_packet_time
            sendPacketFinishedTime = profile.send_packet_finished_time
            startReadPacketTime = profile.start_read_packet_time
            readPacketFinishedTime = profile.read_packet_finished_time
            startEncodePacketTime = profile.start_encode_packet_time
            encodePacketFinishedTime = profile.encode_packet_finished_time
            startDecodePacketTime = profile.start_decode_packet_time
            decodePacketFinishedTime = profile.decode_packet_finished_time
            channelType = MarsStn.Channel(rawValue: profile.channel_type) ?? .none
            transportProtocol = MarsStn.TransportProtocol(rawValue: profile.transport_protocol) ?? .default
            rtt = profile.rtt
            netType = string(profile.nettype)
            super.init()
        }
    }

    /// How a dns question went.
    @objc(StnQuestionDnsProfile)
    public final class DnsProfile: NSObject {
        /// When the question was asked.
        @objc public let startTime: UInt64
        /// When it came back; `0` while it has not.
        @objc public let endTime: UInt64
        /// What was asked for.
        @objc public let host: String
        /// One of `ErrCmdType`'s: `0` ok, `1` false, `2` dial, `3` dns, `4`
        /// socket, `5` http, `6` netmsgxp, `7` endecode, `8` server, `9` local,
        /// `10` canceld.
        @objc public let errType: Int32
        /// The code the question ended with.
        @objc public let errCode: Int32
        /// Which dns it was: `1` the app's, `2` the platform's.
        @objc public let dnsType: Int32

        deinit {
            // Nothing to release: the readings and the one string are all this
            // object holds, and they go with it. The declaration is what
            // `required_deinit` asks a class for.
        }

        /// The question a report carries, as the Swift reads it.
        internal init(_ profile: MarsStnDnsProfile) {
            startTime = profile.start_time
            endTime = profile.end_time
            host = string(profile.host)
            errType = profile.err_type
            errCode = profile.err_code
            dnsType = profile.dnstype
            super.init()
        }
    }

    /// Which of the eighteen STN asked: the readings below are the ones this
    /// one names, and the rest are the zeros they start out as.
    @objc public let kind: Kind
    /// The host: `makesureAuthed`, `onNewDns`, `shortLinkNetworkError`.
    @objc public let host: String
    /// The user: `makesureAuthed`, `req2Buf`, `buf2Resp`, `onTaskEnd`.
    @objc public let userID: String
    /// The link: `onPush`, `identifyCheckBuffer`, `identifyResponse`.
    @objc public let channelID: String
    /// The ip a network error happened on.
    @objc public let ipAddress: String
    /// The port a network error happened on.
    @objc public let port: UInt16
    /// `onPush`, `identifyCheckBuffer`.
    @objc public let cmdid: UInt32
    /// `onPush`, `req2Buf`, `buf2Resp`, `onTaskEnd`.
    @objc public let taskID: UInt32
    /// `trafficData` — how much went out.
    @objc public let sent: Int64
    /// `trafficData` — how much came in.
    @objc public let received: Int64
    /// `req2Buf`, `buf2Resp` — the channel the task is going out on.
    @objc public let channelSelect: MarsStn.Channel
    /// `req2Buf` — the sequence the task goes out with.
    @objc public let sequence: UInt16
    /// `onPush`, `buf2Resp`, `identifyResponse` — the bytes.
    @objc public let body: Data
    /// `identifyResponse` — the `hash` of the C ABI: the hash the app handed
    /// out, which the answer is judged against. `Hash` is not a name a class
    /// can carry, because `NSObject` has one of its own.
    @objc public let hashBytes: Data
    /// `onTaskEnd`, the two network errors — one of `ErrCmdType`'s: `0` ok,
    /// `1` false, `2` dial, `3` dns, `4` socket, `5` http, `6` netmsgxp,
    /// `7` endecode, `8` server, `9` local, `10` canceld.
    @objc public let errType: Int32
    /// `onTaskEnd`, the two network errors.
    @objc public let errCode: Int32
    /// `onTaskEnd` — the connect the task ran on.
    @objc public let profile: CgiProfile?
    /// `reportTaskProfile` — the report, as the JSON
    /// `mars_stn::task_profile_json` writes.
    @objc public let profileJSON: String?
    /// `reportDnsProfile` — how the question went.
    @objc public let dns: DnsProfile?
    /// `reportConnectStatus` — whether STN can reach anything at all.
    @objc public let netStatusAll: MarsStn.NetStatus
    /// `reportConnectStatus` — the long link, in the same integers.
    @objc public let netStatusLongLink: MarsStn.NetStatus
    /// `longLinkStatusChange` — what the default long link is in.
    @objc public let linkStatus: MarsStn.LinkStatus
    /// `onNewDns` — whether the host is a long-link one.
    @objc public let isLongLinkHost: Bool
    /// `reportTaskLimited` — what the task is being weighed against.
    @objc public let checkType: Int32
    /// `reportTaskLimited` — what that gate weighed it against: how long ago
    /// the same body went out, or how many bytes the funnel would not take.
    @objc public let limit: UInt32
    /// `reportTaskLimited` — the task itself.
    @objc public let task: StnTask?

    deinit {
        // Nothing to release: every reading is a copy the C ABI's question was
        // read into, and they go with this object. The declaration is what
        // `required_deinit` asks a class for.
    }

    /// The question STN asked, as the Swift reads it.
    ///
    /// Every reading is copied out of the C struct, which is only good while the
    /// ask it was handed to runs.
    internal init(_ question: MarsStnQuestion) {
        kind = Kind(rawValue: question.kind.rawValue) ?? .nothing
        host = string(question.host)
        userID = string(question.user_id)
        channelID = string(question.channel_id)
        ipAddress = string(question.ip)
        port = question.port
        cmdid = question.cmdid
        taskID = question.taskid
        sent = question.send
        received = question.recv
        channelSelect = MarsStn.Channel(rawValue: question.channel_select) ?? .none
        sequence = question.sequence
        body = data(from: question.body, count: question.body_count)
        hashBytes = data(from: question.hash, count: question.hash_count)
        errType = question.err_type
        errCode = question.err_code
        profile = question.profile.map { CgiProfile($0.pointee) }
        profileJSON = question.profile_json.map { String(cString: $0) }
        dns = question.dns.map { DnsProfile($0.pointee) }
        netStatusAll = MarsStn.NetStatus(rawValue: question.net_status_all) ?? .none
        netStatusLongLink = MarsStn.NetStatus(rawValue: question.net_status_longlink) ?? .none
        linkStatus = MarsStn.LinkStatus(rawValue: question.link_status) ?? .none
        isLongLinkHost = question.longlink_host != 0
        checkType = question.check_type
        limit = question.limit
        task = question.task.map { StnTask($0.pointee) }
        super.init()
    }
}
