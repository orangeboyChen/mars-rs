#!/bin/sh
# Builds one C++ harness against a checkout of upstream Tencent/mars: by default
# the encoder in `upstream_encode.cpp`, or the file `MARS_SRC` points at.
#
#   sh scripts/compat/upstream_build.sh [out-binary]
#
# The checkout is cloned (shallow) into `target/upstream` on first use — nothing
# is vendored into this repository — and reused afterwards. Override it with
# `MARS_UPSTREAM_DIR=/path/to/mars` to build against an existing checkout.
#
# `mars/xlog` cannot be built as a library on its own: `appender.cc` reaches
# into `mars/comm` (AutoBuffer, PtrBuffer, the mmap helpers, xlogger,
# `tickcount`), into `mars/xlog/crypt` (micro-ecc), into the bundled
# `mars/boost` (filesystem, iostreams, system) and, on Apple, into
# `mars/comm/objc`. The list below is the transitive closure of what
# `appender.cc` actually pulls in — upstream's own CMake globs directories
# instead and builds far more than the cross-read test needs.
set -e

REPO=$(cd "$(dirname "$0")/../.." && pwd)
SRC=${MARS_SRC:-$REPO/scripts/compat/upstream_encode.cpp}
OUT=${1:-$REPO/target/compat/upstream_encode}
UP=${MARS_UPSTREAM_DIR:-$REPO/target/upstream/Tencent-mars}

if [ ! -d "$UP/mars/xlog" ]; then
    mkdir -p "$(dirname "$UP")"
    echo "cloning Tencent/mars into $UP" >&2
    git clone --depth 1 https://github.com/Tencent/mars.git "$UP"
fi

OS=$(uname -s)
OBJ=$(mktemp -d)
trap 'rm -rf "$OBJ"' EXIT

INC="-I$UP -I$UP/mars -I$UP/mars/comm -I$UP/mars/comm/xlogger -I$UP/mars/xlog \
-I$UP/mars/xlog/src -I$UP/mars/xlog/crypt -I$UP/mars/xlog/crypt/micro-ecc-master \
-I$UP/mars/boost"

# `strutil.cc` wants `openssl/md5.h` and the log buffer wants `zstd.h`; on
# macOS both are a Homebrew prefix away.
if [ -d /opt/homebrew/opt/openssl@3/include ]; then
    INC="$INC -I/opt/homebrew/opt/openssl@3/include"
    LIBS="-L/opt/homebrew/opt/openssl@3/lib -L/opt/homebrew/lib -lcrypto"
else
    LIBS="-lcrypto"
fi
LIBS="$LIBS -lz -lzstd -lpthread"

# C sources: the headers declare these `extern "C"`, so they have to be compiled
# as C — compiling them as C++ mangles the definitions and the link fails.
for f in "$UP"/mars/comm/time_utils.c \
         "$UP"/mars/comm/xlogger/xloggerbase.c \
         "$UP"/mars/comm/xlogger/loginfo_extract.c \
         "$UP"/mars/xlog/crypt/micro-ecc-master/uECC.c \
         "$UP"/mars/comm/assert/__assert.c; do
    cc -O2 -w $INC -c "$f" -o "$OBJ/c_$(basename "$f").o"
done

for f in "$UP"/mars/xlog/src/appender.cc \
         "$UP"/mars/xlog/src/formater.cc \
         "$UP"/mars/xlog/src/log_base_buffer.cc \
         "$UP"/mars/xlog/src/log_zlib_buffer.cc \
         "$UP"/mars/xlog/src/log_zstd_buffer.cc \
         "$UP"/mars/xlog/src/xlogger_interface.cc \
         "$UP"/mars/xlog/crypt/log_crypt.cc \
         "$UP"/mars/xlog/unix/ConsoleLog.cc \
         "$UP"/mars/comm/autobuffer.cc \
         "$UP"/mars/comm/ptrbuffer.cc \
         "$UP"/mars/comm/mmap_util.cc \
         "$UP"/mars/comm/strutil.cc \
         "$UP"/mars/comm/boost_exception.cc \
         "$UP"/mars/comm/xlogger/xlogger.cc \
         "$UP"/mars/comm/xlogger/xlogger_category.cc \
         "$UP"/mars/comm/unix/xlogger_threadinfo.cc \
         "$UP"/mars/comm/tickcount.cc \
         "$UP"/mars/boost/libs/filesystem/src/codecvt_error_category.cpp \
         "$UP"/mars/boost/libs/filesystem/src/operations.cpp \
         "$UP"/mars/boost/libs/filesystem/src/path.cpp \
         "$UP"/mars/boost/libs/filesystem/src/path_traits.cpp \
         "$UP"/mars/boost/libs/filesystem/src/portability.cpp \
         "$UP"/mars/boost/libs/filesystem/src/unique_path.cpp \
         "$UP"/mars/boost/libs/filesystem/src/utf8_codecvt_facet.cpp \
         "$UP"/mars/boost/libs/iostreams/src/mapped_file.cpp \
         "$UP"/mars/boost/libs/system/src/error_code.cpp \
         "$SRC"; do
    c++ -std=c++14 -O2 -w $INC -c "$f" -o "$OBJ/cxx_$(basename "$f").o"
done

# The file-protection attribute helper is Objective-C++, and Apple-only.
if [ "$OS" = "Darwin" ]; then
    c++ -x objective-c++ -std=c++14 -O2 -w $INC -c "$UP"/mars/comm/objc/data_protect_attr.mm \
        -o "$OBJ/objc_protect.o"
    LIBS="$LIBS -framework Foundation"
fi

mkdir -p "$(dirname "$OUT")"
c++ -o "$OUT" "$OBJ"/*.o $LIBS
echo "$OUT"
