package io.github.orangeboychen.marsrs.xlog

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertTrue
import kotlinx.cinterop.ExperimentalForeignApi
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
        val xlog = Xlog.open(XlogConfig(logDir = dir, namePrefix = PREFIX, level = LogLevel.VERBOSE))
        try {
            assertTrue(xlog.isOpen, "no appender is open for $PREFIX in $dir")
            assertTrue(xlog.isLoggable(LogLevel.INFO), "an appender opened at verbose drops info")

            xlog.level = LogLevel.WARNING
            assertEquals(LogLevel.WARNING, xlog.level, "the level set is not the level answered")
            xlog.level = LogLevel.VERBOSE

            xlog.i("Net", "a record from a Kotlin Multiplatform test")
            xlog.flushNow()

            // The directory, and not a file: what the C++'s
            // `GetCurrentLogPath` answers, and what every platform of the port
            // answers with it.
            assertEquals(dir, xlog.currentLogPath, "the directory is not the one it was opened with")

            // A day of files, asked of this appender and out of its own prefix
            // and directory.
            val today = xlog.logFiles(TODAY)
            assertTrue(today.isNotEmpty(), "the appender wrote no file under $dir")
            assertTrue(access(today.first(), F_OK) == 0, "${today.first()} does not exist")
            assertTrue(
                xlog.logFileNames(TODAY).isNotEmpty(),
                "the day that has a file has no name for it"
            )
            assertTrue(
                xlog.logFiles(YESTERDAY).isEmpty(),
                "yesterday has no file: ${xlog.logFiles(YESTERDAY)}"
            )
        } finally {
            xlog.close()
        }
        assertFalse(xlog.isOpen, "close() left the appender of $PREFIX open")

        // A closed appender answers neither: an empty list and not a path is
        // what a caller that reads them back gets.
        assertEquals(null, xlog.currentLogPath)
        assertTrue(xlog.logFiles(TODAY).isEmpty())
        assertTrue(xlog.logFileNames(TODAY).isEmpty())
    }

    /**
     * A prefix is one appender to the C ABI, so two `Xlog`s of one prefix are
     * one appender and `close` on either closes both — which is what
     * [Xlog.isOpen] has to answer on the one that did not close.
     */
    @Test
    fun twoObjectsOfOnePrefixAreOneAppender() {
        val dir = "${getenv("TMPDIR")?.toKString() ?: DEFAULT_TEMP_DIR}/marsrs-kmp-two-${getpid()}"
        val first = Xlog.open(XlogConfig(logDir = dir, namePrefix = PREFIX, level = LogLevel.VERBOSE))
        val second = Xlog.open(XlogConfig(logDir = dir, namePrefix = PREFIX, level = LogLevel.VERBOSE))

        first.i("Net", "through the first of two")
        first.flushNow()
        assertNotNull(second.currentLogPath, "the second of one prefix has no directory")
        assertTrue(second.logFiles(TODAY).isNotEmpty(), "the second of one prefix sees no file")

        first.close()
        assertFalse(second.isOpen, "close() on one of a prefix left the other open")
        assertEquals(null, second.currentLogPath)
        assertTrue(second.logFiles(TODAY).isEmpty())
        // Safe twice, and safe on the one that did not close.
        second.close()
        assertFalse(first.isOpen)
    }

    private companion object {
        const val DEFAULT_TEMP_DIR = "/tmp"

        const val PREFIX = "marsrs"

        const val TODAY = 0L

        const val YESTERDAY = 1L
    }
}
