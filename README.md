# mars-xlog, in Rust

Rust implementation of the **xlog** logging pipeline of
[Tencent/mars](https://github.com/Tencent/mars). It writes and reads exactly
the `.xlog` format the C++ implementation produced — encrypted or plain,
zlib- or zstd-compressed, sync or async — and is the whole logging stack:

| crate                 | what it is                                                        |
|-----------------------|-------------------------------------------------------------------|
| `mars-core`      | block buffer, log file framing, zlib/zstd helpers                 |
| `mars-comm`      | common utilities: `strutil`, `tickcount`, thread, message queue, alarm |
| `mars-stn`       | the task model and the anti-avalanche / dynamic-timeout policies   |
| `mars-sdt`       | the network diagnosis: check profiles, the plan a mode turns into  |
| `mars-crypt`     | ECDH + AES-GCM record encryption                                  |
| `mars-buffer`    | the mmap append buffer (`LogZlibBuffer` / `LogZstdBuffer`)        |
| `mars-appender`  | the process-wide appender and per-instance loggers                |
| `mars-ffi`       | C ABI (`cdylib` + `staticlib`) and its hand-written header        |
| `mars-jni`       | JNI bindings of `io.github.marsrs`: `Xlog`, `StnLogic`, `SdtLogic`|
| `mars-compat`    | CLI plus the golden `.xlog` files that pin the wire format        |

## Build and test

```bash
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Cross-compiling needs the usual toolchain plus, for Android, an NDK (only for
the linker):

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
export ANDROID_HOME=...            # .cargo/config.toml forces 16 KiB pages
cargo build --release -p mars-jni --target aarch64-linux-android
```

`unsafe` appears in exactly three places: the C ABI shims of `mars-ffi`,
the single `memmap2::MmapOptions::map_mut` call of `mars-appender` (there
is no safe API for creating a mapping) and the one `JString::from_raw` of
`mars-jni`, which re-wraps a borrowed local ref. All three carry SAFETY notes.

## The format is pinned by golden files

The 16 `.xlog` files in `crates/mars-compat/fixtures` were written by the
*original C++* encoders — one per combination of zlib/zstd, sync/async,
encryption on/off and flush policy — and `cargo test -p mars-compat`
decodes every one of them back to the exact text that went in. That is what
keeps the port readable/writable against files produced by the C++ it replaced.

Two differences are known and accepted:

* **zlib** — the port uses `zlib-rs`, whose output is not byte-identical to
  system zlib above a few KB (still decodable by both).
* **zstd** — the C++ build vendored 1.4.4, this one uses the `zstd` crate
  1.5.x, so compressed sizes differ.

## Consumers

* Android: `mars-jni` builds `libmarsxlog.so`, which the `mars-xlog` AAR
  of <https://github.com/orangeboyChen/mars> packages per ABI.
* Apple: `mars-ffi` builds the static library inside
  `MarsXlog.xcframework`, wrapped by the Swift API of that repository.

## License

MIT, like the upstream project — see [LICENSE](LICENSE).
