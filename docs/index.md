---
layout: home

hero:
  name: mars-rs
  text: mars, in Rust
  tagline: The .xlog logger, the STN task pipeline and the SDT network diagnosis of Tencent/mars — from Rust, Swift, Kotlin, Flutter, React Native, HarmonyOS or plain C.
  actions:
    - theme: brand
      text: Get started
      link: /xlog/getting-started
    - theme: alt
      text: On GitHub
      link: https://github.com/orangeboyChen/mars-rs

features:
  - title: Files the C++ tooling already reads
    details: What this writes is the .xlog the C++ implementation writes — same framing, same compression, same encryption — so upstream's decoders, and any tooling built on them, read your logs without a conversion step.
  - title: One shape on every platform
    details: Open an appender once, write a tag and a message through it, flush before you upload. Rust, Swift, Kotlin, Kotlin Multiplatform, Flutter, React Native, HarmonyOS and C spell the same three steps with the same options.
  - title: Off the logging thread
    details: "The default async mode hands the record to a writer thread through a memory-mapped cache, so a write returns without waiting for the disk. `requestFlush()` asks for the drain and returns at once, `flushNow()` drains it on the calling thread, and `await flush()` hands it to another thread and answers when it is over."
  - title: Compressed, encrypted, rotated
    details: zlib or zstd per file, ECDH + TEA per record when you give it a public key, and a size or an age at which a file is closed and a new one opened.
  - title: The task pipeline, and the diagnosis
    details: "STN runs a request as a task — queued, retried, timed out, reported, over a short link or a long link the app keeps. SDT answers why a host stopped answering: ping, DNS, TCP and HTTP, and a JSON report of what each found."
  - title: No threads but yours
    details: "Neither half runs on a thread of its own, so what would have been a thread is a call the host makes — `run_pending()` and `due_time()`, `runChecks` and its probes."
---
