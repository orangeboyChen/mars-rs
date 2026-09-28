---
layout: home

hero:
  name: mars-rs
  text: mars 的 Rust 实现
  tagline: Tencent/mars 的 .xlog 日志、STN 任务链路和 SDT 网络诊断 —— 从 Rust、Swift、Kotlin、Kotlin Multiplatform 或 C 调用。
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
    details: 每个文件 zlib 或 zstd 压缩，给了公钥就按 ECDH + TEA 加密每条记录，还可以按大小或时间关掉旧文件、开新文件。
  - title: 任务链路，和网络诊断
    details: "STN 把一个请求当成一个任务跑 —— 排队、重试、超时、上报，走在短连接或 App 自己维持的长连接上。SDT 回答一个主机为什么不再回应：ping、DNS、TCP、HTTP，以及一份写清每一项查到了什么的 JSON 报告。"
  - title: 除了你的线程，没有别的线程
    details: "两半都不跑在自己的线程上：C++ 有一个消息队列线程和一个 `__RunOn` 线程，这个移植一个都没有，所以本来会是线程的东西是宿主调的一次调用 —— `run_pending()` 和 `due_time()`、`runChecks` 和它的探针。"
---

这几页是这个移植的文档：日志库、它发到的每个平台，以及跟服务器说话的那半 ——
[任务链路](/zh/stn) 和 [网络诊断](/zh/sdt)。不是每个包都带着那半：Flutter 和 React
Native 的两个包今天都只有日志库。

## 选你的平台

每个 release 都按平台发包 —— 只有两个是发到自己的 registry 上的：

| 你的应用 | 用哪个 | 页面 |
|---|---|---|
| Rust | `marsrs` 或 `marsrs-xlog` | [Rust](/zh/platforms/rust) |
| iOS / watchOS，Swift | SwiftPM 的 `MarsRSXlog` | [SwiftPM](/zh/platforms/swift) |
| iOS / watchOS，Swift 或 Objective-C | `MarsRSXlog` 这个 pod | [CocoaPods](/zh/platforms/cocoapods) |
| Android，Kotlin 或 Java | JitPack 上的 `marsrs` / `xlog` | [Android](/zh/platforms/android) |
| Kotlin Multiplatform | `marsrs-kmp` / `xlog-kmp` | [Kotlin Multiplatform](/zh/platforms/kotlin-multiplatform) |
| Flutter | `marsrs_flutter_xlog` / `marsrs_flutter` | [Flutter](/zh/platforms/flutter) |
| React Native | `marsrs-react-native-xlog` / `marsrs-react-native` | [React Native](/zh/platforms/react-native) |
| 任何能调 C 的语言 | `marsrs-<version>-<host>` 压缩包 | [C ABI](/zh/platforms/c-abi) |
| HarmonyOS | `marsrs-harmony-<version>.tar.gz` 里的三个 `.so` | [HarmonyOS](/zh/platforms/harmonyos) |

`xlog` 只有日志；`marsrs` 再加上 STN 任务链路和 SDT 网络诊断 —— 和 crates.io 上那两个 crate 是同一对。只打日志的 App 用前者，而每个平台做的都是同一个切分：Apple 上是 `MarsRSXlog` 或 `MarsRSNet`，JitPack 上是 `xlog` 或 `marsrs`，共享 Kotlin 模块是 `xlog-kmp` 或 `marsrs-kmp`。

Flutter 插件和 React Native 模块在 pub.dev 和 npm 上：`flutter pub add`、
`npm install` 装的是上面最新的版本。

## 不是日志的那一半

两页，每半一页，各自都写着哪些平台带着它：

- [任务链路（STN）](/zh/stn) —— 一个任务排队、走短连接或长连接发出去、重试、超时、上报，而 App 回答 STN 跑的过程中问的那些问题。
- [网络诊断（SDT）](/zh/sdt) —— 对两条连接的那些主机做 ping、DNS、TCP、HTTP，以及一份写清每一项查到了什么的 JSON 报告。

## 其余几页

- [快速开始](/zh/getting-started) —— 每个平台的依赖和一段能跑的代码。
- [配置项](/zh/configuration) —— 每个配置项、默认值、以及在各个平台上的名字。
- [日志文件](/zh/log-files) —— 文件在哪、叫什么、什么时候要 flush、怎么读回来。
