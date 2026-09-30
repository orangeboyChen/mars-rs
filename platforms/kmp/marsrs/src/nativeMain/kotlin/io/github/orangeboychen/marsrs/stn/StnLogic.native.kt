package io.github.orangeboychen.marsrs.stn

import io.github.orangeboychen.marsrs.Held
import io.github.orangeboychen.marsrs.net.ffi.MarsStnAnswer
import io.github.orangeboychen.marsrs.net.ffi.MarsStnAnswerDecoded
import io.github.orangeboychen.marsrs.net.ffi.MarsStnAnswerEncoded
import io.github.orangeboychen.marsrs.net.ffi.MarsStnAnswerEnded
import io.github.orangeboychen.marsrs.net.ffi.MarsStnAnswerFailed
import io.github.orangeboychen.marsrs.net.ffi.MarsStnAnswerIdentified
import io.github.orangeboychen.marsrs.net.ffi.MarsStnAnswerIps
import io.github.orangeboychen.marsrs.net.ffi.MarsStnAnswerLimit
import io.github.orangeboychen.marsrs.net.ffi.MarsStnAnswerNothing
import io.github.orangeboychen.marsrs.net.ffi.MarsStnAnswerYes
import io.github.orangeboychen.marsrs.net.ffi.MarsStnCgiProfile
import io.github.orangeboychen.marsrs.net.ffi.MarsStnDnsProfile
import io.github.orangeboychen.marsrs.net.ffi.MarsStnHeader
import io.github.orangeboychen.marsrs.net.ffi.MarsStnLonglinkConfig
import io.github.orangeboychen.marsrs.net.ffi.MarsStnQuestion
import io.github.orangeboychen.marsrs.net.ffi.MarsStnTask
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_clear_tasks
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_create_longlink
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_destroy_longlink
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_disable_longlink
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_due_time
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_gen_sequence_id
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_gen_task_id
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_has_task
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_keep_signalling
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_longlink_is_connected
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_longlink_is_connected_ext
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_makesure_longlink_connected
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_makesure_longlink_connected_ext
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_mark_main_longlink
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_noop_task_id
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_redo_tasks
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_reset
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_reset_and_init_encoder_version
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_run_pending
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_set_app
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_set_backup_ips
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_set_client_version
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_set_debug_ip
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_set_longlink_svr_addr
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_set_shortlink_svr_addr
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_set_signalling_strategy
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_start_task
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_stop_signalling
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_stop_task
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_touch_tasks
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_trig_nooping
import kotlin.concurrent.atomics.AtomicInt
import kotlin.concurrent.atomics.ExperimentalAtomicApi
import kotlinx.cinterop.ByteVar
import kotlinx.cinterop.COpaquePointer
import kotlinx.cinterop.CPointer
import kotlinx.cinterop.CPointerVar
import kotlinx.cinterop.ExperimentalForeignApi
import kotlinx.cinterop.MemScope
import kotlinx.cinterop.StableRef
import kotlinx.cinterop.UByteVar
import kotlinx.cinterop.UShortVar
import kotlinx.cinterop.alloc
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
 * The Kotlin/Native `actual`: the C ABI of `crates/marsrs-ffi` (`mars_stn.h`)
 * through cinterop, which is the same source set for every native target —
 * iOS, watchOS, tvOS, macOS, Linux and Windows — because the C ABI is the same
 * header on all of them.
 *
 * Every member is one symbol of the header, so a task this pipeline runs is a
 * task of the process-wide core the C++ keeps in a singleton. The eighteen
 * questions are the one function pointer of `mars_stn_set_app`, which cannot
 * capture the closure an app hands over: the closure lives in an [AppBox], the
 * box is the context every question is handed back with, and [StnLogic] is what
 * keeps it alive.
 *
 * The port asks while it holds the whole pipeline, so an `ask` must not call
 * another [StnLogic] — and it must be asked on a thread that is not the one
 * inside an ask, which is what the C ABI promises too.
 */
@OptIn(ExperimentalForeignApi::class, ExperimentalAtomicApi::class)
public actual object StnLogic {
    /**
     * The app that is installed, which is the only thing the C ABI does not give
     * back: `setApp` remembers it so that the next app releases it.
     *
     * Read and written under [swap], because `setApp` is three steps and not
     * one — remember the new app, hand it to the C ABI, release the old one —
     * and two threads left to run them in any order leave the C ABI holding
     * the context of a box the other one has already released. Making the
     * field atomic is not enough: it is the install and the release that have
     * to be one step, and not only the write of the field.
     */
    private var installed: StableRef<AppBox>? = null

    /**
     * The lock `setApp` holds over those three steps, and the only one in this
     * file: a question is asked with the logic held, so nothing else here can
     * wait for a thread that is inside one.
     *
     * A spin and not a mutex, which is what `kotlin.concurrent.atomics` gives
     * on every target this module builds, and an app is installed once — a
     * thread that waits is one that waits microseconds.
     */
    private val swap = AtomicInt(0)

    public actual fun setApp(ask: ((Question) -> Answer)?) {
        val reference = if (ask == null) null else StableRef.create(AppBox(ask))
        while (!swap.compareAndSet(0, 1)) {
            // another `setApp` is between two of its steps
        }
        val previous = installed
        try {
            if (reference == null) {
                mars_stn_set_app(null, null)
            } else {
                mars_stn_set_app(reference.asCPointer(), staticCFunction(::asked))
            }
            installed = reference
        } finally {
            swap.value = 0
        }
        // The box of the app before this one is held until the swap is over, and
        // not released by the assignment that replaces it: a question can be in
        // flight while this runs, and the callback rebuilds the box out of the
        // pointer it was handed, so a box freed before `mars_stn_set_app` has put
        // the new context in its place is a question that dereferences a freed
        // one. Both are locals of this call, so the old box dies here at the
        // earliest — after the swap, which is what the C ABI asks for: `ctx`
        // has to stay alive until another `mars_stn_set_app` takes its place.
        //
        previous?.dispose()
    }

    public actual fun reset() {
        mars_stn_reset()
    }

    public actual fun resetAndInitEncoderVersion(version: Int, name: String?) {
        mars_stn_reset_and_init_encoder_version(version, name)
    }

    public actual fun setLonglinkSvrAddr(host: String?, ports: IntArray?, debugIP: String?) {
        memScoped {
            mars_stn_set_longlink_svr_addr(
                host,
                ushorts(ports),
                ports?.size?.toUInt() ?: 0u,
                debugIP
            )
        }
    }

    public actual fun setShortlinkSvrAddr(port: Int, debugIP: String?) {
        mars_stn_set_shortlink_svr_addr(port.toUShort(), debugIP)
    }

    public actual fun setDebugIP(host: String?, ip: String?) {
        mars_stn_set_debug_ip(host, ip)
    }

    public actual fun setBackupIPs(host: String?, ips: Array<String>?) {
        memScoped {
            mars_stn_set_backup_ips(
                host,
                strings(ips?.toList() ?: emptyList()),
                ips?.size?.toUInt() ?: 0u
            )
        }
    }

    public actual fun startTask(task: Task) {
        // A negative id names no task here, the way the JNI `actual` reads one:
        // `-1` is `0xFFFF_FFFF` on the C side, which is the noop's own id — the
        // one the long link keeps itself alive with — and a task started under
        // it is one a noop of the link's would answer for.
        if (task.taskID < 0) return
        memScoped {
            mars_stn_start_task(task.native(this).ptr)
        }
    }

    public actual fun stopTask(taskID: Int) {
        // As in [startTask]: `-1` is the noop's id and not a task the app named,
        // and a `stopTask` of it would stop what the link's own heartbeat is.
        if (taskID < 0) return
        mars_stn_stop_task(taskID.toUInt())
    }

    // The same reading of a negative: no task has it, so none is in the queues.
    public actual fun hasTask(taskID: Int): Boolean = taskID >= 0 && mars_stn_has_task(taskID.toUInt()) > 0

    public actual fun redoTask() {
        mars_stn_redo_tasks()
    }

    public actual fun touchTasks() {
        mars_stn_touch_tasks()
    }

    public actual fun clearTask() {
        mars_stn_clear_tasks()
    }

    public actual fun runPending() {
        mars_stn_run_pending()
    }

    public actual fun dueTime(): Long? {
        val due = mars_stn_due_time()
        return if (due < 0) null else due
    }

    // The `1` or `0` the C ABI answers — whether there *was* a default link to
    // connect — is what the Swift package hands back and what the JNI bridge has
    // no room for, its symbol being a `void` one, so the common API answers
    // nothing and this discards it.
    public actual fun makesureLongLinkConnected() {
        mars_stn_makesure_longlink_connected()
    }

    public actual fun makesureLongLinkConnectedExt(name: String) {
        memScoped {
            mars_stn_makesure_longlink_connected_ext(name)
        }
    }

    public actual fun longLinkIsConnected(): Boolean = mars_stn_longlink_is_connected() > 0

    public actual fun longLinkIsConnectedExt(name: String): Boolean =
        memScoped { mars_stn_longlink_is_connected_ext(name) > 0 }

    public actual fun disableLongLink() {
        mars_stn_disable_longlink()
    }

    public actual fun noopTaskID(): Int = mars_stn_noop_task_id().toInt()

    public actual fun createLonglink(config: LonglinkConfig): Boolean = memScoped {
        mars_stn_create_longlink(config.native(this).ptr) == 0
    }

    // The five asks of this object that read a `1` or a `0` are `> 0` and not
    // `!= 0`: the C ABI answers `1` for yes, `0` for no and `MARS_STN_ERR_PANIC`
    // — which is `-1` — for a panic it caught inside the call, and `!= 0` reads
    // that panic as a yes: a link that is up, a link that is gone, when the call
    // never reached the net core. `0` is what the JNI `actual` answers for the
    // same question, so the two agree on either side of the seam.
    public actual fun destroyLonglink(name: String?): Boolean = mars_stn_destroy_longlink(name) > 0

    public actual fun markMainLonglink(name: String?): Boolean = mars_stn_mark_main_longlink(name) > 0

    public actual fun setSignallingStrategy(period: Long, keepTime: Long) {
        mars_stn_set_signalling_strategy(period, keepTime)
    }

    public actual fun keepSignalling() {
        mars_stn_keep_signalling()
    }

    public actual fun stopSignalling() {
        mars_stn_stop_signalling()
    }

    public actual fun setClientVersion(version: Int) {
        // `0` and not `0xFFFF_FFFF`, which is what a `-1` would read as: the
        // version goes out in every long-link package, and the JNI `actual`
        // clamps it the same way rather than hand the link a number of four
        // billion.
        mars_stn_set_client_version(version.coerceAtLeast(0).toUInt())
    }

    public actual fun genTaskID(): Int = mars_stn_gen_task_id().toInt()

    public actual fun genSequenceId(): Int = mars_stn_gen_sequence_id().toInt()

    public actual fun trigNooping() {
        mars_stn_trig_nooping()
    }

    /** The ports of the long link, which the C ABI takes as one array beside its count. */
    private fun MemScope.ushorts(ports: IntArray?): CPointer<UShortVar>? {
        if (ports == null) {
            return null
        }
        val array = allocArray<UShortVar>(ports.size)
        ports.forEachIndexed { index, port -> array[index] = port.toUShort() }
        return array
    }
}

/**
 * The app STN asks, behind the context `mars_stn_set_app` hands back with every
 * question.
 *
 * The answer is the other half of the seam: the pipeline reads the bytes a task
 * sends and the addresses a resolve found *after* the app has answered, so they
 * cannot be the pointers of the scope the ask ran in. They are copies, and the
 * box holds them ([Held]) until the next question is asked.
 */
@OptIn(ExperimentalForeignApi::class)
internal class AppBox(private val ask: (Question) -> Answer) {
    private val held = Held()

    /** What the app answered, written into the struct the pipeline reads. */
    fun answer(question: MarsStnQuestion, out: CPointer<MarsStnAnswer>) {
        // The answer of the question before this one has been read by the time
        // the next one is asked, which is what the header says and the only time
        // a copy is known to be dead.
        held.clear()
        // An exception that escapes this function escapes into C, and the
        // frame this one is called from is a `staticCFunction`: there is
        // nothing above it to catch it, and Kotlin/Native ends the process
        // on an uncaught one. A question an app answered badly then costs
        // the app its process, which is the one thing a callback the app
        // cannot see must not do. What is answered instead is the same
        // thing the Android actual answers: the question is printed, and
        // the pipeline is told nothing was answered at all.
        val answered = try {
            ask(question.toQuestion())
        } catch (e: Throwable) {
            e.printStackTrace()
            Answer.None
        }
        try {
            write(answered, out)
        } catch (e: Throwable) {
            // The half that allocates is this one, and what it catches is a
            // `Throwable` and not an `Exception`: `held.strings` and
            // `held.bytes` take `nativeHeap.allocArray`, and what that throws
            // when there is nothing left to give is an `OutOfMemoryError`,
            // which is an `Error`. What the struct holds then is what `write`
            // put into it first, which is nothing answered at all.
            e.printStackTrace()
        }
    }

    fun dispose() {
        held.clear()
    }

    /**
     * Zeroed first and then filled in for the kind it is: every field of a
     * `MarsStnAnswer` is read for the `kind` in it and left alone for the others,
     * which is what the header promises the pipeline does — and the struct it is
     * written into is the port's, of a value it does not say it zeroed.
     */
    @OptIn(ExperimentalForeignApi::class)
    private fun write(answer: Answer, out: CPointer<MarsStnAnswer>) {
        val native = out.pointed
        native.kind = MarsStnAnswerNothing
        native.yes = 0
        native.ips = null
        native.ip_count = 0u
        native.bytes = null
        native.byte_count = 0u
        native.error_code = 0
        native.handle = 0
        native.mode = 0
        native.hash = null
        native.hash_count = 0u
        native.cmdid = 0u
        native.limit = 0u
        when (answer) {
            is Answer.None -> Unit

            is Answer.Yes -> {
                native.kind = MarsStnAnswerYes
                native.yes = if (answer.yes) 1 else 0
            }

            is Answer.Addresses -> {
                native.kind = MarsStnAnswerIps
                native.ips = held.strings(answer.addresses)
                native.ip_count = answer.addresses.size.toUInt()
            }

            is Answer.Encoded -> {
                native.kind = MarsStnAnswerEncoded
                native.bytes = held.bytes(answer.bytes)
                native.byte_count = answer.bytes.size.toUInt()
            }

            is Answer.Failed -> {
                native.kind = MarsStnAnswerFailed
                native.error_code = answer.errorCode
            }

            is Answer.Decoded -> {
                native.kind = MarsStnAnswerDecoded
                native.error_code = answer.errorCode
                native.handle = answer.handle.value
            }

            is Answer.Ended -> {
                native.kind = MarsStnAnswerEnded
                native.error_code = answer.errorCode
            }

            is Answer.Identified -> {
                native.kind = MarsStnAnswerIdentified
                native.mode = answer.mode.value
                native.bytes = held.bytes(answer.bytes)
                native.byte_count = answer.bytes.size.toUInt()
                native.hash = held.bytes(answer.hash)
                native.hash_count = answer.hash.size.toUInt()
                native.cmdid = answer.cmdid.toUInt()
            }

            is Answer.Limit -> {
                native.kind = MarsStnAnswerLimit
                native.limit = answer.limit.toUInt()
            }
        }
    }
}

/**
 * One of the eighteen questions, as the [Question] the app reads: every string
 * and every buffer is copied out of the C struct, which is only good while the
 * ask it was handed to runs.
 */
@OptIn(ExperimentalForeignApi::class)
private fun MarsStnQuestion.toQuestion(): Question = Question().apply {
    kind = kindOf(this@toQuestion.kind.toInt())
    host = text(this@toQuestion.host)
    userID = text(this@toQuestion.user_id)
    channelID = text(this@toQuestion.channel_id)
    ipAddress = text(this@toQuestion.ip)
    port = this@toQuestion.port.toInt()
    cmdid = this@toQuestion.cmdid.toInt()
    taskID = this@toQuestion.taskid.toInt()
    sent = this@toQuestion.send
    received = this@toQuestion.recv
    channelSelect = this@toQuestion.channel_select
    sequence = this@toQuestion.sequence.toInt()
    body = bytes(this@toQuestion.body, this@toQuestion.body_count)
    hash = bytes(this@toQuestion.hash, this@toQuestion.hash_count)
    errType = this@toQuestion.err_type
    errCode = this@toQuestion.err_code
    profile = this@toQuestion.profile?.pointed?.toCgiProfile()
    profileJSON = this@toQuestion.profile_json?.toKString()
    dns = this@toQuestion.dns?.pointed?.toDnsProfile()
    netStatusAll = this@toQuestion.net_status_all
    netStatusLongLink = this@toQuestion.net_status_longlink
    linkStatus = this@toQuestion.link_status
    isLongLinkHost = this@toQuestion.longlink_host != 0
    checkType = this@toQuestion.check_type
    limit = this@toQuestion.limit.toInt()
    task = this@toQuestion.task?.pointed?.toTask()
}

/** The question the C ABI asked, by the integer it names it with. */
private fun kindOf(value: Int): Question.Kind =
    Question.Kind.entries.firstOrNull { it.value == value } ?: Question.Kind.Nothing

/** One NUL-terminated string of a question, copied out of the struct. */
@OptIn(ExperimentalForeignApi::class)
private fun text(value: CPointer<ByteVar>?): String = value?.toKString() ?: ""

/** `body` or `hash`: a buffer and its count, copied out of the struct. */
@OptIn(ExperimentalForeignApi::class)
private fun bytes(value: CPointer<UByteVar>?, count: UInt): ByteArray {
    val size = count.toInt()
    if (value == null || size <= 0) {
        return ByteArray(0)
    }
    return ByteArray(size) { index -> value[index].toByte() }
}

@OptIn(ExperimentalForeignApi::class)
private fun MarsStnCgiProfile.toCgiProfile(): CgiProfile = CgiProfile(
    taskStartTime = start_time.toLong(),
    startConnectTime = start_connect_time.toLong(),
    connectSuccessfulTime = connect_successful_time.toLong(),
    startSendPacketTime = start_send_packet_time.toLong(),
    sendPacketFinishedTime = send_packet_finished_time.toLong(),
    startReadPacketTime = start_read_packet_time.toLong(),
    readPacketFinishedTime = read_packet_finished_time.toLong(),
    startEncodePacketTime = start_encode_packet_time.toLong(),
    encodePacketFinishedTime = encode_packet_finished_time.toLong(),
    startDecodePacketTime = start_decode_packet_time.toLong(),
    decodePacketFinishedTime = decode_packet_finished_time.toLong(),
    rtt = rtt.toLong(),
    channelType = channel_type,
    protocolType = transport_protocol,
    netType = text(nettype)
)

@OptIn(ExperimentalForeignApi::class)
private fun MarsStnDnsProfile.toDnsProfile(): DnsProfile = DnsProfile(
    startTime = start_time.toLong(),
    endTime = end_time.toLong(),
    host = text(host),
    errType = err_type,
    errCode = err_code,
    dnsType = dnstype
)

/** The task a `reportTaskLimited` question carries, which is the app's own. */
@OptIn(ExperimentalForeignApi::class)
private fun MarsStnTask.toTask(): Task = Task().apply {
    taskID = this@toTask.taskid.toInt()
    cmdID = this@toTask.cmdid.toInt()
    channelSelect = this@toTask.channel_select
    cgi = text(this@toTask.cgi).ifEmpty { null }
    shortLinkHostList = strings(this@toTask.shortlink_host_list.items, this@toTask.shortlink_host_list.count)
    sendOnly = this@toTask.send_only != 0
    needAuthed = this@toTask.need_authed != 0
    limitFlow = this@toTask.limit_flow != 0
    limitFrequency = this@toTask.limit_frequency != 0
    channelStrategy = this@toTask.channel_strategy
    networkStatusSensitive = this@toTask.network_status_sensitive != 0
    priority = this@toTask.priority
    retryCount = this@toTask.retry_count
    serverProcessCost = this@toTask.server_process_cost
    totalTimeout = this@toTask.total_timeout
    reportArg = text(this@toTask.report_arg).ifEmpty { null }
    longPolling = this@toTask.long_polling != 0
    longPollingTimeout = this@toTask.long_polling_timeout
    clientSequenceId = this@toTask.client_sequence_id.toInt()
}

/** One unit of work, as the `MarsStnTask` the C ABI reads: every copy lives in [scope]. */
@OptIn(ExperimentalForeignApi::class)
private fun Task.native(scope: MemScope): MarsStnTask {
    val native = scope.alloc<MarsStnTask>()
    native.taskid = taskID.toUInt()
    native.cmdid = cmdID.toUInt()
    native.channel_select = channelSelect
    native.cgi = cgi?.let { scope.cstring(it) }
    native.send_only = flag(sendOnly)
    native.need_authed = flag(needAuthed)
    native.limit_flow = flag(limitFlow)
    native.limit_frequency = flag(limitFrequency)
    native.channel_strategy = channelStrategy
    native.network_status_sensitive = flag(networkStatusSensitive)
    native.priority = priority
    native.retry_count = retryCount
    native.server_process_cost = serverProcessCost
    native.total_timeout = totalTimeout
    native.report_arg = reportArg?.let { scope.cstring(it) }
    native.long_polling = flag(longPolling)
    native.long_polling_timeout = longPollingTimeout
    native.client_sequence_id = clientSequenceId.toUShort()
    val hosts = shortLinkHostList ?: emptyList()
    native.shortlink_host_list.items = scope.strings(hosts)
    native.shortlink_host_list.count = hosts.size.toUInt()
    val pairs = headers?.map { (name, value) -> name to value } ?: emptyList()
    native.headers = scope.headers(pairs)
    native.header_count = pairs.size.toUInt()
    return native
}

/**
 * What a long link of the app's own is made from, as the struct the C ABI reads:
 * every string is a copy in this scope, because a `const char*` inside a struct
 * is a pointer the port reads after the call and not the [String] a parameter is.
 */
@OptIn(ExperimentalForeignApi::class)
private fun LonglinkConfig.native(scope: MemScope): MarsStnLonglinkConfig {
    val native = scope.alloc<MarsStnLonglinkConfig>()
    native.name = name?.let { scope.cstring(it) }
    val hosts = hostList ?: emptyList()
    native.host_list.items = scope.strings(hosts)
    native.host_list.count = hosts.size.toUInt()
    native.is_keep_alive = flag(isKeepAlive)
    native.group = group?.let { scope.cstring(it) }
    native.is_main = flag(isMain)
    native.link_type = linkType
    native.need_tls = flag(needTls)
    return native
}

/**
 * One string the port reads, as a NUL-terminated copy in this scope: a
 * `const char*` field of a struct is a pointer and not the [String] a
 * `const char*` parameter of a symbol is, so a string that crosses the boundary
 * inside a struct is copied and the copy kept alive by the scope of the call.
 */
@OptIn(ExperimentalForeignApi::class)
private fun MemScope.cstring(value: String): CPointer<ByteVar> {
    val encoded = value.encodeToByteArray()
    val pointer = allocArray<ByteVar>(encoded.size + 1)
    encoded.forEachIndexed { index, byte -> pointer[index] = byte }
    pointer[encoded.size] = 0
    return pointer
}

/** The C ABI reads `0` and `1` where Kotlin reads `false` and `true`. */
private fun flag(value: Boolean): Int = if (value) 1 else 0

/** The headers of a short-link task, as pairs the C ABI reads beside a count. */
@OptIn(ExperimentalForeignApi::class)
private fun MemScope.headers(headers: List<Pair<String, String>>): CPointer<MarsStnHeader> {
    val array = allocArray<MarsStnHeader>(headers.size)
    headers.forEachIndexed { index, (name, value) ->
        array[index].name = cstring(name)
        array[index].value = cstring(value)
    }
    return array
}

/** The hosts a short-link task may go to, as strings beside a count. */
@OptIn(ExperimentalForeignApi::class)
private fun MemScope.strings(values: List<String>): CPointer<CPointerVar<ByteVar>> {
    val array = allocArray<CPointerVar<ByteVar>>(values.size)
    values.forEachIndexed { index, value -> array[index] = cstring(value) }
    return array
}

/** The strings of a `MarsStnStrings`, which is one array and its count. */
@OptIn(ExperimentalForeignApi::class)
private fun strings(items: CPointer<CPointerVar<ByteVar>>?, count: UInt): List<String> {
    if (items == null) {
        return emptyList()
    }
    return List(count.toInt()) { index -> text(items[index]) }
}

/**
 * The one function pointer the eighteen questions are funnelled through: the
 * box behind [StableRef] is rebuilt out of the context, and what the app
 * answered is written into the struct the port reads.
 */
@OptIn(ExperimentalForeignApi::class)
private fun asked(ctx: COpaquePointer?, question: CPointer<MarsStnQuestion>?, answer: CPointer<MarsStnAnswer>?) {
    if (ctx == null || question == null || answer == null) {
        return
    }
    ctx.asStableRef<AppBox>().get().answer(question.pointed, answer)
}
