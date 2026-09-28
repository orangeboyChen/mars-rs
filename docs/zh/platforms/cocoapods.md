# CocoaPods

```ruby
# Podfile
platform :ios, '12.0'
use_frameworks!

pod 'MarsRSXlog', :podspec => 'https://raw.githubusercontent.com/orangeboyChen/mars-rs/v0.1.0/MarsRSXlog.podspec'
```

三个 pod，和[Swift 包](/zh/platforms/swift)的三个 product 同名：

| pod | 拿到什么 |
|---|---|
| `MarsRSXlog` | 日志：`Xlog`、`XlogConfig`、`LogLevel`、`AppenderMode`、`CompressMode` |
| `MarsRSNet` | STN 任务链路和 SDT 网络诊断 |
| `MarsRS` | 两半都重新导出 |

这一行的关键是 `:podspec`。这些 pod 不在任何 spec repo 上 —— 一次 release 发布的是
`marsrs-cocoapods-xlog-<version>.zip`、`marsrs-cocoapods-net-<version>.zip`，以及该 tag
下的 podspec；podspec 里写的才是 `pod install` 要下载的压缩包。只写
`pod 'MarsRSXlog'`，是去 trunk CDN 上找一个并不在那里的 pod。

两半都要的 app 把三个都写上：

```ruby
pod 'MarsRS', :podspec => 'https://raw.githubusercontent.com/orangeboyChen/mars-rs/v0.1.0/MarsRS.podspec'
pod 'MarsRSXlog', :podspec => 'https://raw.githubusercontent.com/orangeboyChen/mars-rs/v0.1.0/MarsRSXlog.podspec'
pod 'MarsRSNet', :podspec => 'https://raw.githubusercontent.com/orangeboyChen/mars-rs/v0.1.0/MarsRSNet.podspec'
```

`MarsRS` 自己不带 framework —— 它只有两行 `@_exported import`，每半一行 —— 所以它
从 tag 里取自己那个 module 的 Swift，两个 framework 则由另外两行给出的 pod 提供。

`MarsRSXlog` 和 `MarsRSNet` 支持 iOS 12.0 和 watchOS 10.0，Swift 5，各自带着 Swift 包
解析到的同样四个 slice。`MarsRSXlog` 替 app 链接 `CoreFoundation`：
`iana-time-zone` 要用它问时区，而 Rust 静态库自己写不进链接参数。

## Swift

Swift 的接口就是 [SwiftPM](/zh/platforms/swift) 那页上的那套 —— 一次 `import`，同样
的 `Xlog`、`XlogConfig`、`LogLevel`，因为 pod 和 Swift 包是同一个 framework 上的同一份
Swift。

## Objective-C

这个移植里没有一行 Objective-C 源码。`Xlog`、`XlogConfig` 和三个 enum 都是 `@objc`，
Objective-C 文件 import 的头文件，是编译器从它们生成的那一份，就在 pod 的 module 里：

```objc
@import MarsRSXlog;
```

```objc
XlogConfig *config = [[XlogConfig alloc] initWithLogDirectory:logDirectory.path];
config.namePrefix = @"marsrs";
config.level = LogLevelInfo;

NSError *error = nil;
Xlog *log = [[Xlog alloc] initWithConfig:config error:&error];
if (log == nil) {
    NSLog(@"no appender: %ld %@", (long)error.code, error.localizedDescription);
    return;
}
log.isConsoleLogEnabled = YES;

if ([log isEnabledFor:LogLevelDebug]) {
    [log writeWithLevel:LogLevelDebug message:@"cold start" tag:@"startup"];
}

[log flushWithSync:YES];   // 读文件或上传前
[log close];
```

`initWithConfig:error:` 就是 Swift 里那个会抛错的构造器：失败时返回 `nil`，并给出一个
`NSError`，它的 `code` 是一个 `XlogError` —— `XlogErrorEmptyLogDirectory`、
`XlogErrorEmptyNamePrefix`、`XlogErrorInvalidCompressionLevel`、
`XlogErrorNegativeCacheDays`、`XlogErrorRefused` —— `localizedDescription` 则是 Swift 的
`description` 给出的同一句话。domain 是 `MarsRSXlog.XlogError`。

Objective-C 写一条记录用 `writeWithLevel:message:tag:`：Swift 的 `file`、`function`、
`line` 是在调用处填进去的，而 Objective-C 没有 `#file` 可填，所以这里写出的记录里，文件名
和函数名是空的、行号是 0。要把“在哪写的”写进记录，用长那个版本，`__FILE__`、
`__PRETTY_FUNCTION__`、`__LINE__` 就填在那里：

```objc
[log log:LogLevelInfo message:@"written from Objective-C" tag:@"objc"
        file:@__FILE__ function:@__PRETTY_FUNCTION__ line:__LINE__];
```

| 作用 | 怎么写 |
|---|---|
| 改级别 | `log.level = LogLevelWarning` |
| 切异步 / 同步 | `log.mode = AppenderModeSync` |
| 同时打到控制台 | `log.isConsoleLogEnabled = YES` |
| 到某个大小换文件 | `log.maxFileSizeBytes = 8 * 1024 * 1024` |
| 到某个时间删文件 | `log.maxAliveTimeSeconds = 10 * 24 * 3600` |
| 还开着吗 | `log.isOpen` |
| 当前文件在哪 | `[Xlog currentLogPath]` |

`XlogConfig` 的每个字段都能这样赋值，都在[配置项](/zh/configuration)那页：

```objc
config.mode = AppenderModeAsync;
config.compression = CompressModeZstd;
config.compressionLevel = 6;
config.cacheDays = 3;
config.publicKey = publicKey;   // 留空写出的文件不加密
```

Objective-C 拿不到的是网络那半：`MarsStn` 和 `MarsSdt` 是把 C ABI 那些平铺的名字归到一起
的命名空间，而 Swift 里“静态成员的 `enum`”不是 Objective-C 看得见的类型。要跑任务或做
网络诊断的 app，那部分得用 Swift 写。
