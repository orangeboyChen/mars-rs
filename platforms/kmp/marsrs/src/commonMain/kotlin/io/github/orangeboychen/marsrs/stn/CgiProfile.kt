package io.github.orangeboychen.marsrs.stn

/**
 * The connect a task ran on, as the app's report wants it: every reading is a
 * `gettickcount()`.
 *
 * Two of the eleven are ones only one of the two bridges fills in, and they
 * stay at their start on the other: [sendPacketFinishedTime] is the C ABI's
 * alone — `mars_stn.h` carries it and the C++'s Java `CgiProfile` has no field
 * for it — and [netType] is the C ABI's alone for the same reason. Every other
 * reading is the two of them alike.
 */
public class CgiProfile internal constructor(
    /** When the run began. */
    public val taskStartTime: Long,
    /** When the connect began. */
    public val startConnectTime: Long,
    /** When the connect came back, socket or no socket. */
    public val connectSuccessfulTime: Long,
    /** When the request went out. */
    public val startSendPacketTime: Long,
    /** When the write was done; `0` on Android, where the bridge reads no such tick. */
    public val sendPacketFinishedTime: Long,
    /** When the read of the answer began. */
    public val startReadPacketTime: Long,
    /** When the last of the answer came back. */
    public val readPacketFinishedTime: Long,
    /** How long the pair that won took to answer. */
    public val rtt: Long,
    /** The link the task went out on: one of [Task.E_SHORT], [Task.E_LONG] and [Task.E_BOTH]. */
    public val channelType: Int,
    /** How the task went out: `0` is the link's own choice, `1` TCP, `2` QUIC, `3` either. */
    public val protocolType: Int,
    /** The network the connect was made on; empty on Android, where the bridge hands no name. */
    public val netType: String
)
