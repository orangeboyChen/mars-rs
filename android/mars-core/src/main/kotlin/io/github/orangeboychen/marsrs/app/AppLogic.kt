package io.github.orangeboychen.marsrs.app

/**
 * The class of the app's attributes
 *
 * The four statics `mars-jni` calls (`getAppFilePath`, `getAccountInfo`,
 * `getClientVersion`, `getDeviceType`) and the two classes it reads the answers
 * out of — [AccountInfo] and [DeviceInfo]. The field names are the ones the
 * Rust asks for, so they are `@JvmField`s and not properties with a getter.
 */
object AppLogic {

    const val TAG: String = "mars.AppLogic"

    /**
     * The account info class
     */
    class AccountInfo(
        /** The account number */
        @JvmField var uin: Long = 0,
        /** The user name */
        @JvmField var userName: String = ""
    )

    /**
     * The terminal device info class
     */
    class DeviceInfo(
        /** The device name */
        @JvmField var devicename: String,
        /** The device type */
        @JvmField var devicetype: String
    )

    /**
     * The callback interface of the app's information
     */
    interface ICallBack {
        /**
         * STN stores its configuration files — which IP and port it connects to, how often it beats
         * a heart and so on — and the directory it stores them under is the one the app names.
         * @return the app directory
         */
        fun getAppFilePath(): String?

        /**
         * STN adjusts how it connects to the login state of the client, and while the user is not
         * logged in it connects less often — so it asks for the account to tell whether the user is
         * logged in.
         * @return the user's account info
         */
        fun getAccountInfo(): AccountInfo?

        /**
         * The client version is what lets STN tell the network strategy files it stored apart.
         * @return the client version
         */
        fun getClientVersion(): Int

        /**
         * The device type is what puts the client's reports into one bucket or another, which is
         * what it analyses its data by.
         */
        fun getDeviceType(): DeviceInfo?
    }

    private var callBack: ICallBack? = null

    /**
     * Sets the instance mars calls back on: what mars asks the app, it asks of this one.
     */
    @JvmStatic
    fun setCallBack(callback: ICallBack?) {
        callBack = callback
    }

    /**
     * The mars callback that gets the app directory
     */
    @JvmStatic
    fun getAppFilePath(): String? {
        return try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return null
            }
            imp.getAppFilePath()
        } catch (e: Exception) {
            e.printStackTrace()
            null
        }
    }

    /**
     * The mars callback that gets the user's account info
     */
    @JvmStatic
    private fun getAccountInfo(): AccountInfo? {
        return try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return null
            }
            imp.getAccountInfo()
        } catch (e: Exception) {
            e.printStackTrace()
            null
        }
    }

    /**
     * The mars callback that gets the client version
     */
    @JvmStatic
    private fun getClientVersion(): Int {
        return try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return 0
            }
            imp.getClientVersion()
        } catch (e: Exception) {
            e.printStackTrace()
            0
        }
    }

    /**
     * The mars callback that gets the terminal device info
     */
    @JvmStatic
    private fun getDeviceType(): DeviceInfo? {
        return try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return null
            }
            imp.getDeviceType()
        } catch (e: Exception) {
            e.printStackTrace()
            null
        }
    }
}
