package io.github.orangeboychen.marsrs.xlog

/**
 * What an [Xlog] is opened with: the Kotlin face of the `Xlog.XLogConfig`
 * whose fields `marsrs-jni` reads by name.
 *
 * Every property has the default the C++ project's own `XLogConfig` carries,
 * so the only one an app has to give is [logDir]:
 *
 * ```kotlin
 * val xlog = Xlog.open(
 *     XlogConfig(
 *         logDir = File(context.filesDir, "xlog/log").path,
 *         cacheDir = File(context.filesDir, "xlog/cache").path,
 *         namePrefix = "marsrs",
 *         level = LogLevel.INFO,
 *         mode = AppenderMode.ASYNC,
 *     )
 * )
 * ```
 *
 * A config the appender cannot honour is refused here and not by the `.so`:
 * `marsrs-jni` answers a config it does not like by opening nothing, and an app
 * that finds out three days later that it has no logs has no way back.
 * [require] is what turns that silence into an [IllegalArgumentException] at
 * the call.
 */
data class XlogConfig @JvmOverloads constructor(
    /**
     * The directory the `.xlog` files are written to, created when it is
     * not there. The one property with no default, because it is the one
     * `marsrs-jni` refuses a config without.
     */
    val logDir: String,
    /**
     * What every file of this appender starts with (`marsrs_20260927.xlog`).
     * The `nameprefix` an instance is looked up by, so an app that opens
     * two of them gives them two names.
     */
    val namePrefix: String = DEFAULT_NAME_PREFIX,
    /**
     * The level the appender starts at: a record less severe than this is
     * dropped. [Xlog.level] moves it afterwards.
     */
    val level: LogLevel = LogLevel.INFO,
    /**
     * Whether a write reaches the file before it returns.
     */
    val mode: AppenderMode = AppenderMode.ASYNC,
    /**
     * The directory of the memory-mapped cache file of [AppenderMode.ASYNC],
     * created when it is not there. `null` — the default — puts it next to
     * the log files, in [logDir].
     */
    val cacheDir: String? = null,
    /**
     * How many days a cache file of [AppenderMode.ASYNC] is kept before it is
     * dropped: `0`, the default, keeps every one of them.
     */
    val cacheDays: Int = NO_CACHE_DAYS,
    /**
     * What a closed log file is compressed with.
     */
    val compressMode: CompressMode = CompressMode.ZLIB,
    /**
     * How hard [compressMode] tries, `0` (the default) to `9`: `0` is the
     * compressor's own compromise, `9` is the smallest file over the
     * longest compression. The C++ spells the same numbers
     * `COMPRESS_LEVEL1`..`COMPRESS_LEVEL9`.
     */
    val compressLevel: Int = DEFAULT_COMPRESS_LEVEL,
    /**
     * The public key of the elliptic-curve pair whose private key reads the
     * log file back. Empty — the default — writes a file any reader of a
     * mars log file can open; the C++ project's `pubkey` is the same
     * string, and the port does not encrypt, so what an app puts here is
     * what its own tooling sees.
     */
    val pubKey: String = ""
) {
    init {
        require(logDir.isNotBlank()) { "logDir must not be blank: marsrs-jni opens nothing without one" }
        require(namePrefix.isNotBlank()) { "namePrefix must not be blank: it is what an instance is looked up by" }
        require(cacheDays >= 0) { "cacheDays must not be negative, was $cacheDays" }
        require(compressLevel in DEFAULT_COMPRESS_LEVEL..MAX_COMPRESS_LEVEL) {
            "compressLevel must be in $DEFAULT_COMPRESS_LEVEL..$MAX_COMPRESS_LEVEL, was $compressLevel"
        }
    }

    /** The `Xlog.XLogConfig` `marsrs-jni` reads this config's fields out of. */
    internal fun toNative(): Xlog.XLogConfig = Xlog.XLogConfig().apply {
        this.level = this@XlogConfig.level.native
        this.mode = this@XlogConfig.mode.native
        this.logdir = this@XlogConfig.logDir
        this.nameprefix = this@XlogConfig.namePrefix
        this.pubkey = this@XlogConfig.pubKey
        this.compressmode = this@XlogConfig.compressMode.native
        this.compresslevel = this@XlogConfig.compressLevel
        this.cachedir = this@XlogConfig.cacheDir
        this.cachedays = this@XlogConfig.cacheDays
    }

    private companion object {
        const val DEFAULT_NAME_PREFIX = "xlog"
        const val NO_CACHE_DAYS = 0
        const val DEFAULT_COMPRESS_LEVEL = 0
        const val MAX_COMPRESS_LEVEL = 9
    }
}
