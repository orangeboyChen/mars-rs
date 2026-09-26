package io.github.orangeboychen.marsrs.comm

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.NetworkInfo
import android.net.wifi.WifiInfo
import android.net.wifi.WifiManager
import android.provider.Settings
import android.telephony.PhoneStateListener
import android.telephony.SignalStrength
import android.telephony.TelephonyManager

import io.github.orangeboychen.marsrs.xlog.Log

/**
 * The network-status helpers `C2Java.getStatisticsNetType`, `getCurSIMInfo` and
 * `isNetworkConnected` read — the C++ project's `NetStatusUtil`, which is a
 * class of statics and nothing else, so this is an `object`.
 */
object NetStatusUtil {

    private const val TAG = "MicroMsg.NetStatusUtil"

    const val NON_NETWORK: Int = -1
    const val WIFI: Int = 0
    const val UNINET: Int = 1
    const val UNIWAP: Int = 2
    const val WAP_3G: Int = 3
    const val NET_3G: Int = 4
    const val CMWAP: Int = 5
    const val CMNET: Int = 6
    const val CTWAP: Int = 7
    const val CTNET: Int = 8
    const val MOBILE: Int = 9
    const val LTE: Int = 10

    /** No specific network policy, use system default. */
    const val POLICY_NONE: Int = 0x0
    /** Reject network usage on metered networks when application in background. */
    const val POLICY_REJECT_METERED_BACKGROUND: Int = 0x1

    const val TBACKGROUND_NOT_LIMITED: Int = 0x0
    const val TBACKGROUND_PROCESS_LIMITED: Int = 0x1
    const val TBACKGROUND_DATA_LIMITED: Int = 0x2
    const val TBACKGROUND_WIFI_LIMITED: Int = 0x3

    const val NO_SIM_OPERATOR: Int = 0

    @JvmStatic
    fun dumpNetStatus(context: Context) {
        try {
            val connectivityManager =
                context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            @Suppress("DEPRECATION")
            val activeNetInfo = connectivityManager?.activeNetworkInfo
            Log.i(TAG, activeNetInfo.toString())
        } catch (e: Exception) {
            Log.e(TAG, "", e)
        }
    }

    @JvmStatic
    fun isConnected(context: Context): Boolean {
        val conMan = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
        @Suppress("DEPRECATION")
        val activeNetInfo = conMan?.activeNetworkInfo
        var connect = false
        try {
            connect = activeNetInfo?.isConnected == true
        } catch (e: Exception) {
        }
        return connect
    }

    @JvmStatic
    fun getNetTypeString(context: Context): String {
        val connectivityManager =
            context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
        if (connectivityManager == null) {
            return "NON_NETWORK"
        }
        @Suppress("DEPRECATION")
        val activeNetInfo = connectivityManager.activeNetworkInfo
        if (activeNetInfo == null) {
            return "NON_NETWORK"
        }

        return if (activeNetInfo.type == ConnectivityManager.TYPE_WIFI) {
            "WIFI"
        } else {
            activeNetInfo.extraInfo ?: "MOBILE"
        }
    }

    @JvmStatic
    fun getNetWorkType(context: Context): Int {
        return try {
            val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            @Suppress("DEPRECATION")
            val netInfo = manager?.activeNetworkInfo
            if (netInfo != null) netInfo.type else NON_NETWORK
        } catch (e: Exception) {
            e.printStackTrace()
            NON_NETWORK
        }
    }

    @JvmStatic
    fun getNetType(context: Context): Int {
        val connectivityManager =
            context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
        if (connectivityManager == null) {
            return NON_NETWORK
        }
        @Suppress("DEPRECATION")
        val activeNetInfo = connectivityManager.activeNetworkInfo
        if (activeNetInfo == null) {
            return NON_NETWORK
        }

        if (activeNetInfo.type == ConnectivityManager.TYPE_WIFI) {
            return WIFI
        }

        val extraInfo = activeNetInfo.extraInfo
        if (extraInfo != null) {
            if (extraInfo.equals("uninet", ignoreCase = true)) return UNINET
            if (extraInfo.equals("uniwap", ignoreCase = true)) return UNIWAP
            if (extraInfo.equals("3gwap", ignoreCase = true)) return WAP_3G
            if (extraInfo.equals("3gnet", ignoreCase = true)) return NET_3G
            if (extraInfo.equals("cmwap", ignoreCase = true)) return CMWAP
            if (extraInfo.equals("cmnet", ignoreCase = true)) return CMNET
            if (extraInfo.equals("ctwap", ignoreCase = true)) return CTWAP
            if (extraInfo.equals("ctnet", ignoreCase = true)) return CTNET
            if (extraInfo.equals("LTE", ignoreCase = true)) return LTE
        }
        return MOBILE
    }

    @JvmStatic
    fun getISPCode(context: Context): Int {
        val tel = context.getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager
        if (tel == null) {
            return NO_SIM_OPERATOR
        }

        val simOperator = tel.simOperator
        if (simOperator == null || simOperator.length < 5) { // IMSI
            return NO_SIM_OPERATOR
        }
        /*
         * http://developer.android.com/reference/android/telephony/TelephonyManager.html#getSimOperator()
         * public String getSimOperator ()
         * Returns the MCC+MNC (mobile country code + mobile network code) of the provider of the SIM. 5 or 6 decimal digits.
         */
        val mccMnc = StringBuilder()
        return try {
            var len = simOperator.length
            if (len > 6) {
                len = 6
            }
            for (i in 0 until len) {
                if (!Character.isDigit(simOperator[i])) {
                    if (mccMnc.isEmpty()) { // not begin append
                        continue
                    } else {
                        break
                    }
                }
                mccMnc.append(simOperator[i])
            }
            Integer.valueOf(mccMnc.toString())
        } catch (e: Exception) {
            e.printStackTrace()
            0
        }
    }

    @JvmStatic
    fun getISPName(context: Context): String {
        val tel = context.getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager
        if (tel == null) {
            return ""
        }

        val maxLength = 100 // ???

        val name = tel.simOperatorName ?: ""
        return if (name.length <= maxLength) name else name.substring(0, maxLength)
    }

    @JvmStatic
    fun guessNetSpeed(context: Context): Int {
        return try {
            val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            @Suppress("DEPRECATION")
            val netInfo = manager?.activeNetworkInfo
            if (netInfo?.type == ConnectivityManager.TYPE_WIFI) {
                return 100 * 1024
            }

            when (netInfo?.subtype) {
                TelephonyManager.NETWORK_TYPE_GPRS -> 4 * 1024
                TelephonyManager.NETWORK_TYPE_EDGE -> 8 * 1024

                TelephonyManager.NETWORK_TYPE_UMTS,
                TelephonyManager.NETWORK_TYPE_CDMA,
                TelephonyManager.NETWORK_TYPE_EVDO_0,
                TelephonyManager.NETWORK_TYPE_EVDO_A,
                TelephonyManager.NETWORK_TYPE_1xRTT,
                TelephonyManager.NETWORK_TYPE_HSDPA,
                TelephonyManager.NETWORK_TYPE_HSUPA,
                TelephonyManager.NETWORK_TYPE_HSPA,
                TelephonyManager.NETWORK_TYPE_IDEN,
                TelephonyManager.NETWORK_TYPE_EVDO_B,
                TelephonyManager.NETWORK_TYPE_LTE,
                TelephonyManager.NETWORK_TYPE_EHRPD,
                TelephonyManager.NETWORK_TYPE_HSPAP -> 100 * 1024

                else -> 100 * 1024
            }
        } catch (e: Exception) {
            e.printStackTrace()
            100 * 1024
        }
    }

    @JvmStatic
    fun isMobile(context: Context): Boolean {
        return try {
            val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            @Suppress("DEPRECATION")
            val netInfo = manager?.activeNetworkInfo
            netInfo?.type != ConnectivityManager.TYPE_WIFI
        } catch (e: Exception) {
            e.printStackTrace()
            false
        }
    }

    @JvmStatic
    fun is2G(context: Context): Boolean {
        return try {
            val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            @Suppress("DEPRECATION")
            val netInfo = manager?.activeNetworkInfo
            if (netInfo?.type == ConnectivityManager.TYPE_WIFI) {
                return false
            }
            netInfo?.subtype == TelephonyManager.NETWORK_TYPE_EDGE ||
                netInfo?.subtype == TelephonyManager.NETWORK_TYPE_GPRS ||
                netInfo?.subtype == TelephonyManager.NETWORK_TYPE_CDMA
        } catch (e: Exception) {
            e.printStackTrace()
            false
        }
    }

    @JvmStatic
    fun is4G(context: Context): Boolean {
        return try {
            val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            @Suppress("DEPRECATION")
            val netInfo = manager?.activeNetworkInfo
            if (netInfo?.type == ConnectivityManager.TYPE_WIFI) {
                return false
            }
            // TODO:may be 5G in the future
            netInfo != null && netInfo.subtype >= TelephonyManager.NETWORK_TYPE_LTE
        } catch (e: Exception) {
            e.printStackTrace()
            false
        }
    }

    @JvmStatic
    fun isWap(context: Context): Boolean = isWap(getNetType(context))

    @JvmStatic
    fun isWap(type: Int): Boolean =
        type == UNIWAP || type == CMWAP || type == CTWAP || type == WAP_3G

    @JvmStatic
    fun is3G(context: Context): Boolean {
        return try {
            val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            @Suppress("DEPRECATION")
            val netInfo = manager?.activeNetworkInfo
            if (netInfo?.type == ConnectivityManager.TYPE_WIFI) {
                return false
            }
            netInfo != null &&
                netInfo.subtype >= TelephonyManager.NETWORK_TYPE_EVDO_0 &&
                netInfo.subtype < TelephonyManager.NETWORK_TYPE_LTE
        } catch (e: Exception) {
            e.printStackTrace()
            false
        }
    }

    @JvmStatic
    fun isWifi(context: Context): Boolean = isWifi(getNetType(context))

    @JvmStatic
    fun isWifi(type: Int): Boolean = type == WIFI

    @JvmStatic
    fun getWifiInfo(context: Context): WifiInfo? {
        return try {
            val conMan = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            @Suppress("DEPRECATION")
            val netInfo = conMan?.activeNetworkInfo
            if (netInfo == null || ConnectivityManager.TYPE_WIFI != netInfo.type) {
                return null
            }
            val wifiMgr = context.getSystemService(Context.WIFI_SERVICE) as? WifiManager
            @Suppress("DEPRECATION")
            wifiMgr?.connectionInfo
        } catch (e: Exception) {
            e.printStackTrace()
            null
        }
    }

    private fun searchIntentByClass(context: Context, className: String): Intent? {
        return try {
            val pmPack = context.packageManager
            @Suppress("DEPRECATION")
            val packinfos = pmPack.getInstalledPackages(0)
            if (packinfos != null && packinfos.size > 0) {
                for (i in packinfos.indices) {
                    try {
                        val mainIntent = Intent()
                        mainIntent.setPackage(packinfos[i].packageName)
                        val sampleActivityInfos = pmPack.queryIntentActivities(mainIntent, 0)
                        val activityCount = sampleActivityInfos?.size ?: 0
                        if (activityCount > 0) {
                            try {
                                for (j in 0 until activityCount) {
                                    val activityInfo: ActivityInfo = sampleActivityInfos!![j].activityInfo
                                    val activityName = activityInfo.name

                                    if (activityName.contains(className)) {
                                        val mIntent = Intent("/")
                                        val comp = ComponentName(activityInfo.packageName, activityInfo.name)
                                        mIntent.component = comp
                                        mIntent.action = "android.intent.action.VIEW"
                                        context.startActivity(mIntent)
                                        return mIntent
                                    }
                                }
                            } catch (eee: Exception) {
                                eee.printStackTrace()
                            }
                        }
                    } catch (ee: Exception) {
                        ee.printStackTrace()
                    }
                }
            }
            null
        } catch (e: Exception) {
            e.printStackTrace()
            null
        }
    }

    @JvmStatic
    fun startSettingItent(context: Context, type: Int) {
        when (type) {
            TBACKGROUND_NOT_LIMITED -> {
                // may not be happen
            }

            TBACKGROUND_DATA_LIMITED -> {
                try {
                    val mIntent = Intent("/")
                    val comp = ComponentName(
                        "com.android.providers.subscribedfeeds",
                        "com.android.settings.ManageAccountsSettings"
                    )
                    mIntent.component = comp
                    mIntent.action = "android.intent.action.VIEW"
                    context.startActivity(mIntent)
                } catch (e: Exception) {
                    try {
                        val mIntent = Intent("/")
                        val comp = ComponentName(
                            "com.htc.settings.accountsync",
                            "com.htc.settings.accountsync.ManageAccountsSettings"
                        )
                        mIntent.component = comp
                        mIntent.action = "android.intent.action.VIEW"
                        context.startActivity(mIntent)
                    } catch (ee: Exception) {
                        searchIntentByClass(context, "ManageAccountsSettings")
                    }
                }
            }

            TBACKGROUND_PROCESS_LIMITED -> {
                try {
                    val mIntent = Intent("/")
                    val comp = ComponentName("com.android.settings", "com.android.settings.DevelopmentSettings")
                    mIntent.component = comp
                    mIntent.action = "android.intent.action.VIEW"
                    context.startActivity(mIntent)
                } catch (e: Exception) {
                    searchIntentByClass(context, "DevelopmentSettings")
                }
            }

            TBACKGROUND_WIFI_LIMITED -> {
                try {
                    val it = Intent()
                    it.action = Settings.ACTION_WIFI_IP_SETTINGS
                    context.startActivity(it)
                } catch (e: Exception) {
                    searchIntentByClass(context, "AdvancedSettings")
                }
            }
        }
    }

    @JvmStatic
    fun getWifiSleeepPolicy(context: Context): Int {
        @Suppress("DEPRECATION")
        return Settings.System.getInt(
            context.contentResolver,
            Settings.System.WIFI_SLEEP_POLICY,
            Settings.System.WIFI_SLEEP_POLICY_NEVER
        )
    }

    @JvmStatic
    fun isLimited(type: Int): Boolean =
        type == TBACKGROUND_DATA_LIMITED || type == TBACKGROUND_PROCESS_LIMITED || type == TBACKGROUND_WIFI_LIMITED

    @JvmStatic
    fun getBackgroundLimitType(context: Context): Int {
        if (android.os.Build.VERSION.SDK_INT >= 14) {
            try {
                val activityManagerNative = Class.forName("android.app.ActivityManagerNative")
                val am = activityManagerNative.getMethod("getDefault").invoke(activityManagerNative)
                val limit = am.javaClass.getMethod("getProcessLimit").invoke(am)
                if ((limit as? Int) == 0) {
                    return TBACKGROUND_PROCESS_LIMITED
                }
            } catch (e: Exception) {
                e.printStackTrace()
            }
        }
        return try {
            val policy = getWifiSleeepPolicy(context)
            @Suppress("DEPRECATION")
            if (policy == Settings.System.WIFI_SLEEP_POLICY_NEVER || getNetType(context) != WIFI) {
                TBACKGROUND_NOT_LIMITED
            } else if (policy == Settings.System.WIFI_SLEEP_POLICY_NEVER_WHILE_PLUGGED ||
                policy == Settings.System.WIFI_SLEEP_POLICY_DEFAULT
            ) {
                TBACKGROUND_WIFI_LIMITED
            } else {
                TBACKGROUND_NOT_LIMITED
            }
        } catch (e: Exception) {
            e.printStackTrace()
            TBACKGROUND_NOT_LIMITED
        }
    }

    @JvmStatic
    fun isImmediatelyDestroyActivities(context: Context): Boolean {
        @Suppress("DEPRECATION")
        return Settings.System.getInt(
            context.contentResolver,
            Settings.System.ALWAYS_FINISH_ACTIVITIES,
            0
        ) != 0
    }

    @JvmStatic
    @Suppress("DEPRECATION")
    fun getProxyInfo(context: Context, strProxy: StringBuffer): Int {
        return try {
            val apiHost = android.net.Proxy.getDefaultHost()
            val apiPort = android.net.Proxy.getDefaultPort()
            if (apiHost != null && apiHost.isNotEmpty() && apiPort > 0) {
                strProxy.append(apiHost)
                return apiPort
            }
            val vmHost = System.getProperty("http.proxyHost")
            val vmPort = System.getProperty("http.proxyPort")
            var ivmPort = 80
            if (vmPort != null && vmPort.isNotEmpty()) {
                ivmPort = Integer.parseInt(vmPort)
            }
            if (vmHost != null && vmHost.isNotEmpty()) {
                strProxy.append(vmHost)
                return ivmPort
            }
            0
        } catch (e: Exception) {
            e.printStackTrace()
            0
        }
    }

    @JvmStatic
    fun isKnownDirectNet(context: Context): Boolean {
        val type = getNetType(context)
        return (CMNET == type) || (UNINET == type) || (NET_3G == type) ||
            (CTNET == type) || (LTE == type) || (WIFI == type)
    }

    @JvmStatic
    fun isNetworkConnected(context: Context): Boolean {
        val connectivityManager =
            context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
        if (connectivityManager == null) {
            return false
        }
        @Suppress("DEPRECATION")
        val activeNetInfo = connectivityManager.activeNetworkInfo
        if (activeNetInfo == null) {
            return false
        }

        return activeNetInfo.state == NetworkInfo.State.CONNECTED
    }

    const val NETTYPE_NOT_WIFI: Int = 0
    const val NETTYPE_WIFI: Int = 1

    const val UNKNOW_TYPE: Int = 999

    private var nowStrength = 0

    /*
     *
     *  all * 1000
     *
     * networktype:TelephonyManager.NETWORK_TYPE_GPRS =  * 1
     * networktype:TelephonyManager.NETWORK_TYPE_EDGE = 2
     * networktype:TelephonyManager.NETWORK_TYPE_CDMA = 4
     * networktype:TelephonyManager.NETWORK_TYPE_EVDO_0 = 5
     * networktype:TelephonyManager.NETWORK_TYPE_EVDO_A = 6
     * networktype:TelephonyManager.NETWORK_TYPE_EVDO_B = 12
     * networktype:TelephonyManager.NETWORK_TYPE_HSDPA = 8
     * networktype:TelephonyManager.NETWORK_TYPE_UMTS = 3
     * networktype:TelephonyManager.NETWORK_TYPE_LTE = 13
     * networktype:TelephonyManager.NETWORK_TYPE_IDEN = 11
     * networktype:TelephonyManager.NETWORK_TYPE_HSUPA = 9
     * networktype:TelephonyManager.NETWORK_TYPE_1xRTT = 7
     * networktype:TelephonyManager.NETWORK_TYPE_HSPA = 10
     * networktype:TelephonyManager.NETWORK_TYPE_EHRPD = 14
     * networktype:TelephonyManager.NETWORK_TYPE_HSPAP = 15
     * networktype:TelephonyManager.NETWORK_TYPE_UNKNOWN = 0  --> 999
     */

    class StrengthListener : PhoneStateListener() {
        @Suppress("DEPRECATION")
        override fun onSignalStrengthsChanged(signalStrength: SignalStrength?) {
            super.onSignalStrengthsChanged(signalStrength)
            if (signalStrength == null) {
                return
            }
            nowStrength = if (!signalStrength.isGsm) {
                signalStrength.cdmaDbm
            } else {
                signalStrength.gsmSignalStrength
            }
        }
    }

    @JvmStatic
    fun getNetTypeForStat(context: Context?): Int {
        if (context == null) {
            return UNKNOW_TYPE
        }
        return try {
            val connectivityManager =
                context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            if (connectivityManager == null) {
                return UNKNOW_TYPE
            }
            @Suppress("DEPRECATION")
            val activeNetInfo = connectivityManager.activeNetworkInfo
            if (activeNetInfo == null) {
                return UNKNOW_TYPE
            }
            if (activeNetInfo.type == ConnectivityManager.TYPE_WIFI) {
                return NETTYPE_WIFI
            }
            val subType = activeNetInfo.subtype
            if (subType == TelephonyManager.NETWORK_TYPE_UNKNOWN) {
                return UNKNOW_TYPE
            }
            subType * 1000 // NOTE HERE ~~~
        } catch (e: Exception) {
            e.printStackTrace()
            UNKNOW_TYPE
        }
    }

    @JvmStatic
    fun getStrength(context: Context?): Int {
        if (context == null) {
            return 0
        }
        return try {
            if (getNetTypeForStat(context) == NETTYPE_WIFI) {
                val wifiManager = context.getSystemService(Context.WIFI_SERVICE) as? WifiManager
                @Suppress("DEPRECATION")
                Math.abs(wifiManager?.connectionInfo?.rssi ?: 0)
            } else {
                (context.getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager)?.listen(
                    StrengthListener(),
                    PhoneStateListener.LISTEN_SIGNAL_STRENGTHS
                )
                Math.abs(nowStrength)
            }
        } catch (e: Exception) {
            e.printStackTrace()
            0
        }
    }
}
