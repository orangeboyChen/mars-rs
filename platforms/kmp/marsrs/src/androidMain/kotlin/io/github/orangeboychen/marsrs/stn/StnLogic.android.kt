package io.github.orangeboychen.marsrs.stn

import java.io.ByteArrayOutputStream

/**
 * The Android `actual`: `libmarsrsxlog.so`, the JNI bridge of `crates/marsrs-jni`.
 *
 * The object is this class in this package because that is what the symbols the
 * Rust exports are called — `Java_io_github_orangeboychen_marsrs_stn_StnLogic_reset`
 * and its siblings — and so is the [CgiProfile] below, whose name `marsrs-jni`
 * puts in the descriptor of the `onTaskEnd` it calls. Every member is `@JvmStatic`
 * for the same reason: the bridge asks JNI for
 * `io/github/orangeboychen/marsrs/stn/StnLogic` and calls the method on the
 * class, which is what the Android AAR declares too.
 *
 * The thirteen private statics at the end are the other direction: the questions
 * STN asks while it runs a task, which the bridge calls on this class the way
 * the C++'s `StnLogic_C2Java.cc` calls them on its Java. Each one is a [Question]
 * the app's `ask` is handed and an [Answer] it answers with, which is the seam
 * the two `actual`s share — on Kotlin/Native the same questions come out of one
 * function pointer of the C ABI. Five of the eighteen never reach the app here,
 * because the bridge answers them itself; see [Question.Kind].
 *
 * The two `actual`s differ in one more place: [CgiProfile] carries the ten ticks
 * and the rtt, and the two integers `marsrs-jni` fills in, and not the two
 * readings only the C ABI has, so a profile an app reads on Android answers `0`
 * and `""` for them.
 */
public actual object StnLogic {
    init {
        // `marsrsxlog`, the `crate-name` of `marsrs-jni`: loaded before the first
        // symbol of it is called, and loading it twice is nothing.
        System.loadLibrary(LIBRARY)
    }

    /** The app STN asks, which is the one [setApp] was handed. */
    @Volatile
    private var app: ((Question) -> Answer)? = null

    public actual fun setApp(ask: ((Question) -> Answer)?) {
        app = ask
    }

    // The symbols `marsrs-jni` exports.
    //
    // `actual external`, and not an `actual` that forwards: the name a method
    // carries in the class file *is* the symbol JNI resolves, so a forward would
    // have to be the one carrying it. `private` is what the ones below are,
    // because Kotlin mangles the name of an `internal` function.
    //
    // [dueTime] is the one the two cannot agree on by themselves: the bridge
    // answers a `long` and the common API a `Long?`, so the symbol is the private
    // one, which `@JvmName` leaves the name the Rust exports.

    @JvmStatic
    public actual external fun reset()

    @JvmStatic
    public actual external fun resetAndInitEncoderVersion(version: Int, name: String?)

    @JvmStatic
    public actual external fun setLonglinkSvrAddr(host: String?, ports: IntArray?, debugIP: String?)

    @JvmStatic
    public actual external fun setShortlinkSvrAddr(port: Int, debugIP: String?)

    @JvmStatic
    public actual external fun setDebugIP(host: String?, ip: String?)

    @JvmStatic
    public actual external fun setBackupIPs(host: String?, ips: Array<String>?)

    @JvmStatic
    public actual external fun startTask(task: Task)

    @JvmStatic
    public actual external fun stopTask(taskID: Int)

    @JvmStatic
    public actual external fun hasTask(taskID: Int): Boolean

    @JvmStatic
    public actual external fun redoTask()

    @JvmStatic
    public actual external fun touchTasks()

    @JvmStatic
    public actual external fun clearTask()

    @JvmStatic
    public actual external fun runPending()

    /**
     * What [StnLogic.dueTime] says, which is where the words are.
     *
     * `@JvmName`, because the class has to carry exactly one method named
     * `dueTime`: JNI resolves the symbol by that name, and a second one — this
     * — would make the runtime look for the mangled long name
     * `Java_..._StnLogic_dueTime__` instead, which `marsrs-jni` does not export.
     */
    @JvmName("dueTimeOrNull")
    public actual fun dueTime(): Long? = dueTimeNative().takeIf { it >= 0 }

    @JvmStatic
    @JvmName("dueTime")
    private external fun dueTimeNative(): Long

    @JvmStatic
    public actual external fun makesureLongLinkConnected()

    @JvmStatic
    public actual external fun makesureLongLinkConnectedExt(name: String)

    @JvmStatic
    public actual external fun longLinkIsConnected(): Boolean

    @JvmStatic
    public actual external fun longLinkIsConnectedExt(name: String): Boolean

    @JvmStatic
    public actual external fun disableLongLink()

    @JvmStatic
    public actual external fun noopTaskID(): Int

    @JvmStatic
    public actual external fun createLonglink(config: LonglinkConfig): Boolean

    @JvmStatic
    public actual external fun destroyLonglink(name: String?): Boolean

    @JvmStatic
    public actual external fun markMainLonglink(name: String?): Boolean

    @JvmStatic
    public actual external fun setSignallingStrategy(period: Long, keepTime: Long)

    @JvmStatic
    public actual external fun keepSignalling()

    @JvmStatic
    public actual external fun stopSignalling()

    @JvmStatic
    public actual external fun setClientVersion(version: Int)

    @JvmStatic
    public actual external fun genTaskID(): Int

    @JvmStatic
    public actual external fun genSequenceId(): Int

    @JvmStatic
    public actual external fun trigNooping()

    // The thirteen the bridge calls back: one [Question] each, and the [Answer]
    // the app answered read for the kind it asked. An app that answered nothing —
    // or that was never handed over — gets the answer STN takes when the app said
    // nothing, which is the one `marsrs-stn`'s `App` answers and the one the C
    // ABI gives a host with no app: the same answers on both `actual`s, because a
    // shared module that answers a task differently on Android than it does on
    // iOS is a module with two behaviours to reason about.

    @JvmStatic
    private fun makesureAuthed(host: String?): Boolean =
        when (val answer = ask(Question.Kind.MakesureAuthed) { this.host = host.orEmpty() }) {
            is Answer.Yes -> answer.yes
            // An app that did not answer is logged in, which is what the port
            // answers on every other platform.
            else -> true
        }

    @JvmStatic
    private fun trafficData(send: Long, recv: Long) {
        ask(Question.Kind.TrafficData) {
            this.sent = send
            this.received = recv
        }
    }

    @JvmStatic
    private fun onNewDns(host: String?, isLongLinkHost: Boolean): Array<String>? {
        val answer = ask(Question.Kind.OnNewDns) {
            this.host = host.orEmpty()
            // What the bridge was asked and Kotlin/Native is told: a shared
            // module that branches on this reads one thing on either target.
            this.isLongLinkHost = isLongLinkHost
        }
        return when (answer) {
            is Answer.Addresses -> answer.addresses.toTypedArray()
            else -> null
        }
    }

    @JvmStatic
    private fun onPush(channelID: String?, cmdid: Int, taskid: Int, data: ByteArray?) {
        ask(Question.Kind.OnPush) {
            this.channelID = channelID.orEmpty()
            this.cmdid = cmdid
            this.taskID = taskid
            this.body = data ?: ByteArray(0)
        }
    }

    @JvmStatic
    private fun req2Buf(
        taskID: Int,
        userContext: Any?,
        reqBuffer: ByteArrayOutputStream?,
        errCode: IntArray?,
        channelSelect: Int,
        host: String?,
        sequence: Int
    ): Boolean {
        val answer = ask(Question.Kind.Req2Buf) {
            this.taskID = taskID
            this.channelSelect = channelSelect
            this.sequence = sequence
            this.host = host.orEmpty()
            this.userID = userContext as? String ?: ""
        }
        return when (answer) {
            // What the task sends, which is what the bridge reads out of the
            // stream when this answers `true`.
            is Answer.Encoded -> {
                reqBuffer?.write(answer.bytes)
                true
            }

            // The `false` of the C++'s `req2Buf`, which is a task that ends with
            // the code the app put in the array the bridge reads after it.
            is Answer.Failed -> {
                if (errCode != null && errCode.isNotEmpty()) {
                    errCode[0] = answer.errorCode
                }
                false
            }

            else -> false
        }
    }

    @JvmStatic
    private fun buf2Resp(
        taskID: Int,
        userContext: Any?,
        respBuffer: ByteArray?,
        errCode: IntArray?,
        channelSelect: Int,
        sequence: IntArray?
    ): Int {
        val answer = ask(Question.Kind.Buf2Resp) {
            this.taskID = taskID
            this.channelSelect = channelSelect
            this.body = respBuffer ?: ByteArray(0)
            this.userID = userContext as? String ?: ""
        }
        return when (answer) {
            is Answer.Decoded -> {
                if (errCode != null && errCode.isNotEmpty()) {
                    errCode[0] = answer.errorCode
                }
                answer.handle.value
            }

            // An answer nobody read is a good one, which is what the port
            // answers on every other platform: `Err` is what a task that
            // could not be decoded ends with, and an app that said nothing
            // is not an app that read the answer and refused it.
            else -> FailHandle.Normal.value
        }
    }

    @JvmStatic
    private fun onTaskEnd(taskID: Int, userContext: Any?, errType: Int, errCode: Int, profile: CgiProfile?): Int {
        val answer = ask(Question.Kind.OnTaskEnd) {
            this.taskID = taskID
            this.errType = errType
            this.errCode = errCode
            this.profile = profile?.toCommon()
            this.userID = userContext as? String ?: ""
        }
        return when (answer) {
            is Answer.Ended -> answer.errorCode
            else -> 0
        }
    }

    @JvmStatic
    private fun reportConnectStatus(status: Int, longlinkstatus: Int) {
        ask(Question.Kind.ReportConnectStatus) {
            this.netStatusAll = status
            this.netStatusLongLink = longlinkstatus
        }
    }

    @JvmStatic
    private fun getLongLinkIdentifyCheckBuffer(
        channelID: String?,
        reqBuf: ByteArrayOutputStream?,
        reqBufHash: ByteArrayOutputStream?,
        cmdID: IntArray?
    ): Int {
        val answer = ask(Question.Kind.IdentifyCheckBuffer) { this.channelID = channelID.orEmpty() }
        return when (answer) {
            is Answer.Identified -> {
                reqBuf?.write(answer.bytes)
                reqBufHash?.write(answer.hash)
                if (cmdID != null && cmdID.isNotEmpty()) {
                    cmdID[0] = answer.cmdid
                }
                answer.mode.value
            }

            // Asked again on the next connect, which is what the port answers
            // on every other platform. `Never` would mark this connection
            // checked and stop STN asking for good, which is an answer an app
            // that said nothing did not give.
            else -> IdentifyMode.NextConnect.value
        }
    }

    @JvmStatic
    private fun onLongLinkIdentifyResp(channelID: String?, respBuf: ByteArray?, reqBufHash: ByteArray?): Boolean =
        when (
            val answer = ask(Question.Kind.IdentifyResponse) {
                this.channelID = channelID.orEmpty()
                this.body = respBuf ?: ByteArray(0)
                this.hash = reqBufHash ?: ByteArray(0)
            }
        ) {
            is Answer.Yes -> answer.yes
            else -> false
        }

    @JvmStatic
    private fun requestDoSync() {
        ask(Question.Kind.RequestSync)
    }

    @JvmStatic
    private fun requestNetCheckShortLinkHosts(): Array<String>? =
        when (val answer = ask(Question.Kind.NetCheckShortLinkHosts)) {
            is Answer.Addresses -> answer.addresses.toTypedArray()
            else -> null
        }

    @JvmStatic
    private fun reportTaskProfile(taskString: String?) {
        ask(Question.Kind.ReportTaskProfile) { this.profileJSON = taskString }
    }

    /** What `marsrs-jni` hands [onTaskEnd]: the ten ticks and the rtt, and the two integers its `cgi_profile` fills in. */
    internal class CgiProfile {
        @JvmField
        var taskStartTime: Long = 0

        @JvmField
        var startConnectTime: Long = 0

        @JvmField
        var connectSuccessfulTime: Long = 0

        @JvmField
        var startSendPacketTime: Long = 0

        @JvmField
        var startReadPacketTime: Long = 0

        @JvmField
        var readPacketFinishedTime: Long = 0

        @JvmField
        var startEncodePacketTime: Long = 0

        @JvmField
        var encodePacketFinishedTime: Long = 0

        @JvmField
        var startDecodePacketTime: Long = 0

        @JvmField
        var decodePacketFinishedTime: Long = 0

        @JvmField
        var rtt: Long = 0

        @JvmField
        var channelType: Int = 0

        @JvmField
        var protocolType: Int = 0
    }

    /**
     * One question, asked of the app: [Answer.None] when there is no app, and
     * when the app's own `ask` threw, which is the C++'s `printStackTrace` and
     * not a question STN goes without.
     */
    private fun ask(kind: Question.Kind, fill: Question.() -> Unit = {}): Answer {
        val question = Question().apply {
            this.kind = kind
            fill()
        }
        return try {
            app?.invoke(question) ?: Answer.None
        } catch (e: Exception) {
            e.printStackTrace()
            Answer.None
        }
    }

    /** The app's profile: the two readings only the C ABI has are `0` and empty here. */
    private fun CgiProfile.toCommon(): io.github.orangeboychen.marsrs.stn.CgiProfile =
        io.github.orangeboychen.marsrs.stn.CgiProfile(
            taskStartTime = taskStartTime,
            startConnectTime = startConnectTime,
            connectSuccessfulTime = connectSuccessfulTime,
            startSendPacketTime = startSendPacketTime,
            sendPacketFinishedTime = 0,
            startReadPacketTime = startReadPacketTime,
            readPacketFinishedTime = readPacketFinishedTime,
            startEncodePacketTime = startEncodePacketTime,
            encodePacketFinishedTime = encodePacketFinishedTime,
            startDecodePacketTime = startDecodePacketTime,
            decodePacketFinishedTime = decodePacketFinishedTime,
            rtt = rtt,
            channelType = channelType,
            protocolType = protocolType,
            netType = ""
        )

    /** `marsrsxlog` — the `crate-name` of `marsrs-jni`, i.e. the library every symbol above lives in. */
    private const val LIBRARY = "marsrsxlog"
}
