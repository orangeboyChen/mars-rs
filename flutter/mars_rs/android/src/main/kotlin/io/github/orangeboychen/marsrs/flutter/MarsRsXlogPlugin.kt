// The Android half of the `mars_rs` plugin: the six methods of the plugin's
// channel, each of them a straight call of a member of `Xlog` — the Kotlin face
// of `libmarsxlog.so` in the `mars-rs` AAR, the AAR of the whole port. The
// xlog-only plugin is `mars_rs_xlog`, and this file is byte-for-byte its Kotlin:
// xlog is the whole C ABI today, so the two plugins are one package under two
// names and they diverge the day STN and SDT land — here, and not there.
//
// `Xlog` is the same class `android/mars-core` publishes, so what this file is
// is a channel over an API that already exists, and the API it is a channel over
// is the instance one: `Xlog.open` opens the *process-wide* appender with the
// compression and the cache the C++'s Java hard-codes, which is why
// `newXlogInstance` is what `open` below calls — a caller who asked for zstd
// would otherwise get zlib. It is the instance `MarsXlogInstance` of
// `Sources/MarsRSXlog/Xlog.swift` is, so the Dart surface and the Swift one
// come out the same shape.
//
// Every `call.argument` is the field `lib/mars_rs.dart` put there, and the
// numbers are the ones `mars_xlog.h` gives a level, a mode and a compression.

package io.github.orangeboychen.marsrs.flutter

import android.os.Looper
import android.os.Process
import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.MethodChannel.MethodCallHandler
import io.flutter.plugin.common.MethodChannel.Result
import io.github.orangeboychen.marsrs.xlog.Xlog

/** The Android half of `mars_rs`: the whole port, which today is xlog. */
class MarsRsXlogPlugin :
    FlutterPlugin,
    MethodCallHandler {

    private var channel: MethodChannel? = null

    /** The instance `Xlog.newXlogInstance` gave; `0` before `open`, after `close`. */
    private var instance: Long = 0

    /** What `close` releases the instance by: the prefix it was opened with. */
    private var namePrefix: String = ""

    private val xlog = Xlog()

    override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel = MethodChannel(binding.binaryMessenger, CHANNEL).also {
            it.setMethodCallHandler(this)
        }
    }

    override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel?.setMethodCallHandler(null)
        channel = null
    }

    override fun onMethodCall(call: MethodCall, result: Result) {
        // A Dart caller gets a `PlatformException` and not a crash: an appender
        // that refuses a configuration, or a `write` before an `open`, is a
        // caller's mistake to answer.
        try {
            when (call.method) {
                "open" -> open(call, result)
                "write" -> write(call, result)
                "flush" -> flush(call, result)
                "setLevel" -> setLevel(call, result)
                "setConsoleLog" -> setConsoleLog(call, result)
                "close" -> close(result)
                else -> result.notImplemented()
            }
        } catch (e: Exception) {
            result.error("mars_rs_xlog", e.message, null)
        }
    }

    /** `Xlog.newXlogInstance`, over the configuration the Dart caller sent. */
    private fun open(call: MethodCall, result: Result) {
        val logDirectory = call.argument<String>("logDirectory")
        if (logDirectory.isNullOrEmpty()) {
            result.error("mars_rs_xlog", "logDirectory is empty", null)
            return
        }
        val config = Xlog.XLogConfig().apply {
            level = call.argument<Int>("level") ?: Xlog.LEVEL_INFO
            mode = call.argument<Int>("mode") ?: Xlog.APPENDER_MODE_ASYNC
            logdir = logDirectory
            nameprefix = call.argument<String>("namePrefix").orEmpty()
            pubkey = call.argument<String>("publicKey").orEmpty()
            compressmode = call.argument<Int>("compression") ?: Xlog.ZLIB_MODE
            compresslevel = call.argument<Int>("compressionLevel") ?: 0
            cachedir = call.argument<String>("cacheDirectory")
            cachedays = call.argument<Int>("cacheDays") ?: 0
        }
        // `0` is the answer for a configuration the appender refused, and an
        // instance the caller has no handle to is a caller who would write
        // through the process-wide appender instead.
        val handle = xlog.newXlogInstance(config)
        if (handle == 0L) {
            result.error("mars_rs_xlog", "the appender refused the configuration", null)
            return
        }
        instance = handle
        namePrefix = config.nameprefix.orEmpty()
        result.success(null)
    }

    /** `Xlog.logWrite2`. The file, the function and the line are left empty:
     * there is no Dart frame to name, and the C++ writes an empty one too. */
    private fun write(call: MethodCall, result: Result) {
        Xlog.logWrite2(
            instance,
            call.argument<Int>("level") ?: Xlog.LEVEL_INFO,
            call.argument<String>("tag"),
            "",
            "",
            0,
            Process.myPid(),
            Process.myTid().toLong(),
            Looper.getMainLooper().thread.id,
            call.argument<String>("message")
        )
        result.success(null)
    }

    /** `Xlog.appenderFlush`. */
    private fun flush(call: MethodCall, result: Result) {
        xlog.appenderFlush(instance, call.argument<Boolean>("sync") ?: false)
        result.success(null)
    }

    /** `Xlog.setLogLevel`. */
    private fun setLevel(call: MethodCall, result: Result) {
        xlog.setLogLevel(instance, call.argument<Int>("level") ?: Xlog.LEVEL_INFO)
        result.success(null)
    }

    /** `Xlog.setConsoleLogOpen`. */
    private fun setConsoleLog(call: MethodCall, result: Result) {
        xlog.setConsoleLogOpen(instance, call.argument<Boolean>("enabled") ?: false)
        result.success(null)
    }

    /** `Xlog.releaseXlogInstance`: closes the appender `open` made. */
    private fun close(result: Result) {
        xlog.releaseXlogInstance(namePrefix)
        instance = 0
        namePrefix = ""
        result.success(null)
    }

    companion object {
        /** The channel `lib/mars_rs_xlog.dart` talks on. */
        private const val CHANNEL = "mars_rs_xlog"
    }
}
