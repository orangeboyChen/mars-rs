package io.github.orangeboychen.marsrs.demo

import android.app.Activity
import android.os.Build
import android.os.Bundle
import android.widget.Button
import android.widget.TextView
import io.github.orangeboychen.marsrs.xlog.AppenderMode
import io.github.orangeboychen.marsrs.xlog.LogLevel
import io.github.orangeboychen.marsrs.xlog.Xlog
import io.github.orangeboychen.marsrs.xlog.XlogConfig
import java.io.File

/**
 * The Android demo of mars-rs: one screen that opens the appender, writes one
 * record at every level, and says where the file went.
 *
 * It is the Android twin of `demo/rust` and `demo/c` — the same six records in
 * the same order — and it is the whole of what an app writes:
 *
 * 1. describe the files — [XlogConfig];
 * 2. open the appender and keep the handle — [Xlog.open];
 * 3. write — `xlog.i(tag, message)` and the five beside it;
 * 4. drain it before anything reads the file — [Xlog.flush];
 * 5. close it when the app goes away — [Xlog.close].
 *
 * Nothing here loads a library: the AAR carries `libmarsrsxlog.so` for
 * `arm64-v8a`, `armeabi-v7a` and `x86_64`, and `Xlog` loads it.
 *
 * Two things this demo deliberately does *not* show:
 *
 * - **The whole port.** `marsrs` — the other AAR — carries STN and SDT beside
 *   xlog, and an app that takes it has one more obligation this one does not:
 *   `Mars.init(context, handler)` before `Mars.onCreate(true)` on the very
 *   first start, and an `AppLogic.ICallBack` that says where the port may keep
 *   its own files. An app that only logs takes `xlog` and writes this.
 * - **Encryption.** `XlogConfig.pubKey` is the 128 hex characters of a public
 *   key, and an appender opened with one writes records only the matching
 *   private key decrypts. `xlog keygen` makes the pair.
 */
class MainActivity : Activity() {

    /**
     * The appender. Opened once in [onCreate] and closed in [onDestroy], which
     * is the shape every app wants: one appender for the process, held for as
     * long as the process lives.
     *
     * Two `Xlog`s of one `namePrefix` are one appender, so a second call to
     * [Xlog.open] with `"marsrs"` hands back the same one — and closing either
     * closes both.
     */
    private lateinit var xlog: Xlog

    private lateinit var status: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        status = findViewById(R.id.status)

        xlog = openXlog()
        writeStartupRecords()

        // Every write after the first is one method call and, when the app is
        // about to read the file or upload it, one flush.
        findViewById<Button>(R.id.write).setOnClickListener {
            xlog.i("button", "a record written at ${System.currentTimeMillis()}")
            // `flushNow` waits for the write, so the record is on disk when
            // the click handler returns. `flush()` is the same drain as a
            // `suspend` call, and `signalFlush()` only signals the writer
            // thread and comes straight back.
            xlog.flushNow()
            status.text = statusText()
        }
    }

    override fun onDestroy() {
        // Drains what is left and closes the appender. An app that skips it
        // loses whatever the writer thread still held, which with an async
        // appender is the last records it wrote.
        //
        // Everything else of `xlog` throws after this: `level`, `mode` and the
        // two size limits all answer `IllegalStateException`, and a write
        // silently does nothing.
        xlog.close()
        super.onDestroy()
    }

    /**
     * Steps 1 and 2: the configuration, and the open.
     *
     * Both directories are inside this app's own `filesDir`, which is the
     * directory the system gives an app to keep files in and the one that goes
     * when the app is uninstalled — so the demo needs no storage permission,
     * and an app that writes somewhere shared has a permission to ask for and
     * not a path to change.
     */
    private fun openXlog(): Xlog {
        val root = File(filesDir, "xlog")
        val opened = Xlog.open(
            XlogConfig(
                // Where the `.xlog` files land. The one option with no
                // default.
                logDir = File(root, "log").path,
                // Where the mmap cache lives while a record is on its way to
                // the file. `null` writes straight into `logDir`, which is what
                // an app that does not care takes.
                cacheDir = File(root, "cache").path,
                // The prefix of every file: `marsrs_YYYYMMDD.xlog`. It is what
                // tells this app's files from another component's in a shared
                // directory, and it is also the key two `Xlog`s share an
                // appender by.
                namePrefix = "marsrs",
                // The level a record has to reach. Verbose so that all six
                // records survive to be read back; an app that ships sets
                // `LogLevel.INFO`, and `LogLevel.NONE` for a build that writes
                // nothing at all.
                level = LogLevel.VERBOSE,
                // `ASYNC` hands the record to a writer thread and returns;
                // `SYNC` writes it before it does. An app takes `ASYNC` — the
                // whole point of the pipeline is that a log call does not block
                // the thread that made it.
                mode = AppenderMode.ASYNC,
            ),
            // The context the appender registers its lifecycle callback on: it
            // flushes when the UI goes away and on low memory, so an app that
            // passes one never has to remember to. `Xlog` takes
            // `applicationContext` from whatever it is handed, so passing the
            // Activity is safe — and passing `applicationContext` is the same
            // thing said out loud.
            applicationContext,
        )

        // Mirror every record to logcat as well. On in a debug build, off in
        // one that ships: logcat is world-readable, and the records an app
        // writes are the app's own business.
        opened.consoleLogEnabled = BuildConfig.DEBUG
        // Close a file at 8 MiB and drop one at ten days. Both start at 0,
        // which is not the same 0 twice: a maximum size of 0 never splits a
        // file, and a lifetime of 0 is the C++'s own ten days.
        opened.maxFileSizeBytes = 8 * 1024 * 1024L
        opened.maxAliveTimeSeconds = 10 * 24 * 3600L

        return opened
    }

    /** Step 3: one record at every level, and step 4: the flush. */
    private fun writeStartupRecords() {
        // The shape of `android.util.Log`: a tag and a message, and one method
        // per level. `xlog.log(level, tag, message)` is the one to call when
        // the level is not known until the call.
        xlog.v("trace", "the finest record there is")
        xlog.d("net", "resolved 3 addresses for example.com")
        xlog.i("startup", "cold start in 412 ms")
        xlog.w("net", "retrying after 1204 ms")
        xlog.e("login", "login failed: token expired")
        xlog.f("login", "giving up after 3 attempts")

        // A record that is expensive to build is worth asking about first: a
        // record the level drops still costs its caller the `String`.
        if (xlog.isLoggable(LogLevel.DEBUG)) {
            xlog.d("startup", "onCreate ran on ${Build.MODEL}")
        }

        // Step 4: everything above is on disk before the next line runs.
        xlog.flushNow()
        status.text = statusText()
    }

    /**
     * Where the files are. `Xlog` answers the directory the appender is writing
     * into — and not a file, because that is what the C++ function it ports
     * answers. The file itself is `<namePrefix>_<YYYYMMDD>.xlog` inside it.
     */
    private fun statusText(): String = "writing into ${File(filesDir, "xlog/log")}"
}
