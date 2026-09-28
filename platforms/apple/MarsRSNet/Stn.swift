// The Swift face of the C ABI in `mars_stn.h` (crate `marsrs-ffi`, the `stn`
// feature): the task pipeline, and the one thing a caller supplies — the app the
// eighteen questions are asked of.
//
// This is the net half of the port: `MarsRSNet` carries this file and
// `Sdt.swift`, and `MarsRS` re-exports the module, so `import MarsRS` is all an
// app that runs tasks writes. `MarsRSNet` is not a product of its own: it is the
// half that is not xlog, because xlog is the half an app that only logs takes —
// and the framework it is built from carries no `mars_xlog_*` symbol, which is
// what lets an app that takes both link xlog's exactly once.
//
// The binary target of Package.swift is a static library plus a module map, so
// what `import MarsRSNetFFI` gives a caller is the C surface itself: pointers to
// C strings, arrays with a count beside them and tagged structs whose fields are
// read for the one tag they carry. This file is that surface in Swift —
// `String`s, enums with the answers and an app the caller hands over as a
// closure — and re-exports the C module, so `mars_stn_start_task` and friends
// are still reachable from here for whoever prefers them.
//
// Three of the types of that surface are not nested here but are types of the
// module, named again by the typealiases below: an extension may not carry an
// access modifier and neither may its members, so a type nested in `MarsStn`
// has to be declared in this one body, and this file is held to 400 lines —
// which holds the values a task is made of, but not `StnTask`, `StnQuestion`
// and `StnAnswer` as well. `MarsStn.Task` is what an app writes either way.
//
// Every call is a straight translation of a symbol in the header; nothing here
// adds behaviour the C ABI does not have.

import Foundation

import MarsRSNetFFI

/// `StnLogic` of `mars/stn/stn.h`: the pipeline a task is started on.
public enum MarsStn {
    /// One unit of work, which is the `StnTask` of `StnTask.swift`.
    public typealias Task = StnTask
    /// One of the eighteen questions, which is the `StnQuestion` of
    /// `StnQuestion.swift`.
    public typealias Question = StnQuestion
    /// What the app answered, which is the `StnAnswer` of `StnAnswer.swift`.
    public typealias Answer = StnAnswer

    /// The `Task::CHANNEL_*` integers: what `Channel` is made of, under the
    /// names the C++ gives them.
    private enum ChannelValue: Int32 {
        case short = 0x1
        case long = 0x2
        case minorLong = 0x4
        case normal = 0x5
        case all = 0x7
    }

    /// The links a task may go out on: the `Task::CHANNEL_*` integers, which are
    /// a set and not one value — `0x3` is the short link and the long one.
    ///
    /// `[]` is no channel at all, which is a task the queues refuse.
    public struct Channel: OptionSet {
        /// The `Task::CHANNEL_*` integers.
        public var rawValue: Int32

        public init(rawValue: Int32) {
            self.rawValue = rawValue
        }

        /// `Task::CHANNEL_SHORT`.
        public static let short = Self(rawValue: ChannelValue.short.rawValue)
        /// `Task::CHANNEL_LONG`.
        public static let long = Self(rawValue: ChannelValue.long.rawValue)
        /// `Task::CHANNEL_BOTH` — the two above, which is what the set is for.
        public static let both: Self = [.short, .long]
        /// `Task::CHANNEL_MINOR_LONG`.
        public static let minorLong = Self(rawValue: ChannelValue.minorLong.rawValue)
        /// `Task::CHANNEL_NORMAL`.
        public static let normal = Self(rawValue: ChannelValue.normal.rawValue)
        /// `Task::CHANNEL_ALL` — every link there is.
        public static let all = Self(rawValue: ChannelValue.all.rawValue)
    }

    /// `Task::TRANSPORT_PROTOCOL*` — how a task goes out.
    public enum TransportProtocol: Int32 {
        /// The link's own choice.
        case `default` = 0
        /// TCP.
        case tcp = 1
        /// QUIC.
        case quic = 2
        /// Whichever of the two the link has.
        case mixed = 3
    }

    /// One header of a task: a name and a value.
    public struct Header {
        public var name: String
        public var value: String

        /// The only public initializer: the memberwise one a struct gets is
        /// internal, so without this one the type is one an app can read and
        /// never write.
        public init(name: String, value: String) {
            self.name = name
            self.value = value
        }
    }

    /// `NetStatus` — whether STN can reach anything at all, which is what the
    /// app is told in `ReportConnectStatus`.
    public enum NetStatus: Int32 {
        /// Not known yet.
        case unknown = -1
        /// Nothing to reach.
        case unavailable = 0
        /// The gateway would not answer.
        case gatewayFailed = 1
        /// The server would not answer.
        case serverFailed = 2
        /// A link is being made.
        case connecting = 3
        /// A link is up.
        case connected = 4
        /// Everything is down.
        case down = 5
    }

    /// `LongLinkStatus` — what the default long link is in.
    public enum LinkStatus: Int32 {
        /// Nothing is happening on it.
        case idle = 0
        /// A connect is being made.
        case connecting = 1
        /// It is up.
        case connected = 2
        /// It was up and is not any more.
        case disconnected = 3
        /// A connect was tried and failed.
        case failed = 4
    }

    /// `kTaskFailHandle*` — what STN does with a task whose answer was not a
    /// good one.
    public enum FailHandle: Int32 {
        /// The task failed the way a task fails.
        case normal = 0
        /// Whatever the net core does with a failure it was not told about.
        case `default` = -1
        /// Every task that is out is run again.
        case retryAllTasks = -12
        /// The session is over.
        case sessionTimeout = -13
        /// This task is over.
        case taskEnd = -14
        /// This task took too long.
        case taskTimeout = -15
        /// This task is over, and the app is not told.
        case silentTaskEnd = -16
    }

    /// When the check a new long link is used with goes out: the `mode` of
    /// `IdentifyCheckBuffer`.
    public enum IdentifyMode: Int32 {
        /// With this very connect.
        case now = 0
        /// With the next connect.
        case nextConnect = 1
        /// The check never goes out, and STN stops asking: every integer but
        /// `0` and `1` is read as this one.
        case never = 2
    }

    /// `SetCallback` — the app STN talks to: the eighteen questions, funnelled
    /// through one closure.
    ///
    /// The app is asked while the process-wide pipeline is held, so `ask` must
    /// not call another `MarsStn`: everything it needs is in the question it is
    /// given.
    public static func setApp(_ ask: @escaping (StnQuestion) -> StnAnswer) {
        let app = AppBox(ask)
        // The box is the context of every question, and `installed` is what
        // keeps it alive: the C ABI hands the pointer back with a question and
        // never gives it back.
        //
        // The one before it is held until the swap is over, and not released by
        // the assignment that replaces it: a question can be in flight while
        // this runs, and the callback rebuilds the box out of the pointer with
        // `takeUnretainedValue()`, so a box freed before `mars_stn_set_app`
        // has put the new context in its place is a question that
        // dereferences a freed one. Both are locals of this call, so the old
        // box dies here at the earliest — after the swap.
        let previous = installed
        mars_stn_set_app(Unmanaged.passUnretained(app).toOpaque()) { ctx, question, answer in
            guard let ctx, let question, let answer else {
                return
            }
            let asked = Unmanaged<AppBox>.fromOpaque(ctx).takeUnretainedValue()
            answer.pointee = asked.answer(to: question.pointee)
        }
        withExtendedLifetime(previous) {
            installed = app
        }
    }

    /// `Reset` — a net core made again from nothing: the tasks, the signalling
    /// session and the addresses the setters handed to the net source are gone
    /// with the one before it. The app is not: it is the caller's.
    public static func reset() {
        mars_stn_reset()
    }

    /// `ResetAndInitEncoderVersion` — reset plus the encoder of the new net core.
    public static func resetAndInitEncoderVersion(_ version: Int32, name: String) {
        name.withCString { mars_stn_reset_and_init_encoder_version(version, $0) }
    }

    /// `SetLonglinkSvrAddr` — the host and ports the long link goes out on, and
    /// the ip that makes it reach `host` without asking dns (empty for none).
    public static func setLongLinkServerAddress(
        _ host: String,
        ports: [UInt16],
        debugIP: String = ""
    ) {
        withCStrings([host, debugIP]) { strings in
            ports.withUnsafeBufferPointer { buffer in
                mars_stn_set_longlink_svr_addr(
                    strings[0],
                    buffer.baseAddress,
                    UInt32(ports.count),
                    strings[1]
                )
            }
        }
    }

    /// `SetShortlinkSvrAddr`.
    public static func setShortLinkServerAddress(port: UInt16, debugIP: String = "") {
        debugIP.withCString { mars_stn_set_shortlink_svr_addr(port, $0) }
    }

    /// `SetDebugIP` — a host reached without asking dns; an empty ip drops it.
    public static func setDebugIP(host: String, ipAddress: String) {
        withCStrings([host, ipAddress]) { strings in
            mars_stn_set_debug_ip(strings[0], strings[1])
        }
    }

    /// `SetBackupIPs` — the ips a host falls back to; an empty list drops the
    /// host.
    public static func setBackupIPs(host: String, ips: [String]) {
        withCStrings([host]) { strings in
            withStrings(ips) { list in
                mars_stn_set_backup_ips(strings[0], list, UInt32(ips.count))
            }
        }
    }

    /// `StartTask` — one unit of work.
    ///
    /// - Returns: `MARS_STN_OK`, or `MARS_STN_ERR_REFUSED` when the task is not
    ///   one the queues would take — no channel to go out on, a timeout the C++
    ///   refuses — or `MARS_STN_ERR_PANIC`.
    @discardableResult
    public static func start(_ task: StnTask) -> Int32 {
        withTask(task) { mars_stn_start_task($0) }
    }

    /// `StopTask` — whether it was one of ours.
    public static func stop(taskID: UInt32) -> Bool {
        mars_stn_stop_task(taskID) != 0
    }

    /// `HasTask` — whether the task is in one of the queues.
    public static func hasTask(_ taskID: UInt32) -> Bool {
        mars_stn_has_task(taskID) != 0
    }

    /// `RedoTask` — every task that is out is run again.
    public static func redoTasks() {
        mars_stn_redo_tasks()
    }

    /// `TouchTasks` — the queues are sorted again.
    public static func touchTasks() {
        mars_stn_touch_tasks()
    }

    /// `ClearTask` — every task that is out is thrown away.
    public static func clearTasks() {
        mars_stn_clear_tasks()
    }

    /// `MakesureLongLinkConnected` — whether there was a default link to connect.
    public static func makeSureLongLinkConnected() -> Bool {
        mars_stn_makesure_longlink_connected() != 0
    }

    /// `SetSignallingStrategy` — for every keeper in the process. A period or a
    /// keep time of `0` leaves the `SignallingKeeper` defaults alone.
    public static func setSignallingStrategy(period: Int64, keepTime: Int64) {
        mars_stn_set_signalling_strategy(period, keepTime)
    }

    /// `KeepSignalling`.
    public static func keepSignalling() {
        mars_stn_keep_signalling()
    }

    /// `StopSignalling`.
    public static func stopSignalling() {
        mars_stn_stop_signalling()
    }

    /// `SetClientVersion` — the version every long-link package goes out with.
    public static func setClientVersion(_ version: UInt32) {
        mars_stn_set_client_version(version)
    }

    /// `NetCore::GetNextHeartbeatTime` — how long the app's loop may wait before
    /// it calls [`runPending`] again: the soonest of the two queues, the zombie
    /// check and the timing sync's alarm, as milliseconds left, and `0` for a
    /// pass that is already due.
    ///
    /// `nil` is an error the C ABI answered with — [`MARS_STN_ERR_NO_DUE`] for a
    /// core with nothing to wait for, [`MARS_STN_ERR_PANIC`] for a panic — and
    /// not a delay, which is never negative.
    ///
    /// A task that is out is always waiting on something, so this is not a
    /// heartbeat an app may ignore: a task that is started and never drained
    /// sits in its queue until the process ends.
    public static var dueTime: UInt64? {
        let due = mars_stn_due_time()
        return due < 0 ? nil : UInt64(due)
    }

    /// What the C++'s message queue thread would have done: the follow-ups, one
    /// at a time in the order they were posted, and then one pass of everything
    /// the two queues and the zombies only do when they are asked — a task's
    /// first-package timeout is one of those, and a zombie started again is
    /// another.
    ///
    /// The C++ runs this on threads of its own; this port has none, so it is the
    /// app's loop that calls it, and [`dueTime`] is how long it may wait. The
    /// two are one pair: a `MarsStn` that let an app start a task and never
    /// drain it would be a pipeline an app can fill and never empty.
    public static func runPending() {
        mars_stn_run_pending()
    }

    /// `GenTaskID` — one counter for the whole process.
    public static func generateTaskID() -> UInt32 {
        mars_stn_gen_task_id()
    }

    /// `GenSequenceId` — an `unsigned short`, like the C++'s.
    public static func generateSequenceID() -> UInt16 {
        mars_stn_gen_sequence_id()
    }

    /// `TrigNooping` — a noop on the default long link, and a heartbeat of `0`.
    public static func triggerNooping() {
        mars_stn_trig_nooping()
    }

    /// The app that is installed, which is the only thing the C ABI does not
    /// give back: `setApp` remembers it so that the next app releases it.
    private static var installed: AppBox?
}
