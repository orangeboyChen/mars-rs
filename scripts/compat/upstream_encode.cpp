// The C++ half of the cross-read test: the encoder that mirrors
// `xlog-compat encode` exactly, so that the two implementations can be pointed
// at each other's files.
//
//   upstream_encode --mode=zlib --compress=1 --sync=0 --flush-every=0 \
//       --pubkey=HEX --records=records.bin --out=a.xlog
//
// Every option is the one `crates/mars-compat/src/lib.rs` reads, with the same
// meaning: `sync` picks `Write(data, len, out_buff)` (one block per record, the
// way `__WriteFile` calls it) over `Write(data, len)` + `Flush(out)`, and
// `flush-every` is how often the async path drains the region. `--pubkey`
// empty is "no server key", i.e. the no-crypt magics.
//
// `--appender=1` is the other mode: the records go through the real
// `XloggerAppender` and `--out` is the log directory, so what lands on disk is
// the `<prefix>_YYYYMMDD.xlog` an app would ship, not the block bytes.
//
// It drives the real `LogZlibBuffer` / `LogZstdBuffer` over a region of
// `kBufferBlockLength` bytes, which is what `XloggerAppender` does through its
// mmap'd cache file — the file bytes are these bytes.

#include <dirent.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <string>
#include <vector>

#include "mars/comm/autobuffer.h"
#include "mars/comm/xlogger/xloggerbase.h"
#include "mars/xlog/appender.h"
#include "mars/xlog/src/log_zlib_buffer.h"
#include "mars/xlog/src/log_zstd_buffer.h"
#include "mars/xlog/xlogger.h"

using namespace mars::xlog;

namespace {

std::string opt(int argc, char** argv, const char* key, const char* fallback) {
    size_t len = strlen(key);
    for (int i = 1; i < argc; ++i) {
        if (strncmp(argv[i], "--", 2) != 0) continue;
        const char* arg = argv[i] + 2;
        if (strncmp(arg, key, len) != 0 || arg[len] != '=') continue;
        return std::string(arg + len + 1);
    }
    return std::string(fallback);
}

bool flag(int argc, char** argv, const char* key, bool fallback) {
    std::string value = opt(argc, argv, key, fallback ? "1" : "0");
    return value == "1" || value == "true";
}

/// One record per line; a single trailing newline is not a record — the same
/// rule `mars_compat::read_records` uses, so both sides encode the same number
/// of records out of the same file.
std::vector<std::string> read_records(const std::string& path) {
    FILE* file = fopen(path.c_str(), "rb");
    if (file == NULL) {
        fprintf(stderr, "read %s: cannot open\n", path.c_str());
        exit(1);
    }
    std::vector<std::string> records;
    std::string line;
    int c = 0;
    while ((c = fgetc(file)) != EOF) {
        if (c == '\n') {
            records.push_back(line);
            line.clear();
        } else {
            line.push_back((char)c);
        }
    }
    fclose(file);
    if (!line.empty()) records.push_back(line);
    return records;
}

/// The same records through the real `XloggerAppender`: `appender_open`,
/// `xinfo2` per record, `appender_flush_sync`, `appender_close`. What comes out
/// is a `<prefix>_YYYYMMDD.xlog` in `_out_dir` — the file an app would ship to
/// a server — rather than the block bytes the log buffer produces, so the cross
/// test can decode what an appender actually wrote.
///
/// The paths of the files it left are printed, one per line.
int encode_with_appender(const std::string& out_dir,
                         const std::vector<std::string>& records,
                         const std::string& mode,
                         const std::string& pubkey,
                         bool sync,
                         int level) {
    mars::xlog::XLogConfig config;
    config.mode_ = sync ? mars::xlog::kAppenderSync : mars::xlog::kAppenderAsync;
    config.logdir_ = out_dir;
    config.nameprefix_ = "cross";
    config.pub_key_ = pubkey;
    config.compress_mode_ = mode == "zstd" ? mars::xlog::kZstd : mars::xlog::kZlib;
    config.compress_level_ = level;
    config.cache_days_ = 0;

    mars::xlog::appender_set_console_log(false);
    xlogger_SetLevel(kLevelAll);
    mars::xlog::appender_open(config);

    for (size_t i = 0; i < records.size(); ++i) {
        // `xlogger_Write` rather than `xinfo2(TSF"%0", ...)`: the body goes in
        // as it is, with no type-safe format pass over it, and the appender
        // still stamps the header the way it does for an app.
        XLoggerInfo info;
        memset(&info, 0, sizeof(info));
        info.level = kLevelInfo;
        info.tag = "cross";
        info.filename = "upstream_encode.cpp";
        info.func_name = "encode_with_appender";
        info.line = (int)i;
        gettimeofday(&info.timeval, NULL);
        xlogger_Write(&info, records[i].c_str());
    }

    mars::xlog::appender_flush_sync();
    mars::xlog::appender_close();

    DIR* dir = opendir(out_dir.c_str());
    if (dir == NULL) {
        fprintf(stderr, "read %s: opendir failed\n", out_dir.c_str());
        return 1;
    }
    int found = 0;
    struct dirent* ent = NULL;
    while ((ent = readdir(dir)) != NULL) {
        size_t len = strlen(ent->d_name);
        if (len > 5 && strcmp(ent->d_name + len - 5, ".xlog") == 0) {
            printf("%s/%s\n", out_dir.c_str(), ent->d_name);
            found += 1;
        }
    }
    closedir(dir);
    if (found == 0) {
        fprintf(stderr, "the appender left no .xlog in %s\n", out_dir.c_str());
        return 1;
    }
    return 0;
}

}  // namespace

int main(int argc, char** argv) {
    std::string mode = opt(argc, argv, "mode", "zlib");
    bool is_compress = flag(argc, argv, "compress", true);
    bool sync = flag(argc, argv, "sync", false);
    int level = atoi(opt(argc, argv, "level", "6").c_str());
    size_t region_len = (size_t)atol(opt(argc, argv, "region", "153600").c_str());
    size_t flush_every = (size_t)atol(opt(argc, argv, "flush-every", "0").c_str());
    std::string pubkey = opt(argc, argv, "pubkey", "");
    std::string out_path = opt(argc, argv, "out", "");
    std::string records_path = opt(argc, argv, "records", "");

    if (out_path.empty() || records_path.empty()) {
        fprintf(stderr,
                "usage: upstream_encode --mode=zlib|zstd [--compress=1] [--sync=0] "
                "[--flush-every=N] [--level=6] [--pubkey=HEX] --records=PATH --out=PATH\n");
        return 1;
    }

    std::vector<std::string> records = read_records(records_path);

    // `--appender=1`: the records go through the process-wide appender, and
    // `--out` is the log directory it writes into.
    if (flag(argc, argv, "appender", false)) {
        return encode_with_appender(out_path, records, mode, pubkey, sync, level);
    }

    std::vector<char> region(region_len, 0);
    const char* key = pubkey.empty() ? NULL : pubkey.c_str();

    LogBaseBuffer* buffer = NULL;
    if (mode == "zlib") {
        buffer = new LogZlibBuffer(region.data(), region.size(), is_compress, key);
    } else if (mode == "zstd") {
        buffer = new LogZstdBuffer(region.data(), region.size(), is_compress, key, level);
    } else {
        fprintf(stderr, "--mode must be zlib or zstd, got `%s`\n", mode.c_str());
        return 1;
    }

    std::string bytes;
    if (sync) {
        for (size_t i = 0; i < records.size(); ++i) {
            AutoBuffer block;
            if (!buffer->Write(records[i].data(), records[i].size(), block)) {
                fprintf(stderr, "record %zu: Write rejected it\n", i);
                return 1;
            }
            bytes.append(reinterpret_cast<const char*>(block.Ptr()), block.Length());
        }
    } else {
        size_t buffered = 0;
        for (size_t i = 0; i < records.size(); ++i) {
            if (flush_every > 0 && buffered >= flush_every) {
                AutoBuffer block;
                buffer->Flush(block);
                bytes.append(reinterpret_cast<const char*>(block.Ptr()), block.Length());
                buffered = 0;
            }
            if (!buffer->Write(records[i].data(), records[i].size())) {
                fprintf(stderr, "record %zu (%zu bytes) did not fit in the region\n", i, records[i].size());
                return 1;
            }
            buffered += 1;
        }
        AutoBuffer block;
        buffer->Flush(block);
        bytes.append(reinterpret_cast<const char*>(block.Ptr()), block.Length());
    }

    delete buffer;

    FILE* out = fopen(out_path.c_str(), "wb");
    if (out == NULL) {
        fprintf(stderr, "write %s: cannot open\n", out_path.c_str());
        return 1;
    }
    fwrite(bytes.data(), 1, bytes.size(), out);
    fclose(out);

    printf("encode: %zu records -> %zu bytes\n", records.size(), bytes.size());
    return 0;
}
