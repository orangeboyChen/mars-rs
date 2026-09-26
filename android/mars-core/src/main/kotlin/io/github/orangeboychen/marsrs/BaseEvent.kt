package io.github.orangeboychen.marsrs

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.NetworkInfo
import android.net.wifi.WifiInfo
import android.net.wifi.WifiManager
import android.util.Log

/**
 * The base event notification class — every `external` here is one
 * `Java_io_github_orangeboychen_marsrs_BaseEvent_*` symbol of `mars-jni`, so
 * every one of them is `@JvmStatic`: a native that is not a static of this
 * class is not the symbol JNI looks up.
 *
 * `onInitConfigBeforeOnCreate` is declared by the C++ but not by its own Java
 * class; the port has it, because the encoder version it hands over is the one
 * `StnLogic.resetAndInitEncoderVersion` sets as well.
 */
object BaseEvent {

    @JvmStatic
    external fun onCreate()

    @JvmStatic
    external fun onDestroy()

    @JvmStatic
    external fun onNetworkChange()

    @JvmStatic
    external fun onForeground(forground: Boolean)

    @JvmStatic
    external fun onSingalCrash(sig: Int)

    @JvmStatic
    external fun onExceptionCrash()

    @JvmStatic
    external fun onInitConfigBeforeOnCreate(packerEncoderVersion: Int)

    /**
     * Listens for a network change: the client registers this broadcast to tell mars STN that the
     * network changed.
     */
    class ConnectionReceiver : BroadcastReceiver() {

        override fun onReceive(context: Context?, intent: Intent?) {
            if (context == null || intent == null) {
                return
            }

            val mgr = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            var netInfo: NetworkInfo? = null
            try {
                @Suppress("DEPRECATION")
                netInfo = mgr?.activeNetworkInfo
            } catch (e: Exception) {
                Log.i(TAG, "getActiveNetworkInfo failed.")
            }

            checkConnInfo(context, netInfo)
        }

        fun checkConnInfo(context: Context, activeNetInfo: NetworkInfo?) {
            if (activeNetInfo == null) {
                lastActiveNetworkInfo = null
                lastWifiInfo = null
                BaseEvent.onNetworkChange()
            } else if (activeNetInfo.detailedState != NetworkInfo.DetailedState.CONNECTED) {
                if (lastConnected) {
                    lastActiveNetworkInfo = null
                    lastWifiInfo = null
                    BaseEvent.onNetworkChange()
                }
                lastConnected = false
            } else {
                if (isNetworkChange(context, activeNetInfo)) {
                    BaseEvent.onNetworkChange()
                }
                lastConnected = true
            }
        }

        fun isNetworkChange(context: Context, activeNetInfo: NetworkInfo): Boolean {
            val isWifi = activeNetInfo.type == ConnectivityManager.TYPE_WIFI
            val last = lastActiveNetworkInfo
            if (isWifi) {
                val wifiManager = context.getSystemService(Context.WIFI_SERVICE) as? WifiManager

                @Suppress("DEPRECATION")
                val wi = wifiManager?.connectionInfo
                val lastWifi = lastWifiInfo
                if (wi != null && lastWifi != null &&
                    lastWifi.bssid != null && lastWifi.ssid != null &&
                    lastWifi.bssid == wi.bssid &&
                    lastWifi.ssid == wi.ssid &&
                    lastWifi.networkId == wi.networkId
                ) {
                    Log.w(TAG, "Same Wifi, do not NetworkChanged")
                    return false
                }
                lastWifiInfo = wi
            } else if (last != null && last.extraInfo != null && activeNetInfo.extraInfo != null &&
                last.extraInfo == activeNetInfo.extraInfo &&
                last.subtype == activeNetInfo.subtype &&
                last.type == activeNetInfo.type
            ) {
                return false
            } else if (last != null && last.extraInfo == null && activeNetInfo.extraInfo == null &&
                last.subtype == activeNetInfo.subtype &&
                last.type == activeNetInfo.type
            ) {
                Log.w(TAG, "Same Network, do not NetworkChanged")
                return false
            }

            lastActiveNetworkInfo = activeNetInfo

            return true
        }

        companion object {
            @JvmField
            var lastActiveNetworkInfo: NetworkInfo? = null

            @JvmField
            var lastWifiInfo: WifiInfo? = null

            @JvmField
            var lastConnected: Boolean = true

            const val TAG: String = "mars.ConnectionReceiver"
        }
    }
}
