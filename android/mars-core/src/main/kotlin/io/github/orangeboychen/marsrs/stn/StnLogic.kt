/**
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
        var taskID: Int = 0      // unique task identify

        @JvmField
        var channelSelect: Int = 0   // short,long or both

        @JvmField
        var cmdID: Int = 0

        @JvmField
        var cgi: String? = null

        @JvmField
        var shortLinkHostList: ArrayList<String>? = null    // host or ip

        @JvmField
        var sendOnly: Boolean = false

        @JvmField
        var needAuthed: Boolean = false

        @JvmField
        var limitFlow: Boolean = false

        @JvmField
        var limitFrequency: Boolean = false

        @JvmField
        var channelStrategy: Int = 0     // normal or fast

        @JvmField
        var networkStatusSensitive: Boolean = false

        @JvmField
        var priority: Int = 0    // @see priority

        @JvmField
        var retryCount: Int = -1

        @JvmField
        var serverProcessCost: Int = 0   // 该TASK等待SVR处理的最长时间,也即预计的SVR处理耗时

        @JvmField
        var totalTimeout: Int = 0        // total timeout, in ms

        @JvmField
        var userContext: Any? = null     // user context

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
            const val EShort: Int = 0x1
            const val ELong: Int = 0x2
            const val EBoth: Int = 0x3

            // protocol type
            const val ETransportProtocolTCP: Int = 1
            const val ETransportProtocolQUIC: Int = 2
        }
    }

    const val INVALID_TASK_ID: Int = -1

    // STN callback errType
    const val ectOK: Int = 0
    const val ectFalse: Int = 1
    const val ectDial: Int = 2
    const val ectDns: Int = 3
    const val ectSocket: Int = 4
    const val ectHttp: Int = 5
    const val ectNetMsgXP: Int = 6
    const val ectEnDecode: Int = 7
    const val ectServer: Int = 8
    const val ectLocal: Int = 9

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
    @JvmField
    var ECHECK_NOW: Int = 0

    @JvmField
    var ECHECK_NEXT: Int = 1

    @JvmField
    var ECHECK_NEVER: Int = 2

    // buf2Resp fail handle type
    @JvmField
    var RESP_FAIL_HANDLE_NORMAL: Int = 0

    @JvmField
    var RESP_FAIL_HANDLE_DEFAULT: Int = -1

    @JvmField
    var RESP_FAIL_HANDLE_SESSION_TIMEOUT: Int = -13

    @JvmField
    var RESP_FAIL_HANDLE_TASK_END: Int = -14

    @JvmField
    var TASK_END_SUCCESS: Int = 0

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
        var protocolType: Int = 0    // 协议类型
    }

    /**
     * Created by caoshaokun on 16/2/1.
     *
     * APP使用信令通道必须实现该接口 — the port asks the app the fifteen
     * questions below.
     */
    interface ICallBack {
        /**
         * SDK要求上层做认证操作(可能新发起一个AUTH CGI)
         */
        fun makesureAuthed(host: String?): Boolean

        /**
         * SDK要求上层做域名解析.上层可以实现传统DNS解析,或者自己实现的域名/IP映射
         */
        fun onNewDns(host: String?): Array<String>?

        /**
         * 收到SVR PUSH下来的消息
         */
        fun onPush(cmdid: Int, taskid: Int, data: ByteArray?)

        /**
         * SDK要求上层对TASK组包
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
         * SDK要求上层对TASK解包
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
         * 任务结束回调
         */
        fun onTaskEnd(taskID: Int, userContext: Any?, errType: Int, errCode: Int, profile: CgiProfile?): Int

        /**
         * 流量统计
         */
        fun trafficData(send: Int, recv: Int)

        /**
         * 连接状态通知
         * @param status 综合状态，即长连+短连的状态
         * @param longlinkstatus 仅长连的状态
         */
        fun reportConnectInfo(status: Int, longlinkstatus: Int)

        /**
         * SDK要求上层生成长链接数据校验包,在长链接连接上之后使用,用于验证SVR身份
         * @return ECHECK_NOW(需要校验), ECHECK_NEVER(不校验), ECHECK_NEXT(下一次再询问)
         */
        fun getLongLinkIdentifyCheckBuffer(
            identifyReqBuf: ByteArrayOutputStream?,
            hashCodeBuffer: ByteArrayOutputStream?,
            reqRespCmdID: IntArray?
        ): Int

        /**
         * SDK要求上层解连接校验回包.
         */
        fun onLongLinkIdentifyResp(buffer: ByteArray?, hashCodeBuffer: ByteArray?): Boolean

        /** 请求做sync */
        fun requestDoSync()

        fun requestNetCheckShortLinkHosts(): Array<String>?

        /** 是否登录 */
        fun isLogoned(): Boolean

        fun reportTaskProfile(taskString: String?)
    }

    private var callBack: ICallBack? = null

    /** 初始化网络层回调实例 App实现NetworkCallBack接口 */
    @JvmStatic
    fun setCallBack(callback: ICallBack?) {
        callBack = callback
    }

    /**
     * DEBUG IP 说明
     * setLonglinkSvrAddr,setShortlinkSvrAddr,setDebugIP 均可用于设置DEBUG IP
     * setLonglinkSvrAddr: 设置长链接的DEBUG IP;
     * setShortlinkSvrAddr: 设置短连接的DEBUG IP;
     * setDebugIP: 设置对应HOST(不区分长短链)的DEBUG IP;
     *
     * 优先级:
     * setDebugIP 为最高优先级
     * 同一个接口, 以最后设置的值为准
     */

    /**
     * @param host      长链接域名
     * @param ports     长链接端口列表
     * @param debugIP   长链接调试IP.如果有值,则忽略 host设置, 并使用该IP.
     */
    @JvmStatic
    external fun setLonglinkSvrAddr(host: String?, ports: IntArray?, debugIP: String?)

    @JvmStatic
    fun setLonglinkSvrAddr(host: String?, ports: IntArray?) {
        setLonglinkSvrAddr(host, ports, null)
    }

    /**
     * @param port      短链接(HTTP)端口
     * @param debugIP   短链接调试IP.如果有值,则所有TASK走短链接时,使用该IP代替TASK中的HOST
     */
    @JvmStatic
    external fun setShortlinkSvrAddr(port: Int, debugIP: String?)

    @JvmStatic
    fun setShortlinkSvrAddr(port: Int) {
        setShortlinkSvrAddr(port, null)
    }

    /**
     * 设置DEBUG IP
     * @param host  要设置的域名
     * @param ip    该域名对应的IP
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

    /** 重做所有长短连任务. 注意这个接口会重连长链接. */
    @JvmStatic
    external fun redoTask()

    /** 停止并清除所有未完成任务. */
    @JvmStatic
    external fun clearTask()

    /** 重新排序任务队列 — 与 C++ 的 `touchTasks` 相同，端口有而 C++ 的 Java 类没有声明。 */
    @JvmStatic
    external fun touchTasks()

    /** 停止并清除所有未完成任务并重新初始化 */
    @JvmStatic
    external fun reset()

    /** 停止并清除所有未完成任务并重新初始化, 重新设置encoder version */
    @JvmStatic
    external fun resetAndInitEncoderVersion(packerEncoderVersion: Int, packerEncoderName: String?)

    /**
     * 设置备份IP,用于long/short svr均不可用的场景下
     * @param host  域名
     * @param ips   域名对应的IP列表
     */
    @JvmStatic
    external fun setBackupIPs(host: String?, ips: Array<String>?)

    /** 检测长链接状态.如果没有连接上,则会尝试重连. */
    @JvmStatic
    external fun makesureLongLinkConnected()

    // signalling

    /**
     * 信令保活
     * @param period 信令保活间隔,默认5S
     * @param keepTime 信令保活时间,默认20S
     */
    @JvmStatic
    external fun setSignallingStrategy(period: Long, keepTime: Long)

    /** 发送一个信令保活包(如果有必要) */
    @JvmStatic
    external fun keepSignalling()

    /** 停止信令保活 */
    @JvmStatic
    external fun stopSignalling()

    /** 设置客户端版本 放入长连私有协议头部 */
    @JvmStatic
    external fun setClientVersion(clientVersion: Int)

    /** 获取底层已加载模块 */
    @JvmStatic
    private external fun getLoadLibraries(): ArrayList<String>?

    @JvmStatic
    external fun genTaskID(): Int

    /** 一个随机的 seq, 与 C++ 的 `unsigned short` 相同 — 端口有而 C++ 的 Java 类没有声明。 */
    @JvmStatic
    external fun genSequenceId(): Int

    /** 触发一次 noop — 端口有而 C++ 的 Java 类没有声明。 */
    @JvmStatic
    external fun trigNooping()

    /**
     * 要求上层进行AUTH操作.
     * 如果一个TASK要求AUTH状态而当前没有AUTH态,组件就会回调此方法
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
     * 长连host设置到网络层 网络层向上层请求host dns结果
     * 短连task中设置host  网络层向上层请求host dns结果
     * @param host  域名
     * @return 空：底层实现解析
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
     * 收到server push消息
     * @param cmdid     PUSH的CMDID,这个应该是APP跟SVR约定的值
     * @param data      PUSH下来的数据
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
     * 网络层获取上层发送的数据内容
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
     * 网络层将收到的信令回包交给上层解析
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
     * 信令回包网络层处理完毕回调上层
     */
    @JvmStatic
    private fun onTaskEnd(
        taskID: Int,
        userContext: Any?,
        errType: Int,
        errCode: Int,
        profile: CgiProfile?
    ): Int {
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

    /** 上报信令消耗的流量 */
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
     * 网络层向上层反馈网络连接状态
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
     * 长连信令校验
     * @return  ECHECK_NOW = 0, ECHECK_NEXT = 1, ECHECK_NEVER = 2
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
     * 长连信令校验回包
     */
    @JvmStatic
    private fun onLongLinkIdentifyResp(
        channelID: String?,
        respBuf: ByteArray?,
        reqBufHash: ByteArray?
    ): Boolean {
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
     * Task运行完成时，STN将Task的运行时状态及统计数据返回给上层
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
