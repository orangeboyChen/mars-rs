// The constants below carry the name the C++ project's Java gives them, spelled
// the way Kotlin spells a constant: `K_PING_CHECK` there is `K_PING_CHECK` here.
// The JNI reaches a constant by the number it carries and not by its name, so
// nothing on the Rust side had to change with them.

package io.github.orangeboychen.marsrs.comm

// `PlatformComm$C2Java` — the nine static methods `marsrs-jni` calls when it
// wants to know what the device is on (`platform_comm.rs`'s `Ask`); the C++
// asked the same nine of the same class. Every one of them is `@JvmStatic`,
// because a method JNI calls as a static of `PlatformComm$C2Java` has to *be*
// one.
//
// What the C++ asked as well and this port does not: `startAlarm`, `stopAlarm`
// and `wakeupLock_new` — the port keeps its timers and its wake lock in Rust
// (`crate::alarm`, `crate::wakerlock`), so there is nobody here to ask.

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkInfo
import android.net.wifi.WifiInfo as AndroidWifiInfo
import android.net.wifi.WifiManager
import android.os.Handler
import android.telephony.TelephonyManager

/**
 * What mars gets
 *
 * The [C2Java] nested object is the class `marsrs-jni` looks up; the three info
 * classes are what three of its answers come back as, and their fields are
 * `@JvmField`s because the Rust reads them by name.
 */
object PlatformComm {

    @JvmField
    @Volatile
    var context: Context? = null

    @JvmField
    @Volatile
    var handler: Handler? = null

    const val E_NO_NET: Int = -1
    const val E_WIFI: Int = 1
    const val E_MOBILE: Int = 2
    const val E_OTHER_NET: Int = 3

    const val NETTYPE_NOT_WIFI: Int = 0
    const val NETTYPE_WIFI: Int = 1
    const val NETTYPE_WAP: Int = 2
    const val NETTYPE_2G: Int = 3
    const val NETTYPE_3G: Int = 4
    const val NETTYPE_4G: Int = 5
    const val NETTYPE_UNKNOWN: Int = 6
    const val NETTYPE_NON: Int = -1

    /** The WiFi info class */
    class WifiInfo {
        @JvmField
        var ssid: String? = null

        @JvmField
        var bssid: String? = null
    }

    /** The SIM card info class */
    class SIMInfo {
        @JvmField
        var ispCode: String? = null

        @JvmField
        var ispName: String? = null
    }

    /** The access point info */
    class APNInfo {
        @JvmField
        var netType: Int = 0

        @JvmField
        var subNetType: Int = 0

        @JvmField
        var extraInfo: String? = null
    }

    /**
     * How the platform callbacks are initialized: the nine methods of C2Java all
     * take what they ask about from the context that is kept here.
     *
     * What is kept is the application context of the one handed over, and not
     * that one: what the nine ask of it — a system service, a network, a signal
     * — is the process' and not an Activity's, and a field nothing clears would
     * hold that Activity for the life of the process.
     */
    @JvmStatic
    fun init(ncontext: Context?, nhandler: Handler?) {
        context = ncontext?.applicationContext
        handler = nhandler

        NetworkSignalUtil.initNetworkSignalUtil(context)
    }

    /**
     * Lets go of what [init] kept — the context, the handler and the phone-state
     * listener — which is the only way an app that is done with the port has of
     * dropping a context it handed over. Nothing of [C2Java] answers afterwards:
     * every one of the nine reads the context this clears.
     */
    @JvmStatic
    fun release() {
        NetworkSignalUtil.release()
        context = null
        handler = null
    }

    object C2Java {

        /**
         * The mars callback that gets the network type
         * @return WiFi/Mobile/NoNet
         */
        @JvmStatic
        fun getNetInfo(): Int {
            val conMan = context?.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
                ?: return E_NO_NET

            @Suppress("DEPRECATION")
            val netInfo = conMan.activeNetworkInfo ?: return E_NO_NET

            return try {
                when (netInfo.type) {
                    ConnectivityManager.TYPE_WIFI -> E_WIFI

                    ConnectivityManager.TYPE_MOBILE,
                    ConnectivityManager.TYPE_MOBILE_DUN,
                    ConnectivityManager.TYPE_MOBILE_HIPRI,
                    ConnectivityManager.TYPE_MOBILE_MMS,
                    ConnectivityManager.TYPE_MOBILE_SUPL -> E_MOBILE

                    else -> E_OTHER_NET
                }
            } catch (e: Exception) {
                e.printStackTrace()
                E_OTHER_NET
            }
        }

        /**
         * The mars callback that gets the HTTP proxy info.
         *
         * `-1`, and always: what `marsrs-jni` reads as "there is no proxy".
         * Android has no process-wide HTTP proxy for a library to read any more
         * — `android.net.Proxy` is deprecated, and what took its place
         * (`ConnectivityManager.getDefaultProxy`) is a proxy of the network and
         * not one an app sets for the process. What stood here was forty lines
         * of reading behind a constant no caller could turn on, so every app
         * got `-1` and none of them was told why. `NetStatusUtil.getProxyInfo`
         * is the one that still reads one, for an app that wants it.
         */
        @JvmStatic
        fun getProxyInfo(strProxy: StringBuffer): Int = -1

        @JvmStatic
        fun getStatisticsNetType(): Int {
            val ctx = context ?: return 0

            return try {
                val ret = NetStatusUtil.getNetType(ctx)
                when {
                    ret == NetStatusUtil.NON_NETWORK -> NETTYPE_NON
                    NetStatusUtil.is2G(ctx) -> NETTYPE_2G
                    NetStatusUtil.is3G(ctx) -> NETTYPE_3G
                    NetStatusUtil.is4G(ctx) -> NETTYPE_4G
                    NetStatusUtil.isWifi(ret) -> NETTYPE_WIFI
                    NetStatusUtil.isWap(ret) -> NETTYPE_WAP
                    else -> NETTYPE_UNKNOWN
                }
            } catch (e: Exception) {
                e.printStackTrace()
                NETTYPE_NON
            }
        }

        /** Gets the details of the current WiFi */
        @JvmStatic
        fun getCurWifiInfo(): WifiInfo? {
            return try {
                val ctx = context ?: return null

                val conMan = ctx.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
                    ?: return null

                @Suppress("DEPRECATION")
                val netInfo = conMan.activeNetworkInfo
                if (netInfo == null || ConnectivityManager.TYPE_WIFI != netInfo.type) {
                    return null
                }

                val wifiMgr = ctx.getSystemService(Context.WIFI_SERVICE) as? WifiManager ?: return null

                @Suppress("DEPRECATION")
                val wifiInfo = wifiMgr.connectionInfo ?: return null

                WifiInfo().apply {
                    ssid = wifiInfo.ssid
                    bssid = wifiInfo.bssid
                }
            } catch (e: Exception) {
                e.printStackTrace()
                null
            }
        }

        /** Gets the current SIM card info */
        @JvmStatic
        fun getCurSIMInfo(): SIMInfo? {
            return try {
                val ctx = context ?: return null

                val ispCode = NetStatusUtil.getISPCode(ctx)
                if (NetStatusUtil.NO_SIM_OPERATOR == ispCode) {
                    return null
                }

                SIMInfo().apply {
                    this.ispCode = "" + ispCode
                    this.ispName = NetStatusUtil.getISPName(ctx)
                }
            } catch (e: Exception) {
                e.printStackTrace()
                null
            }
        }

        /** Gets the access point info */
        @JvmStatic
        fun getAPNInfo(): APNInfo? {
            return try {
                val ctx = context ?: return null
                val manager = ctx.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
                    ?: return null

                @Suppress("DEPRECATION")
                val netInfo: NetworkInfo = manager.activeNetworkInfo ?: return null

                APNInfo().apply {
                    netType = netInfo.type
                    subNetType = netInfo.subtype
                    extraInfo = if (ConnectivityManager.TYPE_WIFI != netInfo.type) {
                        netInfo.extraInfo ?: ""
                    } else {
                        getCurWifiInfo()?.ssid
                    }
                }
            } catch (e: Exception) {
                e.printStackTrace()
                null
            }
        }

        @JvmStatic
        fun getCurRadioAccessNetworkInfo(): Int {
            val ctx = context ?: return TelephonyManager.NETWORK_TYPE_UNKNOWN

            return try {
                val telephonyManager = ctx.getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager
                telephonyManager?.networkType ?: TelephonyManager.NETWORK_TYPE_UNKNOWN
            } catch (e: Exception) {
                e.printStackTrace()
                TelephonyManager.NETWORK_TYPE_UNKNOWN
            }
        }

        /**
         * The mars callback that gets the network signal strength
         */
        @JvmStatic
        fun getSignal(isWifi: Boolean): Long {
            return try {
                if (context == null) return 0

                if (isWifi) NetworkSignalUtil.getWifiSignalStrength() else NetworkSignalUtil.getGSMSignalStrength()
            } catch (e: Exception) {
                e.printStackTrace()
                0
            }
        }

        /** The mars callback that asks whether the terminal's network is connected */
        @JvmStatic
        fun isNetworkConnected(): Boolean {
            val ctx = context ?: return false

            return try {
                NetStatusUtil.isNetworkConnected(ctx)
            } catch (e: Exception) {
                e.printStackTrace()
                false
            }
        }
    }
}
