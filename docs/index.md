---
layout: home

hero:
  name: mars-rs
  text: mars, in Rust
  tagline: The xlog logging pipeline, the STN task model and the SDT network diagnosis of Tencent/mars, in Rust — writing and reading exactly the .xlog the C++ produced.
  actions:
    - theme: brand
      text: Get started
      link: /getting-started
    - theme: alt
      text: The .xlog format
      link: /format/
    - theme: alt
      text: What a record costs
      link: /performance

features:
  - title: The same .xlog files
    details: xlog is the half of the port that is pinned to a file format — it writes and reads exactly the .xlog the C++ implementation produced, encrypted or plain, zlib- or zstd-compressed, sync or async. Sixteen golden files written by the original C++ encoders are what pin it.
  - title: One package per platform
    details: crates.io, SwiftPM, two AARs on JitPack, a Kotlin Multiplatform module and a C ABI tarball — every release ships all of them, built by .github/workflows/release.yml.
  - title: Measured against the C++
    details: One record costs 830 ns where upstream pays 2038, and the eight things the port does differently are written down on the performance page — including the three that were measured and left alone.
  - title: unsafe in three places
    details: The C ABI shims of marsrs-ffi, the single mmap of marsrs-appender and one JString::from_raw of marsrs-jni. All three carry SAFETY notes.
---

## The crates

| crate | what it is |
|---|---|
| `marsrs` | the whole port, re-exported: `xlog`, `stn`, `sdt`, `comm`, `bytes` |
| `marsrs-xlog` | xlog alone — the pair's other half, for an app that only logs |
| `marsrs-core` | block buffer, log file framing, zlib/zstd helpers |
| `marsrs-comm` | common utilities: `strutil`, `tickcount`, thread, message queue, alarm |
| `marsrs-stn` | the task model and the anti-avalanche / dynamic-timeout policies |
| `marsrs-sdt` | the network diagnosis: check profiles, the plan a mode turns into |
| `marsrs-crypt` | ECDH + AES-GCM record encryption |
| `marsrs-buffer` | the mmap append buffer (`LogZlibBuffer` / `LogZstdBuffer`) |
| `marsrs-appender` | the process-wide appender and per-instance loggers |
| `marsrs-ffi` | C ABI (`cdylib` + `staticlib`): `mars_xlog.h`, `mars_sdt.h` behind the `sdt` feature and `mars_stn.h` behind the `stn` one |
| `marsrs-jni` | JNI bindings of `io.github.orangeboychen.marsrs`: `Xlog`, `StnLogic`, `SdtLogic` |
| `marsrs-compat` | CLI plus the golden `.xlog` files that pin the wire format |

The two at the top of that table are the crates a Rust caller depends on:
`marsrs`, the whole port, and `marsrs-xlog`, xlog alone — the pair the C++
project publishes as `mars-core` and `mars-xlog`, and this one as `marsrs` and
`marsrs-xlog`. SwiftPM spells its three products the same way — `marsrs`,
`marsrs-xlog`, `marsrs-net` — and calls the modules behind them `MarsRS`,
`MarsRSXlog` and `MarsRSNet`. The seven between them and `marsrs-ffi` are
implementation details of the pair, and are on crates.io only because a
published crate cannot depend on a crate that is not: crates.io resolves a
dependency out of the registry and not out of a path.

## Where to go next

- [Getting started](/getting-started) — the dependency, the snippet and the
  link for whichever platform the app is on.
- [The .xlog format](/format/) — what the 16 golden files under
  `crates/marsrs-compat/fixtures` pin, and the two differences that are known.
- [Cross-checked the other way](/format/cross-check) — a file this port wrote,
  read by the C++ decoder, 16 combinations in both directions.
- [What a record costs](/performance) — nanoseconds per record against
  upstream, and the optimizations behind the numbers.
- [Build, test and lint](/project/build) — the four commands, the three lint
  gates and where `unsafe` is.
