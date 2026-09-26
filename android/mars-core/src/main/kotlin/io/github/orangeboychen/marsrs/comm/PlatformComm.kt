package io.github.orangeboychen.marsrs.comm

// `PlatformComm$C2Java` — the nine static methods `mars-jni` calls when it
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
import android.net.Proxy
import android.net.wifi.WifiManager
import android.os.Handler
import android.telephony.TelephonyManager
import android.net.wifi.WifiInfo as AndroidWifiInfo

/**
 * mars获取
 *
 * The [C2Java] nested object is the class `mars-jni` looks up; the three info
 * classes are what three of its answers come back as, and their fields are
 * `@JvmField`s because the Rust reads them by name.
 */
object PlatformComm {

    private const val IS_PROXY_ON = false

    @JvmField
    var context: Context? = null

    @JvmField
    var handler: Handler? = null

    const val ENoNet: Int = -1
    const val EWifi: Int = 1
    const val EMobile: Int = 2
    const val EOtherNet: Int = 3

    const val NETTYPE_NOT_WIFI: Int = 0
    const val NETTYPE_WIFI: Int = 1
    const val NETTYPE_WAP: Int = 2
    const val NETTYPE_2G: Int = 3
    const val NETTYPE_3G: Int = 4
    const val NETTYPE_4G: Int = 5
    const val NETTYPE_UNKNOWN: Int = 6
    const val NETTYPE_NON: Int = -1

    /** WiFi信息类 */
    class WifiInfo {
        @JvmField
        var ssid: String? = null

        @JvmField
        var bssid: String? = null
    }

    /** 手机卡信息类 */
    class SIMInfo {
        @JvmField
        var ispCode: String? = null

        @JvmField
        var ispName: String? = null
    }

    /** 接入点信息 */
    class APNInfo {
        @JvmField
        var netType: Int = 0

        @JvmField
        var subNetType: Int = 0

        @JvmField
        var extraInfo: String? = null
    }

    /** 平台回调的初始化：C2Java 的九个方法都要从这里的 context 取它们问的东西。 */
    @JvmStatic
    fun init(ncontext: Context?, nhandler: Handler?) {
        context = ncontext
        handler = nhandler

        NetworkSignalUtil.InitNetworkSignalUtil(ncontext)
    }

    object C2Java {

        /**
         * mars回调获取网络类型
         * @return WiFi/Mobile/NoNet
         */
        @JvmStatic
        fun getNetInfo(): Int {
            val conMan = context?.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
                ?: return ENoNet

            @Suppress("DEPRECATION")
            val netInfo = conMan.activeNetworkInfo ?: return ENoNet

            return try {
                when (netInfo.type) {
                    ConnectivityManager.TYPE_WIFI -> EWifi
                    ConnectivityManager.TYPE_MOBILE,
                    ConnectivityManager.TYPE_MOBILE_DUN,
                    ConnectivityManager.TYPE_MOBILE_HIPRI,
                    ConnectivityManager.TYPE_MOBILE_MMS,
                    ConnectivityManager.TYPE_MOBILE_SUPL -> EMobile

                    else -> EOtherNet
                }
            } catch (e: Exception) {
                e.printStackTrace()
                EOtherNet
            }
        }

        /**
         * mars回调获取Http代理信息
         */
        @JvmStatic
        @Suppress("DEPRECATION")
        fun getProxyInfo(strProxy: StringBuffer): Int {
            if (!IS_PROXY_ON) {
                return -1
            }

            var proxyPort = -1
            var proxy = ""
            try {
                proxy = Proxy.getDefaultHost() ?: ""
                proxyPort = Proxy.getDefaultPort()
                if (proxy.isNotEmpty() && proxyPort > 0) {
                    strProxy.append(proxy)
                    return proxyPort
                }
                proxy = System.getProperty("http.proxyHost") ?: ""
                val vmPort = System.getProperty("http.proxyPort")
                if (vmPort != null && vmPort.isNotEmpty()) {
                    proxyPort = Integer.parseInt(vmPort)
                }
                if (proxy.isNotEmpty()) {
                    strProxy.append(proxy)
                    return proxyPort
                }
            } catch (e: Exception) {
                e.printStackTrace()
            }

            strProxy.append(proxy)
            return proxyPort
        }

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

        /** 获取当前WiFi的具体信息 */
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

        /** 获取当前手机卡信息 */
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

        /** 获取接入点信息 */
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
         * mars回调获取网络信号强度
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

        /** mars回调查看终端网络是否已连接状态 */
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
