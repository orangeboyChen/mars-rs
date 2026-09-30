// The NAPI module of the HarmonyOS xlog package: `libmarsrs_xlog.so`, the
// native half behind `src/main/ets/xlog/Xlog.ets`.
//
// It is a thin seam over `crates/marsrs-ffi`'s C ABI and nothing else — no
// logging logic lives here. What it holds that the C ABI does not is the one
// thing an ArkTS caller cannot hold: the `long long` handle
// `mars_xlog_new_instance` answers. So this module keeps a prefix -> handle
// table of its own, and every method an `Xlog` calls carries the name prefix
// and not a handle: the ArkTS side is one `Xlog` per prefix, the table is one
// handle per prefix, and the two are the same one-to-one by construction.
//
// That table is also what makes `Xlog.open` of an open prefix a second name for
// the same appender and not a second appender: the C ABI is one appender per
// prefix, and a second `mars_xlog_new_instance` over the first would be a
// handle nothing releases.
//
// Every method is synchronous and returns no promise, which is why the ArkTS
// above it is shaped like the Kotlin and not like the Flutter: a NAPI method
// that answers no promise is a call, and the record is in the C ABI before the
// call returns.
//
// C, in a `.cpp` file: a shim that reaches for `std::mutex` asks the process
// that loads it for a C++ runtime it would otherwise never load, and the
// OpenHarmony SDK this is built with does not even carry libc++'s headers in
// the tree scripts/build_harmony.sh unpacks. `pthread` is in the C library
// here, as it is everywhere the Rust core is built for, so the table is a
// malloc'd array under a pthread mutex and the module resolves nothing the
// appender does not already resolve.
//
// The library links `libmars_ffi.a` — the staticlib of `marsrs-ffi` — so the
// HAR carries one `.so` per ABI and an app that takes it resolves nothing else.
// `scripts/build_harmony_napi.sh` is what links it, with the OpenHarmony SDK's
// clang, because hvigor does not build this: the `.so` a release packs into the
// HAR is built the way `libmars_ffi.so` next to it is.
//
// `libace_napi` is not linked and not needed at link time: the napi symbols this
// calls are answered by the process that loads the library — an app's own — and
// the linker is told that with `--allow-shlib-undefined`, which is how a NAPI
// module of the SDK's own is built.

#include <napi/native_api.h>

#include <pthread.h>
#include <stdlib.h>
#include <string.h>

#include "mars_xlog.h"

// `XlogConfig.namePrefix` of the ArkTS: what an appender is opened with when a
// caller gives none. The ArkTS defaults it before it reaches this module, and
// this is the same default again, so a caller that hands the config through
// unspelled lands on the same appender either way.
static const char* const kDefaultNamePrefix = "xlog";

// One appender this module has opened, by name prefix.
typedef struct {
    char* prefix;
    long long handle;
} Instance;

// What `mars_xlog_new_instance` answered, by name prefix. Guarded by a mutex:
// every method may be called from any thread, and `Xlog` of the ArkTS says so.
static pthread_mutex_t g_lock = PTHREAD_MUTEX_INITIALIZER;
static Instance* g_instances = NULL;
static size_t g_instances_count = 0;
static size_t g_instances_capacity = 0;

// --- values ---------------------------------------------------------------

// A copy of `s`, malloc'd, or NULL. `strdup` is not spelled for what it is here
// on purpose: which feature macros a libc of this platform wants before it
// declares that one is not a question this shim needs an answer to.
static char* Duplicate(const char* s) {
    size_t length = strlen(s) + 1;
    char* out = (char*)malloc(length);
    if (out == NULL) {
        return NULL;
    }
    memcpy(out, s, length);
    return out;
}

// The string of `value`, malloc'd, or NULL when it is not a string. The length
// is asked for first and the buffer is that long, so a message of any size
// crosses whole and no path is cut off at an arbitrary limit.
static char* CopyString(napi_env env, napi_value value) {
    napi_valuetype type = napi_undefined;
    if (napi_typeof(env, value, &type) != napi_ok || type != napi_string) {
        return NULL;
    }
    size_t length = 0;
    if (napi_get_value_string_utf8(env, value, NULL, 0, &length) != napi_ok) {
        return NULL;
    }
    char* out = (char*)malloc(length + 1);
    if (out == NULL) {
        return NULL;
    }
    size_t written = 0;
    if (napi_get_value_string_utf8(env, value, out, length + 1, &written) != napi_ok) {
        free(out);
        return NULL;
    }
    out[written] = '\0';
    return out;
}

// A string field of the config object, or NULL when it is absent, or not a
// string, or empty. An absent one is left alone, so `fallback` is what the
// caller gets — which is how the defaults above are spelled at the call sites.
static char* NamedString(napi_env env, napi_value object, const char* key, const char* fallback) {
    napi_value property = NULL;
    if (napi_get_named_property(env, object, key, &property) != napi_ok || property == NULL) {
        return fallback == NULL ? NULL : Duplicate(fallback);
    }
    char* value = CopyString(env, property);
    if (value == NULL || value[0] == '\0') {
        free(value);
        return fallback == NULL ? NULL : Duplicate(fallback);
    }
    return value;
}

// A number field of the config object, or `fallback` when it is absent or is
// not a number. Every number of the config is an int at the C ABI, so the double
// ArkTS hands over is read as one here and not carried as a double.
static int NamedInt(napi_env env, napi_value object, const char* key, int fallback) {
    napi_value property = NULL;
    if (napi_get_named_property(env, object, key, &property) != napi_ok || property == NULL) {
        return fallback;
    }
    napi_valuetype type = napi_undefined;
    if (napi_typeof(env, property, &type) != napi_ok || type != napi_number) {
        return fallback;
    }
    int32_t value = 0;
    if (napi_get_value_int32(env, property, &value) != napi_ok) {
        return fallback;
    }
    return value;
}

// The string argument at `index`, malloc'd, or NULL: what every method that
// takes a name prefix starts with.
static char* ArgString(napi_env env, napi_callback_info info, size_t index) {
    size_t argc = index + 1;
    napi_value argv[2] = {NULL, NULL};
    napi_value self = NULL;
    if (napi_get_cb_info(env, info, &argc, argv, &self, NULL) != napi_ok || argc <= index) {
        return NULL;
    }
    return CopyString(env, argv[index]);
}

// --- the table ------------------------------------------------------------

// The handle of a prefix, or 0 when this module has no appender of that name —
// which is every method's answer for "the appender is closed": a call after
// `close` is dropped, and not a use of a handle that was released.
static long long HandleOf(const char* namePrefix) {
    if (namePrefix == NULL) {
        return 0;
    }
    pthread_mutex_lock(&g_lock);
    long long handle = 0;
    for (size_t i = 0; i < g_instances_count; ++i) {
        if (strcmp(g_instances[i].prefix, namePrefix) == 0) {
            handle = g_instances[i].handle;
            break;
        }
    }
    pthread_mutex_unlock(&g_lock);
    return handle;
}

// `1` when the table remembers `handle` under `namePrefix` — overwriting an
// entry of that name, which is what makes `open` of an open prefix an update
// and not a second appender — and `0` when it cannot: no memory for the entry.
static int Remember(const char* namePrefix, long long handle) {
    pthread_mutex_lock(&g_lock);
    for (size_t i = 0; i < g_instances_count; ++i) {
        if (strcmp(g_instances[i].prefix, namePrefix) == 0) {
            g_instances[i].handle = handle;
            pthread_mutex_unlock(&g_lock);
            return 1;
        }
    }
    if (g_instances_count == g_instances_capacity) {
        size_t capacity = g_instances_capacity == 0 ? 4 : g_instances_capacity * 2;
        Instance* grown = (Instance*)realloc(g_instances, capacity * sizeof(*grown));
        if (grown == NULL) {
            pthread_mutex_unlock(&g_lock);
            return 0;
        }
        g_instances = grown;
        g_instances_capacity = capacity;
    }
    char* prefix = Duplicate(namePrefix);
    if (prefix == NULL) {
        pthread_mutex_unlock(&g_lock);
        return 0;
    }
    g_instances[g_instances_count].prefix = prefix;
    g_instances[g_instances_count].handle = handle;
    ++g_instances_count;
    pthread_mutex_unlock(&g_lock);
    return 1;
}

// `mars_xlog_release_instance`, and the table's own entry for the prefix: an
// appender closed here is a prefix the next `open` opens afresh, which is what
// makes `close` and `open` of one prefix a pair and not a leak. The last entry
// is moved into the hole, because the table is a set and not a list.
static void Forget(const char* namePrefix) {
    pthread_mutex_lock(&g_lock);
    for (size_t i = 0; i < g_instances_count; ++i) {
        if (strcmp(g_instances[i].prefix, namePrefix) == 0) {
            free(g_instances[i].prefix);
            g_instances[i] = g_instances[g_instances_count - 1];
            --g_instances_count;
            break;
        }
    }
    pthread_mutex_unlock(&g_lock);
}

// --- napi values ----------------------------------------------------------

static napi_value Boolean(napi_env env, bool value) {
    napi_value result = NULL;
    napi_get_boolean(env, value, &result);
    return result;
}

static napi_value Int32(napi_env env, int32_t value) {
    napi_value result = NULL;
    napi_create_int32(env, value, &result);
    return result;
}

static napi_value Undefined(napi_env env) {
    napi_value result = NULL;
    napi_get_undefined(env, &result);
    return result;
}

// --- open(config): boolean -------------------------------------------------

static napi_value Open(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {NULL};
    napi_value self = NULL;
    if (napi_get_cb_info(env, info, &argc, argv, &self, NULL) != napi_ok || argc < 1 ||
        argv[0] == NULL) {
        return Boolean(env, false);
    }

    // `logDir` is the one field with no default, and the C ABI answers
    // MARS_XLOG_ERR_EMPTY_LOG_DIR without it: an appender with nowhere to write
    // is refused here rather than opened and found empty.
    char* logDir = NamedString(env, argv[0], "logDir", NULL);
    if (logDir == NULL) {
        return Boolean(env, false);
    }
    char* namePrefix = NamedString(env, argv[0], "namePrefix", kDefaultNamePrefix);
    char* pubKey = NamedString(env, argv[0], "pubKey", NULL);
    char* cacheDir = NamedString(env, argv[0], "cacheDir", NULL);
    if (namePrefix == NULL) {
        free(logDir);
        free(pubKey);
        free(cacheDir);
        return Boolean(env, false);
    }

    MarsXLogConfig config;
    config.mode = NamedInt(env, argv[0], "mode", MarsAppenderAsync);
    config.log_dir = logDir;
    config.name_prefix = namePrefix;
    config.pub_key = pubKey;
    config.compress_mode = NamedInt(env, argv[0], "compressMode", MarsCompressZlib);
    config.compress_level = NamedInt(env, argv[0], "compressLevel", 0);
    config.cache_dir = cacheDir;
    config.cache_days = NamedInt(env, argv[0], "cacheDays", 0);
    int level = NamedInt(env, argv[0], "level", MarsLevelInfo);

    long long handle = mars_xlog_get_instance(namePrefix);
    int opened = 0;
    if (handle == 0) {
        // A config the C ABI refuses is a negative `MARS_XLOG_ERR_*` code and
        // never `0`, which is the process-wide appender: an `open` that read `0`
        // as "opened" would register a logger this module never opened, and every
        // call after this one would write through it.
        handle = mars_xlog_new_instance(&config, level);
        opened = handle > 0;
    }
    if (handle > 0 && !Remember(namePrefix, handle)) {
        // The table is what every call after this one goes through, so an
        // appender it cannot remember is an appender it cannot reach: closed
        // again, rather than opened and left for the process to leak.
        //
        // Only the instance this call opened is the one closed here. A handle
        // `get_instance` answered with is an appender someone else registered
        // under this prefix — a release by name is the release of theirs, and
        // it would close a live logger, and flush its cache, out from under
        // whoever opened it.
        if (opened) {
            mars_xlog_release_instance(namePrefix);
        }
        handle = 0;
    }

    free(logDir);
    free(namePrefix);
    free(pubKey);
    free(cacheDir);
    return Boolean(env, handle > 0);
}

// --- level ----------------------------------------------------------------

static napi_value GetLevel(napi_env env, napi_callback_info info) {
    char* namePrefix = ArgString(env, info, 0);
    long long handle = HandleOf(namePrefix);
    free(namePrefix);
    // Two ways of having no level to answer: a prefix this table never opened,
    // which is what `close` leaves behind and what a call after it is, and a
    // handle the C ABI does not know — it answers -1, and -1 is not a
    // `LogLevel`. Both are answered with `MarsLevelInfo`, the level an
    // appender is opened at when the caller asked for none, which is the one
    // the ArkTS holds beside it.
    int level = handle == 0 ? MarsLevelInfo : mars_xlog_get_level(handle);
    return Int32(env, level < 0 ? MarsLevelInfo : level);
}

static napi_value SetLevel(napi_env env, napi_callback_info info) {
    char* namePrefix = ArgString(env, info, 0);
    long long handle = HandleOf(namePrefix);
    free(namePrefix);
    if (handle == 0) {
        return Undefined(env);
    }
    size_t argc = 2;
    napi_value argv[2] = {NULL, NULL};
    napi_value self = NULL;
    if (napi_get_cb_info(env, info, &argc, argv, &self, NULL) == napi_ok && argc >= 2) {
        int32_t level = MarsLevelInfo;
        if (napi_get_value_int32(env, argv[1], &level) == napi_ok) {
            mars_xlog_set_level_instance(handle, level);
        }
    }
    return Undefined(env);
}

// --- the four settings the C ABI has no getter for -------------------------

static napi_value SetMode(napi_env env, napi_callback_info info) {
    char* namePrefix = ArgString(env, info, 0);
    long long handle = HandleOf(namePrefix);
    free(namePrefix);
    if (handle == 0) {
        return Undefined(env);
    }
    size_t argc = 2;
    napi_value argv[2] = {NULL, NULL};
    napi_value self = NULL;
    if (napi_get_cb_info(env, info, &argc, argv, &self, NULL) == napi_ok && argc >= 2) {
        int32_t mode = MarsAppenderAsync;
        if (napi_get_value_int32(env, argv[1], &mode) == napi_ok) {
            mars_xlog_set_mode_instance(handle, mode);
        }
    }
    return Undefined(env);
}

static napi_value SetConsoleLogEnabled(napi_env env, napi_callback_info info) {
    char* namePrefix = ArgString(env, info, 0);
    long long handle = HandleOf(namePrefix);
    free(namePrefix);
    if (handle == 0) {
        return Undefined(env);
    }
    size_t argc = 2;
    napi_value argv[2] = {NULL, NULL};
    napi_value self = NULL;
    if (napi_get_cb_info(env, info, &argc, argv, &self, NULL) == napi_ok && argc >= 2) {
        bool enabled = false;
        if (napi_get_value_bool(env, argv[1], &enabled) == napi_ok) {
            mars_xlog_set_console_log_instance(handle, enabled ? 1 : 0);
        }
    }
    return Undefined(env);
}

// A number of bytes ArkTS hands over as a `number`, which is a double: read as
// one and narrowed, because `mars_xlog_set_max_file_size_instance` takes
// `unsigned long long` and a size above 2^53 is not a size an app has.
static napi_value SetMaxFileSize(napi_env env, napi_callback_info info) {
    char* namePrefix = ArgString(env, info, 0);
    long long handle = HandleOf(namePrefix);
    free(namePrefix);
    if (handle == 0) {
        return Undefined(env);
    }
    size_t argc = 2;
    napi_value argv[2] = {NULL, NULL};
    napi_value self = NULL;
    if (napi_get_cb_info(env, info, &argc, argv, &self, NULL) == napi_ok && argc >= 2) {
        double bytes = 0;
        // `0` is a size and not a missing one: `mars_xlog.h` reads it as "do
        // not split", so what is refused here is a negative — and a NaN, which
        // is a `number` ArkTS computed and gave no size with. So is every
        // value the 64-bit integer cannot hold — an infinity, and any `number`
        // from 2^64 up: the cast of one is undefined behaviour, and what it
        // would have left the appender with is a limit nobody asked for.
        // The bound is exact, 2^64 being a `double` there is one of.
        if (napi_get_value_double(env, argv[1], &bytes) == napi_ok && bytes >= 0 &&
            bytes < 18446744073709551616.0) {
            mars_xlog_set_max_file_size_instance(handle, (unsigned long long)bytes);
        }
    }
    return Undefined(env);
}

static napi_value SetMaxAliveTime(napi_env env, napi_callback_info info) {
    char* namePrefix = ArgString(env, info, 0);
    long long handle = HandleOf(namePrefix);
    free(namePrefix);
    if (handle == 0) {
        return Undefined(env);
    }
    size_t argc = 2;
    napi_value argv[2] = {NULL, NULL};
    napi_value self = NULL;
    if (napi_get_cb_info(env, info, &argc, argv, &self, NULL) == napi_ok && argc >= 2) {
        int32_t seconds = 0;
        if (napi_get_value_int32(env, argv[1], &seconds) == napi_ok) {
            mars_xlog_set_max_alive_duration_instance(handle, (long long)seconds);
        }
    }
    return Undefined(env);
}

// --- writing --------------------------------------------------------------

static napi_value IsLoggable(napi_env env, napi_callback_info info) {
    char* namePrefix = ArgString(env, info, 0);
    long long handle = HandleOf(namePrefix);
    free(namePrefix);
    size_t argc = 2;
    napi_value argv[2] = {NULL, NULL};
    napi_value self = NULL;
    if (handle == 0 ||
        napi_get_cb_info(env, info, &argc, argv, &self, NULL) != napi_ok || argc < 2) {
        return Boolean(env, false);
    }
    int32_t level = MarsLevelInfo;
    if (napi_get_value_int32(env, argv[1], &level) != napi_ok) {
        return Boolean(env, false);
    }
    return Boolean(env, mars_xlog_is_enabled_for(handle, level) != 0);
}

// `mars_xlog_write_instance`: the file, the function and the line of the record
// are left empty, the way the other platforms leave them — there is no ArkTS
// frame worth naming in a record, and the C++ writes an empty one too.
static napi_value Log(napi_env env, napi_callback_info info) {
    char* namePrefix = ArgString(env, info, 0);
    long long handle = HandleOf(namePrefix);
    free(namePrefix);
    size_t argc = 4;
    napi_value argv[4] = {NULL, NULL, NULL, NULL};
    napi_value self = NULL;
    if (handle == 0 ||
        napi_get_cb_info(env, info, &argc, argv, &self, NULL) != napi_ok || argc < 4) {
        return Undefined(env);
    }
    int32_t level = MarsLevelInfo;
    // A level this module cannot read is a record it does not write: the C ABI
    // drops a level that is not one of the six, so writing the default would be
    // a record at a level the caller did not ask for — the same answer
    // `IsLoggable` gives a level it cannot read, and the one every other
    // method here gives an argument it cannot read.
    if (napi_get_value_int32(env, argv[1], &level) != napi_ok) {
        return Undefined(env);
    }
    char* tag = CopyString(env, argv[2]);
    char* message = CopyString(env, argv[3]);
    mars_xlog_write_instance(handle, level, tag == NULL ? "" : tag, "", "", 0,
                             message == NULL ? "" : message);
    free(tag);
    free(message);
    return Undefined(env);
}

// `mars_xlog_request_flush_instance` and `mars_xlog_flush_now_instance`, as two
// methods and not as one carrying a `sync`: whether the caller waits is not a
// detail a call site can be trusted to spell, and it is the whole of the
// difference between the two — `RequestFlush` tells the writer thread it may
// drain and returns at once, `FlushNow` drains on the calling thread and is
// over when it returns.
static napi_value RequestFlush(napi_env env, napi_callback_info info) {
    char* namePrefix = ArgString(env, info, 0);
    long long handle = HandleOf(namePrefix);
    free(namePrefix);
    if (handle == 0) {
        return Undefined(env);
    }
    mars_xlog_request_flush_instance(handle);
    return Undefined(env);
}

static napi_value FlushNow(napi_env env, napi_callback_info info) {
    char* namePrefix = ArgString(env, info, 0);
    long long handle = HandleOf(namePrefix);
    free(namePrefix);
    if (handle == 0) {
        return Undefined(env);
    }
    mars_xlog_flush_now_instance(handle);
    return Undefined(env);
}

static napi_value Close(napi_env env, napi_callback_info info) {
    char* namePrefix = ArgString(env, info, 0);
    if (namePrefix == NULL) {
        return Undefined(env);
    }
    // A prefix this table never opened is not one this module releases: a
    // release by name is the release of whatever appender the C ABI holds under
    // that name — an appender another caller opened, or the process-wide one an
    // empty prefix names — and `close` of one is a close of theirs. Every other
    // method answers an unknown prefix by dropping the call, and so does this
    // one.
    if (HandleOf(namePrefix) == 0) {
        free(namePrefix);
        return Undefined(env);
    }
    Forget(namePrefix);
    mars_xlog_release_instance(namePrefix);
    free(namePrefix);
    return Undefined(env);
}

// --- the module -----------------------------------------------------------

static napi_value Init(napi_env env, napi_value exports) {
    napi_property_descriptor properties[] = {
        {"open", NULL, Open, NULL, NULL, NULL, napi_default, NULL},
        {"getLevel", NULL, GetLevel, NULL, NULL, NULL, napi_default, NULL},
        {"setLevel", NULL, SetLevel, NULL, NULL, NULL, napi_default, NULL},
        {"setMode", NULL, SetMode, NULL, NULL, NULL, napi_default, NULL},
        {"setConsoleLogEnabled", NULL, SetConsoleLogEnabled, NULL, NULL, NULL, napi_default, NULL},
        {"setMaxFileSize", NULL, SetMaxFileSize, NULL, NULL, NULL, napi_default, NULL},
        {"setMaxAliveTime", NULL, SetMaxAliveTime, NULL, NULL, NULL, napi_default, NULL},
        {"isLoggable", NULL, IsLoggable, NULL, NULL, NULL, napi_default, NULL},
        {"log", NULL, Log, NULL, NULL, NULL, napi_default, NULL},
        {"requestFlush", NULL, RequestFlush, NULL, NULL, NULL, napi_default, NULL},
        {"flushNow", NULL, FlushNow, NULL, NULL, NULL, napi_default, NULL},
        {"close", NULL, Close, NULL, NULL, NULL, napi_default, NULL},
    };
    napi_define_properties(env, exports, sizeof(properties) / sizeof(properties[0]), properties);
    return exports;
}

// `nm_modname` is the name an ArkTS `import ... from 'libmarsrs_xlog.so'`
// resolves: the library's own name with its `lib` and its `.so` off.
static napi_module g_module = {
    .nm_version = 1,
    .nm_flags = 0,
    .nm_filename = NULL,
    .nm_register_func = Init,
    .nm_modname = "marsrs_xlog",
    .nm_priv = NULL,
    .reserved = {NULL},
};

extern "C" __attribute__((constructor)) void RegisterMarsrsXlogModule(void) {
    napi_module_register(&g_module);
}
