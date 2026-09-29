package io.github.orangeboychen.marsrs.stn

/**
 * One of the eighteen questions STN asks while it runs a task, in the arguments
 * the C++ hands the app.
 *
 * Every reading is filled in for the [kind] in it and left at its start for the
 * others, so an app switches on the kind and reads what it names: [host] is the
 * host of [Kind.MakesureAuthed], [Kind.OnNewDns] and
 * [Kind.ShortLinkNetworkError]; [ipAddress] and [port] the pair of the two
 * network errors; [channelID] the link of [Kind.OnPush],
 * [Kind.IdentifyCheckBuffer] and [Kind.IdentifyResponse]; [body] what was
 * pushed, what came back and what the server answered.
 *
 * The readings are `var`s with an `internal` setter because the only code that
 * fills one in is the `actual` that was asked: what an app gets is a question
 * it reads, and twenty-five of them is not a list an app writes out.
 *
 * Five of the eighteen are asked on Kotlin/Native and not on Android, because
 * the JNI bridge asks Java thirteen — the thirteen the C++'s own
 * `StnLogic_C2Java.cc` asks, which is what the Android AAR answers:
 * [Kind.LongLinkNetworkError], [Kind.ShortLinkNetworkError],
 * [Kind.LongLinkStatusChange], [Kind.ReportTaskLimited] and
 * [Kind.ReportDnsProfile] are the five the bridge answers itself.
 */
public class Question internal constructor() {
    public var kind: Kind = Kind.Nothing
        internal set

    /** The host: `makesureAuthed`, `onNewDns`, `shortLinkNetworkError`. */
    public var host: String = ""
        internal set

    /** The user: `makesureAuthed`, `req2Buf`, `buf2Resp`, `onTaskEnd`. */
    public var userID: String = ""
        internal set

    /** The link: `onPush`, `identifyCheckBuffer`, `identifyResponse`. */
    public var channelID: String = ""
        internal set

    /** The ip a network error happened on. */
    public var ipAddress: String = ""
        internal set

    /** The port a network error happened on. */
    public var port: Int = 0
        internal set

    /** `onPush`, `identifyCheckBuffer`. */
    public var cmdid: Int = 0
        internal set

    /** `onPush`, `req2Buf`, `buf2Resp`, `onTaskEnd`. */
    public var taskID: Int = 0
        internal set

    /** `trafficData` — how much went out. */
    public var sent: Long = 0
        internal set

    /** `trafficData` — how much came in. */
    public var received: Long = 0
        internal set

    /** `req2Buf`, `buf2Resp` — the channel the task is going out on. */
    public var channelSelect: Int = 0
        internal set

    /** `req2Buf` — the sequence the task goes out with. */
    public var sequence: Int = 0
        internal set

    /** `onPush`, `buf2Resp`, `identifyResponse` — the bytes; empty when none came. */
    public var body: ByteArray = ByteArray(0)
        internal set

    /** `identifyResponse` — the hash the app handed out; empty when there is none. */
    public var hash: ByteArray = ByteArray(0)
        internal set

    /**
     * `onTaskEnd`, the two network errors — one of `ErrCmdType`'s: `0` ok, `1`
     * false, `2` dial, `3` dns, `4` socket, `5` http, `6` netmsgxp, `7`
     * endecode, `8` server, `9` local, `10` canceld.
     */
    public var errType: Int = 0
        internal set

    /** `onTaskEnd`, the two network errors — the code the run ended with. */
    public var errCode: Int = 0
        internal set

    /** `onTaskEnd` — the connect the task ran on. */
    public var profile: CgiProfile? = null
        internal set

    /** `reportTaskProfile` — the report, as the JSON the port writes. */
    public var profileJSON: String? = null
        internal set

    /** `reportDnsProfile` — how a dns question went. */
    public var dns: DnsProfile? = null
        internal set

    /**
     * `reportConnectStatus` — whether STN can reach anything at all: `-1` not
     * known yet, `0` nothing to reach, `1` the gateway would not answer, `2` the
     * server would not answer, `3` a link is being made, `4` a link is up, `5`
     * everything is down.
     */
    public var netStatusAll: Int = -1
        internal set

    /** `reportConnectStatus` — the long link, in the same integers. */
    public var netStatusLongLink: Int = -1
        internal set

    /**
     * `longLinkStatusChange` — what the default long link is in: `0` idle, `1`
     * connecting, `2` connected, `3` disconnected, `4` failed.
     */
    public var linkStatus: Int = 0
        internal set

    /** `onNewDns` — whether the host is a long-link one. */
    public var isLongLinkHost: Boolean = false
        internal set

    /** `reportTaskLimited` — what the task is being weighed against. */
    public var checkType: Int = 0
        internal set

    /**
     * `reportTaskLimited` — what that gate weighed it against: how long ago the
     * same body went out, or how many bytes the funnel would not take. What the
     * app answers is this number, changed or not.
     */
    public var limit: Int = 0
        internal set

    /** `reportTaskLimited` — the task itself. */
    public var task: Task? = null
        internal set

    /**
     * Which of the eighteen questions STN asked.
     *
     * The integers are the C ABI's own: they name a question, the way the C++'s
     * eighteen virtuals do by name.
     */
    public enum class Kind(internal val value: Int) {
        /** Nothing was asked: the kind a question starts out as, and one STN never asks with. */
        Nothing(0),

        /** Is the app logged in for this host and user? */
        MakesureAuthed(1),

        /** How much went out and came in. */
        TrafficData(2),

        /** The ips the app knows for a host. */
        OnNewDns(3),

        /** Something the server sent that no task asked for. */
        OnPush(4),

        /** What a task is to send. */
        Req2Buf(5),

        /** How the app reads an answer that came back. */
        Buf2Resp(6),

        /** A task that is over. */
        OnTaskEnd(7),

        /** The connection, as the app is asked to see it. */
        ReportConnectStatus(8),

        /** The main long link's errors. */
        LongLinkNetworkError(9),

        /** The short link's errors. */
        ShortLinkNetworkError(10),

        /** What the default long link is in. */
        LongLinkStatusChange(11),

        /** The check a new long link is used with. */
        IdentifyCheckBuffer(12),

        /** Whether the answer is the one the check asked for. */
        IdentifyResponse(13),

        /** The app is asked to sync. */
        RequestSync(14),

        /** The hosts the network check may probe. */
        NetCheckShortLinkHosts(15),

        /** Everything a task left behind. */
        ReportTaskProfile(16),

        /** A task the app asked to have limited. */
        ReportTaskLimited(17),

        /** How a dns question went. */
        ReportDnsProfile(18)
    }
}
