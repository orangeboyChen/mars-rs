package io.github.orangeboychen.marsrs.stn

import java.util.Arrays

/**
 * 网络任务统计信息类
 *
 * What `StnLogic.ICallBack.reportTaskProfile` hands the app is the JSON of one
 * of these; the fields are the C++ project's, `@JvmField` so that Java reads
 * them as it read the C++ class's.
 */
class TaskProfile {

    @JvmField
    var taskId: Int = 0

    @JvmField
    var cmdId: Int = 0

    @JvmField
    var cgi: String? = null

    @JvmField
    var startTaskTime: Long = 0

    @JvmField
    var endTaskTime: Long = 0

    @JvmField
    var dyntimeStatus: Int = 0

    @JvmField
    var errCode: Int = 0

    @JvmField
    var errType: Int = 0

    @JvmField
    var channelSelect: Int = 0

    @JvmField
    var historyNetLinkers: Array<ConnectProfile>? = null

    class ConnectProfile {
        @JvmField
        var startTime: Long = 0

        @JvmField
        var dnsTime: Long = 0

        @JvmField
        var dnsEndTime: Long = 0

        @JvmField
        var connTime: Long = 0

        @JvmField
        var connErrCode: Int = 0

        @JvmField
        var tryIPCount: Int = 0

        @JvmField
        var ip: String? = null

        @JvmField
        var port: Int = 0

        @JvmField
        var host: String? = null

        @JvmField
        var ipType: Int = 0

        @JvmField
        var disconnTime: Long = 0

        @JvmField
        var disconnErrType: Long = 0

        @JvmField
        var disconnErrCode: Long = 0

        override fun toString(): String {
            return "ConnectProfile{" +
                "startTime=" + startTime +
                ", dnsTime=" + dnsTime +
                ", dnsEndTime=" + dnsEndTime +
                ", connTime=" + connTime +
                ", connErrCode=" + connErrCode +
                ", tryIPCount=" + tryIPCount +
                ", ip='" + ip + '\'' +
                ", port=" + port +
                ", host='" + host + '\'' +
                ", ipType=" + ipType +
                ", disconnTime=" + disconnTime +
                ", disconnErrType=" + disconnErrType +
                ", disconnErrCode=" + disconnErrCode +
                '}'
        }
    }

    override fun toString(): String {
        return "TaskProfile{" +
            "taskId=" + taskId +
            ", cmdId=" + cmdId +
            ", cgi='" + cgi + '\'' +
            ", startTaskTime=" + startTaskTime +
            ", endTaskTime=" + endTaskTime +
            ", dyntimeStatus=" + dyntimeStatus +
            ", errCode=" + errCode +
            ", errType=" + errType +
            ", channelSelect=" + channelSelect +
            ", historyNetLinkers=" + Arrays.toString(historyNetLinkers) +
            '}'
    }
}
