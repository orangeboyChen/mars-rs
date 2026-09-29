package io.github.orangeboychen.marsrs.demo

import io.github.orangeboychen.marsrs.xlog.Xlog

/**
 * The entry point of the Kotlin Multiplatform demo: the only source in the build
 * that is not shared, because it is the only one that has to say where the
 * program starts.
 *
 * Run it with
 *
 *     ./gradlew runDebugExecutableMacosArm64
 *
 * (`runReleaseExecutableLinuxX64` on Linux). An app replaces this file with the
 * platform's own entry point — an `Activity` on Android, a `@main` on iOS — and
 * keeps everything in `commonMain` as it is.
 */
fun main() {
    // A directory relative to where the program was started, which the appender
    // creates if it is not there. A real app hands over the directory the
    // platform gives it: `context.filesDir` on Android, an
    // `NSSearchPathForDirectoriesInDomains` on Apple, `XDG_DATA_HOME` on Linux.
    val logDir = "log"

    val xlog = Xlog.open(demoConfig(logDir))

    // Mirror every record to stderr as well, so a run shows what went into the
    // file. Off in a build that ships.
    xlog.consoleLogEnabled = true
    // Close a file at 8 MiB and drop one at ten days. Both start at 0, which
    // is not the same 0 twice: a maximum size of 0 never splits a file, and a
    // lifetime of 0 is the C++'s own ten days.
    xlog.maxFileSizeBytes = 8 * 1024 * 1024L
    xlog.maxAliveTimeSeconds = 10 * 24 * 3600L

    // Everything below is `commonMain`: the same six records, whichever
    // platform compiled them.
    xlog.writeDemoRecords()

    println("writing into $logDir — ${xlog.namePrefix}_<YYYYMMDD>.xlog")

    // Drains what is left and closes the appender. An app that skips it loses
    // whatever the writer thread still held, which with an async appender is the
    // last records it wrote.
    xlog.close()
}
