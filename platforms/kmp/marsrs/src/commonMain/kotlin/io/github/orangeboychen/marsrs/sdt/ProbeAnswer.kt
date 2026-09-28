package io.github.orangeboychen.marsrs.sdt

/**
 * What one probe of a diagnosis answered — the network the port has none of,
 * answered by the app's [SdtLogic.IProbe].
 *
 * Every case carries the readings a run of that probe leaves behind, and nothing
 * else: a resolve answers addresses, a noop a round trip, the CGI a status and a
 * ping a loss rate. [None] is a probe nobody answered, which every check reads
 * as a failure.
 */
public sealed class ProbeAnswer {
    /** Nobody answered — a host with no network to probe with. */
    public data object None : ProbeAnswer()

    /** `socket_gethostbyname` — the resolve of one host name. */
    public data class Dns(
        /** `error_code` — the return value of the probe: `0` and above is one that worked. */
        public val errorCode: Int,
        /** How long the probe took, in milliseconds. */
        public val rtt: Long,
        /** The addresses the resolve found, in the order they came. */
        public val addresses: List<String>
    ) : ProbeAnswer()

    /** `TcpQuery` — a noop out, and whatever came back. */
    public data class Tcp(
        /** `tcp_send` — `0` and above is a noop that went out. */
        public val sent: Int,
        /** `tcp_receive` — `0` and above is an answer that came back. */
        public val received: Int,
        /** Whether what came back was the answer to the noop that went out. */
        public val isNoopResponse: Boolean,
        /** How long the probe took, in milliseconds. */
        public val rtt: Long
    ) : ProbeAnswer()

    /** `SendHttpQuery` — the answer of the net-check CGI. */
    public data class Http(
        /** The return value of the probe: `0` and above is one that worked. */
        public val errorCode: Int,
        /** The HTTP status of the answer. */
        public val statusCode: Int,
        /** How long the probe took, in milliseconds. */
        public val rtt: Long
    ) : ProbeAnswer()

    /** `PingQuery::RunPingQuery` — one run of pings. */
    public data class Ping(
        /** The return value of the probe: `0` and above is one that worked. */
        public val errorCode: Int,
        /** How long the probe took, in milliseconds. */
        public val rtt: Long,
        /** `PingStatus::loss_rate` — `1.0` is every ping of the run lost. */
        public val lossRate: Float,
        /** `PingStatus::avgrtt` — what the report's `rttStr` is made of. */
        public val averageRTT: Float
    ) : ProbeAnswer()
}
