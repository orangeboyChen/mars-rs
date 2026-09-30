package io.github.orangeboychen.marsrs.sdt

import io.github.orangeboychen.marsrs.Held
import io.github.orangeboychen.marsrs.net.ffi.MARS_SDT_ERR_NO_SPACE
import io.github.orangeboychen.marsrs.net.ffi.MARS_SDT_OK
import io.github.orangeboychen.marsrs.net.ffi.MarsSdtAnswer
import io.github.orangeboychen.marsrs.net.ffi.MarsSdtCheckVar
import io.github.orangeboychen.marsrs.net.ffi.MarsSdtDns
import io.github.orangeboychen.marsrs.net.ffi.MarsSdtHosts
import io.github.orangeboychen.marsrs.net.ffi.MarsSdtHttp
import io.github.orangeboychen.marsrs.net.ffi.MarsSdtIpPort
import io.github.orangeboychen.marsrs.net.ffi.MarsSdtNothing
import io.github.orangeboychen.marsrs.net.ffi.MarsSdtPing
import io.github.orangeboychen.marsrs.net.ffi.MarsSdtQuery
import io.github.orangeboychen.marsrs.net.ffi.MarsSdtTcp
import io.github.orangeboychen.marsrs.net.ffi.mars_sdt_cancel_active_check
import io.github.orangeboychen.marsrs.net.ffi.mars_sdt_http_netcheck_cgi
import io.github.orangeboychen.marsrs.net.ffi.mars_sdt_is_checking
import io.github.orangeboychen.marsrs.net.ffi.mars_sdt_plan
import io.github.orangeboychen.marsrs.net.ffi.mars_sdt_reset
import io.github.orangeboychen.marsrs.net.ffi.mars_sdt_run_checks
import io.github.orangeboychen.marsrs.net.ffi.mars_sdt_set_http_netcheck_cgi
import io.github.orangeboychen.marsrs.net.ffi.mars_sdt_start_active_check
import io.github.orangeboychen.marsrs.net.ffi.mars_sdt_take_report
import kotlin.concurrent.Volatile
import kotlin.concurrent.atomics.AtomicReference
import kotlin.concurrent.atomics.ExperimentalAtomicApi
import kotlinx.cinterop.ByteVar
import kotlinx.cinterop.COpaquePointer
import kotlinx.cinterop.CPointer
import kotlinx.cinterop.ExperimentalForeignApi
import kotlinx.cinterop.MemScope
import kotlinx.cinterop.StableRef
import kotlinx.cinterop.allocArray
import kotlinx.cinterop.asStableRef
import kotlinx.cinterop.get
import kotlinx.cinterop.memScoped
import kotlinx.cinterop.pointed
import kotlinx.cinterop.ptr
import kotlinx.cinterop.set
import kotlinx.cinterop.staticCFunction
import kotlinx.cinterop.toKString

/**
 * The Kotlin/Native `actual`: the C ABI of `crates/marsrs-ffi` (`mars_sdt.h`)
 * through cinterop, which is the same source set for every native target,
 * because the C ABI is the same header on all of them.
 *
 * Every member is one symbol of the header. The four probes are the one
 * function pointer `mars_sdt_run_checks` takes, which cannot capture the
 * [SdtLogic.IProbe] an app hands over: it lives in a [ProbeBox], the box is the
 * context every query is handed back with, and the run is over — and the box
 * released — when the call answers.
 *
 * The report is the one thing the C ABI hands back differently from the JNI
 * bridge: there the port calls `SdtLogic.reportSignalDetectResults` itself, here
 * it is [takeReport] the caller asks. So [runChecks] takes the report of a run
 * that went, hands it to [SdtLogic.ICallBack] when one is listening, and puts it
 * by for the [takeReport] that comes looking for it — which is what makes the
 * two platforms one API, and what makes the two ways of asking answer the same
 * document however the app chose to be told.
 */
@OptIn(ExperimentalForeignApi::class, ExperimentalAtomicApi::class)
public actual object SdtLogic {
    /** The callback the report of a run is handed to, which is the app's. */
    @Volatile
    private var callBack: ICallBack? = null

    /**
     * The reports of runs that are over and that nothing has taken yet.
     *
     * The C ABI has one buffer and [drainReport] empties it, so a report handed
     * to a callback is a report a later [takeReport] would find gone: what the
     * common API promises is one document either way — the callback gets it, and
     * an app that would rather ask keeps its own copy to ask for.
     *
     * Kept whether or not a callback was listening, because the other `actual`
     * does: Android's [takeReport] is an `external` over the results the bridge
     * keeps, and `reportSignalDetectResults` is a call it makes beside keeping
     * them and not instead of it, so an app there is handed a document whether
     * it listened or asked. What an app that never asks pays for the ones it
     * leaves is what it pays on Android today — a run of two links' hosts is a
     * few hundred bytes — and [reset] is what throws them away.
     *
     * Replaced whole and never written into, and an [AtomicReference] of an
     * immutable [List] and not a `MutableList` two threads share: a run puts a
     * report by on the thread that ran it and a [takeReport] takes one on the
     * thread that asked, and there is nothing here to lock the two with —
     * `synchronized` is the JVM's, `platform.posix` has no mutex on
     * `mingwX64`, and a coroutine's `Mutex` is one only a `suspend` function
     * can wait on.
     */
    private val pending = AtomicReference<List<String>>(emptyList())

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

    public actual fun startActiveCheck(
        longLink: Array<Link>,
        shortLink: Array<Link>,
        mode: Int,
        timeout: Int
    ): Boolean = memScoped {
        val started = mars_sdt_start_active_check(
            hosts(longLink),
            longLink.size.toUInt(),
            hosts(shortLink),
            shortLink.size.toUInt(),
            mode,
            // A timeout below zero is one that was not given, which is what
            // `0` says on the C side. `(-1).toUInt()` is forty-two hundred
            // million milliseconds — a number no probe was asked for, and the
            // one the JNI path would read as `0` instead.
            timeout.coerceAtLeast(0).toUInt()
        )
        started == MARS_SDT_OK
    }

    public actual fun cancelActiveCheck() {
        mars_sdt_cancel_active_check()
    }

    public actual fun isChecking(): Boolean = mars_sdt_is_checking() != 0

    public actual fun plan(): List<Check>? {
        val count = mars_sdt_plan(null, 0u).toInt()
        if (count <= 0) {
            return emptyList()
        }
        return memScoped {
            val checks = allocArray<MarsSdtCheckVar>(count)
            val written = mars_sdt_plan(checks, count.toUInt()).toInt()
            (0 until written).mapNotNull { index -> checkOf(checks[index].toInt()) }
        }
    }

    public actual fun runChecks(networkType: Int, probe: IProbe): Boolean {
        val box = ProbeBox(probe)
        val reference = StableRef.create(box)
        return try {
            val ran = mars_sdt_run_checks(
                reference.asCPointer(),
                staticCFunction(::probed),
                networkType
            )
            // The report of the run, which is the one thing the C ABI has no
            // callback of its own for: taken here, put by for the `takeReport`
            // that comes looking for it, and handed to the callback as well, the
            // way the JNI bridge hands it over from inside the run. The take
            // empties the buffer it takes from, so a report that was not put by
            // is one no later `takeReport` finds: the copy is what lets an app
            // that installed a callback still ask, which is the same answer the
            // Android `actual` gives it.
            if (ran == MARS_SDT_OK) {
                val report = drainReport()
                if (report != null) {
                    putBy(report)
                }
                callBack?.reportSignalDetectResults(report)
            }
            ran == MARS_SDT_OK
        } finally {
            reference.dispose()
            box.dispose()
        }
    }

    public actual fun takeReport(): String? {
        // What a run is holding for a caller that asks, and otherwise what the
        // C ABI is: one document either way, and taking it empties it.
        //
        // A document of no results is the C ABI's way of saying it was holding
        // none — no count of the results comes with a report, so `{"details":[]}`
        // is all there is to go on — and it is the one `null` this promises,
        // the way the Android `actual` answers it: an app that asks whether
        // anything was reported is answered without parsing a document.
        return (takePending() ?: drainReport())?.takeUnless { it == NO_RESULTS }
    }

    /** Puts [report] by for the [takeReport] that comes looking for it. */
    private fun putBy(report: String) {
        while (true) {
            val waiting = pending.load()
            if (pending.compareAndSet(waiting, waiting + report)) {
                return
            }
        }
    }

    /** The oldest report nothing has taken yet, or `null` when there is none. */
    private fun takePending(): String? {
        while (true) {
            val waiting = pending.load()
            if (waiting.isEmpty()) {
                return null
            }
            if (pending.compareAndSet(waiting, waiting.drop(1))) {
                return waiting.first()
            }
        }
    }

    /**
     * The document the C ABI is holding, which is the one of no results when it
     * is holding none: [takeReport] is what answers `null` for that, and not
     * this. `null` here is an error the C ABI answered instead of a document —
     * a buffer that never grew big enough, or a diagnosis that is gone.
     *
     * Not [takeReport], which hands over a report a run put by first: a run that
     * reads this through that one would be handed the report of an *earlier*
     * run whenever one was waiting.
     */
    private fun drainReport(): String? {
        var size = REPORT_BUFFER_SIZE
        while (size <= REPORT_BUFFER_LIMIT) {
            val (text, written) = memScoped {
                val buffer = allocArray<ByteVar>(size)
                val wrote = mars_sdt_take_report(buffer, size.toUInt())
                (if (wrote < 0) null else buffer.toKString()) to wrote
            }
            if (text != null) {
                return text
            }
            // The C ABI answers "no room" and not the size it needs, so the only
            // way to ask for more is to ask again with more. It keeps the results
            // it could not hand over, so this retry is not one that takes an empty
            // report.
            if (written != MARS_SDT_ERR_NO_SPACE) {
                return null
            }
            size += size
        }
        return null
    }

    public actual fun reset() {
        // Every result waiting to be taken goes with the rest of the diagnosis,
        // which is what `mars_sdt_reset` says and what an app that resets before
        // it asks expects: a report of a run it threw away is not one it is
        // handed afterwards.
        pending.store(emptyList())
        mars_sdt_reset()
    }

    public actual fun setHttpNetcheckCGI(cgi: String?) {
        mars_sdt_set_http_netcheck_cgi(cgi)
    }

    public actual fun httpNetcheckCGI(): String? = memScoped {
        val buffer = allocArray<ByteVar>(CGI_BUFFER_SIZE)
        val written = mars_sdt_http_netcheck_cgi(buffer, CGI_BUFFER_SIZE.toUInt())
        if (written < 0) null else buffer.toKString()
    }

    /** The two links' hosts, as the `MarsSdtHosts` the C ABI reads; every copy lives in this scope. */
    private fun MemScope.hosts(links: Array<Link>): CPointer<MarsSdtHosts> {
        val array = allocArray<MarsSdtHosts>(links.size)
        links.forEachIndexed { index, link ->
            array[index].name = cstring(link.name)
            // The nth host with the nth port, and a host whose partner is
            // missing is not one the run probes: [Link] says an unmatched list
            // is cut short, and the JNI bridge pairs the two the same way, so
            // one `Link` means one thing on either `actual`. A port of `0` for
            // a host that has none is a TCP check of nowhere, and it is a check
            // only Kotlin/Native would have run.
            val pairs = minOf(link.hosts.size, link.ports.size)
            val ports = allocArray<MarsSdtIpPort>(pairs)
            for (pair in 0 until pairs) {
                ports[pair].ip = cstring(link.hosts[pair])
                ports[pair].port = portOf(link.ports[pair])
            }
            array[index].ports = ports
            array[index].port_count = pairs.toUInt()
        }
        return array
    }

    /** The largest port an `unsigned short` carries. */
    private const val MAX_PORT = 65_535

    /**
     * One port of a link, as the `unsigned short` the C ABI carries.
     *
     * A number the `unsigned short` cannot hold is `0` and not its own low
     * sixteen bits: `70000` truncated is `4464` and `-1` is `65535`, which are
     * ports another host really listens on, and a diagnosis that probes them is
     * one that reports on the wrong place. `0` is the port the JNI bridge
     * answers for the same number, which is what makes one `Link` mean one
     * thing on either `actual`.
     */
    private fun portOf(port: Int): UShort = if (port in 0..MAX_PORT) port.toUShort() else 0.toUShort()

    /** The buffer `httpNetcheckCGI` writes into; a URL never fills it. */
    private const val CGI_BUFFER_SIZE = 1_024

    /** Where `takeReport` starts: a run of two links' hosts is a few hundred bytes of JSON. */
    private const val REPORT_BUFFER_SIZE = 4_096

    /** Where it stops: a diagnosis that reports more than a megabyte is not one an app parses. */
    private const val REPORT_BUFFER_LIMIT = 1_048_576

    /**
     * The document `report_json` writes for no results at all, which is what the
     * C ABI hands over when there was nothing to take: the one [takeReport]
     * answers `null` for.
     */
    private const val NO_RESULTS = "{\"details\":[]}"

    /**
     * One string the port reads, as a NUL-terminated copy in this scope: a
     * `const char*` field of a struct is a pointer and not the [String] a
     * `const char*` parameter of a symbol is, so a name and an ip are copied and
     * the copies kept alive by the scope of the call.
     */
    @OptIn(ExperimentalForeignApi::class)
    private fun MemScope.cstring(value: String): CPointer<ByteVar> {
        val encoded = value.encodeToByteArray()
        val pointer = allocArray<ByteVar>(encoded.size + 1)
        encoded.forEachIndexed { index, byte -> pointer[index] = byte }
        pointer[encoded.size] = 0
        return pointer
    }
}

/**
 * The four probes of one run, behind the context `mars_sdt_run_checks` hands
 * back with every query.
 *
 * The answer of a probe is read *after* the probe has returned, so the
 * addresses of a resolve cannot be the ones of the scope it ran in: they are
 * copies, and the box holds them ([Held]) until the next probe is asked.
 */
@OptIn(ExperimentalForeignApi::class)
internal class ProbeBox(private val probe: SdtLogic.IProbe) {
    private val held = Held()

    /** What one query asked, and what the app answered: written into [out]. */
    fun answer(query: MarsSdtQuery, out: CPointer<MarsSdtAnswer>) {
        held.clear()
        // The same guard the stn callback above it has, for the same reason:
        // this runs under a `staticCFunction`, so an exception the probe
        // throws goes into C with no frame above it to catch it, and
        // Kotlin/Native ends the process on one. A probe that threw is
        // answered the way the Android actual answers it — printed, and
        // then not answered at all.
        val host = query.host?.toKString() ?: ""
        val timeout = query.timeout.toInt()
        val answered = try {
            when (query.kind.toInt()) {
                PROBE_DNS -> probe.dns(host, timeout)
                PROBE_TCP -> probe.tcp(host, query.port.toInt(), timeout)
                PROBE_HTTP -> probe.http(host, timeout)
                PROBE_PING -> probe.ping(host, timeout)
                else -> ProbeAnswer.None
            }
        } catch (e: Exception) {
            e.printStackTrace()
            ProbeAnswer.None
        }
        write(answered, out)
    }

    fun dispose() {
        held.clear()
    }

    /** Zeroed first and then filled in for the kind it is, the way the header reads it. */
    private fun write(answer: ProbeAnswer, out: CPointer<MarsSdtAnswer>) {
        val native = out.pointed
        native.kind = MarsSdtNothing
        native.error_code = 0
        native.rtt = 0u
        native.ips = null
        native.ip_count = 0u
        native.sent = 0
        native.received = 0
        native.is_noop_resp = 0.toUByte()
        native.status_code = 0
        native.loss_rate = 0f
        native.avgrtt = 0f
        when (answer) {
            is ProbeAnswer.None -> Unit

            is ProbeAnswer.Dns -> {
                native.kind = MarsSdtDns
                native.error_code = answer.errorCode
                native.rtt = answer.rtt.toULong()
                native.ips = held.strings(answer.addresses)
                native.ip_count = answer.addresses.size.toUInt()
            }

            is ProbeAnswer.Tcp -> {
                native.kind = MarsSdtTcp
                native.rtt = answer.rtt.toULong()
                native.sent = answer.sent
                native.received = answer.received
                native.is_noop_resp = if (answer.isNoopResponse) 1.toUByte() else 0.toUByte()
            }

            is ProbeAnswer.Http -> {
                native.kind = MarsSdtHttp
                native.error_code = answer.errorCode
                native.rtt = answer.rtt.toULong()
                native.status_code = answer.statusCode
            }

            is ProbeAnswer.Ping -> {
                native.kind = MarsSdtPing
                native.error_code = answer.errorCode
                native.rtt = answer.rtt.toULong()
                native.loss_rate = answer.lossRate
                native.avgrtt = answer.averageRTT
            }
        }
    }

    private companion object {
        /** The `MarsSdtKind` integers, which the query carries and the answer is read for. */
        const val PROBE_DNS = 1

        const val PROBE_TCP = 2

        const val PROBE_HTTP = 3

        const val PROBE_PING = 4
    }
}

/**
 * The one function pointer the four probes are funnelled through: the box behind
 * the [StableRef] is rebuilt out of the context, and what the app answered is
 * written into the struct the run reads.
 */
@OptIn(ExperimentalForeignApi::class)
private fun probed(ctx: COpaquePointer?, query: CPointer<MarsSdtQuery>?, answer: CPointer<MarsSdtAnswer>?) {
    if (ctx == null || query == null || answer == null) {
        return
    }
    ctx.asStableRef<ProbeBox>().get().answer(query.pointed, answer)
}
