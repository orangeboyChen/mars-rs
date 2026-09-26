// The constants below carry the name the C++ project's Java gives them, spelled
// the way Kotlin spells a constant: `K_PING_CHECK` there is `K_PING_CHECK` here.
// The JNI reaches a constant by the number it carries and not by its name, so
// nothing on the Rust side had to change with them.

package io.github.orangeboychen.marsrs.sdt

import io.github.orangeboychen.marsrs.Mars

/**
 * The signal detection utility class
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
        const val K_PING_CHECK: Int = 0
        const val K_DNS_CHECK: Int = 1
        const val K_NEW_DNS_CHECK: Int = 2
        const val K_TCP_CHECK: Int = 3
        const val K_HTTP_CHECK: Int = 4
    }

    object TcpCheckErrCode {
        const val K_TCP_SUCC: Int = 0
        const val K_TCP_NON_ERR: Int = 1
        const val K_SELECT_ERR: Int = -1
        const val K_PIPE_INTR: Int = -2
        const val K_SND_RCV_ERR: Int = -3
        const val K_ASSERT_ERR: Int = -4
        const val K_TIMEOUT_ERR: Int = -5
        const val K_SELECT_EXP_ERR: Int = -6
        const val K_PIPE_EXP: Int = -7
        const val K_CONNECT_ERR: Int = -8
        const val K_TCP_RESP_ERR: Int = -9
    }

    /** The signal detection callback interface, which starts a signal detection */
    interface ICallBack {
        fun reportSignalDetectResults(resultsJson: String?)
    }

    private var callBack: ICallBack? = null

    /** Sets the signal detection callback instance: it is how the results reach the app */
    @JvmStatic
    fun setCallBack(callback: ICallBack?) {
        callBack = callback
    }

    /** Sets the URI of an HTTP connectivity check */
    @JvmStatic
    external fun setHttpNetcheckCGI(requestURI: String?)

    /** Gets the modules the native side has loaded */
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
