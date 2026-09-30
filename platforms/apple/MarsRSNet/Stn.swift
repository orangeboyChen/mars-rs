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
// has to be declared in this one body, and this file is held to 520 lines —
// which holds the values a task is made of, but not `StnTask`, `StnQuestion`
// and `StnAnswer` as well. `MarsStn.Task` is what an app writes either way.
//
// Every call is a straight translation of a symbol in the header; nothing here
// adds behaviour the C ABI does not have.
//
// `MarsStn` is a class and not an `enum` of statics, and every value of it is a
// class or an `@objc` enum: an app written in Objective-C runs tasks too, and
// what Objective-C cannot import is a Swift `enum` of statics, a `struct` or an
// `OptionSet`. The names it gives them are not the C ones — `MarsStnTaskHeader`,
// and not `MarsStnHeader` — because a Swift type and the C struct it is
// translated from cannot carry one name in a language with no namespaces.

import Foundation

import MarsRSNetFFI

/// `StnLogic` of `mars/stn/stn.h`: the pipeline a task is started on.
@objc
public final class MarsStn: NSObject {
    /// One unit of work, which is the `StnTask` of `StnTask.swift`.
    public typealias Task = StnTask
    /// One of the eighteen questions, which is the `StnQuestion` of
    /// `StnQuestion.swift`.
    public typealias Question = StnQuestion
    /// What the app answered, which is the `StnAnswer` of `StnAnswer.swift`.
    public typealias Answer = StnAnswer
    /// What a long link of the app's own is made from: `StnLonglinkConfig`.
    public typealias LonglinkConfig = StnLonglinkConfig

    /// The links a task may go out on: the `Task::CHANNEL_*` integers, which are
    /// a set and not one value — `0x3` is the short link and the long one.
    ///
    /// An enum and not an `OptionSet`, which is what it was while Swift was the
    /// only language that reached it: Objective-C has no set of flags, so the
    /// two combinations are cases of their own.
    @objc(MarsStnChannel)
    public enum Channel: Int32 {
        /// No channel at all, which is a task the queues refuse.
        case none = 0x0
        /// `Task::CHANNEL_SHORT`.
        case short = 0x1
        /// `Task::CHANNEL_LONG`.
        case long = 0x2
        /// `Task::CHANNEL_BOTH` — the two above.
        case both = 0x3
        /// `Task::CHANNEL_MINOR_LONG`.
        case minorLong = 0x4
        /// `Task::CHANNEL_NORMAL`.
        case normal = 0x5
        /// `Task::CHANNEL_ALL` — every link there is.
        case all = 0x7
    }

    /// `Task::TRANSPORT_PROTOCOL*` — how a task goes out.
    @objc(MarsStnTransportProtocol)
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
    ///
    /// A class and not a struct, because an app that writes Objective-C fills
    /// one in too — the reason `XlogConfig` is one.
    @objc(MarsStnTaskHeader)
    public final class Header: NSObject {
        /// The name, which is the one the request writes on the line.
        @objc public var name: String

        /// The value the server reads for that name.
        @objc public var value: String

        /// The only way in: neither field has a default of its own.
        @objc
        public init(name: String, value: String) {
            self.name = name
            self.value = value
            super.init()
        }

        deinit {
            // Nothing to release: the two strings are all this object holds,
            // and they go with it. The declaration is what `required_deinit`
            // asks a class for.
        }
    }

    /// `NetStatus` — whether STN can reach anything at all, which is what the
    /// app is told in `ReportConnectStatus`.
    ///
    /// `.none` is a status the C ABI did not hand over: what an integer outside
    /// these is read as, and what a question of another kind carries — which
    /// Objective-C has no optional enum to say.
    @objc(MarsStnNetStatus)
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
        /// A status the C ABI did not hand over.
        case none = -2
    }

    /// `LongLinkStatus` — what the default long link is in; `.none` is a status
    /// the C ABI did not hand over, as it is in [`NetStatus`].
    @objc(MarsStnLinkStatus)
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
        /// A status the C ABI did not hand over.
        case none = -1
    }

    /// `kTaskFailHandle*` — what STN does with a task whose answer was not a
    /// good one.
    @objc(MarsStnFailHandle)
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
    @objc(MarsStnIdentifyMode)
    public enum IdentifyMode: Int32 {
        /// With this very connect.
        case now = 0
        /// With the next connect.
        case nextConnect = 1
        /// Never — which is any other integer, and stops STN asking.
        case never = 2
    }

    /// `SetCallback` — the app STN talks to: the eighteen questions, funnelled
    /// through one closure.
    ///
    /// The app is asked while the process-wide pipeline is held, so `ask` must
    /// not call another `MarsStn`: everything it needs is in the question it is
    /// given. Objective-C hands the same closure over as a block.
    ///
    /// [`clearApp()`] is the way back out of this — a closure is not an optional
    /// here, and the C ABI's way of saying "no app" is a `NULL` `ask`.
    @objc
    public static func setApp(_ ask: @escaping (StnQuestion) -> StnAnswer) {
        let app = AppBox(ask)
        lock.lock()
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
        //
        // The lock is what makes the read of `installed`, the swap of the
        // context and the write back one step: Swift initializes a static
        // lazily and atomically, but a *later* read of a reference-counted
        // `var` beside a later write of it is a retain and a release of one
        // object with nothing between them.
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
        lock.unlock()
    }

    /// Takes the app away, and STN answers the eighteen questions itself again:
    /// a `NULL` `ask` is what `mars_stn.h` calls an app that answers nothing,
    /// which gets STN's own answers — and [`setApp`] cannot hand one in, its
    /// closure not being an optional.
    ///
    /// The box this drops is held until the swap is over, as it is in [`setApp`]:
    /// a question asked before this call can still be in flight, and it reads
    /// the box out of the context it was handed.
    public static func clearApp() {
        lock.lock()
        let previous = installed
        mars_stn_set_app(nil, nil)
        withExtendedLifetime(previous) {
            installed = nil
        }
        lock.unlock()
    }

    /// `Reset` — a net core made again from nothing: the tasks, the signalling
    /// session and the addresses the setters handed to the net source are gone
    /// with the one before it. The app is not: it is the caller's.
    @objc
    public static func reset() {
        mars_stn_reset()
    }

    /// `ResetAndInitEncoderVersion` — reset plus the encoder of the new net core.
    @objc
    public static func resetAndInitEncoderVersion(_ version: Int32, name: String) {
        name.withCString { mars_stn_reset_and_init_encoder_version(version, $0) }
    }

    /// `SetLonglinkSvrAddr` — the host and ports the long link goes out on, and
    /// the ip that makes it reach `host` without asking dns (empty for none).
    @objc
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
    @objc
    public static func setShortLinkServerAddress(port: UInt16, debugIP: String = "") {
        debugIP.withCString { mars_stn_set_shortlink_svr_addr(port, $0) }
    }

    /// `SetDebugIP` — a host reached without asking dns; an empty ip drops it.
    @objc
    public static func setDebugIP(host: String, ipAddress: String) {
        withCStrings([host, ipAddress]) { strings in
            mars_stn_set_debug_ip(strings[0], strings[1])
        }
    }

    /// `SetBackupIPs` — the ips a host falls back to; an empty list drops the
    /// host.
    @objc
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
    @objc
    @discardableResult
    public static func start(_ task: StnTask) -> Int32 {
        withTask(task) { mars_stn_start_task($0) }
    }

    /// `StopTask` — whether it was one of ours.
    @objc
    public static func stop(taskID: UInt32) -> Bool {
        mars_stn_stop_task(taskID) != 0
    }

    /// `HasTask` — whether the task is in one of the queues.
    @objc
    public static func hasTask(_ taskID: UInt32) -> Bool {
        mars_stn_has_task(taskID) != 0
    }

    /// `RedoTask` — every task that is out is run again.
    @objc
    public static func redoTasks() {
        mars_stn_redo_tasks()
    }

    /// `TouchTasks` — the queues are sorted again.
    @objc
    public static func touchTasks() {
        mars_stn_touch_tasks()
    }

    /// `ClearTask` — every task that is out is thrown away.
    @objc
    public static func clearTasks() {
        mars_stn_clear_tasks()
    }

    /// `MakesureLongLinkConnected` — whether there was a default link to connect.
    @objc
    public static func makeSureLongLinkConnected() -> Bool {
        mars_stn_makesure_longlink_connected() != 0
    }

    /// `MakesureLonglinkConnected_ext` — the link `name` was made with is asked
    /// to connect; a name no link was made with is nothing at all.
    @objc
    public static func makeSureLongLinkConnected(name: String) {
        name.withCString { mars_stn_makesure_longlink_connected_ext($0) }
    }

    /// `LongLinkIsConnected` — whether the default long link is up, which is
    /// `kConnected` and nothing else, so one that is still connecting is not.
    @objc
    public static func isLongLinkConnected() -> Bool {
        mars_stn_longlink_is_connected() != 0
    }

    /// `LongLinkIsConnected_ext` — whether the link `name` was made with is up,
    /// `false` for a name no link has.
    @objc
    public static func isLongLinkConnected(name: String) -> Bool {
        name.withCString { mars_stn_longlink_is_connected_ext($0) != 0 }
    }

    /// `DisableLongLink` — no task goes out on a long link again, and only
    /// `reset` opens it again: the C++'s is a one-way door.
    @objc
    public static func disableLongLink() {
        mars_stn_disable_longlink()
    }

    /// `getNoopTaskID` — the task id of the noop, the one task no app started,
    /// which `req2Buf` and `onPush` are asked for too.
    @objc
    public static func noopTaskID() -> UInt32 {
        mars_stn_noop_task_id()
    }

    /// `CreateLonglink_ext` — a long link of the app's own, made the way the
    /// default one was; a name a link already has is that link, not a second one.
    ///
    /// - Returns: whether the link is there, which the C++ cannot say — its
    ///   `CreateLonglink_ext` is `void`.
    @objc
    @discardableResult
    public static func createLongLink(_ config: StnLonglinkConfig) -> Bool {
        withLonglinkConfig(config) { mars_stn_create_longlink($0) == MARS_STN_OK }
    }

    /// `DestroyLonglink_ext` — the link of that name is gone, and every task
    /// that was going out on it is failed.
    ///
    /// - Returns: whether a link of that name was there.
    @objc
    @discardableResult
    public static func destroyLongLink(_ name: String) -> Bool {
        name.withCString { mars_stn_destroy_longlink($0) != 0 }
    }

    /// `MarkMainLonglink_ext` — the link of that name is the one whose errors
    /// and status the app is told about, and the one the calls that mean "the"
    /// long link reach.
    ///
    /// - Returns: whether it is the main one now; `false` when no link has that
    ///   name, or when it already was.
    @objc
    @discardableResult
    public static func markMainLongLink(_ name: String) -> Bool {
        name.withCString { mars_stn_mark_main_longlink($0) != 0 }
    }

    /// `SetSignallingStrategy` — for every keeper in the process. A period or a
    /// keep time of `0` leaves the `SignallingKeeper` defaults alone.
    @objc
    public static func setSignallingStrategy(period: Int64, keepTime: Int64) {
        mars_stn_set_signalling_strategy(period, keepTime)
    }

    /// `KeepSignalling`.
    @objc
    public static func keepSignalling() {
        mars_stn_keep_signalling()
    }

    /// `StopSignalling`.
    @objc
    public static func stopSignalling() {
        mars_stn_stop_signalling()
    }

    /// `SetClientVersion` — the version every long-link package goes out with.
    @objc
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
    ///
    /// An `NSNumber` and not a `UInt64?`, because Objective-C has no optional
    /// integer: `nil` is still the error above, and what the number holds is the
    /// milliseconds, which Objective-C reads with `unsignedLongLongValue`.
    @objc public static var dueTime: NSNumber? {
        let due = mars_stn_due_time()
        return due < 0 ? nil : NSNumber(value: due)
    }

    /// What the C++'s message queue thread would have done: the follow-ups, one
    /// at a time in the order they were posted, and then one pass of everything
    /// the two queues and the zombies only do when they are asked — a task's
    /// first-package timeout is one of those, and a zombie started again is
    /// another.
    ///
    /// The C++ runs this on threads of its own; this port has none, so it is the
    /// app's loop that calls it, and [`dueTime`] is how long it may wait. The
    /// two are one pair:
    /// `MarsStn` that let an app start a task and not drain it would be a
    /// pipeline an app can fill and never empty.
    @objc
    public static func runPending() {
        mars_stn_run_pending()
    }

    /// `GenTaskID` — one counter for the whole process.
    @objc
    public static func generateTaskID() -> UInt32 {
        mars_stn_gen_task_id()
    }

    /// `GenSequenceId` — an `unsigned short`, like the C++'s.
    @objc
    public static func generateSequenceID() -> UInt16 {
        mars_stn_gen_sequence_id()
    }

    /// `TrigNooping` — a noop on the default long link, and a heartbeat of `0`.
    @objc
    public static func triggerNooping() {
        mars_stn_trig_nooping()
    }

    /// `ActiveLogic::OnForeground` — the app came to the front, or left it,
    /// which is what a task asks before it wakes a long link that is down, and
    /// what the anti-avalanche check and the timing sync are told about.
    ///
    /// `BaseEvent.onForeground` on Android is this call; a host of the C ABI has
    /// no `BaseEvent` to make it on, so it makes it here. One that never does
    /// gets the C++'s `ActiveLogic` as it is made — not in front, so nothing is
    /// woken for a task — and ten minutes in the background end that, which
    /// [`runPending`] counts.
    @objc
    public static func onForeground(_ isForeground: Bool) {
        mars_stn_on_foreground(isForeground ? 1 : 0)
    }

    /// `GetSignalOnNetworkChange` — the network under the app changed.
    ///
    /// The C++ clears the net cache before it fires this; the port reads the
    /// network on every ask, so there is no cache to clear.
    @objc
    public static func onNetworkChange() {
        mars_stn_on_network_change()
    }

    deinit {
        // Nothing to release: the app is the only thing `MarsStn` holds, and it
        // is `installed`. The declaration is what `required_deinit` asks for.
    }

    /// There is nothing an app makes: every member of this type is a `static`,
    /// and the class is what lets Objective-C write them.
    override private init() {
        super.init()
    }

    /// What guards [`installed`], and with it the read-modify-write [`setApp`]
    /// and [`clearApp`] make of it: two threads may install an app at once, and
    /// a static `var` is only *initialized* atomically — a later read of a
    /// reference-counted one beside a later write of it is a retain and a
    /// release of the same object with nothing between them.
    private static let lock = NSLock()

    /// The app that is installed, which is the only thing the C ABI does not
    /// give back: `setApp` remembers it so that the next app releases it.
    private static var installed: AppBox?
}
