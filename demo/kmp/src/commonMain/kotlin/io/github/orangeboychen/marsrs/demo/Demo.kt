package io.github.orangeboychen.marsrs.demo

import io.github.orangeboychen.marsrs.xlog.AppenderMode
import io.github.orangeboychen.marsrs.xlog.CompressMode
import io.github.orangeboychen.marsrs.xlog.LogLevel
import io.github.orangeboychen.marsrs.xlog.Xlog
import io.github.orangeboychen.marsrs.xlog.XlogConfig

/**
 * The shared half of the demo — the part every platform compiles, and the part
 * an app would keep in its shared module.
 *
 * It is the same six records `demo/rust`, `demo/c` and `demo/android` write, in
 * the same order, so that the four read side by side: what differs between the
 * platforms is how the appender gets opened and where the directory comes from,
 * and not what an app writes once it has one.
 */

/** The prefix of every file: `marsrs_YYYYMMDD.xlog`. */
const val LOG_PREFIX: String = "marsrs"

/**
 * The configuration, given the directory the files go in.
 *
 * The directory is a parameter because it is the one thing that really is
 * different on every platform — a phone has a directory the system gives the
 * app and a desktop has a working directory — and because `commonMain` has no
 * file system of its own to ask for one.
 */
fun demoConfig(logDir: String): XlogConfig = XlogConfig(
    // Where the `.xlog` files land. The one option with no default.
    logDir = logDir,
    // Where the mmap cache lives while a record is on its way to the file.
    // `null` writes straight into `logDir`, which is what an app that does not
    // care takes.
    cacheDir = null,
    namePrefix = LOG_PREFIX,
    // The level a record has to reach. Verbose so that all six records survive
    // to be read back; an app that ships sets `LogLevel.INFO`, and
    // `LogLevel.NONE` for a build that writes nothing at all.
    level = LogLevel.VERBOSE,
    // `ASYNC` hands the record to a writer thread and returns; `SYNC` writes it
    // before it does. An app takes `ASYNC` — the whole point of the pipeline is
    // that a log call does not block the thread that made it.
    mode = AppenderMode.ASYNC,
    // zlib, which is what the C++ project writes. `ZSTD` is the smaller file
    // and the newer decoder.
    compressMode = CompressMode.ZLIB,
)

/**
 * One record at every level, and the flush that leaves them on disk.
 *
 * The `Xlog` it takes is the one the platform opened: on Android that is the
 * same class over JNI, and everywhere else the same class over the C ABI, and
 * neither is visible from here.
 */
fun Xlog.writeDemoRecords() {
    // One method per level, and `log(level, tag, message)` for the call whose
    // level is not known until it runs.
    v("trace", "the finest record there is")
    d("net", "resolved 3 addresses for example.com")
    i("startup", "cold start in 412 ms")
    w("net", "retrying after 1204 ms")
    e("login", "login failed: token expired")
    f("login", "giving up after 3 attempts")

    // A record that is expensive to build is worth asking about first: a record
    // the level drops still costs its caller the `String`.
    if (isLoggable(LogLevel.DEBUG)) {
        d("startup", "the appender is $namePrefix")
    }

    // `sync = true` waits for the write, so every record above is on disk when
    // this returns. An app calls it before it reads the files, uploads them, or
    // exits.
    //
    // `flush(sync = true)`, and not `flushNow()`: this demo is compiled against
    // the package on GitHub Packages and not against this tree, and the name
    // that release published is this one — `flushNow` is the name the module
    // carries now, and it is not one the version `marsrs` above names has.
    flush(sync = true)
}
