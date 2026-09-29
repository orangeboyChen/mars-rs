package io.github.orangeboychen.marsrs.comm

import android.content.Context
import android.net.wifi.WifiManager
import android.telephony.PhoneStateListener
import android.telephony.SignalStrength
import android.telephony.TelephonyManager
import android.util.Log

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

    /**
     * What the last `onSignalStrengthsChanged` read — written on the phone's
     * thread and read on whichever thread `C2Java.getSignal` comes in on.
     */
    @Volatile
    private var strength: Long = DEFAULT_STRENGTH

    /**
     * The context the Wifi signal is read through: written by
     * [initNetworkSignalUtil] and [release] on the thread an app called them
     * on, and read by `getWifiSignalStrength` on whichever thread
     * `C2Java.getSignal` comes in on.
     */
    @Volatile
    private var context: Context? = null

    /**
     * What [initNetworkSignalUtil] put on the air, and what [release] takes off
     * it: one call writes it and the next reads it, and the two are not always
     * made on one thread — `Mars.init` and `Mars.release` are an app's to call
     * from wherever it likes.
     */
    @Volatile
    private var listener: PhoneStateListener? = null

    /**
     * Starts reading the signal strength, which takes a context and keeps it:
     * what is kept is that context's application context, so an Activity handed
     * over here is not held for the process by the field above.
     */
    @JvmStatic
    @Suppress("DEPRECATION")
    fun initNetworkSignalUtil(ncontext: Context?) {
        // What the call before this one put on the air, if there was one:
        // `TelephonyManager` keeps every listener it is handed, so a second
        // `init` that did not take the first one off is two of them reading
        // the signal — and [release] knows the last one's and no other's.
        release()
        context = ncontext?.applicationContext
        val mgr = context?.getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager ?: return
        val signal = object : PhoneStateListener() {
            override fun onSignalStrengthsChanged(signalStrength: SignalStrength?) {
                super.onSignalStrengthsChanged(signalStrength)
                if (signalStrength != null) {
                    calSignalStrength(signalStrength)
                }
            }
        }
        listener = signal
        // `LISTEN_SIGNAL_STRENGTHS` is a read of the phone state, and an app
        // that was not granted `READ_PHONE_STATE` is answered with a
        // `SecurityException` and not with a silence — which used to come out of
        // `Mars.init` and take the whole port's startup with it, for an app that
        // never asked how strong its signal was.
        try {
            mgr.listen(signal, PhoneStateListener.LISTEN_SIGNAL_STRENGTHS)
        } catch (e: SecurityException) {
            Log.w(TAG, "no READ_PHONE_STATE: the signal strength is not read", e)
        }
    }

    /**
     * Takes the listener off the air and drops the context: the way an app lets
     * go of what [initNetworkSignalUtil] kept, which nothing else does — a
     * listener left registered is one `TelephonyManager` goes on reading the
     * signal into for the rest of the process. The context is not what leaks:
     * it is the application context, which the process keeps in any case.
     *
     * `TelephonyManager.listen` and `PhoneStateListener` are deprecated from
     * API 31 on, and what took their place is a `TelephonyCallback` this module
     * cannot use: `minSdk` is 21, so the deprecated pair is the only one an
     * install on a device below 31 has.
     */
    @JvmStatic
    @Suppress("DEPRECATION")
    fun release() {
        val signal = listener
        val mgr = context?.getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager
        if (signal != null && mgr != null) {
            mgr.listen(signal, PhoneStateListener.LISTEN_NONE)
        }
        listener = null
        context = null
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
        val nSig: Int = if (sig.isGsm) {
            sig.gsmSignalStrength
        } else {
            (sig.cdmaDbm + CDMA_DBM_OFFSET) / CDMA_DBM_DIVISOR
        }
        // One write, and a clamped one: [strength] is read on whatever thread
        // `C2Java.getSignal` comes in on, and a CDMA reading below -113 dBm
        // answers a negative percent for as long as the clamp is a second
        // write of a field another thread is free to read between the two.
        strength = if (sig.isGsm && nSig == GSM_UNKNOWN_SIGNAL) {
            0
        } else {
            (nSig * (FULL_PERCENT_AS_FLOAT / GSM_LEVELS_AS_FLOAT)).toLong().coerceIn(0, FULL_PERCENT)
        }
    }
}
