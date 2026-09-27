package io.github.orangeboychen.marsrs.xlog

import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_make_logfile_name
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue
import kotlinx.cinterop.ByteVar
import kotlinx.cinterop.ExperimentalForeignApi
import kotlinx.cinterop.allocArray
import kotlinx.cinterop.memScoped
import kotlinx.cinterop.toKString
import platform.posix.F_OK
import platform.posix.access
import platform.posix.getenv
import platform.posix.getpid

/**
 * What a Kotlin/Native target can be asked to prove, and what no JVM one can be
 * here: that the archive `scripts/build_kmp_native.sh` built for it is the one
 * the klib embeds, that the linker flags of `mars_ffi.def` are enough to link
 * it, and that a record written from Kotlin reaches a file.
 *
 * It runs for the host the build runs on — `macosArm64Test` on an Apple Silicon
 * Mac, `linuxX64Test` on Linux — and nowhere else: a test binary of another
 * target cannot be run by this one.
 */
@OptIn(ExperimentalForeignApi::class)
class XlogTest {
    @Test
    fun aRecordReachesTheLogFile() {
        // The appender creates the directory itself, the way the C++'s does.
        val dir = "${getenv("TMPDIR")?.toKString() ?: DEFAULT_TEMP_DIR}/marsrs-kmp-${getpid()}"
        val xlog = Xlog(XlogConfig(logDir = dir, namePrefix = PREFIX, level = LogLevel.VERBOSE))
        try {
            assertTrue(xlog.isOpen, "no appender is open for $PREFIX in $dir")
            assertTrue(xlog.isLoggable(LogLevel.INFO), "an appender opened at verbose drops info")

            xlog.level = LogLevel.WARNING
            assertEquals(LogLevel.WARNING, xlog.level, "the level set is not the level answered")
            xlog.level = LogLevel.VERBOSE

            xlog.i("Net", "a record from a Kotlin Multiplatform test")
            xlog.flush(sync = true)

            val path = logFilePath(dir)
            assertTrue(path.isNotEmpty(), "the appender opened no log file under $dir")
            assertTrue(access(path, F_OK) == 0, "$path does not exist")
        } finally {
            xlog.close()
        }
        assertFalse(xlog.isOpen, "close() left the appender of $PREFIX open")
    }

    /**
     * The name the C ABI gives today's file of [PREFIX] in `dir`:
     * `mars_xlog_make_logfile_name`, which is what an app that collects logs
     * asks rather than guessing at `marsrs_20260927.xlog` itself.
     */
    private fun logFilePath(dir: String): String = memScoped {
        val out = allocArray<ByteVar>(PATH_MAX)
        val written = mars_xlog_make_logfile_name(TODAY, PREFIX, dir, FIRST_NAME, out, PATH_MAX.toUInt())
        if (written < 0) "" else out.toKString()
    }

    private companion object {
        const val PATH_MAX = 4096

        const val DEFAULT_TEMP_DIR = "/tmp"

        const val PREFIX = "marsrs"

        const val TODAY = 0

        const val FIRST_NAME = 0u
    }
}
