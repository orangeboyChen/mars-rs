package io.github.orangeboychen.marsrs.sdt

import java.util.Arrays

/**
 * 信令探测结果信息类
 *
 * What [SdtLogic.ICallBack.reportSignalDetectResults] hands the app is the JSON
 * of one of these; nothing here is read by `mars-jni`, so the fields are
 * `@JvmField`s only because that is what a Java consumer of the AAR sees.
 */
class SignalDetectResult {

    @JvmField
    var details: Array<ResultDetail>? = null

    class ResultDetail {
        @JvmField
        var detectType: Int = 0

        @JvmField
        var errorCode: Int = 0

        @JvmField
        var networkType: Int = 0

        @JvmField
        var detectIP: String? = null

        @JvmField
        var connTime: Long = 0

        @JvmField
        var port: Int = 0

        @JvmField
        var rtt: Int = 0

        @JvmField
        var rttStr: String? = null

        @JvmField
        var httpStatusCode: Int = 0

        @JvmField
        var pingCheckCount: Int = 0

        @JvmField
        var pingLossRate: String? = null

        @JvmField
        var dnsDomain: String? = null

        @JvmField
        var localDns: String? = null

        @JvmField
        var dnsIP1: String? = null

        @JvmField
        var dnsIP2: String? = null

        override fun toString(): String {
            return "ResultDetail{" +
                "detectType=" + detectType +
                ", errorCode=" + errorCode +
                ", networkType=" + networkType +
                ", detectIP='" + detectIP + '\'' +
                ", connTime=" + connTime +
                ", port=" + port +
                ", rtt=" + rtt +
                ", rttStr='" + rttStr + '\'' +
                ", httpStatusCode=" + httpStatusCode +
                ", pingCheckCount=" + pingCheckCount +
                ", pingLossRate='" + pingLossRate + '\'' +
                ", dnsDomain='" + dnsDomain + '\'' +
                ", localDns='" + localDns + '\'' +
                ", dnsIP1='" + dnsIP1 + '\'' +
                ", dnsIP2='" + dnsIP2 + '\'' +
                '}'
        }
    }

    override fun toString(): String {
        return "SignalDetectResult{" +
            "details=" + Arrays.toString(details) +
            '}'
    }
}
