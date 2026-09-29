// The C++ half of the SDT cross-read test:
// `mars/sdt/src/checkimpl/http_url_parser.h`, driven the way
// `scripts/compat/sdt.sh` drives the Rust of the same header.
//
//   upstream_sdt url --url=U
//
// `url` prints what the parser read out of one URL — one line per thing, in the
// order below, and the same lines `sdt-compat url` prints, so the two can be
// diffed:
//
//   host <Host()>  — empty for a URL that did not parse
//   port <Port()>  — 80 for one that named none
//   path <Path()>  — / for one that named no path, and still empty for a URL
//                    that did not parse at all
//
// `HttpUrlParser` parses in its constructor and keeps the `bool Parse()`
// answers to itself, so an empty `Host()` is all a caller can see of a URL it
// could not read — the host, the port and the path are the whole of its surface.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "mars/sdt/src/checkimpl/http_url_parser.h"

namespace {

// `NULL` for an option the command line did not give, which is not the same
// thing as one it gave empty: `--url=` is the URL of nothing at all, and that is
// a URL the parser is asked about like any other.
const char* opt(int argc, char** argv, const char* key) {
    size_t len = strlen(key);
    for (int i = 2; i < argc; ++i) {
        if (strncmp(argv[i], "--", 2) != 0) continue;
        const char* arg = argv[i] + 2;
        if (strncmp(arg, key, len) != 0 || arg[len] != '=') continue;
        return arg + len + 1;
    }
    return NULL;
}

}  // namespace

int main(int argc, char** argv) {
    std::string command = (argc > 1) ? argv[1] : "";

    if (command != "url") {
        fprintf(stderr, "usage: upstream_sdt url --url=U\n");
        return 1;
    }

    const char* url = opt(argc, argv, "url");
    if (url == NULL) {
        fprintf(stderr, "url needs --url=U\n");
        return 1;
    }

    mars::sdt::HttpUrlParser parser(url);
    printf("host %s\n", parser.Host());
    printf("port %u\n", (unsigned int)parser.Port());
    printf("path %s\n", parser.Path());
    return 0;
}
