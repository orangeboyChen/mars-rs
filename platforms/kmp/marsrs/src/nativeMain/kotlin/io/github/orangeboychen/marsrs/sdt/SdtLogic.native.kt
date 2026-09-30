package io.github.orangeboychen.marsrs.sdt

import io.github.orangeboychen.marsrs.Held
import io.github.orangeboychen.marsrs.net.ffi.MARS_SDT_ERR_NO_CHECK
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
import kotlin.native.ThreadLocal
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
     * One document and not one per run: two runs since the last [takeReport]
     * are two documents the C ABI answered, and what the common API promises
     * is one — which is why [putBy] merges the [details] of the one it is
     * given into the one that is already here and not append it to a list.
     *
     * Replaced whole and never written into, and an [AtomicReference] of an
     * immutable [String] and not a `MutableList` two threads share: a run puts
     * a report by on the thread that ran it and a [takeReport] takes one on
     * the thread that asked, and there is nothing here to lock the two with —
     * `synchronized` is the JVM's, `platform.posix` has no mutex on
     * `mingwX64`, and a coroutine's `Mutex` is one only a `suspend` function
     * can wait on.
     */
    private val pending = AtomicReference("")

    /**
     * Whether the thread this is read on is inside a run of the checks, which
     * is what refuses a second one: see [runChecks], where the refusal is and
     * where the reason is.
     *
     * One flag per thread and not one flag for the whole process: what has to
     * be refused is the run a *probe* of a run asks for, which is a run the
     * thread asking is already inside, and a run another thread asks for is
     * not that one — it is a run the C ABI makes wait behind the first, which
     * is what the other `actual` does with its own lock as well.
     */
    @ThreadLocal
    private var running = false

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

    // `> 0` and not `!= 0`, the way every other question of this kind is
    // asked in this tree: the C ABI answers `0` for no and `1` for yes, and
    // `MARS_SDT_ERR_PANIC` — `-1` — for a panic it caught inside the call,
    // which `!= 0` reads back as a check in flight.
    public actual fun isChecking(): Boolean = mars_sdt_is_checking() > 0

    public actual fun plan(): List<Check>? {
        val count = mars_sdt_plan(null, 0u).toInt()
        if (count <= 0) {
            return emptyList()
        }
        return memScoped {
            val checks = allocArray<MarsSdtCheckVar>(count)
            val written = mars_sdt_plan(checks, count.toUInt()).toInt()
            // What `mars_sdt_plan` answers is how long the plan is, and not how
            // much of it it wrote: a plan that grew between the two calls — a
            // `startActiveCheck` on another thread — is one it filled the buffer
            // with `count` of, and reading `written` of it would read past the
            // array. What was written is at most what there was room for.
            (0 until written.coerceAtMost(count)).mapNotNull { index -> checkOf(checks[index].toInt()) }
        }
    }

    public actual fun runChecks(networkType: Int, probe: IProbe): Boolean {
        // A run started from inside a run — from one of the four probes, or from
        // the callback the report is handed to — is refused and not run, which
        // is what the common API promises: the C ABI holds its own mutex across
        // every check of the run, so a second `mars_sdt_run_checks` on the
        // thread that is inside the first never comes back, and the `finally`
        // below this takes the probe away from the run that is still asking it.
        //
        // Android's `actual` asks `Thread.holdsLock` for the same thing, which
        // is a question about one thread: the flag above is one per thread, so
        // the run a second thread asks for is not refused, it waits behind the
        // first the way it does there.
        if (running) {
            return false
        }
        val box = ProbeBox(probe)
        val reference = StableRef.create(box)
        return try {
            running = true
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
            // A run of no checks answers `MARS_SDT_ERR_NO_CHECK`, and it is a
            // run the Android `actual` reports: the bridge hands the callback
            // the results the run produced, which for none is the document of
            // no results, so an app that listened is told its run was over
            // and not left waiting for a report that never comes. It is
            // [takeReport] that answers `null` for that document.
            if (ran == MARS_SDT_OK || ran == MARS_SDT_ERR_NO_CHECK) {
                val report = drainReport()
                if (report != null) {
                    putBy(report)
                }
                // A callback that threw is answered here and not by the run:
                // the report was put by before it was called, so what an app
                // that asks is handed is the report either way, and the JNI
                // bridge behind the Android `actual` clears a pending
                // exception of its own, so the two seams answer the app with
                // the answer of the run and not with a throw of its callback.
                // Printed and not rethrown, the way a probe that threw is.
                // An `Error` of the app's is caught as well and not only an
                // `Exception`: what an uncaught one does on Kotlin/Native is
                // end the process, and the frame this is called from has
                // nothing above it to catch it — the run is over either way,
                // and the report of it was put by before this was called.
                try {
                    callBack?.reportSignalDetectResults(report)
                } catch (e: Throwable) {
                    e.printStackTrace()
                }
            }
            ran == MARS_SDT_OK
        } finally {
            running = false
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

    /**
     * Puts [report] by for the [takeReport] that comes looking for it, merged
     * into the document that is already here and not kept beside it: the
     * [details] of two runs are one array of checks in the answer an app is
     * given, the way the Android `actual` gives it, and a list of documents is
     * an answer of one run at a time.
     */
    private fun putBy(report: String) {
        while (true) {
            val waiting = pending.load()
            if (pending.compareAndSet(waiting, mergeReports(waiting, report))) {
                return
            }
        }
    }

    /** Every report nothing has taken yet, or `null` when there is none. */
    private fun takePending(): String? {
        while (true) {
            val waiting = pending.load()
            if (waiting.isEmpty()) {
                return null
            }
            if (pending.compareAndSet(waiting, "")) {
                return waiting
            }
        }
    }

    /**
     * The [details] of [report] behind the ones of [into]: a report is
     * `{"details":[ … ]}` and nothing beside it, so one document of two runs
     * is the comma between the two arrays.
     */
    private fun mergeReports(into: String, report: String): String {
        val added = report.removePrefix(DETAILS).removeSuffix("]}")
        if (added.isEmpty()) {
            return into
        }
        val had = into.removePrefix(DETAILS).removeSuffix("]}")
        val details = if (had.isEmpty()) added else "$had,$added"
        return "$DETAILS$details]}"
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
        pending.store("")
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

    /** What every report starts with, and the whole of one of no results. */
    private const val DETAILS = "{\"details\":["

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
        // The same guard the stn callback has, for the same reason: this runs
        // under a `staticCFunction`, so an exception the probe throws goes
        // into C with no frame above it to catch it, and Kotlin/Native ends
        // the process on one. A probe that threw is answered the way the
        // Android actual answers it — printed, and then not answered at all.
        // A `Throwable` and not an `Exception`, which is what the stn half
        // catches: an app's `Error` is as fatal here as its exception.
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
        } catch (e: Throwable) {
            e.printStackTrace()
            ProbeAnswer.None
        }
        // Its own guard, the way the stn callback has it, and for the reason
        // written there: the half that allocates is this one — `held.strings`
        // and `held.bytes` take `nativeHeap.allocArray`, and what that throws
        // when there is nothing left to give is an `OutOfMemoryError`, which
        // an `Exception` does not catch. An `Error` that escapes a
        // `staticCFunction` ends the process, so what the struct holds is
        // what `write` put into it first, which is nothing answered at all.
        try {
            write(answered, out)
        } catch (e: Throwable) {
            e.printStackTrace()
        }
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
