package io.github.orangeboychen.marsrs.sdt

/**
 * `SdtLogic` of `mars/sdt/sdt.h`: the diagnosis of the two links' hosts, which
 * is the other half of the net half on every platform of a Kotlin Multiplatform
 * project.
 *
 * One `expect`, two `actual`s: over the C ABI of `crates/marsrs-ffi`
 * (`mars_sdt.h`) on every Kotlin/Native target, and over the JNI bridge of
 * `crates/marsrs-jni` on Android. The names are the ones the Android AAR
 * publishes — `startActiveCheck`, `runChecks`, `takeReport` — because a shared
 * module that moves between `marsrs-kmp` and `marsrs` has nothing to rename when
 * it does.
 *
 * The one thing an app supplies is the network: DNS, TCP, HTTP and ping are four
 * probes the port refuses to open for itself, so they cross the boundary as the
 * four methods of [IProbe], and [runChecks] drives the run the C++ drives from
 * its `__RunOn` thread. Answer [ProbeAnswer.None] and the check that asked
 * fails, and a failed check ends the run — which is what a host with no network
 * gets. A ping is the one exception: a ping nobody sent is a check that did not
 * run, so it is left out of the report and the run goes on behind it.
 *
 * A probe is asked while the process-wide diagnosis is held, so it must not call
 * another [SdtLogic]: everything it needs is in the query it is given.
 */
public expect object SdtLogic {
    /** Sets the callback the results of the checks are reported to. */
    public fun setCallBack(callback: ICallBack?)

    /**
     * `StartActiveCheck` — a diagnosis of the two links' hosts.
     *
     * @param longLink the long link's hosts, one [Link] per host name
     * @param shortLink the short link's, which is what the HTTP check asks
     * @param mode the [CheckMode] bits, which is what the plan is made of
     * @param timeout milliseconds, or `0` — or less, which is read as `0` — for
     *   a run with no timeout of its own: every probe keeps the default of its kind
     * @return `false` when no check was started: one that is already in flight,
     *   or a `mode` with no check in it. [isChecking] tells the two apart —
     *   `true` is a check of somebody else's to wait for, and `false` is a
     *   request that was never taken. The C ABI the Kotlin/Native `actual`
     *   calls is the seam that answers with the reason instead:
     *   `MARS_SDT_ERR_BUSY` and `MARS_SDT_ERR_BAD_ARG`.
     */
    public fun startActiveCheck(longLink: Array<Link>, shortLink: Array<Link>, mode: Int, timeout: Int): Boolean

    /** `CancelActiveCheck` — the check in flight is asked to stop; what is left of the plan is not run. */
    public fun cancelActiveCheck()

    /** `IsChecking` — whether a check is in flight: one that was started and not run yet, or one that is running. */
    public fun isChecking(): Boolean

    /** `GetSdtingPlan` — the checks the request in flight is going to make, in the order they run. */
    public fun plan(): List<Check>?

    /**
     * `RunCheck` — the planned checks, one probe per check, over the network
     * [probe] answers with, and a report of what they recorded.
     *
     * This is the `__RunOn` thread of the C++, driven by the caller: the port has
     * no threads of its own, so the run happens on the thread that calls this and
     * does not come back until every probe of every planned check has answered.
     * `networkType` is the `comm::getNetInfo()` every check writes into its
     * profiles, which is this app's to answer.
     *
     * The report reaches the app the way it always does: [ICallBack], which the
     * run calls once it is over. [takeReport] hands the same document over to an
     * app that would rather ask for it.
     *
     * @param networkType what `PlatformComm.getNetInfo` answers on Android, and
     *                    the caller's own elsewhere
     * @param probe the four probes, asked while this runs and not after
     * @return `false` when there was no check in flight, when the one there was
     *         got cancelled before its first check, and when this was called
     *         from inside a run of it — which is what a probe or a callback
     *         that starts a second one is answered with, rather than with a
     *         second run
     */
    public fun runChecks(networkType: Int, probe: IProbe): Boolean

    /**
     * The JSON of everything the checks have reported since the last call — the
     * same document [ICallBack] was handed — or `null` when there was nothing to
     * take. Taking it empties it: the next call reports what happened since.
     */
    public fun takeReport(): String?

    /**
     * `Reset` — a diagnosis made again from nothing: the check in flight, the
     * plan it was running and every result waiting to be taken. The callback and
     * the CGI are not: they are the caller's.
     */
    public fun reset()

    /** `SetHttpNetcheckCGI` — the URL the HTTP check asks for. */
    public fun setHttpNetcheckCGI(cgi: String?)

    /** `HttpNetcheckCGI` — the URL the HTTP check asks for. */
    public fun httpNetcheckCGI(): String?

    /** The results of the checks that have finished, as the JSON the C++ hands its own `SdtLogic.ICallBack`. */
    public fun interface ICallBack {
        /**
         * The JSON of the report.
         *
         * This is called on the thread that ran the checks, which is the thread
         * that called [runChecks], and it is called before that call comes back
         * — but after the run itself is over and the diagnosis has been let go,
         * which is the difference that matters: [isChecking], [plan] and
         * [takeReport] are questions an app asks from here as a matter of
         * course, and every one of them is answered. What a callback should
         * still not do is start a second run on this thread: it would run its
         * checks inside the [runChecks] the caller is still waiting on.
         */
        public fun reportSignalDetectResults(resultsJson: String?)
    }

    /**
     * The four probes a diagnosis runs, which is the network the port has none
     * of: `mars/sdt/src/checkimpl/` is four sockets in the C++, and none of them
     * is opened here.
     *
     * A check whose probe answered nothing records a failure, so this is what a
     * diagnosis is made of — and what makes one possible at all on a host that is
     * not a phone.
     */
    public interface IProbe {
        /**
         * `socket_gethostbyname` — the resolve of one host name.
         *
         * @param timeoutMs milliseconds, or `0` for the run's own default
         */
        public fun dns(host: String, timeoutMs: Int): ProbeAnswer

        /**
         * `TcpQuery` — a long-link noop out to `host:port`, and whatever comes
         * back. What goes on the wire is the long link's own package, which is
         * the app's to write.
         *
         * @param timeoutMs milliseconds, or `0` for the run's own default
         */
        public fun tcp(host: String, port: Int, timeoutMs: Int): ProbeAnswer

        /**
         * `SendHttpQuery` — the request that tells the net-check CGI somebody is
         * looking for it.
         *
         * @param timeoutMs milliseconds, or `0` for the run's own default
         */
        public fun http(url: String, timeoutMs: Int): ProbeAnswer

        /**
         * `RunPingQuery` — the ICMP ping of one host.
         *
         * @param timeoutSec **seconds**, and not milliseconds: that is what the
         *     C++ hands a ping, and `0` is the run's own default
         */
        public fun ping(host: String, timeoutSec: Int): ProbeAnswer
    }
}
