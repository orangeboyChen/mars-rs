// The C++ half of the comm cross-read test: `mars/comm/basepacker.cc` and
// `mars/comm/adler32.c`, driven the way `scripts/compat/comm.sh` drives the
// Rust of the same two files.
//
//   upstream_comm adler32 --data=HEX [--seed=N]
//   upstream_comm packer pack --url=U --seq=N --data=HEX [--hash=no] --out=PATH
//   upstream_comm packer unpack --in=PATH
//   upstream_comm simple pack --kind=short|int --data=HEX --out=PATH
//   upstream_comm simple unpack --kind=short|int --in=PATH
//
// `packer unpack` and `simple unpack` print one line: the `int` the C++ answers,
// then what it read out of the package — the URL, the sequence and the length
// for `packer`, the length and the body for `simple`, and the body as hex on
// both. Those are the lines `comm-compat` prints too, so the two can be diffed.
//
// A code that is not `0` is a code and nothing more on either side: the
// `*_CONTINUE*` answers say "read on", and a package that is refused answers the
// `__LINE__` of the check that failed, a positive number that means nothing
// outside `basepacker.cc` — which is why `comm.sh` compares a refused package on
// the sign of its code and not on the code itself.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <string>
#include <vector>

#include "mars/comm/adler32.h"
#include "mars/comm/autobuffer.h"
#include "mars/comm/basepacker.h"

namespace {

// `NULL` for an option the command line did not give, which is not the same
// thing as one it gave empty: `--data=` is a body of no bytes at all.
const char* opt(int argc, char** argv, const char* key) {
    size_t len = strlen(key);
    // From `argv[2]`: the action of `packer` and of `simple` is not an option,
    // and `adler32` has no action at all.
    for (int i = 2; i < argc; ++i) {
        if (strncmp(argv[i], "--", 2) != 0) continue;
        const char* arg = argv[i] + 2;
        if (strncmp(arg, key, len) != 0 || arg[len] != '=') continue;
        return arg + len + 1;
    }
    return NULL;
}

std::vector<unsigned char> unhex(const char* text) {
    std::vector<unsigned char> out;
    if (text == NULL) return out;
    size_t len = strlen(text);
    for (size_t at = 0; at + 1 < len; at += 2) {
        char byte[3] = {text[at], text[at + 1], '\0'};
        out.push_back((unsigned char)strtoul(byte, NULL, 16));
    }
    return out;
}

// A body of no bytes is a pointer and a length of `0`, and not `NULL`: upstream
// reads a `NULL` buffer as its "give me the initial value" answer — `adler32`
// returns `1` for `adler32(seed, NULL, 0)`, and `AutoBuffer::Write` asserts —
// which is a convention about the pointer and not about the bytes, and one the
// port, having no `NULL` to hand over, cannot spell.
const unsigned char* bytes(const std::vector<unsigned char>& data) {
    return data.empty() ? (const unsigned char*)"" : &data[0];
}

std::string hex(const unsigned char* data, size_t len) {
    std::string out;
    out.reserve(len * 2);
    char byte[3];
    for (size_t at = 0; at < len; ++at) {
        snprintf(byte, sizeof(byte), "%02x", data[at]);
        out += byte;
    }
    return out;
}

// The bytes of `--in`, which is what either side packed or a hand-written
// package: the artifact the two readings are of.
std::vector<unsigned char> read_file(const char* path) {
    std::vector<unsigned char> out;
    FILE* f = fopen(path, "rb");
    if (f == NULL) {
        fprintf(stderr, "cannot read %s\n", path);
        exit(1);
    }
    unsigned char buf[4096];
    size_t got = 0;
    while (0 < (got = fread(buf, 1, sizeof(buf), f))) out.insert(out.end(), buf, buf + got);
    fclose(f);
    return out;
}

void write_file(const char* path, const void* data, size_t len) {
    FILE* f = fopen(path, "wb");
    if (f == NULL) {
        fprintf(stderr, "cannot write %s\n", path);
        exit(1);
    }
    if (0 < len) fwrite(data, 1, len, f);
    fclose(f);
}

// `Packer_Pack` / `Packer_Unpack`.
int packer(int argc, char** argv) {
    const char* action = (argc > 2) ? argv[2] : "";

    if (0 == strcmp(action, "pack")) {
        const char* url = opt(argc, argv, "url");
        const char* out_path = opt(argc, argv, "out");
        if (url == NULL || out_path == NULL) {
            fprintf(stderr, "packer pack needs --url=U --out=PATH\n");
            return 1;
        }
        const char* seq_text = opt(argc, argv, "seq");
        const char* hash_text = opt(argc, argv, "hash");
        std::vector<unsigned char> data = unhex(opt(argc, argv, "data"));

        // `strtoul` and not `atoi`: a sequence of `4294967295` is the top of
        // the `unsigned int` the header declares, and `atoi` cannot say it.
        unsigned int sequence = (unsigned int)(seq_text ? strtoul(seq_text, NULL, 10) : 0);
        bool do_hash = (hash_text == NULL) || (0 != strcmp(hash_text, "no"));

        AutoBuffer out;
        Packer_Pack(url, sequence, bytes(data), data.size(), out, do_hash);
        write_file(out_path, out.Ptr(), out.Length());
        return 0;
    }

    if (0 == strcmp(action, "unpack")) {
        const char* in_path = opt(argc, argv, "in");
        if (in_path == NULL) {
            fprintf(stderr, "packer unpack needs --in=PATH\n");
            return 1;
        }
        std::vector<unsigned char> raw = read_file(in_path);

        std::string url;
        unsigned int sequence = 0;
        size_t pack_len = 0;
        AutoBuffer data;
        int code = Packer_Unpack(bytes(raw), raw.size(), url, sequence, pack_len, data);
        printf("%d %s %u %zu %s\n",
               code,
               url.c_str(),
               sequence,
               pack_len,
               hex((const unsigned char*)data.Ptr(), data.Length()).c_str());
        return 0;
    }

    fprintf(stderr, "usage: upstream_comm packer pack|unpack ...\n");
    return 1;
}

// `SimpleShortPack` / `SimpleIntPack` and the two `Unpack` of them.
int simple(int argc, char** argv) {
    const char* action = (argc > 2) ? argv[2] : "";
    const char* kind = opt(argc, argv, "kind");
    bool is_short = (kind != NULL) && (0 == strcmp(kind, "short"));

    if (0 == strcmp(action, "pack")) {
        const char* out_path = opt(argc, argv, "out");
        if (out_path == NULL) {
            fprintf(stderr, "simple pack needs --out=PATH\n");
            return 1;
        }
        std::vector<unsigned char> data = unhex(opt(argc, argv, "data"));

        AutoBuffer out;
        if (is_short)
            SimpleShortPack(bytes(data), data.size(), out);
        else
            SimpleIntPack(bytes(data), data.size(), out);
        write_file(out_path, out.Ptr(), out.Length());
        return 0;
    }

    if (0 == strcmp(action, "unpack")) {
        const char* in_path = opt(argc, argv, "in");
        if (in_path == NULL) {
            fprintf(stderr, "simple unpack needs --in=PATH\n");
            return 1;
        }
        std::vector<unsigned char> raw = read_file(in_path);

        size_t pack_len = 0;
        AutoBuffer data;
        int code = 0;
        if (is_short)
            code = SimpleShortUnpack(bytes(raw), raw.size(), pack_len, data);
        else
            code = SimpleIntUnpack(bytes(raw), raw.size(), pack_len, data);
        printf("%d %zu %s\n", code, pack_len, hex((const unsigned char*)data.Ptr(), data.Length()).c_str());
        return 0;
    }

    fprintf(stderr, "usage: upstream_comm simple pack|unpack ...\n");
    return 1;
}

int checksum(int argc, char** argv) {
    const char* seed_text = opt(argc, argv, "seed");
    std::vector<unsigned char> data = unhex(opt(argc, argv, "data"));
    unsigned long seed = seed_text ? strtoul(seed_text, NULL, 10) : 0;
    printf("%lu\n", adler32(seed, bytes(data), (unsigned int)data.size()));
    return 0;
}

}  // namespace

int main(int argc, char** argv) {
    std::string command = (argc > 1) ? argv[1] : "";

    if (command == "adler32") return checksum(argc, argv);
    if (command == "packer") return packer(argc, argv);
    if (command == "simple") return simple(argc, argv);

    fprintf(stderr,
            "usage: upstream_comm adler32 --data=HEX [--seed=N]\n"
            "       upstream_comm packer pack --url=U --seq=N --data=HEX "
            "[--hash=no] --out=PATH\n"
            "       upstream_comm packer unpack --in=PATH\n"
            "       upstream_comm simple pack --kind=short|int --data=HEX "
            "--out=PATH\n"
            "       upstream_comm simple unpack --kind=short|int --in=PATH\n");
    return 1;
}
