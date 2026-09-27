---
layout: home

hero:
  name: mars-rs
  text: mars 的 Rust 实现
  tagline: 写出与 C++ 版 Tencent/mars 完全相同的 .xlog 日志文件的日志库 —— 从 Rust、Swift、Kotlin、Kotlin Multiplatform 或 C 调用。
  actions:
    - theme: brand
      text: 快速开始
      link: /zh/getting-started
    - theme: alt
      text: 配置项
      link: /zh/configuration
    - theme: alt
      text: GitHub
      link: https://github.com/orangeboyChen/mars-rs

features:
  - title: 上游工具直接能读
    details: 写出来的就是 C++ 实现写的那个 .xlog —— 同样的帧结构、同样的压缩、同样的加密，所以上游的解码脚本和基于它做的工具链可以直接读你的日志，不需要转换。
  - title: 各平台一个形状
    details: 启动时开一个 appender，往里写 tag 和消息，上传前 flush。Rust、Swift、Kotlin、Kotlin Multiplatform 和 C 都在用同样的三步、同样的配置项。
  - title: 写日志不卡在磁盘上
    details: "默认的异步模式把记录交给 mmap 缓存和写线程，write 不等待落盘就返回；`flush(sync: true)` 才是把缓存排空的那一下。"
  - title: 压缩、加密、轮转
    details: 每个文件 zlib 或 zstd 压缩，给了公钥就按 ECDH + AES-GCM 加密每条记录，还可以按大小或时间关掉旧文件、开新文件。
---

## 选你的平台

每个 release 都按平台发包：

| 你的应用 | 用哪个 | 页面 |
|---|---|---|
| Rust | `marsrs` 或 `marsrs-xlog` | [Rust](/zh/platforms/rust) |
| iOS / watchOS，Swift | SwiftPM 的 `MarsRSXlog` | [SwiftPM](/zh/platforms/swift) |
| Android，Kotlin 或 Java | JitPack 上的 `mars-rs` / `mars-rs-xlog` | [Android](/zh/platforms/android) |
| Kotlin Multiplatform | `mars-rs-kmp` / `mars-rs-xlog-kmp` | [Kotlin Multiplatform](/zh/platforms/kotlin-multiplatform) |
| 任何能调 C 的语言 | `mars-rs-<version>-<host>` 压缩包 | [C ABI](/zh/platforms/c-abi) |
| HarmonyOS | `libmars_ffi.so`，目前要自己编译 | [HarmonyOS](/zh/platforms/harmonyos) |

`mars-xlog` 只有日志；`mars-core`（Rust 里是 `marsrs`）再加上 STN 任务链路和 SDT 网络诊断。只打日志的 App 用前者。

## 其余几页

- [快速开始](/zh/getting-started) —— 每个平台的依赖和一段能跑的代码。
- [配置项](/zh/configuration) —— 每个配置项、默认值、以及在各个平台上的名字。
- [日志文件](/zh/log-files) —— 文件在哪、叫什么、什么时候要 flush、怎么读回来。
