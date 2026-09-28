---
layout: home

hero:
  name: mars-rs
  text: mars, in Rust
  tagline: A logger that writes the .xlog files the C++ Tencent/mars writes — from Rust, Swift, Kotlin, Kotlin Multiplatform or plain C.
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
    details: zlib or zstd per file, ECDH + AES-GCM per record when you give it a public key, and a size or an age at which a file is closed and a new one opened.
---

These pages are the xlog documentation: the logger, and every platform it ships
to. STN and the SDT diagnosis have no pages of their own yet, and not every
package carries them either — the Kotlin Multiplatform, Flutter and React Native
ones carry the logger under both names today.

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
logs takes the first.

The Flutter plugin and the React Native module are published to pub.dev and to
npm, and are not in the release archive; neither registry is being published to
yet.

## What the rest of these pages are

- [Getting started](/getting-started) — the dependency and a running example
  for each platform.
- [Configuration](/configuration) — every option, its default, and what it is
  called on each platform.
- [Log files](/log-files) — where they land, what they are called, when to
  flush, and how to read them back.
