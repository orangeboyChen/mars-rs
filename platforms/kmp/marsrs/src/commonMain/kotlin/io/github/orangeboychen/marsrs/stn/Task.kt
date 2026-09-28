package io.github.orangeboychen.marsrs.stn

/**
 * One unit of work: the `Task` of `mars/stn/stn.h`, which is what an app hands
 * [StnLogic.startTask] and what the `task` of a [Question] carries.
 *
 * The fields are the ones the C++'s Java `Task` carries, under the names it
 * gives them — `taskID` and not `taskid` — for two reasons: the JNI bridge of
 * `crates/marsrs-jni` reads a task out of the object by the names of its
 * fields, and the `Task` of the Android AAR is this class under these names, so
 * a shared module that moves between `marsrs-kmp` and `marsrs` renames nothing.
 *
 * The readings the Rust `Task` keeps besides them — the channel's name and
 * group, the minor-long and QUIC hosts, the transport protocol, the redirect —
 * are not fields of this one, because the C++'s Java has none of them either:
 * a task of this class is a task of one shape on every platform.
 *
 * There is no `userContext`, which the C++'s Java has: the port hands `null`
 * for it when it asks the app about a task, on every platform, so a field an
 * app fills in and never gets back is one this class leaves out.
 */
public class Task {
    /**
     * What the task is known by, which [StnLogic.genTaskID] hands out.
     *
     * A new task draws one, and not the `0` a field left to itself would carry:
     * two tasks that answer to the same id are two entries `hasTask` and
     * `stopTask` cannot tell apart, and a callback that names it names the
     * wrong one. The Android AAR's own `Task()` draws one too, so a task of
     * this class is a task with an id whichever of the two the app builds it
     * with.
     */
    public var taskID: Int = StnLogic.genTaskID()

    /** The links the task may go out on: one of the [E_SHORT], [E_LONG] and [E_BOTH] integers. */
    public var channelSelect: Int = 0

    /** The command the server knows the task by, for one that goes out on the long link. */
    public var cmdID: Int = 0

    /** The path the server knows the task by, for one that goes out on the short link. */
    public var cgi: String? = null

    /** The hosts a short-link task may go to, in the order they are tried. */
    public var shortLinkHostList: List<String>? = null

    /** Whether the task sends and does not wait for an answer. */
    public var sendOnly: Boolean = false

    /** Whether the app has to be logged in for this task to go out. */
    public var needAuthed: Boolean = false

    /** Whether the task is one of the flow the app is limited in. */
    public var limitFlow: Boolean = false

    /** Whether the task is one of the frequency the app is limited in. */
    public var limitFrequency: Boolean = false

    /** [ENORMAL] or [EFAST]. */
    public var channelStrategy: Int = ENORMAL

    /** Whether the task is thrown away when the network changes under it. */
    public var networkStatusSensitive: Boolean = false

    /** One of the [ETASK_PRIORITY_0] … [ETASK_PRIORITY_5] integers; the lower, the sooner. */
    public var priority: Int = ETASK_PRIORITY_NORMAL

    /** How many times a task that came back badly is started again; `-1` is the default. */
    public var retryCount: Int = DEFAULT_RETRY_COUNT

    /** How long the server is expected to take, which is what a retry is weighed against. */
    public var serverProcessCost: Int = 0

    /** How long the whole task may take, in milliseconds. */
    public var totalTimeout: Int = 0

    /** What the app's own report of the task is made of. */
    public var reportArg: String? = null

    /** The headers a short-link task goes out with. */
    public var headers: Map<String, String>? = null

    /** Whether the task is one that waits on the long link for what the server pushes. */
    public var longPolling: Boolean = false

    /** How long a [longPolling] task waits, in milliseconds. */
    public var longPollingTimeout: Int = 0

    /** The sequence a long-link package goes out with, which [StnLogic.genSequenceId] hands out. */
    public var clientSequenceId: Int = 0

    public companion object {
        /** `ENORMAL` — the channel strategy that is not the fast one. */
        public const val ENORMAL: Int = 0

        public const val EFAST: Int = 1

        /** The highest priority, and the [ETASK_PRIORITY_0] the C++ numbers it. */
        public const val ETASK_PRIORITY_HIGHEST: Int = 0

        public const val ETASK_PRIORITY_0: Int = 0

        public const val ETASK_PRIORITY_1: Int = 1

        public const val ETASK_PRIORITY_2: Int = 2

        public const val ETASK_PRIORITY_3: Int = 3

        /** The priority a task that says nothing gets. */
        public const val ETASK_PRIORITY_NORMAL: Int = 3

        public const val ETASK_PRIORITY_4: Int = 4

        public const val ETASK_PRIORITY_5: Int = 5

        /** The lowest priority, and the [ETASK_PRIORITY_5] the C++ numbers it. */
        public const val ETASK_PRIORITY_LOWEST: Int = 5

        /** `Task::CHANNEL_SHORT` — the short link, which is the HTTP one. */
        public const val E_SHORT: Int = 0x1

        /** `Task::CHANNEL_LONG` — the long link, which is the app's own protocol. */
        public const val E_LONG: Int = 0x2

        /** `Task::CHANNEL_BOTH` — either of the two. */
        public const val E_BOTH: Int = 0x3

        /** What [retryCount] starts at: the C++'s own default, and not a retry of `0`. */
        public const val DEFAULT_RETRY_COUNT: Int = -1
    }
}
