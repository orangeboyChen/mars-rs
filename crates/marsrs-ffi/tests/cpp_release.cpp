// The claim about `marsrs::xlog::Xlog::close()` that a compile cannot make: it
// closes the appender this handle names, and not the one the prefix answers.
//
// A prefix is one appender to the C ABI, and releasing takes the prefix and
// not the handle — so asking the registry which handle the prefix answers and
// then releasing it is two answers and not one. An `open` of the same prefix
// that lands between the two is handed a handle of its own, and the release
// closes that one while the appender this `Xlog` asked about stays open: a
// close that closes another's logger. `mars_xlog_release_instance_of` is the
// question and the release in one call, and what is under test here is that
// `close()` asks in that spelling.
//
// The C ABI is mocked, so the interleaving is one this file can force: the
// answer `mars_xlog_get_instance` gives is the one the registry gave before the
// second open landed, which is what a caller that asked first is holding.
// Nothing of the Rust library is linked.

#include "mars_xlog.hpp"

#include <atomic>
#include <cstdio>

namespace {

// The registry, as the C ABI sees it: the handle the prefix answers now, the
// answer it gave a caller before the second open landed, and every handle it
// has handed out.
std::atomic<long long> registered{0};
std::atomic<long long> answered{0};
std::atomic<long long> handed[4]{};
std::atomic<int> opens{0};
std::atomic<long long> next_handle{1};

// What the test is watching for: a release that took an appender that is not
// the one it was asked about. `intended` is the handle the closing `Xlog`
// means, which the C ABI is never told — this file knows it because telling
// the two releases apart is the whole of what it is for.
std::atomic<long long> intended{0};
std::atomic<bool> closed_another{false};

}  // namespace

extern "C" {

long long mars_xlog_new_instance(const MarsXLogConfig*, int) {
    const long long handle = next_handle.fetch_add(1);
    handed[opens.fetch_add(1)].store(handle);
    registered.store(handle);
    answered.store(handle);
    return handle;
}

long long mars_xlog_get_instance(const char*) {
    return answered.load();
}

// Releasing by prefix and by nothing else: what the prefix answers *now* is
// what goes, which is the other appender once a second open has landed.
void mars_xlog_release_instance(const char*) {
    if (registered.load() != intended.load()) {
        closed_another.store(true);
    }
    registered.store(0);
}

void mars_xlog_release_instance_of(const char*, long long instance) {
    if (registered.load() != instance) {
        return;
    }
    registered.store(0);
}

void mars_xlog_request_flush_instance(long long) {}
void mars_xlog_flush_now_instance(long long) {}
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
int mars_xlog_current_log_path_instance(long long, char*, unsigned int) {
    return 0;
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
    config.logDir = "/tmp/marsrs-cpp-release";
    config.namePrefix = "marsrs";

    // One appender, closed by the one that opened it.
    {
        marsrs::xlog::Xlog log = marsrs::xlog::Xlog::open(config);
        intended.store(handed[0].load());
        log.close();
        log.close();   // and safe twice
        if (registered.load() != 0) {
            std::printf("a close of an open appender left it registered\n");
            return 1;
        }
        if (closed_another.load()) {
            std::printf("a close released an appender that is not the one it was asked about\n");
            return 1;
        }
    }

    // The same, with an open of the prefix landing after this `Xlog` asked
    // which handle it answers: what is closed is this one's, or nothing.
    {
        marsrs::xlog::Xlog first = marsrs::xlog::Xlog::open(config);
        const long long mine = handed[opens.load() - 1].load();
        intended.store(mine);

        // Another part of the app opens the prefix, and the answer `first` is
        // holding is the one from before it did.
        MarsXLogConfig again{};
        again.log_dir = config.logDir.c_str();
        again.name_prefix = config.namePrefix.c_str();
        const long long theirs = mars_xlog_new_instance(&again, 1);
        answered.store(mine);

        first.close();
        if (registered.load() != theirs) {
            std::printf("a close took the appender another part of the app had just opened\n");
            return 1;
        }
        if (closed_another.load()) {
            std::printf("a close released an appender that is not the one it was asked about\n");
            return 1;
        }
    }
    return 0;
}
