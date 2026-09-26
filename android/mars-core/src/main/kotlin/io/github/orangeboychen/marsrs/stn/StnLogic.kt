/*
 *  The Kotlin face of STN: every `external` is one
 *  `Java_io_github_orangeboychen_marsrs_stn_StnLogic_*` symbol of `mars-jni`,
 *  and the private members below are the ones it calls back — the port of
 *  `com_tencent_mars_stn_StnLogic_C2Java.cc`.
 *
 *  Three things the C++'s own Java class does not declare, because only the
 *  port has them: `genSequenceId`, `trigNooping` and `touchTasks`. Two calls
 *  the port makes carry one argument more than the C++'s — `req2Buf` the
 *  task's `clientSequenceId` and `buf2Resp` an `int[]` for it — and
 *  `resetAndInitEncoderVersion` takes the encoder's name beside its version.
 *
 *  The statics have to *be* statics: `mars-jni` asks JNI for
 *  `io/github/orangeboychen/marsrs/stn/StnLogic` and calls the method on the
 *  class, so every one is `@JvmStatic` — including the private ones JNI calls
 *  back, which is what the C++'s own Java declares too.
 */
// The constants below carry the name the C++ project's Java gives them, spelled
// the way Kotlin spells a constant: `K_PING_CHECK` there is `K_PING_CHECK` here.
// The JNI reaches a constant by the number it carries and not by its name, so
// nothing on the Rust side had to change with them.

package io.github.orangeboychen.marsrs.stn

import io.github.orangeboychen.marsrs.Mars
import java.io.ByteArrayOutputStream
import java.util.ArrayList

object StnLogic {

    const val TAG: String = "mars.StnLogic"

    init {
        Mars.loadDefaultMarsLibrary()
    }

    class Task {

        @JvmField
        var taskID: Int = 0 // unique task identify

        @JvmField
        var channelSelect: Int = 0 // short,long or both

        @JvmField
        var cmdID: Int = 0

        @JvmField
        var cgi: String? = null

        @JvmField
        var shortLinkHostList: ArrayList<String>? = null // host or ip

        @JvmField
        var sendOnly: Boolean = false

        @JvmField
        var needAuthed: Boolean = false

        @JvmField
        var limitFlow: Boolean = false

        @JvmField
        var limitFrequency: Boolean = false

        @JvmField
        var channelStrategy: Int = 0 // normal or fast

        @JvmField
        var networkStatusSensitive: Boolean = false

        @JvmField
        var priority: Int = 0 // @see priority

        @JvmField
        var retryCount: Int = -1

        @JvmField
        var serverProcessCost: Int = 0 // the longest this TASK waits for the SVR: the time it is expected to take

        @JvmField
        var totalTimeout: Int = 0 // total timeout, in ms

        @JvmField
        var userContext: Any? = null // user context

        @JvmField
        var reportArg: String? = null

        @JvmField
        var headers: Map<String, String>? = null

        @JvmField
        var longPolling: Boolean = false

        @JvmField
        var longPollingTimeout: Int = 0

        /** The port's own: what `req2Buf` is handed as its `sequence`. */
        @JvmField
        var clientSequenceId: Int = 0

        constructor() {
            taskID = genTaskID()
            headers = HashMap()
        }

        constructor(
            channelselect: Int,
            cmdid: Int,
            cgi: String?,
            shortLinkHostList: ArrayList<String>?
        ) {
            this.taskID = genTaskID()
            this.channelSelect = channelselect
            this.cmdID = cmdid
            this.cgi = cgi
            this.shortLinkHostList = shortLinkHostList

            this.sendOnly = false
            this.needAuthed = true
            this.limitFlow = true
            this.limitFrequency = true

            this.channelStrategy = ENORMAL
            this.networkStatusSensitive = false
            this.priority = ETASK_PRIORITY_NORMAL
            this.retryCount = -1
            this.serverProcessCost = 0
            this.totalTimeout = 0
            this.userContext = null
            this.headers = HashMap()
            this.longPolling = false
            this.longPollingTimeout = 0
        }

        companion object {
            const val ENORMAL: Int = 0
            const val EFAST: Int = 1

            // priority
            const val ETASK_PRIORITY_HIGHEST: Int = 0
            const val ETASK_PRIORITY_0: Int = 0
            const val ETASK_PRIORITY_1: Int = 1
            const val ETASK_PRIORITY_2: Int = 2
            const val ETASK_PRIORITY_3: Int = 3
            const val ETASK_PRIORITY_NORMAL: Int = 3
            const val ETASK_PRIORITY_4: Int = 4
            const val ETASK_PRIORITY_5: Int = 5
            const val ETASK_PRIORITY_LOWEST: Int = 5

            // channel selective
            const val E_SHORT: Int = 0x1
            const val E_LONG: Int = 0x2
            const val E_BOTH: Int = 0x3

            // protocol type
            const val E_TRANSPORT_PROTOCOL_TCP: Int = 1
            const val E_TRANSPORT_PROTOCOL_QUIC: Int = 2
        }
    }

    const val INVALID_TASK_ID: Int = -1

    // STN callback errType
    const val ECT_OK: Int = 0
    const val ECT_FALSE: Int = 1
    const val ECT_DIAL: Int = 2
    const val ECT_DNS: Int = 3
    const val ECT_SOCKET: Int = 4
    const val ECT_HTTP: Int = 5
    const val ECT_NET_MSG_XP: Int = 6
    const val ECT_EN_DECODE: Int = 7
    const val ECT_SERVER: Int = 8
    const val ECT_LOCAL: Int = 9

    // STN callback errCode
    const val FIRSTPKGTIMEOUT: Int = -500
    const val PKGPKGTIMEOUT: Int = -501
    const val READWRITETIMEOUT: Int = -502
    const val TASKTIMEOUT: Int = -503

    const val SOCKETNETWORKCHANGE: Int = -10086
    const val SOCKETMAKESOCKETPREPARED: Int = -10087
    const val SOCKETWRITENWITHNONBLOCK: Int = -10088
    const val SOCKETREADONCE: Int = -10089
    const val SOCKETSHUTDOWN: Int = -10090
    const val SOCKETRECVERR: Int = -10091
    const val SOCKETSENDERR: Int = -10092

    const val HTTPSPLITHTTPHEADANDBODY: Int = -10194
    const val HTTPPARSESTATUSLINE: Int = -10195

    const val NETMSGXPHANDLEBUFFERERR: Int = -10504

    const val DNSMAKESOCKETPREPARED: Int = -10606

    // reportConnectStatus — status
    const val NETWORK_UNKNOWN: Int = -1
    const val NETWORK_UNAVAILABLE: Int = 0
    const val GATEWAY_FAILED: Int = 1
    const val SERVER_FAILED: Int = 2
    const val CONNECTTING: Int = 3
    const val CONNECTED: Int = 4
    const val SERVER_DOWN: Int = 5

    // longlink identify check
    const val ECHECK_NOW: Int = 0

    const val ECHECK_NEXT: Int = 1

    const val ECHECK_NEVER: Int = 2

    // buf2Resp fail handle type
    const val RESP_FAIL_HANDLE_NORMAL: Int = 0

    const val RESP_FAIL_HANDLE_DEFAULT: Int = -1

    const val RESP_FAIL_HANDLE_SESSION_TIMEOUT: Int = -13

    const val RESP_FAIL_HANDLE_TASK_END: Int = -14

    const val TASK_END_SUCCESS: Int = 0

    /** What `onTaskEnd` is handed — the port fills the nine fields it has. */
    class CgiProfile {
        @JvmField
        var taskStartTime: Long = 0

        @JvmField
        var startConnectTime: Long = 0

        @JvmField
        var connectSuccessfulTime: Long = 0

        @JvmField
        var startHandshakeTime: Long = 0

        @JvmField
        var handshakeSuccessfulTime: Long = 0

        @JvmField
        var startSendPacketTime: Long = 0

        @JvmField
        var startReadPacketTime: Long = 0

        @JvmField
        var readPacketFinishedTime: Long = 0

        @JvmField
        var rtt: Long = 0

        @JvmField
        var channelType: Int = 0

        @JvmField
        var protocolType: Int = 0 // the protocol type
    }

    /**
     * Created by caoshaokun on 16/2/1.
     *
     * An app that uses the signalling channel has to implement this interface — the port asks the
     * app the fifteen questions below.
     */
    interface ICallBack {
        /**
         * The SDK asks the app to authenticate, which may start an AUTH CGI of its own
         */
        fun makesureAuthed(host: String?): Boolean

        /**
         * The SDK asks the app to resolve a host name: the app may answer with ordinary DNS, or
         * with a host-to-IP mapping of its own
         */
        fun onNewDns(host: String?): Array<String>?

        /**
         * A message the SVR pushed down has come in
         */
        fun onPush(cmdid: Int, taskid: Int, data: ByteArray?)

        /**
         * The SDK asks the app to package a TASK
         */
        fun req2Buf(
            taskID: Int,
            userContext: Any?,
            reqBuffer: ByteArrayOutputStream?,
            errCode: IntArray?,
            channelSelect: Int,
            host: String?,
            sequence: Int
        ): Boolean

        /**
         * The SDK asks the app to unpack a TASK
         */
        fun buf2Resp(
            taskID: Int,
            userContext: Any?,
            respBuffer: ByteArray?,
            errCode: IntArray?,
            channelSelect: Int,
            sequence: IntArray?
        ): Int

        /**
         * The callback of a task that ended
         */
        fun onTaskEnd(taskID: Int, userContext: Any?, errType: Int, errCode: Int, profile: CgiProfile?): Int

        /**
         * The traffic statistics
         */
        fun trafficData(send: Int, recv: Int)

        /**
         * A notice of the connection state
         * @param status the state of both channels together, the long link and the short link
         * @param longlinkstatus the state of the long link alone
         */
        fun reportConnectInfo(status: Int, longlinkstatus: Int)

        /**
         * The SDK asks the app for the buffer the long link is checked with, which goes out once
         * the long link is up and is what proves who the SVR is
         * @return ECHECK_NOW (to check), ECHECK_NEVER (not to check), ECHECK_NEXT (to be asked
         *     again the next time)
         */
        fun getLongLinkIdentifyCheckBuffer(
            identifyReqBuf: ByteArrayOutputStream?,
            hashCodeBuffer: ByteArrayOutputStream?,
            reqRespCmdID: IntArray?
        ): Int

        /**
         * The SDK asks the app to parse the answer to the connection check
         */
        fun onLongLinkIdentifyResp(buffer: ByteArray?, hashCodeBuffer: ByteArray?): Boolean

        /** Asks for a sync */
        fun requestDoSync()

        fun requestNetCheckShortLinkHosts(): Array<String>?

        /** Whether the user is logged in */
        fun isLogoned(): Boolean

        fun reportTaskProfile(taskString: String?)
    }

    private var callBack: ICallBack? = null

    /** Sets the instance the network layer calls back on — the app implements NetworkCallBack */
    @JvmStatic
    fun setCallBack(callback: ICallBack?) {
        callBack = callback
    }

    /*
     * About the DEBUG IP
     * setLonglinkSvrAddr, setShortlinkSvrAddr and setDebugIP all set a DEBUG IP:
     * setLonglinkSvrAddr: sets the DEBUG IP of the long link;
     * setShortlinkSvrAddr: sets the DEBUG IP of the short link;
     * setDebugIP: sets the DEBUG IP of a HOST, long link or short link alike;
     *
     * Precedence:
     * setDebugIP has the highest precedence
     * of one and the same setter, the value set last is the one that counts
     */

    /**
     * @param host      the host name of the long link
     * @param ports     the ports of the long link
     * @param debugIP   the DEBUG IP of the long link: if it is set, `host` is ignored and this IP
     *                  is the one used
     */
    @JvmStatic
    external fun setLonglinkSvrAddr(host: String?, ports: IntArray?, debugIP: String?)

    @JvmStatic
    fun setLonglinkSvrAddr(host: String?, ports: IntArray?) {
        setLonglinkSvrAddr(host, ports, null)
    }

    /**
     * @param port      the port of the short link (HTTP)
     * @param debugIP   the DEBUG IP of the short link: if it is set, a TASK that goes over the
     *                  short link uses this IP in place of the HOST the TASK names
     */
    @JvmStatic
    external fun setShortlinkSvrAddr(port: Int, debugIP: String?)

    @JvmStatic
    fun setShortlinkSvrAddr(port: Int) {
        setShortlinkSvrAddr(port, null)
    }

    /**
     * Sets a DEBUG IP
     * @param host  the host name to set it for
     * @param ip    the IP of that host name
     */
    @JvmStatic
    external fun setDebugIP(host: String?, ip: String?)

    // async call
    @JvmStatic
    external fun startTask(task: Task?)

    // sync call
    @JvmStatic
    external fun stopTask(taskID: Int)

    // sync call
    @JvmStatic
    external fun hasTask(taskID: Int): Boolean

    /** Redoes every long-link and short-link task. Note that this one reconnects the long link. */
    @JvmStatic
    external fun redoTask()

    /** Stops and clears every task that has not finished. */
    @JvmStatic
    external fun clearTask()

    /** Reorders the task queue — the `touchTasks` of the C++, which the port has and its Java does not declare. */
    @JvmStatic
    external fun touchTasks()

    /** Stops and clears every task that has not finished, and initializes everything again. */
    @JvmStatic
    external fun reset()

    /** Stops and clears every task that has not finished, re-initializes and sets the encoder version anew. */
    @JvmStatic
    external fun resetAndInitEncoderVersion(packerEncoderVersion: Int, packerEncoderName: String?)

    /**
     * Sets the backup IPs, for when neither the long nor the short svr answers
     * @param host  the host name
     * @param ips   the IPs of that host name
     */
    @JvmStatic
    external fun setBackupIPs(host: String?, ips: Array<String>?)

    /** Checks the state of the long link: if it is not connected, a reconnect is attempted. */
    @JvmStatic
    external fun makesureLongLinkConnected()

    // signalling

    /**
     * Keeps the signalling alive
     * @param period how often it is kept alive, 5s by default
     * @param keepTime how long it is kept alive, 20s by default
     */
    @JvmStatic
    external fun setSignallingStrategy(period: Long, keepTime: Long)

    /** Sends a signalling keep-alive package, if one is needed */
    @JvmStatic
    external fun keepSignalling()

    /** Stops keeping the signalling alive */
    @JvmStatic
    external fun stopSignalling()

    /** Sets the client version, which goes into the header of the long link's private protocol */
    @JvmStatic
    external fun setClientVersion(clientVersion: Int)

    /** Gets the modules the native side has loaded */
    @JvmStatic
    private external fun getLoadLibraries(): ArrayList<String>?

    @JvmStatic
    external fun genTaskID(): Int

    /** A random seq, the same as the C++'s `unsigned short` — the port has it, its Java does not. */
    @JvmStatic
    external fun genSequenceId(): Int

    /** Triggers one noop — which the port declares and the C++'s Java class does not. */
    @JvmStatic
    external fun trigNooping()

    /**
     * Asks the app to authenticate. If a TASK asks for the AUTH state and there is none right now,
     * this is the method the component calls back.
     */
    @JvmStatic
    private fun makesureAuthed(host: String?): Boolean {
        return try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return false
            }
            imp.makesureAuthed(host)
        } catch (e: Exception) {
            e.printStackTrace()
            false
        }
    }

    /**
     * The host the long link is set up with, and the host a short-link task names: the network
     * layer asks the app for what DNS makes of a host.
     * @param host  the host name
     * @return empty: the layer below resolves it itself
     */
    @JvmStatic
    private fun onNewDns(host: String?): Array<String>? {
        return try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return null
            }
            imp.onNewDns(host)
        } catch (e: Exception) {
            e.printStackTrace()
            null
        }
    }

    /**
     * A message the server pushed has come in
     * @param cmdid     the CMDID of the PUSH, which is what the app and the SVR agreed on
     * @param data      the data that was pushed down
     */
    @JvmStatic
    private fun onPush(channelID: String?, cmdid: Int, taskid: Int, data: ByteArray?) {
        try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return
            }
            imp.onPush(cmdid, taskid, data)
        } catch (e: Exception) {
            e.printStackTrace()
        }
    }

    /**
     * The network layer takes the body the app sends
     */
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
        return try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return false
            }
            imp.req2Buf(taskID, userContext, reqBuffer, errCode, channelSelect, host, sequence)
        } catch (e: Exception) {
            e.printStackTrace()
            false
        }
    }

    /**
     * The network layer hands the answer it received to the app to parse
     */
    @JvmStatic
    private fun buf2Resp(
        taskID: Int,
        userContext: Any?,
        respBuffer: ByteArray?,
        errCode: IntArray?,
        channelSelect: Int,
        sequence: IntArray?
    ): Int {
        return try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return RESP_FAIL_HANDLE_TASK_END
            }
            imp.buf2Resp(taskID, userContext, respBuffer, errCode, channelSelect, sequence)
        } catch (e: Exception) {
            e.printStackTrace()
            RESP_FAIL_HANDLE_TASK_END
        }
    }

    /**
     * The network layer is done with the answer and calls the app back
     */
    @JvmStatic
    private fun onTaskEnd(taskID: Int, userContext: Any?, errType: Int, errCode: Int, profile: CgiProfile?): Int {
        return try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return 0
            }
            imp.onTaskEnd(taskID, userContext, errType, errCode, profile)
        } catch (e: Exception) {
            e.printStackTrace()
            0
        }
    }

    /** Reports the traffic the signalling used */
    @JvmStatic
    private fun trafficData(send: Int, recv: Int) {
        try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return
            }
            imp.trafficData(send, recv)
        } catch (e: Exception) {
            e.printStackTrace()
        }
    }

    /**
     * The network layer tells the app what state the network connection is in
     */
    @JvmStatic
    private fun reportConnectStatus(status: Int, longlinkstatus: Int) {
        try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return
            }
            imp.reportConnectInfo(status, longlinkstatus)
        } catch (e: Exception) {
            e.printStackTrace()
        }
    }

    /**
     * The long link's signalling check
     * @return ECHECK_NOW = 0, ECHECK_NEXT = 1, ECHECK_NEVER = 2
     */
    @JvmStatic
    private fun getLongLinkIdentifyCheckBuffer(
        channelID: String?,
        reqBuf: ByteArrayOutputStream?,
        reqBufHash: ByteArrayOutputStream?,
        cmdID: IntArray?
    ): Int {
        return try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return ECHECK_NEVER
            }
            imp.getLongLinkIdentifyCheckBuffer(reqBuf, reqBufHash, cmdID)
        } catch (e: Exception) {
            e.printStackTrace()
            ECHECK_NEVER
        }
    }

    /**
     * The answer to the long link's signalling check
     */
    @JvmStatic
    private fun onLongLinkIdentifyResp(channelID: String?, respBuf: ByteArray?, reqBufHash: ByteArray?): Boolean {
        return try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return false
            }
            imp.onLongLinkIdentifyResp(respBuf, reqBufHash)
        } catch (e: Exception) {
            e.printStackTrace()
            false
        }
    }

    @JvmStatic
    private fun requestNetCheckShortLinkHosts(): Array<String>? {
        return try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return null
            }
            imp.requestNetCheckShortLinkHosts()
        } catch (e: Exception) {
            e.printStackTrace()
            null
        }
    }

    @JvmStatic
    fun requestDoSync() {
        try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return
            }
            imp.requestDoSync()
        } catch (e: Exception) {
            e.printStackTrace()
        }
    }

    @JvmStatic
    fun isLogoned(): Boolean {
        return try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return false
            }
            imp.isLogoned()
        } catch (e: Exception) {
            e.printStackTrace()
            false
        }
    }

    /**
     * When a Task is done, STN hands the app how it ran and what it counted
     */
    @JvmStatic
    private fun reportTaskProfile(taskString: String?) {
        try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return
            }
            imp.reportTaskProfile(taskString)
        } catch (e: Exception) {
            e.printStackTrace()
        }
    }
}
