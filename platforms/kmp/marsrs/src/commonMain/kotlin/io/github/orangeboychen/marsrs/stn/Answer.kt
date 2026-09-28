package io.github.orangeboychen.marsrs.stn

/**
 * What the app answered, which [StnLogic.setApp]'s `ask` answers a [Question]
 * with.
 *
 * One answer per question, and an answer of another kind than the question
 * asked for is no answer: STN takes its own instead, which is what a host with
 * no app gets. [None] is that answer said out loud.
 *
 * The two enums an app answers *with* are the two whose integers are a handful
 * of named ones — [FailHandle] and [IdentifyMode] — and every other reading is
 * the integer or the bytes the C++ hands over, because those are ones an app
 * reads and hands back rather than ones it names.
 */
public sealed class Answer {
    /** Nobody answered: STN takes its own answer, which is what every question gets without an app. */
    public data object None : Answer()

    /** `makesureAuthed`, `identifyResponse`. */
    public data class Yes(public val yes: Boolean) : Answer()

    /** `onNewDns`, `netCheckShortLinkHosts` — the ips the app knows for a host. */
    public data class Addresses(public val addresses: List<String>) : Answer()

    /** `req2Buf` — what the task sends. */
    public data class Encoded(public val bytes: ByteArray) : Answer()

    /** `req2Buf` — the code the task ends with, which is the C++'s `false`. */
    public data class Failed(public val errorCode: Int) : Answer()

    /** `buf2Resp` — the code the task is remembered with, and what STN does with one that did not go well. */
    public data class Decoded(public val errorCode: Int, public val handle: FailHandle) : Answer()

    /** `onTaskEnd` — the code the task is remembered with. */
    public data class Ended(public val errorCode: Int) : Answer()

    /** `identifyCheckBuffer` — the check a new link is used with, and the hash its answer is judged against. */
    public data class Identified(
        public val mode: IdentifyMode,
        public val bytes: ByteArray,
        public val hash: ByteArray,
        public val cmdid: Int
    ) : Answer()

    /** `reportTaskLimited` — what the limit is; `0` is "go ahead". */
    public data class Limit(public val limit: Int) : Answer()
}

/**
 * `kTaskFailHandle*` — what STN does with a task whose answer was not a good
 * one, which is the `handle` of [Answer.Decoded].
 */
public enum class FailHandle(internal val value: Int) {
    /** The task failed the way a task fails. */
    Normal(0),

    /** Whatever the net core does with a failure it was not told about. */
    Default(-1),

    /** Every task that is out is run again. */
    RetryAllTasks(-12),

    /** The session is over. */
    SessionTimeout(-13),

    /** This task is over. */
    TaskEnd(-14),

    /** This task took too long. */
    TaskTimeout(-15),

    /** This task is over, and the app is not told. */
    SilentTaskEnd(-16)
}

/**
 * When the check a new long link is used with goes out: the `mode` of
 * [Answer.Identified].
 */
public enum class IdentifyMode(internal val value: Int) {
    /** With this very connect. */
    Now(0),

    /** With the next connect. */
    NextConnect(1),

    /** Never — which is any other integer, and stops STN asking. */
    Never(2)
}
