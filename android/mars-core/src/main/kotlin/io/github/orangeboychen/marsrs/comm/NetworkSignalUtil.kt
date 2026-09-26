package io.github.orangeboychen.marsrs.comm

import android.content.Context
import android.net.wifi.WifiManager
import android.telephony.PhoneStateListener
import android.telephony.SignalStrength
import android.telephony.TelephonyManager

/**
 * What `C2Java.getSignal` asks when it wants to know how strong the network is.
 * The C++ project's class of the same name is the same four statics.
 */
object NetworkSignalUtil {

    const val TAG: String = "MicroMsg.NetworkSignalUtil"

    private var strength: Long = 10000
    private var context: Context? = null

    @JvmStatic
    @Suppress("DEPRECATION")
    fun InitNetworkSignalUtil(ncontext: Context?) {
        context = ncontext
        val mgr = ncontext?.getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager
        mgr?.listen(
            object : PhoneStateListener() {
                override fun onSignalStrengthsChanged(signalStrength: SignalStrength?) {
                    super.onSignalStrengthsChanged(signalStrength)
                    if (signalStrength != null) {
                        calSignalStrength(signalStrength)
                    }
                }
            },
            PhoneStateListener.LISTEN_SIGNAL_STRENGTHS
        )
    }

    @JvmStatic
    fun getNetworkSignalStrength(isWifi: Boolean): Long = 0

    @JvmStatic
    fun getGSMSignalStrength(): Long = strength

    @JvmStatic
    fun getWifiSignalStrength(): Long {
        val wifiManager = context?.getSystemService(Context.WIFI_SERVICE) as? WifiManager
        @Suppress("DEPRECATION")
        val info = wifiManager?.connectionInfo
        if (info != null && info.bssid != null) {
            // calculateSignalLevel param 2 must < 46, otherwise divide by 0 exception will happen
            var sig = WifiManager.calculateSignalLevel(info.rssi, 10)
            sig = if (sig > 10) 10 else sig
            sig = if (sig < 0) 0 else sig
            return sig.toLong() * 10
        }
        return 0
    }

    @Suppress("DEPRECATION")
    private fun calSignalStrength(sig: SignalStrength) {
        var nSig: Int = if (sig.isGsm) {
            sig.gsmSignalStrength
        } else {
            (sig.cdmaDbm + 113) / 2
        }
        if (sig.isGsm && nSig == 99) {
            strength = 0
        } else {
            strength = (nSig * (100f / 31f)).toLong()
            strength = if (strength > 100) 100 else strength
            strength = if (strength < 0) 0 else strength
        }
    }
}
