// The constants below carry the name the C++ project's Java gives them, spelled
// the way Kotlin spells a constant: `K_PING_CHECK` there is `K_PING_CHECK` here.
// The JNI reaches a constant by the number it carries and not by its name, so
// nothing on the Rust side had to change with them.

package io.github.orangeboychen.marsrs.sdt

import android.util.Log
import io.github.orangeboychen.marsrs.Mars

/**
 * Every `external` is a static of this very class — that is what makes them the
 * `Java_io_github_orangeboychen_marsrs_sdt_SdtLogic_*` symbols `marsrs-jni`
 * exports — and four of them are asked from the native side while a diagnosis
 * runs, which is why there are private statics beside them: the four probes
 * below forward to the [IProbe] the app handed to [runChecks], the way
 * `reportSignalDetectResults` forwards to the [ICallBack] the app handed to
 * [setCallBack].
 *
 * The C++'s Java declares two `native` methods and no more, because there the
 * diagnosis is started from inside the C++, which has the sockets and the
 * threads a run needs. The port has neither, so starting one — and answering
 * the probes it asks — is the app's call and not the port's. That is what the
 * rest of this class is.
 */
object SdtLogic {

    const val TAG: String = "mars.SdtLogic"

    init {
        Mars.loadDefaultMarsLibrary()
    }

    object NetCheckType {
        const val K_PING_CHECK: Int = 0
        const val K_DNS_CHECK: Int = 1
        const val K_NEW_DNS_CHECK: Int = 2
        const val K_TCP_CHECK: Int = 3
        const val K_HTTP_CHECK: Int = 4
    }

    /**
     * The `NET_CHECK_*` bits of the `mode` a diagnosis is started with: a set,
     * like the C++'s, so `K_BASIC or K_LONG` is a run of both.
     */
    object CheckMode {
        /** `NET_CHECK_BASIC` — the ping and the DNS check. */
        const val K_BASIC: Int = 1

        /** `NET_CHECK_LONG` — the TCP check, against the long link's hosts. */
        const val K_LONG: Int = 1 shl 1

        /** `NET_CHECK_SHORT` — the HTTP check, against the net-check CGI. */
        const val K_SHORT: Int = 1 shl 2
    }

    /**
     * Which probe one query asks for: the `probe` of an [Answer], which is how
     * an app says it has no network to probe with — the check that asked reads
     * that as a failure and ends the run, and a ping is a check that did not
     * run, so the run goes on behind it.
     */
    object Probe {
        /** Nobody answered. */
        const val K_NOTHING: Int = 0

        /** `socket_gethostbyname` — the resolve of one host name. */
        const val K_DNS: Int = 1

        /** `TcpQuery` — a long-link noop out, and whatever comes back. */
        const val K_TCP: Int = 2

        /** `SendHttpQuery` — the request that tells the net-check CGI somebody is looking for it. */
        const val K_HTTP: Int = 3

        /** `PingQuery::RunPingQuery`. */
        const val K_PING: Int = 4
    }

    object TcpCheckErrCode {
        const val K_TCP_SUCC: Int = 0
        const val K_TCP_NON_ERR: Int = 1
        const val K_SELECT_ERR: Int = -1
        const val K_PIPE_INTR: Int = -2
        const val K_SND_RCV_ERR: Int = -3
        const val K_ASSERT_ERR: Int = -4
        const val K_TIMEOUT_ERR: Int = -5
        const val K_SELECT_EXP_ERR: Int = -6
        const val K_PIPE_EXP: Int = -7
        const val K_CONNECT_ERR: Int = -8
        const val K_TCP_RESP_ERR: Int = -9
    }

    /**
     * One host name, and the ip/port pairs filed under it: one entry of the
     * `CheckIPPorts` a diagnosis is started with, which is what the report
     * names a result by.
     *
     * The nth host goes with the nth port; a list with no partner is cut short.
     */
    class Link(val name: String, val hosts: Array<String>, val ports: IntArray)

    /**
     * What one probe answers.
     *
     * The four probes do not answer the same readings, and Java has no
     * tagged union to put them in, so this is one class with a field for each
     * and a factory per probe: the fields are read for the one [probe] names
     * and ignored for the rest, which is why every factory fills in its own and
     * leaves the others at zero.
     *
     * [rtt] is the `cost_time` the C++ measures around the call — how long the
     * probe took, which is what spends the timeout of the run.
     */
    class Answer private constructor(
        /** Which probe this is the answer of; [Probe.K_NOTHING] when there is none. */
        val probe: Int,
        /** `error_code` — the return value of the probe: `0` and above is one that worked. */
        val errorCode: Int,
        /** How long the probe took, in milliseconds. */
        val rtt: Long,
        /** The addresses a resolve found, in the order they came. */
        val ips: Array<String>?,
        /** `tcp_send` — `0` and above is a noop that went out. */
        val sent: Int,
        /** `tcp_receive` — `0` and above is an answer that came back. */
        val received: Int,
        /** Whether what came back was the answer to the noop that went out. */
        val isNoopResponse: Boolean,
        /** The HTTP status of the answer to the net-check CGI. */
        val statusCode: Int,
        /** `PingStatus::loss_rate` — `1.0` is every ping of the run lost. */
        val lossRate: Float,
        /** `PingStatus::avgrtt` — what the report's `rttStr` is made of. */
        val averageRTT: Float
    ) {
        companion object {
            /** Nobody answered: a host with no network to probe with. */
            @JvmStatic
            fun nothing(): Answer = Answer(Probe.K_NOTHING, 0, 0, null, 0, 0, false, 0, 0f, 0f)

            /** The resolve of one host name: its return value, how long it took, and what it found. */
            @JvmStatic
            fun dns(errorCode: Int, rtt: Long, ips: Array<String>): Answer =
                Answer(Probe.K_DNS, errorCode, rtt, ips, 0, 0, false, 0, 0f, 0f)

            /** A noop that went out, and what came back. */
            @JvmStatic
            fun tcp(sent: Int, received: Int, isNoopResponse: Boolean, rtt: Long): Answer =
                Answer(Probe.K_TCP, 0, rtt, null, sent, received, isNoopResponse, 0, 0f, 0f)

            /** The answer of the net-check CGI. */
            @JvmStatic
            fun http(errorCode: Int, statusCode: Int, rtt: Long): Answer =
                Answer(Probe.K_HTTP, errorCode, rtt, null, 0, 0, false, statusCode, 0f, 0f)

            /** A run of pings: how many were lost, and how long the ones that came back took. */
            @JvmStatic
            fun ping(errorCode: Int, rtt: Long, lossRate: Float, averageRTT: Float): Answer =
                Answer(Probe.K_PING, errorCode, rtt, null, 0, 0, false, 0, lossRate, averageRTT)
        }
    }

    /** The signal detection callback interface, which starts a signal detection */
    interface ICallBack {
        /**
         * The results of the checks that have finished, as the JSON the C++
         * hands its own `SdtLogic.ICallBack`.
         *
         * This is called on the thread that ran the checks, which is the thread
         * that called [runChecks], and it is called before that call comes
         * back — but after the run has let the diagnosis go, which is the
         * difference that matters: [isChecking], [plan] and [startActiveCheck]
         * are questions an app asks from here as a matter of course, and every
         * one of them is answered. What a handler should still not do is start
         * a second run on this thread: it would run its checks inside the
         * [runChecks] the caller is still waiting on.
         */
        fun reportSignalDetectResults(resultsJson: String?)
    }

    /**
     * The four probes a diagnosis runs, which is the network the port has none
     * of: `mars/sdt/src/checkimpl/` is four sockets in the C++, and none of
     * them is opened here.
     *
     * A check whose probe answered nothing records a failure, so this is what a
     * diagnosis is made of — and what makes one possible at all on a host that
     * is not a phone.
     *
     * A probe is asked while the native side holds the process-wide diagnosis,
     * so it must not call back into this class — `startActiveCheck`,
     * `isChecking`, `plan`, another `runChecks` — from its answer: what it
     * asked would wait for the lock the thread it is running on already holds.
     * Everything a check needs is in the query it was given.
     */
    interface IProbe {
        /**
         * `socket_gethostbyname` — the resolve of one host name.
         * @param timeoutMs milliseconds, or `0` for the run's own default
         */
        fun dns(host: String, timeoutMs: Int): Answer

        /**
         * `TcpQuery` — a long-link noop out to `host:port`, and whatever comes
         * back. What goes on the wire is the long link's own package, which is
         * the app's to write.
         * @param timeoutMs milliseconds, or `0` for the run's own default
         */
        fun tcp(host: String, port: Int, timeoutMs: Int): Answer

        /**
         * `SendHttpQuery` — the request that tells the net-check CGI somebody is
         * looking for it.
         * @param timeoutMs milliseconds, or `0` for the run's own default
         */
        fun http(url: String, timeoutMs: Int): Answer

        /**
         * `RunPingQuery` — the ICMP ping of one host.
         * @param timeoutSec **seconds**, and not milliseconds: that is what the
         *     C++ hands a ping, and `0` is the run's own default
         */
        fun ping(host: String, timeoutSec: Int): Answer
    }

    @Volatile
    private var callBack: ICallBack? = null

    @Volatile
    private var probe: IProbe? = null

    /**
     * What makes a run one at a time: the native side holds the process-wide
     * diagnosis for as long as the run takes anyway, and [probe] is a field the
     * run reads and not an argument it was handed — so two runs at once would
     * let the second one's `finally` take the probe away from the first one's
     * checks, and hand the first one the second one's network while it was at
     * it. Waiting here is what the native lock would have made them do.
     */
    private val runLock = Any()

    /** Sets the signal detection callback instance: it is how the results reach the app */
    @JvmStatic
    fun setCallBack(callback: ICallBack?) {
        callBack = callback
    }

    /**
     * Starts a diagnosis of the two links' hosts, in `mode` and with `timeout`
     * milliseconds to spend on it.
     *
     * @param longLink the long link's hosts, one [Link] per host name
     * @param shortLink the short link's, which is what the HTTP check asks
     * @param mode the [CheckMode] bits, which is what the plan is made of
     * @param timeout milliseconds, or `0` — or less, which is read as `0` — for
     *   a run with no timeout of its own: every probe keeps the default of its kind
     * @return `false` when a check is already in flight
     */
    @JvmStatic
    external fun startActiveCheck(longLink: Array<Link>, shortLink: Array<Link>, mode: Int, timeout: Int): Boolean

    /** Asks the check in flight to stop: what is left of the plan is not run. */
    @JvmStatic
    external fun cancelActiveCheck()

    /** Whether a check is in flight — one that was started and not run yet, or one that is running. */
    @JvmStatic
    external fun isChecking(): Boolean

    /**
     * The checks the request in flight is going to make, in the order they run,
     * as [NetCheckType] integers.
     */
    @JvmStatic
    external fun plan(): IntArray?

    /**
     * Runs the checks of the request in flight — one probe per check, in order
     * — over the network [probe] answers with, and reports what they recorded.
     *
     * This is the `__RunOn` thread of the C++, driven by the app: the port has
     * no threads of its own, so the run happens on the thread that calls this
     * and does not come back until every probe of every planned check has
     * answered. `networkType` is the `comm::getNetInfo()` every check writes
     * into its profiles, which is this app's to answer — `PlatformComm`'s, if
     * nobody has set one.
     *
     * The report reaches the app the way it always does: [ICallBack], which the
     * run calls once it is over. [takeReport] hands the same document over to an
     * app that would rather ask for it.
     *
     * A second run waits for the first: the diagnosis is one process-wide value,
     * so this call does not come back while another thread is inside it.
     *
     * @param networkType what `PlatformComm.getNetInfo` answers
     * @param probe the four probes, asked while this runs and not after
     * @return `false` when there was no check in flight, when the one there was
     *     got cancelled before its first check, and when this call was made
     *     from inside a run of it — which is what a probe or the callback that
     *     starts a second one is answered with, rather than with a second run
     */
    @JvmStatic
    fun runChecks(networkType: Int, probe: IProbe): Boolean {
        // A run started from inside a run — from one of the four probes, or from
        // the [ICallBack] the report is handed to — is refused and not run:
        // `synchronized` is reentrant, so the inner run would be let in, and the
        // `finally` it ends with takes the probe away from the outer run, whose
        // checks from then on ask a `null` probe and are recorded as failures.
        if (Thread.holdsLock(runLock)) {
            Log.w(TAG, "runChecks from inside a run of it: the second one is not started")
            return false
        }
        // The four probes are asked of this class's own statics, from the native
        // call below this one on the stack, on this very thread: that is why the
        // probe is a field for as long as the run is and not an argument the
        // native side was handed. `finally` takes it away again even when a
        // probe threw.
        synchronized(runLock) {
            this.probe = probe
            return try {
                nativeRunChecks(networkType)
            } finally {
                this.probe = null
            }
        }
    }

    @JvmStatic
    private external fun nativeRunChecks(networkType: Int): Boolean

    /**
     * The JSON of everything the checks have reported since the last call — the
     * same document [ICallBack] was handed — or `null` when there was nothing
     * to take. Taking it empties it: the next call reports what happened since.
     */
    @JvmStatic
    external fun takeReport(): String?

    /**
     * A diagnosis made again from nothing: the check in flight, the plan it was
     * running and every result waiting to be taken. The callback and the CGI
     * are not: they are the caller's.
     */
    @JvmStatic
    external fun reset()

    /** Sets the URI of an HTTP connectivity check */
    @JvmStatic
    external fun setHttpNetcheckCGI(requestURI: String?)

    /** The URI of an HTTP connectivity check, which is the URL the HTTP check asks for. */
    @JvmStatic
    external fun httpNetcheckCGI(): String?

    /** Gets the modules the native side has loaded */
    @JvmStatic
    private external fun getLoadLibraries(): ArrayList<String>?

    @JvmStatic
    private fun reportSignalDetectResults(resultsJson: String?) {
        try {
            val imp = callBack
            if (imp == null) {
                NullPointerException("callback is null").printStackTrace()
                return
            }
            imp.reportSignalDetectResults(resultsJson)
        } catch (e: Exception) {
            e.printStackTrace()
        }
    }

    @JvmStatic
    private fun onDnsQuery(host: String, timeoutMs: Int): Answer? = ask { it.dns(host, timeoutMs) }

    @JvmStatic
    private fun onTcpQuery(host: String, port: Int, timeoutMs: Int): Answer? = ask { it.tcp(host, port, timeoutMs) }

    @JvmStatic
    private fun onHttpQuery(url: String, timeoutMs: Int): Answer? = ask { it.http(url, timeoutMs) }

    @JvmStatic
    private fun onPingQuery(host: String, timeoutSec: Int): Answer? = ask { it.ping(host, timeoutSec) }

    /**
     * One probe, handed to the [IProbe] of the run that is in flight: `null`
     * when there is none, which every check but the ping reads as a failure — a
     * ping nobody sent is a check that did not run — and when the app's own
     * probe threw, which is the C++'s `e.printStackTrace()` and not a diagnosis
     * that stops.
     */
    private fun ask(probe: (IProbe) -> Answer): Answer? {
        return try {
            val imp = this.probe
            if (imp == null) {
                NullPointerException("probe is null").printStackTrace()
                return null
            }
            probe(imp)
        } catch (e: Exception) {
            e.printStackTrace()
            null
        }
    }
}
