package io.github.orangeboychen.marsrs.xlog

import io.github.orangeboychen.marsrs.xlog.ffi.mars_xlog_current_log_path
import kotlin.test.Test
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
        Xlog.open(XlogConfig(logDir = dir, namePrefix = "Mars", level = LogLevel.Verbose))
        try {
            Xlog.write(LogLevel.Info, "Net", "a record from a Kotlin Multiplatform test")
            Xlog.flush(sync = true)

            val path = currentLogPath()
            assertTrue(path.isNotEmpty(), "the appender opened no log file under $dir")
            assertTrue(access(path, F_OK) == 0, "$path does not exist")
        } finally {
            Xlog.close()
        }
    }

    private fun currentLogPath(): String = memScoped {
        val out = allocArray<ByteVar>(PATH_MAX)
        val written = mars_xlog_current_log_path(out, PATH_MAX.toUInt())
        if (written < 0) "" else out.toKString()
    }

    private companion object {
        const val PATH_MAX = 4096

        const val DEFAULT_TEMP_DIR = "/tmp"
    }
}
