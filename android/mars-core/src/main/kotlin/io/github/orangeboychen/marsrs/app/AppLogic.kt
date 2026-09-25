package io.github.orangeboychen.marsrs.app

/**
 * APP的属性类
 *
 * The four statics `mars-jni` calls (`getAppFilePath`, `getAccountInfo`,
 * `getClientVersion`, `getDeviceType`) and the two classes it reads the answers
 * out of — [AccountInfo] and [DeviceInfo]. The field names are the ones the
 * Rust asks for, so they are `@JvmField`s and not properties with a getter.
 */
object AppLogic {

    const val TAG: String = "mars.AppLogic"

    /**
     * 帐号信息类
     */
    class AccountInfo(
        /** 帐号 */
        @JvmField var uin: Long = 0,
        /** 用户名 */
        @JvmField var userName: String = ""
    )

    /**
     * 终端设备信息类
     */
    class DeviceInfo(
        /** 设备名称 */
        @JvmField var devicename: String,
        /** 设备类型 */
        @JvmField var devicetype: String
    )

    /**
     * 关于APP信息的回调接口
     */
    interface ICallBack {
        /**
         * STN 会将配置文件进行存储，如连网IPPort策略、心跳策略等，此类信息将会被存储在客户端上层指定的目录下
         * @return APP目录
         */
        fun getAppFilePath(): String?

        /**
         * STN 会根据客户端的登陆状态进行网络连接策略的动态调整，当用户非登陆态时，网络会将连接的频率降低
         * 所以需要获取用户的帐号信息，判断用户是否已登录
         * @return 用户帐号信息
         */
        fun getAccountInfo(): AccountInfo?

        /**
         * 客户端版本号能够帮助 STN 清晰区分存储的网络策略配置文件。
         * @return 客户端版本号
         */
        fun getClientVersion(): Int

        /**
         * 客户端通过获取设备类型，加入到不同的上报统计回调中，供客户端进行数据分析
         */
        fun getDeviceType(): DeviceInfo?
    }

    private var callBack: ICallBack? = null

    /**
     * 设置mars回调接口实例，mars回调上层时会调用该实例的方法
     */
    @JvmStatic
    fun setCallBack(callback: ICallBack?) {
        callBack = callback
    }

    /**
     * mars回调获取APP目录
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
     * mars回调获取用户帐号信息
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
     * mars回调获取客户端版本号
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
     * mars回调获取终端设备信息
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
