// The C++ half of the long-link cross-read test: `mars/stn/proto/longlink_packer.cc`,
// driven the way `scripts/compat/stn.sh` drives the Rust of the same file.
//
//   upstream_stn pack --client-version=0 --cmdid=1 --seq=2 --body=HEX --out=PATH
//   upstream_stn unpack --client-version=0 --in=PATH
//
// `unpack` prints one line: the `int` `longlink_unpack` answered, then the
// cmdid, the seq, how long the whole package is and the body as hex — the four
// the C++ fills in through references, in the order it fills them, and the same
// line `stn-compat` prints.
//
// The header it is built against is the copy under
// `mars/libraries/mars_android_sdk/jni/`, because it is the one that compiles:
// `proto/longlink_packer.h` declares `packer_encoder_version` a non-static
// member and both `longlink_packer.cc` files define it out of line, which no
// compiler takes. The two copies of the `.cc` are byte for byte the same file,
// so what is built here is upstream's long-link packer and not a variation of
// it.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <string>
#include <vector>

#include "mars/comm/autobuffer.h"
// `mars/stn/proto/longlink_packer.h` declares the packer and `SetClientVersion`
// is declared beside it in `stnproto_logic.h` — the header the JNI half of
// upstream calls it through — and not in any of the packer's own headers, so
// a harness that wants to stamp a version has to include both.
#include "mars/libraries/mars_android_sdk/jni/stnproto_logic.h"
#include "mars/libraries/mars_android_sdk/jni/longlink_packer.h"

namespace {

std::string opt(int argc, char** argv, const char* key, const char* fallback) {
    size_t len = strlen(key);
    for (int i = 2; i < argc; ++i) {
        if (strncmp(argv[i], "--", 2) != 0) continue;
        const char* arg = argv[i] + 2;
        if (strncmp(arg, key, len) != 0 || arg[len] != '=') continue;
        return std::string(arg + len + 1);
    }
    return std::string(fallback);
}

unsigned long number(int argc, char** argv, const char* key, const char* fallback) {
    return strtoul(opt(argc, argv, key, fallback).c_str(), NULL, 10);
}

std::vector<unsigned char> unhex(const std::string& text) {
    std::vector<unsigned char> out;
    for (size_t i = 0; i + 1 < text.size(); i += 2) {
        char byte[3] = {text[i], text[i + 1], 0};
        out.push_back((unsigned char)strtoul(byte, NULL, 16));
    }
    return out;
}

std::string hex(const void* data, size_t len) {
    static const char* digits = "0123456789abcdef";
    const unsigned char* bytes = (const unsigned char*)data;
    std::string out;
    out.reserve(len * 2);
    for (size_t i = 0; i < len; ++i) {
        out.push_back(digits[bytes[i] >> 4]);
        out.push_back(digits[bytes[i] & 0xf]);
    }
    return out;
}

std::vector<unsigned char> read_file(const std::string& path) {
    FILE* file = fopen(path.c_str(), "rb");
    if (file == NULL) {
        fprintf(stderr, "read %s: cannot open\n", path.c_str());
        exit(1);
    }
    std::vector<unsigned char> out;
    int c = 0;
    while ((c = fgetc(file)) != EOF) out.push_back((unsigned char)c);
    fclose(file);
    return out;
}

void write_file(const std::string& path, const void* data, size_t len) {
    FILE* file = fopen(path.c_str(), "wb");
    if (file == NULL) {
        fprintf(stderr, "write %s: cannot open\n", path.c_str());
        exit(1);
    }
    if (len > 0) fwrite(data, 1, len, file);
    fclose(file);
}

}  // namespace

int main(int argc, char** argv) {
    std::string command = (argc > 1) ? argv[1] : "";

    if (command != "pack" && command != "unpack") {
        fprintf(stderr,
                "usage: upstream_stn pack --client-version=0 --cmdid=1 --seq=2 "
                "--body=HEX --out=PATH\n"
                "       upstream_stn unpack --client-version=0 --in=PATH\n");
        return 1;
    }

    // `SetClientVersion` first: it is what `longlink_pack` stamps the header
    // with and the only version `longlink_unpack` accepts back, so a pair that
    // disagrees about it answers `LONGLINK_UNPACK_FALSE` on both sides.
    mars::stn::SetClientVersion((uint32_t)number(argc, argv, "client-version", "0"));

    if (command == "pack") {
        uint32_t cmdid = (uint32_t)number(argc, argv, "cmdid", "0");
        uint32_t seq = (uint32_t)number(argc, argv, "seq", "0");
        std::vector<unsigned char> body = unhex(opt(argc, argv, "body", ""));
        std::string out_path = opt(argc, argv, "out", "");
        if (out_path.empty()) {
            fprintf(stderr, "pack needs --out=PATH\n");
            return 1;
        }

        AutoBuffer body_buf, extension, packed;
        if (!body.empty()) body_buf.Write(body.data(), body.size());
        mars::stn::gDefaultLongLinkEncoder.longlink_pack(cmdid, seq, body_buf, extension, packed, NULL);

        write_file(out_path, packed.Ptr(), packed.Length());
        return 0;
    }

    std::string in_path = opt(argc, argv, "in", "");
    if (in_path.empty()) {
        fprintf(stderr, "unpack needs --in=PATH\n");
        return 1;
    }
    std::vector<unsigned char> bytes = read_file(in_path);

    AutoBuffer packed;
    if (!bytes.empty()) packed.Write(bytes.data(), bytes.size());

    uint32_t cmdid = 0;
    uint32_t seq = 0;
    size_t package_len = 0;
    AutoBuffer body, extension;
    int answered = mars::stn::gDefaultLongLinkEncoder.longlink_unpack(
        packed,
        cmdid,
        seq,
        package_len,
        body,
        extension,
        NULL);

    printf("%d %u %u %zu %s\n", answered, cmdid, seq, package_len, hex(body.Ptr(), body.Length()).c_str());
    return 0;
}
