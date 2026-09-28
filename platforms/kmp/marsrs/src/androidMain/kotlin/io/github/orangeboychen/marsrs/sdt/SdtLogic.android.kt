package io.github.orangeboychen.marsrs.sdt

/**
 * The Android `actual`: `libmarsrsxlog.so`, the JNI bridge of `crates/marsrs-jni`.
 *
 * The object is this class in this package because that is what the symbols the
 * Rust exports are called —
 * `Java_io_github_orangeboychen_marsrs_sdt_SdtLogic_startActiveCheck` and its
 * siblings — and so is the [Answer] below, whose name `marsrs-jni` puts in the
 * descriptor of the four probes it calls. Every member is `@JvmStatic` for the
 * same reason: the bridge asks JNI for
 * `io/github/orangeboychen/marsrs/sdt/SdtLogic` and calls the method on the
 * class, which is what the Android AAR declares too.
 *
 * The run is the one thing the two `actual`s reach differently: here the bridge
 * calls `reportSignalDetectResults` on this class from inside the run, because
 * that is what a JNI bridge can do, and on Kotlin/Native the C ABI has no such
 * call, so [runChecks] takes the report itself and hands it to [ICallBack]. An
 * app sees one API either way: a report once, through the callback it handed to
 * [setCallBack].
 */
public actual object SdtLogic {
    init {
        // `marsrsxlog`, the `crate-name` of `marsrs-jni`: loaded before the first
        // symbol of it is called, and loading it twice is nothing.
        System.loadLibrary(LIBRARY)
    }

    /** The callback the report of a run is handed to, which is the app's. */
    private var callBack: ICallBack? = null

    /** What [SdtLogic]'s own KDoc says, which is where the words are. */
    public actual fun interface ICallBack {
        public actual fun reportSignalDetectResults(resultsJson: String?)
    }

    /** What [SdtLogic]'s own KDoc says, which is where the words are. */
    public actual interface IProbe {
        public actual fun dns(host: String, timeoutMs: Int): ProbeAnswer

        public actual fun tcp(host: String, port: Int, timeoutMs: Int): ProbeAnswer

        public actual fun http(url: String, timeoutMs: Int): ProbeAnswer

        public actual fun ping(host: String, timeoutSec: Int): ProbeAnswer
    }

    public actual fun setCallBack(callback: ICallBack?) {
        callBack = callback
    }

    @JvmStatic
    public actual external fun startActiveCheck(
        longLink: Array<Link>,
        shortLink: Array<Link>,
        mode: Int,
        timeout: Int
    ): Boolean

    @JvmStatic
    public actual external fun cancelActiveCheck()

    @JvmStatic
    public actual external fun isChecking(): Boolean

    /**
     * What [SdtLogic.plan] says, which is where the words are.
     *
     * `@JvmName`, because the class has to carry exactly one method named
     * `plan`: JNI resolves the symbol by that name, and a second one — this —
     * would make the runtime look for the mangled long name
     * `Java_..._SdtLogic_plan__` instead, which `marsrs-jni` does not export.
     */
    @JvmName("planChecks")
    public actual fun plan(): List<Check>? = planNative()?.map { checkOf(it) }?.filterNotNull() ?: emptyList()

    @JvmStatic
    @JvmName("plan")
    private external fun planNative(): IntArray?

    public actual fun runChecks(networkType: Int, probe: IProbe): Boolean {
        // The four probes are asked of this class's own statics, from the native
        // call below this one on the stack, on this very thread: that is why the
        // probe is a field for as long as the run is and not an argument the
        // native side was handed. `finally` takes it away again even when a
        // probe threw.
        this.probe = probe
        return try {
            nativeRunChecks(networkType)
        } finally {
            this.probe = null
        }
    }

    @JvmStatic
    private external fun nativeRunChecks(networkType: Int): Boolean

    @JvmStatic
    public actual external fun takeReport(): String?

    @JvmStatic
    public actual external fun reset()

    @JvmStatic
    public actual external fun setHttpNetcheckCGI(cgi: String?)

    @JvmStatic
    public actual external fun httpNetcheckCGI(): String?

    // The report, and the four probes: what the bridge calls on this class while
    // a run is in flight, one static per call, the way `reportSignalDetectResults`
    // is the one the C++'s own `SdtLogic` gets.

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
    private fun onDnsQuery(host: String, timeoutMs: Int): Answer? = asked { it.dns(host, timeoutMs) }

    @JvmStatic
    private fun onTcpQuery(host: String, port: Int, timeoutMs: Int): Answer? = asked { it.tcp(host, port, timeoutMs) }

    @JvmStatic
    private fun onHttpQuery(url: String, timeoutMs: Int): Answer? = asked { it.http(url, timeoutMs) }

    @JvmStatic
    private fun onPingQuery(host: String, timeoutSec: Int): Answer? = asked { it.ping(host, timeoutSec) }

    /**
     * One probe, handed to the [IProbe] of the run that is in flight: `null` when
     * there is none, which every check reads as a failure — and when the app's own
     * probe threw, which is the C++'s `printStackTrace` and not a diagnosis that
     * stops.
     */
    private fun asked(probe: (IProbe) -> ProbeAnswer): Answer? {
        return try {
            val imp = this.probe
            if (imp == null) {
                NullPointerException("probe is null").printStackTrace()
                return null
            }
            probe(imp).toJni()
        } catch (e: Exception) {
            e.printStackTrace()
            null
        }
    }

    /** The probe of the run that is in flight: a field, because the bridge asks it of this class's statics. */
    private var probe: IProbe? = null

    /**
     * What `marsrs-jni` reads out of the object a probe answered: one class for
     * all four, because Java has no tagged union to put them in, so it is the
     * [Answer.probe] field that says which of them the rest of the fields are.
     */
    internal class Answer {
        /** Which probe this is the answer of; `0` when the app answered nothing. */
        @JvmField
        var probe: Int = 0

        @JvmField
        var errorCode: Int = 0

        /** How long the probe took — the `cost_time` the port measures around the call. */
        @JvmField
        var rtt: Long = 0

        @JvmField
        var ips: Array<String>? = null

        @JvmField
        var sent: Int = 0

        @JvmField
        var received: Int = 0

        @JvmField
        var isNoopResponse: Boolean = false

        @JvmField
        var statusCode: Int = 0

        @JvmField
        var lossRate: Float = 0f

        @JvmField
        var averageRTT: Float = 0f
    }

    /** The app's answer, in the one class the four probes are read out of. */
    private fun ProbeAnswer.toJni(): Answer = Answer().apply {
        when (val answer = this@toJni) {
            is ProbeAnswer.None -> Unit

            is ProbeAnswer.Dns -> {
                probe = PROBE_DNS
                errorCode = answer.errorCode
                rtt = answer.rtt
                ips = answer.addresses.toTypedArray()
            }

            is ProbeAnswer.Tcp -> {
                probe = PROBE_TCP
                rtt = answer.rtt
                sent = answer.sent
                received = answer.received
                isNoopResponse = answer.isNoopResponse
            }

            is ProbeAnswer.Http -> {
                probe = PROBE_HTTP
                errorCode = answer.errorCode
                rtt = answer.rtt
                statusCode = answer.statusCode
            }

            is ProbeAnswer.Ping -> {
                probe = PROBE_PING
                errorCode = answer.errorCode
                rtt = answer.rtt
                lossRate = answer.lossRate
                averageRTT = answer.averageRTT
            }
        }
    }

    /** `marsrsxlog` — the `crate-name` of `marsrs-jni`, i.e. the library every symbol above lives in. */
    private const val LIBRARY = "marsrsxlog"

    /** The `probe` integers of [Answer], which are how an app says it has no network to probe with. */
    private const val PROBE_DNS = 1

    private const val PROBE_TCP = 2

    private const val PROBE_HTTP = 3

    private const val PROBE_PING = 4
}
