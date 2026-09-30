/*
 * mars_xlog.hpp — the C++ face of the C ABI in `mars_xlog.h`.
 *
 * This header is the logger in C++: `marsrs::xlog::Xlog::open(config)` answers
 * an appender, and the app writes through what it was answered — the shape
 * every platform of the port spells, `Xlog.open(config)` in Kotlin, in Dart and
 * in TypeScript and `Xlog.open(config)` in Swift. What it is written over is
 * the C ABI of `marsrs-ffi`, and nothing else: every member is one
 * `mars_xlog_*` call, so nothing here is a behaviour the C ABI does not have.
 *
 * A caller that would rather name the C symbols takes `mars_xlog.h`, which this
 * includes. No symbol there installs a process-wide appender, so handle `0`
 * names no logger at all — an app that wants one of its own calls
 * `mars_xlog_new_instance` and holds the handle it answers.
 *
 * The header is header-only and needs C++17: `std::optional<std::string>` and
 * `std::vector<std::string>` are what the three file questions answer, and
 * `std::future<void>` is what `flush()` answers.
 * It throws, because that is what the platforms do — Swift throws an
 * `XlogError`, Kotlin's `XlogConfig` throws on a config it cannot honour — and
 * `Xlog` is move-only, because a prefix is one appender to the C ABI and two
 * copies of one handle is two owners of one close.
 *
 * Threading: every member may be called from any thread, as on every platform.
 * The four an app sets and reads back — the mode, the console, the two sizes —
 * are answers of this object's own and not of the appender's: the C ABI has no
 * getter for them, so what they answer is the last value written through this
 * `Xlog`.
 *
 * Panics: no Rust panic ever crosses the C ABI, and nothing here throws but
 * `Xlog::open`.
 */

#ifndef MARS_XLOG_HPP_
#define MARS_XLOG_HPP_

#include <cstdint>
#include <functional>
#include <future>
#include <optional>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "mars_xlog.h"

#if __cplusplus < 201703L
#error "mars_xlog.hpp needs C++17: std::string_view and std::future are what it is written with"
#endif

namespace marsrs {
namespace xlog {

/** `MarsLogLevel`; the numbers are the C ABI's, and the C++'s `TLogLevel`. */
enum class LogLevel : int {
    Verbose = MarsLevelVerbose,
    Debug = MarsLevelDebug,
    Info = MarsLevelInfo,
    Warning = MarsLevelWarn,
    Error = MarsLevelError,
    Fatal = MarsLevelFatal,
    /** Nothing is written at all: the level the C++'s `kLevelNone` is. */
    None = MARS_LEVEL_NONE,
};

/** `MarsAppenderMode`: whether the appender writes from a thread of its own. */
enum class AppenderMode : int {
    Async = MarsAppenderAsync,
    Sync = MarsAppenderSync,
};

/** `MarsCompressMode`: what compresses a log file once it is closed. */
enum class CompressMode : int {
    Zlib = MarsCompressZlib,
    Zstd = MarsCompressZstd,
};

/** What an appender is opened with: the `XLogConfig` of every platform, with
 * the defaults the C++ gives the fields it is not told. `logDir` is the one
 * field with no default, and the one the C ABI insists on. */
struct XlogConfig {
    /** The directory the log files are written to; created if it is missing. */
    std::string logDir;

    /** What every file of this appender starts with, and what the appender is
     * known by — an app that writes through two of them gives them two. */
    std::string namePrefix = "xlog";

    /** The level the appender is opened at: a record below it is dropped
     * before it is formatted. */
    LogLevel level = LogLevel::Info;

    AppenderMode mode = AppenderMode::Async;

    /** The ECDH public key a log file is encrypted with; empty writes it
     * unencrypted, the way an empty key does in the C++. */
    std::string pubKey;

    CompressMode compressMode = CompressMode::Zlib;

    /** `0` is what the C++ passes on: 6, the appender's own default. */
    int compressLevel = 0;

    /** Where the mmap cache the appender keeps lives; empty puts it in
     * `logDir`, as the C++ does. */
    std::string cacheDir;

    /** How many days of log files are kept; `0` keeps all of them. */
    int cacheDays = 0;
};

/** Why no appender was opened: the five an `XlogConfig` can be refused for. */
class XlogError : public std::runtime_error {
public:
    enum class Reason {
        EmptyLogDirectory,
        EmptyNamePrefix,
        InvalidCompressionLevel,
        NegativeCacheDays,
        /** `mars_xlog_new_instance` answered `0`. */
        Refused,
    };

    XlogError(Reason reason, const char* what) : std::runtime_error(what), reason_(reason) {
    }

    Reason reason() const noexcept {
        return reason_;
    }

private:
    Reason reason_;
};

/** An appender of an app's own: build one when the app starts, then write
 * through it from wherever there is something to say.
 *
 * ```cpp
 * marsrs::xlog::XlogConfig config;
 * config.logDir = "/tmp/mars-log";
 * config.namePrefix = "marsrs";
 *
 * auto log = marsrs::xlog::Xlog::open(config);
 * log.i("startup", "hello from mars");
 *
 * log.flushNow();      // the records are on disk when this returns
 * // log.flush().get() is the same drain off this thread
 * log.close();
 * ```
 *
 * A write is a tag and a message, the pair every platform of the port spells;
 * the file, the function and the line of the record are empty unless the long
 * `log` names them, which is what Kotlin writes too — C++ has no `#file` to
 * fill one in with. A record is dropped before anything is formatted when its
 * level is below `level()`, so a message that is expensive to build is worth an
 * `if (log.isLoggable(LogLevel::Debug))` first.
 *
 * Two `Xlog`s of one `namePrefix` are one appender: the C ABI answers the
 * handle it already has, so closing one of them closes what the other writes
 * through. */
class Xlog {
public:
    /** Opens an appender of its own: its own log directory, prefix, key, mode
     * and cache file, all of them `config`'s.
     *
     * Asking for a prefix that is already open answers the appender that is
     * open and not a second one.
     *
     * @throws XlogError when `config` is one the appender refuses. */
    static Xlog open(const XlogConfig& config) {
        if (config.logDir.empty()) {
            throw XlogError(XlogError::Reason::EmptyLogDirectory,
                            "marsrs-xlog opened no appender without a log directory");
        }
        if (config.namePrefix.empty()) {
            throw XlogError(XlogError::Reason::EmptyNamePrefix,
                            "marsrs-xlog opened no appender without a name prefix");
        }
        if (config.compressLevel < 0 || config.compressLevel > maxCompressionLevel(config.compressMode)) {
            throw XlogError(XlogError::Reason::InvalidCompressionLevel,
                            "compressLevel must be in 0...9 for zlib, 0...22 for zstd");
        }
        if (config.cacheDays < 0) {
            throw XlogError(XlogError::Reason::NegativeCacheDays, "cacheDays must not be negative");
        }

        // The C strings the C ABI reads out of the config, and the `std::string`
        // they belong to: `MarsXLogConfig` copies nothing, so both have to
        // outlive the call.
        const std::string& cacheDir = config.cacheDir.empty() ? config.logDir : config.cacheDir;
        MarsXLogConfig c{};
        c.mode = static_cast<int>(config.mode);
        c.log_dir = config.logDir.c_str();
        c.name_prefix = config.namePrefix.c_str();
        c.pub_key = config.pubKey.c_str();
        c.compress_mode = static_cast<int>(config.compressMode);
        c.compress_level = config.compressLevel;
        c.cache_dir = cacheDir.c_str();
        c.cache_days = config.cacheDays;

        const long long handle = mars_xlog_new_instance(&c, static_cast<int>(config.level));
        if (handle == 0) {
            throw XlogError(XlogError::Reason::Refused, "mars_xlog_new_instance refused the configuration");
        }
        return Xlog(handle, config);
    }

    Xlog(const Xlog&) = delete;
    Xlog& operator=(const Xlog&) = delete;

    /** The handle moves and the appender stays: what a moved-from `Xlog` owns
     * is nothing, so closing it closes nothing. */
    Xlog(Xlog&& other) noexcept
        : handle_(other.handle_),
          namePrefix_(std::move(other.namePrefix_)),
          currentMode_(other.currentMode_),
          consoleLogEnabled_(other.consoleLogEnabled_),
          maxFileSizeBytes_(other.maxFileSizeBytes_),
          maxAliveTimeSeconds_(other.maxAliveTimeSeconds_) {
        other.handle_ = 0;
    }

    Xlog& operator=(Xlog&& other) noexcept {
        if (this != &other) {
            close();
            handle_ = other.handle_;
            namePrefix_ = std::move(other.namePrefix_);
            currentMode_ = other.currentMode_;
            consoleLogEnabled_ = other.consoleLogEnabled_;
            maxFileSizeBytes_ = other.maxFileSizeBytes_;
            maxAliveTimeSeconds_ = other.maxAliveTimeSeconds_;
            other.handle_ = 0;
        }
        return *this;
    }

    /** Closes this appender: what Swift's `deinit` does, and what an `Xlog` of
     * automatic storage gets without being asked. */
    ~Xlog() {
        close();
    }

    /** What every file of this appender starts with, and what it is known by. */
    const std::string& namePrefix() const noexcept {
        return namePrefix_;
    }

    /** Whether this appender is still open: `false` after `close()` — on this
     * `Xlog` and on every other one of this `namePrefix`, which is the same
     * appender and is closed with this one. The prefix and not the handle
     * alone: a prefix is one appender, so a twin that closed it leaves this
     * handle looking open while every write through it is dropped. */
    bool isOpen() const noexcept {
        return handle_ != 0 && handle_ == mars_xlog_get_instance(namePrefix_.c_str());
    }

    /** The level of this appender: a record less severe than this is dropped.
     *
     * Read from the C ABI and not mirrored here, so a level another part of
     * the app set is the one this answers with. */
    LogLevel level() const {
        // A closed `Xlog` answers "nothing is written", which is what the Rust
        // `Xlog::level` answers as `None` and what `isLoggable` already says
        // here: a handle no appender is open for has no level of its own, and
        // the `-1` it is answered with is `(TLogLevel)-1`, the C++'s "log
        // everything" — the opposite of the truth for one.
        if (!isOpen()) {
            return LogLevel::None;
        }
        const int level = mars_xlog_get_level(handle_);
        return level < 0 ? LogLevel::Verbose : static_cast<LogLevel>(level);
    }

    void setLevel(LogLevel level) {
        withHandle([level](long long handle) {
            mars_xlog_set_level_instance(handle, static_cast<int>(level));
        });
    }

    /** Whether a write reaches the file before it returns: what the
     * `XlogConfig` gave, until this says otherwise. The C ABI has no getter for
     * it, so this is the last value this side wrote. */
    AppenderMode mode() const noexcept {
        return currentMode_;
    }

    void setMode(AppenderMode mode) {
        currentMode_ = mode;
        withHandle([mode](long long handle) {
            mars_xlog_set_mode_instance(handle, static_cast<int>(mode));
        });
    }

    /** Whether the console prints the log too — off until an app turns it on. */
    bool consoleLogEnabled() const noexcept {
        return consoleLogEnabled_;
    }

    void setConsoleLogEnabled(bool enabled) {
        consoleLogEnabled_ = enabled;
        withHandle([enabled](long long handle) {
            mars_xlog_set_console_log_instance(handle, enabled ? 1 : 0);
        });
    }

    /** How many bytes a log file may reach before it is closed and a new one
     * opened; `0` is "never split". */
    std::uint64_t maxFileSizeBytes() const noexcept {
        return maxFileSizeBytes_;
    }

    void setMaxFileSizeBytes(std::uint64_t bytes) {
        maxFileSizeBytes_ = bytes;
        withHandle([bytes](long long handle) {
            mars_xlog_set_max_file_size_instance(handle, bytes);
        });
    }

    /** How many seconds a log file is kept; `0` is the C++'s own ten days. */
    std::int64_t maxAliveTimeSeconds() const noexcept {
        return maxAliveTimeSeconds_;
    }

    void setMaxAliveTimeSeconds(std::int64_t seconds) {
        maxAliveTimeSeconds_ = seconds;
        withHandle([seconds](long long handle) {
            mars_xlog_set_max_alive_duration_instance(handle, seconds);
        });
    }

    /** Whether a record of `level` would be written: what an app asks before it
     * builds a message that is expensive to build. */
    bool isLoggable(LogLevel level) const {
        return isOpen() && mars_xlog_is_enabled_for(handle_, static_cast<int>(level)) != 0;
    }

    /** Writes a record of `level`. The file, the function and the line of the
     * call site are the long form's; the short one writes the empty ones, as
     * Kotlin's does. */
    void log(LogLevel level, const std::string& tag, const std::string& message) {
        log(level, tag, message, std::string(), std::string(), 0);
    }

    /** `log` with where the record was written: `__FILE__`,
     * `__PRETTY_FUNCTION__` and `__LINE__` of the call site. */
    void log(LogLevel level,
             const std::string& tag,
             const std::string& message,
             const std::string& file,
             const std::string& function,
             int line) {
        withHandle([&](long long handle) {
            mars_xlog_write_instance(handle,
                                     static_cast<int>(level),
                                     tag.c_str(),
                                     file.c_str(),
                                     function.c_str(),
                                     line,
                                     message.c_str());
        });
    }

    void v(const std::string& tag, const std::string& message) {
        log(LogLevel::Verbose, tag, message);
    }
    void d(const std::string& tag, const std::string& message) {
        log(LogLevel::Debug, tag, message);
    }
    void i(const std::string& tag, const std::string& message) {
        log(LogLevel::Info, tag, message);
    }
    void w(const std::string& tag, const std::string& message) {
        log(LogLevel::Warning, tag, message);
    }
    void e(const std::string& tag, const std::string& message) {
        log(LogLevel::Error, tag, message);
    }
    void f(const std::string& tag, const std::string& message) {
        log(LogLevel::Fatal, tag, message);
    }

    /** Tells the writer thread to take what is in the cache to the log file,
     * and returns at once: nothing is in the file because this returned. What
     * it is for is a drain an app wants soon and does not want to wait for. */
    void requestFlush() {
        withHandle([](long long handle) {
            mars_xlog_request_flush_instance(handle);
        });
    }

    /** Takes what is in the cache to the log file on the calling thread: the
     * records are on disk when it returns, and what it costs is the time the
     * drain takes. */
    void flushNow() {
        withHandle([](long long handle) {
            mars_xlog_flush_now_instance(handle);
        });
    }

    /** `flushNow()` for a caller that can wait without holding a thread: the
     * drain runs on one of its own, and the future answers when it is over.
     *
     * A future nobody waits on is a drain nobody waited for, so this one is
     * `[[nodiscard]]`. */
    [[nodiscard]] std::future<void> flush() {
        if (!isOpen()) {
            std::promise<void> done;
            done.set_value();
            return done.get_future();
        }
        const long long handle = handle_;
        return std::async(std::launch::async, [handle] {
            mars_xlog_flush_now_instance(handle);
        });
    }

    /** Closes this appender: drains what is left and drops it. Writing through
     * this `Xlog` afterwards writes nothing, and asking the C ABI for this
     * `namePrefix` answers `0`. Safe to call twice.
     *
     * Setting through a closed `Xlog` sets nothing either, which is what
     * Swift's does and not what Kotlin's does: Kotlin throws, and what it is
     * protecting is an appender a closed handle would reach. A closed `Xlog`
     * here reaches nothing at all — every member is a no-op — so there is no
     * exception to catch, and a destructor closes without one. */
    void close() {
        if (!isOpen()) {
            return;
        }
        // A prefix is one appender to the C ABI, so two `Xlog`s of one prefix
        // hold one handle between them — and releasing takes the prefix and
        // not the handle, which drops whatever the prefix answers *now*. Once
        // this object's twin closed the appender and a third one reopened the
        // prefix, releasing here would close an appender that is not ours.
        if (mars_xlog_get_instance(namePrefix_.c_str()) == handle_) {
            mars_xlog_release_instance(namePrefix_.c_str());
        }
        handle_ = 0;
    }

    /// The directory this appender writes its files into, or `std::nullopt`
    /// once it is closed.
    ///
    /// A directory and not a file, which is what the C++'s
    /// `GetCurrentLogPath` answers, and there is no "not yet" state: an open
    /// appender has a directory from the moment it is opened. [`logFiles`] is
    /// the day's file inside it.
    std::optional<std::string> currentLogPath() const {
        // A closed `Xlog` answers nothing, as every member of it does: handle
        // `0` names no appender at all, and that is not this appender's
        // question to answer.
        if (!isOpen()) {
            return std::nullopt;
        }
        return path([this](char* out, std::uint32_t len) {
            return mars_xlog_current_log_path_instance(handle_, out, len);
        });
    }

    /// The log files of the day `daysAgo` days ago that are *there* — `0` is
    /// today, `1` is yesterday. `{}` when the directory holds none of that
    /// day's.
    ///
    /// This is a day of files and not the file being written: what
    /// [`currentLogPath`] answers is one, and this is this appender's own
    /// prefix and directory.
    std::vector<std::string> logFiles(int daysAgo) const {
        return dayPaths(daysAgo, mars_xlog_getfilepath_from_timespan_instance);
    }

    /// The paths of the log files of the day `daysAgo` days ago, whether or
    /// not they are *there yet* — the name an app that is about to write, or
    /// that is naming a file to someone else, asks for.
    std::vector<std::string> logFileNames(int daysAgo) const {
        return dayPaths(daysAgo, mars_xlog_make_logfile_name_instance);
    }

private:
    Xlog(long long handle, const XlogConfig& config)
        : handle_(handle),
          namePrefix_(config.namePrefix),
          currentMode_(config.mode),
          consoleLogEnabled_(false),
          maxFileSizeBytes_(0),
          maxAliveTimeSeconds_(0) {
    }

    /// What `body` writes into the buffer it is handed, as a string; `nullopt`
    /// when it wrote nothing.
    ///
    /// `MARS_XLOG_ERR_NO_SPACE` is a buffer that was too small and not an
    /// answer of "no path", so the buffer grows and the question is asked
    /// again: a path of a deep directory is a path an app still wants, and a
    /// walk that took a short buffer for the end of the list would answer a
    /// day with no files in it. Past 64 KiB the question is left unanswered.
    std::optional<std::string> path(
        const std::function<int(char*, std::uint32_t)>& body) const {
        for (std::size_t size = 1024; size <= 65536; size *= 2) {
            std::vector<char> buffer(size);
            int written = body(buffer.data(), static_cast<std::uint32_t>(buffer.size()));
            if (written > 0) {
                return std::string(buffer.data(), static_cast<std::size_t>(written));
            }
            if (written != MARS_XLOG_ERR_NO_SPACE) {
                return std::nullopt;
            }
        }
        return std::nullopt;
    }

    /// The paths of one day, walked index by index until the symbol answers
    /// that there is nothing at that index: the list the C++ fills a
    /// `std::vector` with, asked one at a time.
    std::vector<std::string> dayPaths(
        int daysAgo,
        int (*symbol)(long long, int, unsigned int, char*, unsigned int)) const {
        std::vector<std::string> walked;
        if (!isOpen()) {
            return walked;
        }
        for (unsigned int index = 0;; ++index) {
            auto found = path([&](char* out, std::uint32_t len) {
                return symbol(handle_, daysAgo, index, out, len);
            });
            if (!found) {
                break;
            }
            walked.push_back(*found);
        }
        return walked;
    }

    /** Runs `body` with this appender's handle, and runs nothing at all once
     * `close()` has: handle `0` names no appender at all, so a call through it
     * would silently write nothing. */
    template <typename Body>
    void withHandle(Body body) const {
        if (isOpen()) {
            body(handle_);
        }
    }

    static int maxCompressionLevel(CompressMode mode) {
        // Nine levels is all a deflate has, where zstd's table runs to
        // `ZSTD_maxCLevel()`.
        return mode == CompressMode::Zstd ? 22 : 9;
    }

    long long handle_;
    std::string namePrefix_;
    AppenderMode currentMode_;
    bool consoleLogEnabled_;
    std::uint64_t maxFileSizeBytes_;
    std::int64_t maxAliveTimeSeconds_;
};

}  // namespace xlog
}  // namespace marsrs

#endif /* MARS_XLOG_HPP_ */
