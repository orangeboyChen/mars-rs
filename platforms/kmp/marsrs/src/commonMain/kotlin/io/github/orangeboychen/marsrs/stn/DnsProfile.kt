package io.github.orangeboychen.marsrs.stn

/**
 * How a dns question went, which is what [Question.dns] carries and what the
 * report of a resolve is made of.
 */
public class DnsProfile internal constructor(
    /** When the question was asked. */
    public val startTime: Long,
    /** When it came back; `0` while it has not. */
    public val endTime: Long,
    /** What was asked for. */
    public val host: String,
    /**
     * One of `ErrCmdType`'s: `0` ok, `1` false, `2` dial, `3` dns, `4` socket,
     * `5` http, `6` netmsgxp, `7` endecode, `8` server, `9` local, `10` canceld.
     */
    public val errType: Int,
    /** The code the question ended with. */
    public val errCode: Int,
    /** Which dns it was: `1` the app's, `2` the platform's. */
    public val dnsType: Int
)
