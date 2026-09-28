package io.github.orangeboychen.marsrs.sdt

/**
 * One entry of the hosts a diagnosis is started with: the host and port pairs of
 * one link, under the name that link is known by — which is the name the report
 * files a result under.
 *
 * The nth host goes with the nth port; a list with no partner is cut short.
 */
public class Link(
    /** The name the hosts are known under. */
    public val name: String,
    /** The hosts, as an ip or a name. */
    public val hosts: Array<String>,
    /** The port of the host in the same place. */
    public val ports: IntArray
)

/**
 * One check of a request: `NetCheckType` of `mars/sdt/sdt.h`, which is also the
 * `detectType` of every entry of the report — so a plan and a report name the
 * same checks with the same integers.
 */
public enum class Check(internal val value: Int) {
    /** `kPingCheck`. */
    Ping(0),

    /** `kDnsCheck`. */
    Dns(1),

    /** `kNewDnsCheck` — the resolve against another server, to compare with. */
    NewDns(2),

    /** `kTcpCheck`. */
    Tcp(3),

    /** `kHttpCheck`. */
    Http(4),

    /** `kTracerouteCheck` — planned, but no probe is asked for it yet. */
    Traceroute(5),

    /** `kReqBufCheck` — planned, but no probe is asked for it yet. */
    ReqBuf(6)
}

/**
 * The check one integer of the port names: the `detectType` of a report and the
 * `MarsSdtCheck` of a plan are the same integers, so a plan the two `actual`s
 * read out of the port is read through this — and an integer the port names no
 * check with is a plan entry that is skipped, which is what keeps a check the
 * port grows tomorrow from being one that throws today.
 */
internal fun checkOf(value: Int): Check? = Check.entries.firstOrNull { it.value == value }

/**
 * The `NET_CHECK_*` bits of the `mode` a diagnosis is started with: a set, like
 * the C++'s, so `K_BASIC or K_LONG` is a run of both.
 */
public object CheckMode {
    /** `NET_CHECK_BASIC` — the ping and the DNS check. */
    public const val K_BASIC: Int = 1

    /** `NET_CHECK_LONG` — the TCP check, against the long link's hosts. */
    public const val K_LONG: Int = 1 shl 1

    /** `NET_CHECK_SHORT` — the HTTP check, against the net-check CGI. */
    public const val K_SHORT: Int = 1 shl 2
}
