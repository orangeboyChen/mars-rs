package io.github.orangeboychen.marsrs.sdt

import io.github.orangeboychen.marsrs.Mars

/**
 * 信令探测工具类
 *
 * Two `external`s, both of them statics of this very class — that is what makes
 * them the `Java_io_github_orangeboychen_marsrs_sdt_SdtLogic_*` symbols
 * `mars-jni` exports — and one private static the Rust calls back.
 */
object SdtLogic {

    const val TAG: String = "mars.SdtLogic"

    init {
        Mars.loadDefaultMarsLibrary()
    }

    object NetCheckType {
        const val kPingCheck: Int = 0
        const val kDnsCheck: Int = 1
        const val kNewDnsCheck: Int = 2
        const val kTcpCheck: Int = 3
        const val kHttpCheck: Int = 4
    }

    object TcpCheckErrCode {
        const val kTcpSucc: Int = 0
        const val kTcpNonErr: Int = 1
        const val kSelectErr: Int = -1
        const val kPipeIntr: Int = -2
        const val kSndRcvErr: Int = -3
        const val kAssertErr: Int = -4
        const val kTimeoutErr: Int = -5
        const val kSelectExpErr: Int = -6
        const val kPipeExp: Int = -7
        const val kConnectErr: Int = -8
        const val kTcpRespErr: Int = -9
    }

    /** 信令探测回调接口，启动信令探测 */
    interface ICallBack {
        fun reportSignalDetectResults(resultsJson: String?)
    }

    private var callBack: ICallBack? = null

    /** 设置信令探测回调实例，探测结果将通过该实例通知上层 */
    @JvmStatic
    fun setCallBack(callback: ICallBack?) {
        callBack = callback
    }

    /** 设置一个Http连通状态探测的URI */
    @JvmStatic
    external fun setHttpNetcheckCGI(requestURI: String?)

    /** 获取底层已加载模块 */
    @JvmStatic
    private external fun getLoadLibraries(): ArrayList<String>?

    @JvmStatic
    private fun reportSignalDetectResults(resultsJson: String?) {
        try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return
            }
            imp.reportSignalDetectResults(resultsJson)
        } catch (e: Exception) {
            e.printStackTrace()
        }
    }
}
