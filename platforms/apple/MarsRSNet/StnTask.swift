// `StnTask` — one unit of work, which is `Task` of `mars/stn/stn.h` — and the
// two ways across: the C struct a task is started with, and the Swift one a
// question carries.
//
// The struct is the widest thing in the port: every field of the C one, with the
// five flags as `Bool`s and the seven lists as `[String]`s, because that is what
// an app fills in. Its defaults are the C++'s, which is a task with no channel
// to go out on — one the queues refuse until a `channelSelect` is set.

import Foundation

/// The pointers a `MarsStnTask` is made of, lent for the length of a call: the
/// seven strings of a task, then its five host lists, then its headers, each
/// under the name the C struct reads it by.
private struct LentTask {
    var cgi: UnsafePointer<CChar>?
    var reportArgument: UnsafePointer<CChar>?
    var channelName: UnsafePointer<CChar>?
    var groupName: UnsafePointer<CChar>?
    var userID: UnsafePointer<CChar>?
    var function: UnsafePointer<CChar>?
    var cgiPrefix: UnsafePointer<CChar>?
    var shortLinkHosts = MarsStnStrings(items: nil, count: 0)
    var shortLinkFallbackHosts = MarsStnStrings(items: nil, count: 0)
    var longLinkHosts = MarsStnStrings(items: nil, count: 0)
    var minorLongHosts = MarsStnStrings(items: nil, count: 0)
    var quicHosts = MarsStnStrings(items: nil, count: 0)
    var headers: UnsafePointer<MarsStnHeader>?
}

/// One unit of work: `Task` of `mars/stn/stn.h`, which `MarsStn` names
/// `MarsStn.Task`.
///
/// A class and not a struct, because an app that writes Objective-C fills one in
/// too — the reason `XlogConfig` is a class and not a struct. What Swift gives up
/// is the copy a struct would make: a task is read once, when it is started.
@objc
public final class StnTask: NSObject {
    /// The id STN identifies the task by; `MarsStn.generateTaskID()` hands one
    /// out.
    @objc public var taskID: UInt32 = 0
    /// The command, for a task that goes out on a long link.
    @objc public var cmdid: UInt32 = 0
    /// The channel, for a task the app keeps one per channel.
    @objc public var channelID: UInt64 = 0
    /// The links the task may go out on; no channel at all is a task the queues
    /// refuse.
    @objc public var channelSelect: MarsStn.Channel = .none
    /// How the task goes out.
    @objc public var transportProtocol: MarsStn.TransportProtocol = .default
    /// The cgi, for a task that goes out on a short link.
    @objc public var cgi: String = ""
    /// Whether the task is a send with no answer to wait for.
    @objc public var isSendOnly: Bool = false
    /// Whether the task waits for the app to be logged in; `Task::new`'s
    /// answer, which the app turns off for the one task that goes out before it
    /// has logged in.
    @objc public var needsAuthed: Bool = true
    /// Whether the task is weighed against the flow limit; `Task::new`'s
    /// answer, and the Kotlin `Task` constructor's.
    @objc public var limitsFlow: Bool = true
    /// Whether it is weighed against the frequency limit; likewise.
    @objc public var limitsFrequency: Bool = true
    /// Whether a task with no network to go out on is ended instead of waiting.
    @objc public var isNetworkStatusSensitive: Bool = false
    /// One of the `Task::CHANNEL_*_STRATEGY` integers.
    @objc public var channelStrategy: Int32 = 0
    /// One of the `Task::TASK_PRIORITY_*` integers; `3` — the normal one, and
    /// the one `Task::new` gives — not `0`, which is the highest.
    @objc public var priority: Int32 = 3
    /// How many tries the task has: `-1` is the net core's own count, which is
    /// what `Task::new` gives, and `0` is a task with no try back at all.
    @objc public var retryCount: Int32 = -1
    /// How long the server is expected to take, which is added to the timeout.
    @objc public var serverProcessCost: Int32 = 0
    /// How long the whole task may take, retries and all.
    @objc public var totalTimeout: Int32 = 0
    /// Whether the answer may come late.
    @objc public var allowsLongPolling: Bool = false
    /// How long a late answer may take.
    @objc public var longPollingTimeout: Int32 = 0
    /// What the app's own report is keyed by.
    @objc public var reportArgument: String = ""
    /// The long link the task goes out on; empty is the default one.
    @objc public var channelName: String = ""
    /// The group the task belongs to.
    @objc public var groupName: String = ""
    /// The user the task is sent for.
    @objc public var userID: String = ""
    /// The protocol the app speaks.
    @objc public var appProtocol: Int32 = 0
    /// The headers of the request.
    @objc public var headers: [MarsStn.Header] = []
    /// The short link's hosts.
    @objc public var shortLinkHosts: [String] = []
    /// The hosts a short-link host falls back to.
    @objc public var shortLinkFallbackHosts: [String] = []
    /// The long link's hosts.
    @objc public var longLinkHosts: [String] = []
    /// The hosts of the minor long links.
    @objc public var minorLongHosts: [String] = []
    /// The hosts a quic connection may be made to.
    @objc public var quicHosts: [String] = []
    /// How many minor long links the task may use.
    @objc public var maxMinorLinks: Int32 = 0
    /// What the task is, for the app's own report.
    @objc public var function: String = ""
    /// What the cgi is prefixed with.
    @objc public var cgiPrefix: String = ""
    /// One of `HostRedirectType`'s: `0` none, `1` bare to https, `2` http to
    /// https, `3` new host.
    @objc public var redirectType: Int32 = 0
    /// The sequence the app numbers the task with.
    @objc public var clientSequenceID: UInt16 = 0

    /// A task with every field at the default the C++ gives it, which is a task
    /// with no channel to go out on.
    override public init() {
        super.init()
    }

    /// The only way an app makes one: every field has the default the C++ gives
    /// it, and `channelSelect` is the one an app fills in first, because a task
    /// with no channel to go out on is one the queues refuse.
    @objc
    public convenience init(channelSelect: MarsStn.Channel) {
        self.init()
        self.channelSelect = channelSelect
    }

    deinit {
        // Nothing to release: the strings and the lists are all this object
        // holds, and they go with it. The declaration is what `required_deinit`
        // asks a class for.
    }

    /// The task a question carries, as the Swift reads it.
    internal convenience init(_ task: MarsStnTask) {
        self.init(channelSelect: MarsStn.Channel(rawValue: task.channel_select) ?? .none)
        taskID = task.taskid
        cmdid = task.cmdid
        channelID = task.channel_id
        transportProtocol = MarsStn.TransportProtocol(rawValue: task.transport_protocol) ?? .default
        cgi = string(task.cgi)
        isSendOnly = task.send_only != 0
        needsAuthed = task.need_authed != 0
        limitsFlow = task.limit_flow != 0
        limitsFrequency = task.limit_frequency != 0
        isNetworkStatusSensitive = task.network_status_sensitive != 0
        channelStrategy = task.channel_strategy
        priority = task.priority
        retryCount = task.retry_count
        serverProcessCost = task.server_process_cost
        totalTimeout = task.total_timeout
        allowsLongPolling = task.long_polling != 0
        longPollingTimeout = task.long_polling_timeout
        reportArgument = string(task.report_arg)
        channelName = string(task.channel_name)
        groupName = string(task.group_name)
        userID = string(task.user_id)
        appProtocol = task.protocol
        headers = Self.read(from: task)
        shortLinkHosts = strings(from: task.shortlink_host_list)
        shortLinkFallbackHosts = strings(from: task.shortlink_fallback_hostlist)
        longLinkHosts = strings(from: task.longlink_host_list)
        minorLongHosts = strings(from: task.minorlong_host_list)
        quicHosts = strings(from: task.quic_host_list)
        maxMinorLinks = task.max_minorlinks
        function = string(task.function)
        cgiPrefix = string(task.cgi_prefix)
        redirectType = task.redirect_type
        clientSequenceID = task.client_sequence_id
    }

    /// The headers of `task`: a name and a value per header, read out of the two
    /// pointers of each.
    private static func read(from task: MarsStnTask) -> [MarsStn.Header] {
        guard let pointer = task.headers else {
            return []
        }
        return (0 ..< Int(task.header_count)).map { index in
            MarsStn.Header(name: string(pointer[index].name), value: string(pointer[index].value))
        }
    }
}

/// The C struct `task` is started with, valid for the length of the closure:
/// every string and list of it is lent, because a task is read before the call
/// it was handed to answers.
internal func withTask<R>(_ task: StnTask, body: (UnsafePointer<MarsStnTask>) -> R) -> R {
    withCStrings([
        task.cgi,
        task.reportArgument,
        task.channelName,
        task.groupName,
        task.userID,
        task.function,
        task.cgiPrefix
    ]) { strings in
        withStringLists([
            task.shortLinkHosts,
            task.shortLinkFallbackHosts,
            task.longLinkHosts,
            task.minorLongHosts,
            task.quicHosts
        ]) { hosts in
            withHeaders(task.headers) { headers in
                var lent = LentTask()
                fill(&lent, from: task, strings: strings, hosts: hosts)
                lent.headers = headers
                var value = cTask(task, lent)
                return withUnsafePointer(to: &value, body)
            }
        }
    }
}

/// Fills `lent` in from `strings` and `hosts`, which are in the order a task
/// declares its own in.
private func fill(
    _ lent: inout LentTask,
    from task: StnTask,
    strings: [UnsafePointer<CChar>?],
    hosts: [UnsafePointer<UnsafePointer<CChar>?>?]
) {
    var string = 0
    func nextString() -> UnsafePointer<CChar>? {
        let value = strings[string]
        string += 1
        return value
    }

    // Its own counter, and not `string`'s: the lists are a second array of
    // theirs, five long, so a task whose seven strings have been taken reads
    // past the end of it — `hosts[7]`, of five — before a host is ever lent.
    var host = 0
    func nextHosts(_ count: Int) -> MarsStnStrings {
        let value = hosts[host]
        host += 1
        return MarsStnStrings(items: value, count: UInt32(count))
    }
    lent.cgi = nextString()
    lent.reportArgument = nextString()
    lent.channelName = nextString()
    lent.groupName = nextString()
    lent.userID = nextString()
    lent.function = nextString()
    lent.cgiPrefix = nextString()
    lent.shortLinkHosts = nextHosts(task.shortLinkHosts.count)
    lent.shortLinkFallbackHosts = nextHosts(task.shortLinkFallbackHosts.count)
    lent.longLinkHosts = nextHosts(task.longLinkHosts.count)
    lent.minorLongHosts = nextHosts(task.minorLongHosts.count)
    lent.quicHosts = nextHosts(task.quicHosts.count)
}

/// The C struct of `task`, made of the pointers `lent` holds.
private func cTask(_ task: StnTask, _ lent: LentTask) -> MarsStnTask {
    MarsStnTask(
        taskid: task.taskID,
        cmdid: task.cmdid,
        channel_id: task.channelID,
        channel_select: task.channelSelect.rawValue,
        transport_protocol: task.transportProtocol.rawValue,
        cgi: lent.cgi,
        send_only: task.isSendOnly ? 1 : 0,
        need_authed: task.needsAuthed ? 1 : 0,
        limit_flow: task.limitsFlow ? 1 : 0,
        limit_frequency: task.limitsFrequency ? 1 : 0,
        network_status_sensitive: task.isNetworkStatusSensitive ? 1 : 0,
        channel_strategy: task.channelStrategy,
        priority: task.priority,
        retry_count: task.retryCount,
        server_process_cost: task.serverProcessCost,
        total_timeout: task.totalTimeout,
        long_polling: task.allowsLongPolling ? 1 : 0,
        long_polling_timeout: task.longPollingTimeout,
        report_arg: lent.reportArgument,
        channel_name: lent.channelName,
        group_name: lent.groupName,
        user_id: lent.userID,
        protocol: task.appProtocol,
        headers: lent.headers,
        header_count: UInt32(task.headers.count),
        shortlink_host_list: lent.shortLinkHosts,
        shortlink_fallback_hostlist: lent.shortLinkFallbackHosts,
        longlink_host_list: lent.longLinkHosts,
        minorlong_host_list: lent.minorLongHosts,
        quic_host_list: lent.quicHosts,
        max_minorlinks: task.maxMinorLinks,
        function: lent.function,
        cgi_prefix: lent.cgiPrefix,
        redirect_type: task.redirectType,
        client_sequence_id: task.clientSequenceID
    )
}

/// The C array of `headers`: one name and one value per header, in the order
/// they were handed over.
internal func withHeaders<R>(
    _ headers: [MarsStn.Header],
    body: (UnsafePointer<MarsStnHeader>?) -> R
) -> R {
    guard !headers.isEmpty else {
        return body(nil)
    }
    return withCStrings(headers.map(\.name)) { names in
        withCStrings(headers.map(\.value)) { values in
            let buffer = UnsafeMutablePointer<MarsStnHeader>.allocate(capacity: headers.count)
            defer { buffer.deallocate() }
            for index in headers.indices {
                buffer[index] = MarsStnHeader(name: names[index], value: values[index])
            }
            return body(UnsafePointer(buffer))
        }
    }
}
