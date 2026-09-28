---
layout: home

hero:
  name: mars-rs
  text: mars 的 Rust 实现
  tagline: Tencent/mars 的 .xlog 日志、STN 任务链路和 SDT 网络诊断 —— 从 Rust、Swift、Kotlin、Kotlin Multiplatform 或 C 调用；Flutter、React Native 和 HarmonyOS 只有日志。
  actions:
    - theme: brand
      text: 快速开始
      link: /zh/xlog/getting-started
    - theme: alt
      text: GitHub
      link: https://github.com/orangeboyChen/mars-rs

features:
  - title: 上游工具直接能读
    details: 写出来的就是 C++ 实现写的那个 .xlog —— 同样的帧结构、同样的压缩、同样的加密，所以上游的解码脚本和基于它做的工具链可以直接读你的日志，不需要转换。
  - title: 各平台一个形状
    details: 启动时开一个 appender，往里写 tag 和消息，上传前 flush。Rust、Swift、Kotlin、Kotlin Multiplatform、Flutter、React Native、HarmonyOS 和 C 都在用同样的三步、同样的配置项。
  - title: 写日志不卡在磁盘上
    details: "默认的异步模式把记录交给 mmap 缓存和写线程，write 不等待落盘就返回；`flush(sync: true)` 才是把缓存排空的那一下。"
  - title: 压缩、加密、轮转
    details: 每个文件 zlib 或 zstd 压缩，给了公钥就按 ECDH + TEA 加密每条记录，还可以按大小或时间关掉旧文件、开新文件。
  - title: 任务链路，和网络诊断
    details: "STN 把一个请求当成一个任务跑 —— 排队、重试、超时、上报，走在短连接或 App 自己维持的长连接上。SDT 回答一个主机为什么不再回应：ping、DNS、TCP、HTTP，以及一份写清每一项查到了什么的 JSON 报告。"
  - title: 除了你的线程，没有别的线程
    details: "两半都不跑在自己的线程上：本来会是线程的东西是宿主调的一次调用 —— `run_pending()` 和 `due_time()`、`runChecks` 和它的探针。"
---
