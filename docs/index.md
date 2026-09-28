---
layout: home

hero:
  name: mars-rs
  text: mars, in Rust
  tagline: The .xlog logger, the STN task pipeline and the SDT network diagnosis of Tencent/mars — from Rust, Swift, Kotlin, Kotlin Multiplatform or plain C.
  actions:
    - theme: brand
      text: Get started
      link: /getting-started
    - theme: alt
      text: Configuration
      link: /configuration
    - theme: alt
      text: On GitHub
      link: https://github.com/orangeboyChen/mars-rs

features:
  - title: Files the C++ tooling already reads
    details: What this writes is the .xlog the C++ implementation writes — same framing, same compression, same encryption — so upstream's decoders, and any tooling built on them, read your logs without a conversion step.
  - title: One shape on every platform
    details: Open an appender once, write a tag and a message through it, flush before you upload. Rust, Swift, Kotlin, Kotlin Multiplatform and C spell the same three steps with the same options.
  - title: Off the logging thread
    details: "The default async mode hands the record to a writer thread through a memory-mapped cache, so a write returns without waiting for the disk. `flush(sync: true)` is what drains it."
  - title: Compressed, encrypted, rotated
    details: zlib or zstd per file, ECDH + TEA per record when you give it a public key, and a size or an age at which a file is closed and a new one opened.
  - title: The task pipeline, and the diagnosis
    details: "STN runs a request as a task — queued, retried, timed out, reported, over a short link or a long link the app keeps. SDT answers why a host stopped answering: ping, DNS, TCP and HTTP, and a JSON report of what each found."
  - title: No threads but yours
    details: "Neither half runs on a thread of its own: the C++ has a message-queue thread and a `__RunOn` thread, and this port has neither, so what would have been a thread is a call the host makes — `run_pending()` and `due_time()`, `runChecks` and its probes."
---

These pages are the documentation of the port: the logger, every platform it
ships to, and — on [the task pipeline](/stn) and [the diagnosis](/sdt) — the half
that talks to a server. Not every package carries that half: the Flutter and
React Native ones carry the logger under both names today.

## Pick your platform

Every release ships a package per platform — except the two that go to a
registry of their own:

| your app is | take | page |
|---|---|---|
| Rust | `marsrs` or `marsrs-xlog` | [Rust](/platforms/rust) |
| iOS / watchOS, Swift | the `MarsRSXlog` SwiftPM product | [SwiftPM](/platforms/swift) |
| iOS / watchOS, Swift or Objective-C | the `MarsRSXlog` pod | [CocoaPods](/platforms/cocoapods) |
| Android, Kotlin or Java | `marsrs` or `xlog` on JitPack | [Android](/platforms/android) |
| Kotlin Multiplatform | `marsrs-kmp` or `xlog-kmp` | [Kotlin Multiplatform](/platforms/kotlin-multiplatform) |
| Flutter | `marsrs_flutter_xlog` or `marsrs_flutter` | [Flutter](/platforms/flutter) |
| React Native | `marsrs-react-native-xlog` or `marsrs-react-native` | [React Native](/platforms/react-native) |
| anything with a C FFI | the `marsrs-<version>-<host>` archive | [The C ABI](/platforms/c-abi) |
| HarmonyOS | the three `.so` of `marsrs-harmony-<version>.tar.gz` | [HarmonyOS](/platforms/harmonyos) |

`xlog` is the logger alone; `marsrs` adds the STN task pipeline and the SDT
network diagnosis — the same pair the crates on crates.io are. An app that only
logs takes the first, and the same split is the one every platform makes:
`MarsRSXlog` or `MarsRSNet` on Apple, `xlog` or `marsrs` on JitPack,
`xlog-kmp` or `marsrs-kmp` for a shared Kotlin module.

The Flutter plugin and the React Native module are on pub.dev and on npm:
`flutter pub add` and `npm install` take the newest version there.

## The half that is not the logger

Two pages, one per half, and each of them says which platforms carry it:

- [The task pipeline (STN)](/stn) — a task is queued, sent on the short link or
  the long link, retried, timed out and reported, and the app answers the
  questions STN asks while it runs.
- [The network diagnosis (SDT)](/sdt) — ping, DNS, TCP and HTTP against the hosts
  of the two links, and a JSON report of what each one found.

## What the rest of these pages are

- [Getting started](/getting-started) — the dependency and a running example
  for each platform.
- [Configuration](/configuration) — every option, its default, and what it is
  called on each platform.
- [Log files](/log-files) — where they land, what they are called, when to
  flush, and how to read them back.
