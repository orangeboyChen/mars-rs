# React Native

两个包，一份 API：只打日志的 App 用 `marsrs-react-native-xlog`，要整个端口的用
`marsrs-react-native` —— 今天两者是同一个东西，因为 C ABI 就是 28 个
`mars_xlog_*` 符号、没有别的。STN 和 SDT 将来只落在 `marsrs-react-native` 里。
两个里挑一个，不要都要：它们带着同一份 native 库，一个 App 里有两份是编不过的。

## 安装

```bash
npm install marsrs-react-native-xlog        # 要整个端口就 marsrs-react-native
cd ios && pod install
```

包发在 npm 上 —— 发布那一步还没开，在那之前，
`scripts/package_react_native.sh <version> <asset-dir>` 写出来的
`marsrs-react-native-xlog-<version>.tgz` 就是同一个模块：`npm pack`，版本号打进
`package.json`、apple 那个 job 的 `MarsRSXlog.xcframework` 拷进去。`npm install
./marsrs-react-native-xlog-<version>.tgz` 装的就是它。

包里有、checkout 里没有的是那个 framework —— CocoaPods 解析不了 `Package.swift`
里那个 SwiftPM binary target，所以 iOS 要的是包里带着的 framework，而仓库里不
放。Android 不需要额外东西：它从 JitPack 解析
`io.github.orangeboychen.marsrs:xlog:<version>`，和直接用 AAR 的 App 是同一个
坐标。

App 里不用写这个模块的名字：autolinking 会找到 `android/` 里的 `ReactPackage`
和 `ios/` 里的 pod，也就是把 `Xlog` 放进 `NativeModules` 的那两步。

## 打开、写、flush

```ts
import { AppenderMode, LogLevel, Xlog } from 'marsrs-react-native-xlog';

const xlog = await Xlog.open({
  logDir: `${RNFS.DocumentDirectoryPath}/xlog`,
  namePrefix: 'marsrs',
  level: LogLevel.info,
  mode: AppenderMode.async,
});
await xlog.setConsoleLogEnabled(__DEV__);

await xlog.i('startup', 'hello from mars');

await xlog.flush(true);   // 读文件、上传之前
await xlog.close();
```

`logDir` 是唯一没有默认值的选项，其余都在[配置项](/zh/configuration)页上，用的
是端口的 Kotlin 给的那套名字。

## 每个调用都是 `Promise`

这个模块是 native module 而不是 JSI binding：Apple 那边的二进制是
`MarsRSXlog.xcframework` 里的静态库，没有可以 `dlopen` 的东西。所以每个打到
appender 的调用都要跨到平台线程、返回一个 `Promise` —— `await` 它，或者调用方不
想等这条记录时就让它去：

```ts
void xlog.i('startup', 'cold start');
```

这是这份 API 唯一没法跟端口的 Swift 和 Kotlin 一样的地方 —— 那边一条记录就是一
次调用、没有别的 —— 也是为什么设置项是 `setLevel(…)` 而不是 `level = …`。读回来
的那些 —— `level`、`mode`、`maxFileSizeBytes` … —— 是这个实例自己存的值的
getter，不用跨桥。

## 写

写是 `android.util.Log` 的形状 —— `v`/`d`/`i`/`w`/`e`/`f`，各是一个 tag 加一条
消息；级别要到调用时才知道就用 `log(level, tag, message)`。

```ts
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

```ts
if (await xlog.isLoggable(LogLevel.debug)) {
  await xlog.d('net', expensiveDescription());
}
```

## 开着的时候

| 干什么 | 怎么干 |
|---|---|
| 改级别 | `await xlog.setLevel(LogLevel.warning)` |
| 把级别读回来 | `xlog.level` |
| 切异步 / 同步 | `await xlog.setMode(AppenderMode.sync)` |
| 同时打到控制台 | `await xlog.setConsoleLogEnabled(true)` |
| 到多大就换一个文件 | `await xlog.setMaxFileSize(8 * 1024 * 1024)` |
| 到多久就换一个文件 | `await xlog.setMaxAliveTime(10 * 24 * 3600)` |
| 还开着吗 | `xlog.isOpen` |
| 把缓存排出去 | `await xlog.flush(true)` |

`close()` 排掉剩下的、关掉这个 appender。同一个 `namePrefix` 的两个 `Xlog` 是同
一个 appender —— native 那边是一个模块、每个 prefix 存一个 appender，每个调用都
带上它说的是哪个 prefix —— 所以日志要单独读的那部分 App，给它一个自己的 prefix。

## 里面没有什么

记录里的文件、函数、行号是空的：JS 没有可以填进去的调用栈帧，C++ 自己写的也是空
的。当前文件在哪也不回答 —— 没有 `mars_xlog_current_log_path`、也没有
`Xlog.currentLogPath` —— 因为路径是 App 拿着自己给的那个目录问出来的，[日志文件
](/zh/log-files)页才是写它的地方。
