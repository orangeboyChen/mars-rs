// The C++ half of the comm cross-read test: `mars/comm/basepacker.cc`,
// `mars/comm/adler32.c` and `mars/comm/strutil.cc`, driven the way
// `scripts/compat/comm.sh` drives the Rust of the same files.
//
//   upstream_comm adler32 --data=HEX [--seed=N]
//   upstream_comm packer pack --url=U --seq=N --data=HEX [--hash=no] --out=PATH
//   upstream_comm packer unpack --in=PATH
//   upstream_comm simple pack --kind=short|int --data=HEX --out=PATH
//   upstream_comm simple unpack --kind=short|int --in=PATH
//   upstream_comm socket --ip=TEXT|--v4=HEX|--v6=HEX --port=N [--map=yes]
//   upstream_comm strutil FN --data=HEX [--arg=HEX] [--pos=N]
//
// `strutil` is the string helpers: `--data` is the bytes of the string and
// `--arg` the second string — the delimiters, the prefix or suffix, the
// needle — both hex so that a byte the shell would eat is still one a case can
// name. One line comes back, and it is the same line `comm-compat` prints for
// the same call.
//
// `socket` prints one line of what a caller reads off a `socket_address`: the
// family, the bytes of the address, the port, the four `valid_*`, the three
// `is*` and the length, and then `ip`, `ipv6` and `url`, an empty one printed
// as `-` — the same line `comm-compat` prints for the same address, so the two
// can be diffed.
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
#include "mars/comm/socket/socket_address.h"
#include "mars/comm/strutil.h"

// `socket_address` asks the platform which network it is on in two places —
// `v4tonat64_address` and `fix_current_nat64_addr`, which look the NAT64 prefix
// up over DNS — and a harness has to answer the same line on every run, on a
// machine with no NAT64 in front of it. Neither is a call the harness drives;
// what is below is a stand-in for them so that the rest of the class links, and
// it sits outside the anonymous namespace because a link-time symbol is what
// the two calls need.
bool ConvertV4toNat64V6(const struct in_addr&, struct in6_addr&) {
    return false;
}

bool GetNetworkNat64Prefix(struct in6_addr&) {
    return false;
}

TLocalIPStack local_ipstack_detect() {
    return ELocalIPStack_IPv4;
}

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

// An empty string, which is what `ip()` answers for an address that never
// parsed, is not a field a line can hold: it would move every field behind it.
const char* or_dash(const char* s) {
    return (NULL == s || '\0' == *s) ? "-" : s;
}

socket_address make_address(const char* ip_text,
                            const std::vector<unsigned char>& v4,
                            const std::vector<unsigned char>& v6,
                            uint16_t port) {
    if (ip_text != NULL) return socket_address(ip_text, port);

    if (16 == v6.size()) {
        sockaddr_in6 addr;
        memset(&addr, 0, sizeof(addr));
        addr.sin6_family = AF_INET6;
        addr.sin6_port = htons(port);
        memcpy(&addr.sin6_addr, &v6[0], 16);
        return socket_address(addr);
    }

    if (4 == v4.size()) {
        sockaddr_in addr;
        memset(&addr, 0, sizeof(addr));
        addr.sin_family = AF_INET;
        addr.sin_port = htons(port);
        memcpy(&addr.sin_addr, &v4[0], 4);
        return socket_address(addr);
    }

    // Nothing to build one out of, which is what the C++ is left with when the
    // ip did not parse either.
    return socket_address("", port);
}

// `socket_address` — one line of what a caller reads off an address: the
// family, the bytes of it, the port, the four `valid_*`, the three `is*` and
// the length, and then the three strings, an empty one printed as `-`. One of
// `--ip=TEXT`, `--v4=HEX` and `--v6=HEX` says which address, and `--map=yes`
// puts `v4tov4mapped_address()` behind it.
int addresses(int argc, char** argv) {
    const char* port_text = opt(argc, argv, "port");
    uint16_t port = (uint16_t)(port_text ? strtoul(port_text, NULL, 10) : 0);
    socket_address addr = make_address(opt(argc, argv, "ip"),
                                       unhex(opt(argc, argv, "v4")),
                                       unhex(opt(argc, argv, "v6")),
                                       port);
    if (NULL != opt(argc, argv, "map")) addr.v4tov4mapped_address();

    const sockaddr* sa = &addr.address();
    if (AF_INET == sa->sa_family) {
        printf("v4 %s ", hex((const unsigned char*)&((const sockaddr_in*)sa)->sin_addr, 4).c_str());
    } else if (AF_INET6 == sa->sa_family) {
        printf("v6 %s ",
               hex((const unsigned char*)&((const sockaddr_in6*)sa)->sin6_addr, 16).c_str());
    } else {
        printf("unspec - ");
    }

    printf("%u %d %d %d %d %u %d %d %d %d %s %s %s\n",
           addr.port(),
           addr.valid(),
           addr.isv4(),
           addr.isv6(),
           addr.isv4mapped_address(),
           (unsigned int)addr.address_length(),
           addr.valid_server_address(false, false),
           addr.valid_loopback_ip(),
           addr.valid_broadcast_ip(),
           addr.valid_broadcast_address(),
           or_dash(addr.ip()),
           or_dash(addr.ipv6()),
           or_dash(addr.url()));
    return 0;
}

int checksum(int argc, char** argv) {
    const char* seed_text = opt(argc, argv, "seed");
    std::vector<unsigned char> data = unhex(opt(argc, argv, "data"));
    unsigned long seed = seed_text ? strtoul(seed_text, NULL, 10) : 0;
    printf("%lu\n", adler32(seed, bytes(data), (unsigned int)data.size()));
    return 0;
}

// The string helpers of `mars/comm/strutil.cc`, named the way `comm-compat`
// names them: `--data` is the string, `--arg` the second one and `--pos` where
// a search starts. A helper that found nothing prints what the port prints —
// `-` for a `Str2Hex` that read no hex and `-1` for an `npos` — since an
// unsigned `npos` is not a number a case should have to spell.
int strings(int argc, char** argv) {
    const char* name = (argc > 2) ? argv[2] : "";
    std::vector<unsigned char> raw = unhex(opt(argc, argv, "data"));
    std::string text((const char*)bytes(raw), raw.size());
    std::vector<unsigned char> arg = unhex(opt(argc, argv, "arg"));
    std::string second((const char*)bytes(arg), arg.size());
    const char* pos_text = opt(argc, argv, "pos");
    size_t pos = pos_text ? (size_t)strtoul(pos_text, NULL, 10) : 0;

    // `Trim`, `ToLower` and their like take the string they change, so what
    // goes in is a copy.
    std::string copy = text;
    std::vector<std::string> tokens;

    if (0 == strcmp(name, "url_encode")) {
        printf("%s\n", strutil::URLEncode(text).c_str());
    } else if (0 == strcmp(name, "trim")) {
        printf("%s\n", strutil::Trim(copy).c_str());
    } else if (0 == strcmp(name, "trim_left")) {
        printf("%s\n", strutil::TrimLeft(copy).c_str());
    } else if (0 == strcmp(name, "trim_right")) {
        printf("%s\n", strutil::TrimRight(copy).c_str());
    } else if (0 == strcmp(name, "lower")) {
        printf("%s\n", strutil::cast_lower(text).c_str());
    } else if (0 == strcmp(name, "upper")) {
        printf("%s\n", strutil::cast_upper(text).c_str());
    } else if (0 == strcmp(name, "starts_with")) {
        printf("%d\n", strutil::StartsWith(text, second) ? 1 : 0);
    } else if (0 == strcmp(name, "ends_with")) {
        printf("%d\n", strutil::EndsWith(text, second) ? 1 : 0);
    } else if (0 == strcmp(name, "split_token")) {
        strutil::SplitToken(text, second, tokens);
        for (size_t at = 0; at < tokens.size(); ++at) {
            printf("%s%s", 0 == at ? "" : "|", tokens[at].c_str());
        }
        printf("\n");
    } else if (0 == strcmp(name, "hex2str")) {
        printf("%s\n", strutil::Hex2Str(text).c_str());
    } else if (0 == strcmp(name, "str2hex")) {
        std::string out = strutil::Str2Hex(text);
        printf("%s\n", hex((const unsigned char*)out.data(), out.size()).c_str());
    } else if (0 == strcmp(name, "file_name_from_path")) {
        printf("%s\n", strutil::GetFileNameFromPath(text.c_str()).c_str());
    } else if (0 == strcmp(name, "ci_find_substr")) {
        size_t at = strutil::ci_find_substr(text, second, pos);
        if (std::string::npos == at) {
            printf("-1\n");
        } else {
            printf("%zu\n", at);
        }
    } else if (0 == strcmp(name, "md5")) {
        printf("%s\n", strutil::BufferMD5(text).c_str());
    } else {
        fprintf(stderr, "strutil %s is not a helper this harness drives\n", name);
        return 1;
    }
    return 0;
}

}  // namespace

int main(int argc, char** argv) {
    std::string command = (argc > 1) ? argv[1] : "";

    if (command == "adler32") return checksum(argc, argv);
    if (command == "packer") return packer(argc, argv);
    if (command == "simple") return simple(argc, argv);
    if (command == "socket") return addresses(argc, argv);
    if (command == "strutil") return strings(argc, argv);

    fprintf(stderr,
            "usage: upstream_comm adler32 --data=HEX [--seed=N]\n"
            "       upstream_comm packer pack --url=U --seq=N --data=HEX "
            "[--hash=no] --out=PATH\n"
            "       upstream_comm packer unpack --in=PATH\n"
            "       upstream_comm simple pack --kind=short|int --data=HEX "
            "--out=PATH\n"
            "       upstream_comm simple unpack --kind=short|int --in=PATH\n"
            "       upstream_comm socket --ip=TEXT|--v4=HEX|--v6=HEX "
            "--port=N [--map=yes]\n"
            "       upstream_comm strutil FN --data=HEX [--arg=HEX] "
            "[--pos=N]\n");
    return 1;
}
