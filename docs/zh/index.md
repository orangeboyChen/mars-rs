---
layout: home

hero:
  name: mars-rs
  text: mars 的 Rust 实现
  tagline: Tencent/mars 的 .xlog 日志、STN 任务链路和 SDT 网络诊断 —— 从 Rust、Swift、Kotlin、Flutter、React Native、HarmonyOS 或 C 调用。
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
  - title: 八个平台，一个写法
    details: 启动时开一个 appender，往里写 tag 和消息，上传前 flush。Rust、Swift、Kotlin、Kotlin Multiplatform、Flutter、React Native、HarmonyOS 和 C 都是这三步，配置项也一样。
  - title: 写日志不卡在磁盘上
    details: "默认的异步模式把记录交给 mmap 缓存和写线程，write 不等待落盘就返回；排空也一样可以不占住调用方 —— `requestFlush()` 提出一次排空就返回，`flushNow()` 排到记录落盘才返回，`await flush()` 是同一次排空，交给别的线程去等。"
  - title: 压缩、加密、轮转
    details: 每个文件 zlib 或 zstd 压缩，给了公钥就按 ECDH + TEA 加密每条记录，还可以按大小或时间关掉旧文件、开新文件。
  - title: 任务链路，还有网络诊断
    details: "STN 把一个请求当成一个任务跑 —— 排队、重试、超时、上报，走在短连接或 App 自己维持的长连接上。SDT 回答一个主机为什么不再回应：ping、DNS、TCP、HTTP，以及一份写清每一项查到了什么的 JSON 报告。"
  - title: 没要过的线程不起
    details: "STN 和 SDT 都不自带线程：本该是线程的地方，是宿主自己调的一次调用 —— 管线是 `run_pending()` 和 `due_time()`，诊断是 `runChecks` 和它的探针 —— `Driver::spawn()` 是那个可以要的排空线程。唯一一个不问自起的线程是 appender 异步模式的写线程，异步模式本身就是它；同步模式一个都不起。"
---
