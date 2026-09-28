package io.github.orangeboychen.marsrs.stn

/**
 * `StnLogic` of `mars/stn/stn.h`: the pipeline a task is started on, which is
 * the net half of the port on every platform of a Kotlin Multiplatform project.
 *
 * One `expect`, two `actual`s: over the C ABI of `crates/marsrs-ffi`
 * (`mars_stn.h`) on every Kotlin/Native target, and over the JNI bridge of
 * `crates/marsrs-jni` on Android. The names are the ones the Android AAR
 * publishes — `startTask`, `redoTask`, `dueTime` — because a shared module that
 * moves between `marsrs-kmp` and `marsrs` has nothing to rename when it does.
 *
 * The one thing an app supplies is the [setApp]: the eighteen questions STN
 * asks while it runs a task, funnelled through one function the way the C ABI
 * funnels them through one function pointer. That is the port's seam, and it is
 * the C++'s eighteen virtuals: answer [Answer.None] — or hand no app at all —
 * and STN takes its own answers, which are the ones a host with no app gets.
 *
 * An `ask` is asked while the process-wide pipeline is held, so it must not
 * call another [StnLogic]: everything it needs is in the [Question] it is
 * given.
 *
 * There are no threads in this port, which is what [runPending] and [dueTime]
 * are: the C++ runs what they do on a thread of its own, and here it is the
 * app's loop that calls them. A task that is started and never drained sits in
 * its queue until the process ends, so the two of them are one pair.
 */
public expect object StnLogic {
    /**
     * `SetCallback` — the app STN talks to: the eighteen questions, funnelled
     * through one function. `null` is an app that answers nothing.
     *
     * The app is asked while the process-wide pipeline is held, so `ask` must
     * not call another [StnLogic]: everything it needs is in the question it is
     * given.
     */
    public fun setApp(ask: ((Question) -> Answer)?)

    /**
     * `Reset` — a net core made again from nothing: the tasks, the signalling
     * session and the addresses the setters handed to the net source are gone
     * with the one before it. The app is not: it is the caller's.
     */
    public fun reset()

    /** `ResetAndInitEncoderVersion` — reset plus the encoder of the new net core. */
    public fun resetAndInitEncoderVersion(version: Int, name: String?)

    /**
     * `SetLonglinkSvrAddr` — the host and ports the long link goes out on.
     *
     * @param debugIP the ip that makes `host` reachable without asking dns;
     *                `null` or empty for none
     */
    public fun setLonglinkSvrAddr(host: String?, ports: IntArray?, debugIP: String?)

    /**
     * `SetShortlinkSvrAddr`.
     *
     * @param debugIP the ip a short-link task reaches in place of the host it
     *                names; `null` or empty for none
     */
    public fun setShortlinkSvrAddr(port: Int, debugIP: String?)

    /** `SetDebugIP` — a host reached without asking dns; an empty ip drops it. */
    public fun setDebugIP(host: String?, ip: String?)

    /** `SetBackupIPs` — the ips a host falls back to; an empty list drops the host. */
    public fun setBackupIPs(host: String?, ips: Array<String>?)

    /**
     * `StartTask` — one unit of work.
     *
     * Nothing comes back, and a task the queues refuse is one the app hears
     * about the way it hears about every task that is over: a [Question] of
     * kind [Question.Kind.OnTaskEnd] carrying the code it was refused with.
     */
    public fun startTask(task: Task)

    /** `StopTask` — one task that is out is thrown away. */
    public fun stopTask(taskID: Int)

    /** `HasTask` — whether the task is in one of the queues. */
    public fun hasTask(taskID: Int): Boolean

    /** `RedoTask` — every task that is out is run again, which reconnects the long link. */
    public fun redoTask()

    /** `TouchTasks` — the queues are sorted again. */
    public fun touchTasks()

    /** `ClearTask` — every task that is out is thrown away. */
    public fun clearTask()

    /**
     * What the C++'s message queue thread would have done: the follow-ups, one
     * at a time in the order they were posted, and then one pass of everything
     * the two queues and the zombies only do when they are asked — a task's
     * first-package timeout is one of those things, and a zombie that is started
     * again is another.
     *
     * The C++ runs this on a thread of its own; this port has none, so it is the
     * app's loop that calls it, and [dueTime] is how long it may wait.
     */
    public fun runPending()

    /**
     * How long the app's loop may wait before it calls [runPending] again: the
     * soonest of the two queues, the zombie check and the timing sync's alarm,
     * and `0` when a follow-up is already waiting — in milliseconds, which is
     * what a Kotlin caller on either side of the bridge can act on. A tick
     * would not be: it is measured from an origin only the process that made
     * the reading can compare against.
     *
     * @return that wait, or `null` when there is nothing to wait for — no task
     *         is out, no zombie is being checked, no alarm is armed — which is
     *         the `-1` the JNI bridge answers and the `MARS_STN_ERR_NO_DUE` the
     *         C ABI does.
     */
    public fun dueTime(): Long?

    /**
     * `MakesureLongLinkConnected` — a long link that is not connected is asked to
     * connect.
     *
     * Whether there was a default link to connect is what the C ABI answers (`1`
     * or `0`) and what the Swift package hands back, but not what the JNI bridge
     * does — its symbol is a `void` one — so this is what every platform
     * answers: nothing. A caller that wants to know reads [Question.linkStatus],
     * which is the [Question.Kind.LongLinkStatusChange] the port asks on the way.
     */
    public fun makesureLongLinkConnected()

    /**
     * `MakesureLonglinkConnected_ext` — the long link the app named is asked to
     * connect, if it is not connected already.
     *
     * A name no link was made with is nothing at all: the C++ looks its links
     * up in a map and does nothing when one is not there, so a caller that
     * mistypes a channel hears nothing about it.
     */
    public fun makesureLongLinkConnectedExt(name: String)

    /**
     * `LongLinkIsConnected` — whether the default long link is up.
     *
     * What "up" is is `LongLink::kConnected` and nothing else, so a link that
     * is still connecting answers `false`: this is the question an app asks
     * before it decides whether to start a task now or wait, and the one a
     * caller with no other way to see the link reads to know whether pushes
     * are arriving at all.
     */
    public fun longLinkIsConnected(): Boolean

    /**
     * `LongLinkIsConnected_ext` — whether the long link the app named is up,
     * `false` for a name no link was made with.
     */
    public fun longLinkIsConnectedExt(name: String): Boolean

    /**
     * `DisableLongLink` — no task goes out on a long link again.
     *
     * The C++'s is a one-way door: it is `NetCore`'s "need use longlink" set
     * `false`, and only [reset] — a net core made from nothing — opens it
     * again. There is no `enable`.
     */
    public fun disableLongLink()

    /**
     * `getNoopTaskID` — the task id of the noop, which is the one task no app
     * started.
     *
     * [Question.Kind.Req2Buf] and [Question.Kind.OnPush] are asked for it too,
     * so an app that cannot name it answers the noop as if it were one of its
     * own.
     */
    public fun noopTaskID(): Int

    /**
     * `SetSignallingStrategy` — for every keeper in the process. A period or a
     * keep time of `0` leaves the `SignallingKeeper` defaults alone.
     */
    public fun setSignallingStrategy(period: Long, keepTime: Long)

    /** `KeepSignalling` — a signalling keep-alive package, if one is needed. */
    public fun keepSignalling()

    /** `StopSignalling`. */
    public fun stopSignalling()

    /** `SetClientVersion` — the version every long-link package goes out with. */
    public fun setClientVersion(version: Int)

    /** `GenTaskID` — one counter for the whole process. */
    public fun genTaskID(): Int

    /** `GenSequenceId` — an `unsigned short`, like the C++'s. */
    public fun genSequenceId(): Int

    /** `TrigNooping` — a noop on the default long link, and a heartbeat of `0`. */
    public fun trigNooping()
}
