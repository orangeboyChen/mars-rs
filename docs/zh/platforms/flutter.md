# Flutter

两个插件，一份 API：只打日志的 App 用 `marsrs_xlog`，要整个端口的用 `marsrs`
—— 今天两者是同一个东西，因为这个插件露出来的是日志那半。STN 和 SDT 将来只落在
`marsrs` 里、不落在别处：两个插件背后的那个 C ABI 是带着它们的 ——
见[C ABI](/zh/platforms/c-abi#任务链路) —— 但两者都还不在 Dart 里。两个里挑一个，
不要都要：它们带着同一份 native 库，一个 App 里有两份是编不过的。

## 安装

```bash
flutter pub add marsrs_xlog    # 要整个端口就 marsrs
```

插件在 pub.dev 上：`flutter pub add` 装的是上面最新的版本。release 里也带着插件本身，
就是 `marsrs-flutter-xlog-<version>.tar.gz`，用 `path:` 依赖压缩包里那个目录装的就是它：

```yaml
# pubspec.yaml
dependencies:
  marsrs_xlog:
    path: marsrs-xlog
```

插件里有、checkout 里没有的是那个 framework —— CocoaPods 解析不了
`Package.swift` 里那个 SwiftPM binary target，所以 iOS 要的是插件里带着的
framework，而仓库里不放。Android 不需要额外东西：它从 JitPack 解析
`io.github.orangeboychen.marsrs:xlog:<version>`，和直接用 AAR 的 App 是同一个
坐标。

然后 `flutter pub get`。`pod install` 是 `flutter build ios` 自己的事 —— 这个插
件是个带 podspec 的普通 Flutter 插件，App 的 `Podfile` 里不用写它。

## 打开、写、flush

```dart
import 'package:marsrs_xlog/marsrs_xlog.dart';

final dir = await getTemporaryDirectory();          // path_provider
final xlog = await Xlog.open(
  XlogConfig(
    logDir: '${dir.path}/xlog',
    namePrefix: 'marsrs',
    level: LogLevel.info,
    mode: AppenderMode.async,
  ),
);
await xlog.setConsoleLogEnabled(kDebugMode);

await xlog.i('startup', 'hello from mars');

await xlog.flush(sync: true);   // 读文件、上传之前
await xlog.close();
```

`logDir` 是唯一没有默认值的选项，其余都在[配置项](/zh/configuration)页上，用的
是端口的 Kotlin 给的那套名字。

## 每个调用都是 `Future`

这个插件是 method channel 而不是 `dart:ffi`：Apple 那边的二进制是
`MarsRSXlog.xcframework` 里的静态库，`DynamicLibrary.open` 没有东西可开。所以
下面每个调用都要跨到平台线程、返回一个 `Future` —— `await` 它，或者调用方不想等
这条记录时交给 `unawaited()`：

```dart
unawaited(xlog.i('startup', 'cold start'));
```

这是这份 API 唯一没法跟端口的 Swift 和 Kotlin 一样的地方 —— 那边一条记录就是一
次调用、没有别的 —— 也是为什么设置项是 `setLevel(…)` 而不是 `level = …`。

## 写

写是 `android.util.Log` 的形状 —— `v`/`d`/`i`/`w`/`e`/`f`，各是一个 tag 加一条
消息；级别要到调用时才知道就用 `log(level, tag, message)`。

```dart
await xlog.v('net', '…');
await xlog.d('net', '…');
await xlog.i('startup', '…');
await xlog.w('net', '…');
await xlog.e('login', '…');
await xlog.f('login', '…');

await xlog.log(LogLevel.debug, 'net', '…');
```

低于 appender 打开时那个级别的记录在格式化之前就被丢掉了。构造起来很贵的消息值
得先问一句 —— 记录横竖都会被丢，`isLoggable` 省下的是那个字符串：

```dart
if (await xlog.isLoggable(LogLevel.debug)) {
  await xlog.d('net', expensiveDescription());
}
```

## 开着的时候

| 干什么 | 怎么干 |
|---|---|
| 改级别 | `await xlog.setLevel(LogLevel.warning)` |
| 把级别读回来 | `await xlog.getLevel()` |
| 切异步 / 同步 | `await xlog.setMode(AppenderMode.sync)` |
| 同时打到控制台 | `await xlog.setConsoleLogEnabled(true)` |
| 到多大就换一个文件 | `await xlog.setMaxFileSize(8 * 1024 * 1024)` |
| 到多久就换一个文件 | `await xlog.setMaxAliveTime(10 * 24 * 3600)` |
| 还开着吗 | `xlog.isOpen` |
| 把缓存排出去 | `await xlog.flush(sync: true)` |

`close()` 排掉剩下的、关掉这个 appender。同一个 `namePrefix` 的两个 `Xlog` 是同
一个 appender —— native 那边是一个插件、每个 prefix 存一个 appender，每个调用都
带上它说的是哪个 prefix —— 所以日志要单独读的那部分 App，给它一个自己的 prefix。

## 里面没有什么

记录里的文件、函数、行号是空的：Dart 没有可以填进去的调用栈帧，C++ 自己写的也是
空的。当前文件在哪也不回答 —— 没有 `mars_xlog_current_log_path`、也没有
`Xlog.currentLogPath` —— 因为路径是 App 拿着自己给的那个目录问出来的，[日志文件
](/zh/log-files)页才是写它的地方。

[任务链路](/zh/stn)和[网络诊断](/zh/sdt)都不在里面：没有 `StnLogic`、没有 `SdtLogic`、
没有任务、也没有检查。缺的是 Dart —— 这个插件盖在上面的那个 C ABI 带着这两半，见
[C ABI](/zh/platforms/c-abi#任务链路)。但它解析到的原生侧，在两个平台上带的不是同一个东西：

| | 插件解析到什么 | 所以今天的任务 |
|---|---|---|
| Android | `io.github.orangeboychen.marsrs:marsrs` —— 整个移植的 AAR，STN 和 SDT 都在里面 | App 自己的 Android 代码能起一个，再用自己的 channel 把它带到 Dart |
| iOS | `marsrs-xlog.xcframework` 和 `mars_xlog.h` —— 只有日志，别的都没有 | 不行：`MarsRSNet` 没有被 vendored，所以没有 `MarsStn`、也没有 `MarsSdt` 可以链 |

所以"从平台侧起它"今天是个只对一半的答案：Android 上算，iOS 上要等 net 那个 framework 和
两个 net 头文件被打包到 xlog 那些旁边才算。
