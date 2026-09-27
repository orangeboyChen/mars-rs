# The format is pinned by golden files

The 16 `.xlog` files in `crates/marsrs-compat/fixtures` were written by the
*original C++* encoders — one per combination of zlib/zstd, sync/async,
encryption on/off and flush policy — and `cargo test -p marsrs-compat`
decodes every one of them back to the exact text that went in. That is what
keeps the port readable/writable against files produced by the C++ it replaced.

Two differences are known and accepted:

* **zlib** — the port uses `zlib-rs`, whose output is not byte-identical to
  system zlib above a few KB (still decodable by both).
* **zstd** — the C++ build vendored 1.4.4, this one uses the `zstd` crate
  1.5.x, so compressed sizes differ.

## What a fixture is

One `.xlog` per cell of the matrix the format has: the compressor (zlib or
zstd), the mode (sync or async), whether the records are encrypted and the
flush policy. `marsrs-compat` is the crate that carries them, and it is also a
CLI: `xlog-compat decode --privkey=<hex> --in=a.xlog --out=a.plain` is the
hand-run version of what the test does 16 times, and `xlog-compat encode` is
the encoder [the cross-check](/format/cross-check) mirrors option for option.

The other direction — a file this port wrote, read by the C++ — needs the C++
in the tree, so it is a script and not a test: see
[cross-checked the other way](/format/cross-check).
