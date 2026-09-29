# The C demo of mars-rs: one program that writes a `.xlog` through the C ABI.
#
# `make` builds two things and runs neither: the static library the ABI *is* —
# `cargo build --release -p marsrs-ffi`, which is what puts `libmars_ffi.a` in
# the workspace's `target/release/` — and then this program against it. Nothing
# here downloads anything, because the library is the checkout's own.
#
# An app that takes the release instead of the checkout already has both halves
# and writes the two lines the C ABI page gives it, with no `make` in between:
#
#     cc -I include -o app app.c libmars_ffi.a -lpthread -ldl   # static
#     cc -I include -o app app.c -L. -lmars_ffi                # shared

CC     ?= cc
# `-Wpedantic` because the header is C89-clean on purpose: it is the seam the
# C++ layers of upstream Mars are meant to include, and a caller that is still
# on an older standard has to be able to.
CFLAGS ?= -std=c11 -Wall -Wextra -Wpedantic -O2
CARGO  ?= cargo

# The workspace two levels up: the crate the library is built from, the header
# the program includes, and the `target/` the library lands in.
ROOT    := ../..
FFI     := $(ROOT)/crates/marsrs-ffi
INCLUDE := $(FFI)/include
LIB     := $(ROOT)/target/release/libmars_ffi.a

# What the archive needs linked beside it. `-lpthread -ldl` is the pair the C
# ABI page gives, and it is the Linux one; on Apple platforms the time-zone read
# of `iana-time-zone` goes through CoreFoundation, so the framework is part of
# the link there — the same `linkedFramework("CoreFoundation")` the Swift
# package declares for this very archive.
LDLIBS := -lpthread -ldl
ifeq ($(shell uname -s),Darwin)
LDLIBS += -framework CoreFoundation
endif

.PHONY: all run clean

all: demo

demo: demo.c $(LIB)
	$(CC) $(CFLAGS) -I$(INCLUDE) -o $@ demo.c $(LIB) $(LDLIBS)

# `--release` and not the debug profile: this is the library an app links, and
# the debug one is the same ABI at ten times the size and a tenth of the speed.
$(LIB):
	$(CARGO) build --release --manifest-path $(ROOT)/Cargo.toml -p marsrs-ffi

run: demo
	./demo

clean:
	rm -f demo
