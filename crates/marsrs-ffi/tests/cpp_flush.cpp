// The behaviour of `marsrs::xlog::Xlog` that a compile cannot show: a
// `close()` waits for the drain `flush()` started.
//
// What the drain is handed is the handle's number and not a claim on the
// appender behind it, so a release that lands first leaves it flushing an id
// no appender answers — a silent no-op, and ids are never reused, so nothing
// flushes those records afterwards either. Overtaking the drain takes a drain
// slow enough to overtake, and a drain is not slow on its own, so the C ABI is
// mocked here: every symbol the header calls is answered by this file, and the
// one that matters sleeps.
//
// Nothing of the Rust library is linked. What is under test is the header's own
// logic, and a mocked C ABI leaves all of it intact.

#include "mars_xlog.hpp"

#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <thread>

namespace {

// What the mock is asked: the one handle it answers, whether the drain ran,
// and whether a release beat the drain to the appender.
std::atomic<long long> registered{0};
std::atomic<long long> next_handle{1};
std::atomic<bool> drained{false};
std::atomic<bool> released_before_drain{false};

}  // namespace

extern "C" {

long long mars_xlog_new_instance(const MarsXLogConfig*, int) {
    const long long handle = next_handle.fetch_add(1);
    registered.store(handle);
    return handle;
}

long long mars_xlog_get_instance(const char*) {
    return registered.load();
}

void mars_xlog_release_instance(const char*) {
    if (!drained.load()) {
        released_before_drain.store(true);
    }
    registered.store(0);
}

void mars_xlog_release_instance_of(const char*, long long) {
    if (!drained.load()) {
        released_before_drain.store(true);
    }
    registered.store(0);
}

// The drain: slow, because what this file is about is a `close()` that gets
// there before it does.
void mars_xlog_flush_now_instance(long long) {
    std::this_thread::sleep_for(std::chrono::milliseconds(400));
    drained.store(true);
}

void mars_xlog_request_flush_instance(long long) {}
void mars_xlog_write_instance(long long, int, const char*, const char*, const char*, int, const char*) {}
int mars_xlog_is_enabled_for(long long, int) {
    return 1;
}
int mars_xlog_get_level(long long) {
    return 1;
}
void mars_xlog_set_level_instance(long long, int) {}
void mars_xlog_set_mode_instance(long long, int) {}
void mars_xlog_set_console_log_instance(long long, int) {}
void mars_xlog_set_max_file_size_instance(long long, unsigned long long) {}
void mars_xlog_set_max_alive_duration_instance(long long, long long) {}
int mars_xlog_current_log_path_instance(long long, char* out, unsigned int len) {
    if (len == 0) {
        return 0;
    }
    std::strncpy(out, "/tmp", len - 1);
    out[len - 1] = '\0';
    return 1;
}
int mars_xlog_getfilepath_from_timespan_instance(long long, int, unsigned int, char*, unsigned int) {
    return 0;
}
int mars_xlog_make_logfile_name_instance(long long, int, unsigned int, char*, unsigned int) {
    return 0;
}

}  // extern "C"

int main() {
    marsrs::xlog::XlogConfig config;
    config.logDir = "/tmp/marsrs-cpp-flush";
    config.namePrefix = "marsrs";

    marsrs::xlog::Xlog log = marsrs::xlog::Xlog::open(config);
    std::future<void> awaited = log.flush();
    log.close();

    if (!drained.load()) {
        std::printf("a close let go of the appender before the drain it started ran\n");
        return 1;
    }
    if (released_before_drain.load()) {
        std::printf("a release landed before the drain it was asked to wait for\n");
        return 1;
    }
    awaited.get();
    return 0;
}
