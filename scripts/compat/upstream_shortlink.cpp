// The C++ half of the short-link cross-read test:
// `mars/stn/proto/shortlink_packer.cc`, driven the way
// `scripts/compat/shortlink.sh` drives the Rust of the same file.
//
//   upstream_shortlink pack --url=U [--header=N:V ...] --body=HEX --out=PATH
//   upstream_shortlink parse --in=PATH
//
// `pack` writes what a short-link task goes out as. `parse` reads it back with
// upstream's own `http::Parser` and prints a canonical form of what it found —
// one line per thing, in the order below, and the same lines
// `stn-compat shortlink parse` prints, so the two can be diffed:
//
//   status <TRecvStatus>            — the two enumerations are in the same
//                                     order on either side, so the number
//                                     itself travels
//   request <method> <url> <version>
//   field <name> <value>            — one per field, in the order the head
//                                     holds them, which is by name with
//                                     neither name's case in the way
//   body <hex>
//
// Unlike the long-link harness, this one is built against `mars/stn/proto`:
// `shortlink_packer.h` declares `shortlink_tracker::Create` static, so both
// copies of the `.cc` compile.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <list>
#include <map>
#include <string>
#include <utility>
#include <vector>

#include "mars/comm/autobuffer.h"
#include "mars/comm/http.h"
#include "mars/stn/proto/shortlink_packer.h"

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

// `--header=N:V`, which a case repeats once per field.
std::map<std::string, std::string> headers(int argc, char** argv) {
    std::map<std::string, std::string> out;
    for (int i = 2; i < argc; ++i) {
        if (strncmp(argv[i], "--header=", 9) != 0) continue;
        std::string arg(argv[i] + 9);
        size_t colon = arg.find(':');
        if (colon == std::string::npos) continue;
        out[arg.substr(0, colon)] = arg.substr(colon + 1);
    }
    return out;
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

    if (command != "pack" && command != "parse") {
        fprintf(stderr,
                "usage: upstream_shortlink pack --url=U [--header=N:V ...] "
                "--body=HEX --out=PATH\n"
                "       upstream_shortlink parse --in=PATH\n");
        return 1;
    }

    if (command == "pack") {
        std::string url = opt(argc, argv, "url", "");
        std::vector<unsigned char> body = unhex(opt(argc, argv, "body", ""));
        std::string out_path = opt(argc, argv, "out", "");
        if (out_path.empty()) {
            fprintf(stderr, "pack needs --out=PATH\n");
            return 1;
        }

        AutoBuffer body_buf, extension, packed;
        if (!body.empty()) body_buf.Write(body.data(), body.size());
        mars::stn::shortlink_pack(url, headers(argc, argv), body_buf, extension, packed, NULL);

        write_file(out_path, packed.Ptr(), packed.Length());
        return 0;
    }

    std::string in_path = opt(argc, argv, "in", "");
    if (in_path.empty()) {
        fprintf(stderr, "parse needs --in=PATH\n");
        return 1;
    }
    std::vector<unsigned char> bytes = read_file(in_path);

    // `MemoryBodyReceiver` is upstream's own: the body lands in an `AutoBuffer`
    // the parser does not own, which is why `_manage` is false.
    AutoBuffer body_buf;
    http::MemoryBodyReceiver receiver(body_buf);
    http::Parser parser(&receiver, false);
    http::Parser::TRecvStatus status = parser.Recv(bytes.data(), bytes.size());

    printf("status %d\n", (int)status);
    printf("request %s %s %s\n",
           http::RequestLine::kHttpMethodString[parser.Request().Method()],
           parser.Request().Url().c_str(),
           http::kHttpVersionString[parser.Request().Version()]);

    std::list<std::pair<const std::string, const std::string>> fields =
        parser.Fields().GetAsList();
    for (std::list<std::pair<const std::string, const std::string>>::const_iterator it =
             fields.begin();
         it != fields.end();
         ++it) {
        printf("field %s %s\n", it->first.c_str(), it->second.c_str());
    }
    printf("body %s\n", hex(body_buf.Ptr(), body_buf.Length()).c_str());
    return 0;
}
