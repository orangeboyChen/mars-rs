// The translation unit `tests/cpp_surface.rs` compiles: what keeps
// `include/mars_xlog.hpp` honest.
//
// Every member of `marsrs::xlog::Xlog` is called here once, so a header that
// drifted off the C ABI — a member that names a symbol `mars_xlog.h` does not
// declare, a signature that no longer matches — is a compile error in this file
// and not a link error in an app. Nothing here runs: the Rust test compiles it
// and reads the symbols out of the object file.

#include "mars_xlog.hpp"

#include <iostream>
#include <string>
#include <utility>

// What the compiler does with the header alone, and the one thing a caller of
// `Xlog::open` cannot avoid: the throw, when the config is one the appender
// refuses. Both are here so that the exception type is reachable from a TU that
// includes nothing but the header.
void refused(const marsrs::xlog::XlogConfig& config) {
    try {
        marsrs::xlog::Xlog log = marsrs::xlog::Xlog::open(config);
        log.i("startup", "hello from mars");
    } catch (const marsrs::xlog::XlogError& error) {
        std::cout << error.what() << static_cast<int>(error.reason()) << "\n";
    }
}

// The surface: open, write, set, drain, close.
void surface(const std::string& logDirectory) {
    using marsrs::xlog::LogLevel;
    using marsrs::xlog::Xlog;
    using marsrs::xlog::XlogConfig;

    XlogConfig config;
    config.logDir = logDirectory;
    config.namePrefix = "marsrs";
    config.level = LogLevel::Info;
    config.mode = marsrs::xlog::AppenderMode::Async;
    config.compressMode = marsrs::xlog::CompressMode::Zstd;
    config.compressLevel = 3;
    config.cacheDays = 0;

    Xlog log = Xlog::open(config);
    log.setLevel(LogLevel::Debug);
    log.setMode(marsrs::xlog::AppenderMode::Sync);
    log.setConsoleLogEnabled(true);
    log.setMaxFileSizeBytes(1u << 20);
    log.setMaxAliveTimeSeconds(3600);
    log.setMaxAliveTimeSeconds(-1);   // clamped by the C ABI, as on every platform

    if (log.isLoggable(LogLevel::Debug)) {
        log.d("startup", "hello from mars");
    }
    log.v("startup", "verbose");
    log.i("startup", "info");
    log.w("startup", "warn");
    log.e("startup", "error");
    log.f("startup", "fatal");
    log.log(LogLevel::Info, "startup", "with a call site", __FILE__, __PRETTY_FUNCTION__, __LINE__);

    log.requestFlush();
    log.flushNow();
    log.flush().get();

    std::cout << log.namePrefix() << log.isOpen() << static_cast<int>(log.level())
              << static_cast<int>(log.mode()) << log.consoleLogEnabled() << log.maxFileSizeBytes()
              << log.maxAliveTimeSeconds() << "\n";

    // A prefix is one appender to the C ABI, so this one moves and does not
    // copy: two copies of one handle are two owners of one close.
    Xlog moved = std::move(log);
    moved.close();
    moved.close();   // safe twice, as on every platform
}

// The console sink: where the console copy of a record goes instead of stderr.
void sink() {
    marsrs::xlog::Xlog::setConsoleSink(
        [](marsrs::xlog::LogLevel level,
           std::string_view tag,
           std::string_view file,
           std::string_view function,
           int line,
           std::string_view message) {
            std::cout << static_cast<int>(level) << tag << file << function << line << message << "\n";
        });
    marsrs::xlog::Xlog::setConsoleSink(nullptr);
}
