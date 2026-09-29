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

    /** What `strength` is before the first `onSignalStrengthsChanged`. */
    private const val DEFAULT_STRENGTH = 10000L

    /** The buckets `WifiManager.calculateSignalLevel` is asked to divide into. */
    private const val WIFI_LEVEL_STEPS = 10

    /** A level of `WIFI_LEVEL_STEPS` is a full signal, so one step is this much. */
    private const val WIFI_PERCENT_PER_LEVEL = 10

    /** What turns a negative CDMA dBm into the same 31 steps GSM is counted in. */
    private const val CDMA_DBM_OFFSET = 113
    private const val CDMA_DBM_DIVISOR = 2

    /** What `getGsmSignalStrength` answers when the GSM signal is unknown. */
    private const val GSM_UNKNOWN_SIGNAL = 99
    private const val FULL_PERCENT = 100L
    private const val FULL_PERCENT_AS_FLOAT = 100f

    /** `getGsmSignalStrength` counts the GSM signal in 31 steps. */
    private const val GSM_LEVELS_AS_FLOAT = 31f

    private var strength: Long = DEFAULT_STRENGTH
    private var context: Context? = null

    @JvmStatic
    @Suppress("DEPRECATION")
    fun initNetworkSignalUtil(ncontext: Context?) {
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

    /**
     * `0`, and always: what [PlatformComm] reads is the Wifi one or the GSM one,
     * which it asks for by the network it is on, so this is the one of the four
     * statics the port has nothing to call it for.
     */
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
            // `calculateSignalLevel` divides by the number of steps it is
            // given, so the ten are ten and not a hundred: 46 or more of them
            // is a divide by zero inside Android.
            var sig = WifiManager.calculateSignalLevel(info.rssi, WIFI_LEVEL_STEPS)
            sig = if (sig > WIFI_LEVEL_STEPS) WIFI_LEVEL_STEPS else sig
            sig = if (sig < 0) 0 else sig
            return sig.toLong() * WIFI_PERCENT_PER_LEVEL
        }
        return 0
    }

    @Suppress("DEPRECATION")
    private fun calSignalStrength(sig: SignalStrength) {
        var nSig: Int = if (sig.isGsm) {
            sig.gsmSignalStrength
        } else {
            (sig.cdmaDbm + CDMA_DBM_OFFSET) / CDMA_DBM_DIVISOR
        }
        if (sig.isGsm && nSig == GSM_UNKNOWN_SIGNAL) {
            strength = 0
        } else {
            strength = (nSig * (FULL_PERCENT_AS_FLOAT / GSM_LEVELS_AS_FLOAT)).toLong()
            strength = if (strength > FULL_PERCENT) FULL_PERCENT else strength
            strength = if (strength < 0) 0 else strength
        }
    }
}
