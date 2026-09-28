# React Native

两个包，一份 API：只打日志的 App 用 `marsrs-react-native-xlog`，要整个移植的用
`marsrs-react-native` —— 今天两者是同一个东西，因为这个模块露出来的是日志那半。STN 和
SDT 将来只落在 `marsrs-react-native` 里、不落在别处：两个包背后的那个 C ABI 是带着它们
的 —— 见[C ABI](/zh/platforms/c-abi#任务链路) —— 但两者都还不在 TypeScript 里。两个里
挑一个，不要都要：它们带着同一份 native 库，一个 App 里有两份是编不过的。

这个模块是 TurboModule，也就是新架构要的那个东西：React Native 0.74 或更新，桥
关掉。这是这份 API 唯一向 App 要的东西，其余的都是它换来的 —— TurboModule 不是
bridge module，还在老架构上的 App 没有可以 `TurboModuleRegistry.getEnforcing`
的模块。

## 安装

```bash
npm install marsrs-react-native-xlog        # 要整个移植就 marsrs-react-native
cd ios && pod install
```

包在 npm 上：`npm install` 装的是上面最新的版本。release 里也带着这个模块本身，就是
`marsrs-react-native-xlog-<version>.tgz`。`npm install
./marsrs-react-native-xlog-<version>.tgz` 装的就是它。

包里有、checkout 里没有的是那个 framework —— CocoaPods 解析不了 `Package.swift`
里那个 SwiftPM binary target，所以 iOS 要的是包里带着的 framework，而仓库里不
放。Android 不需要额外东西：它从 JitPack 解析
`io.github.orangeboychen.marsrs:xlog:<version>`，和直接用 AAR 的 App 是同一个
坐标。

App 里不用写这个模块的名字：autolinking 会找到 `android/` 里的 `ReactPackage`
和 `ios/` 里的 pod，也就是把 `Xlog` 放进 `TurboModuleRegistry` 的那两步。
`src/NativeXlog.ts` 是 React Native 的 codegen 读的那份 spec，
`package.json` 里的 `codegenConfig` 就是把它指到 `src` 上的那个开关：
`NativeXlogSpec` 生成到 App 自己的 `React-Codegen` pod 里、也生成到 Android 的
构建里，这个模块的两半都不用把这十一个签名写两遍。

## 打开、写、flush

```ts
import { AppenderMode, LogLevel, Xlog } from 'marsrs-react-native-xlog';

const xlog = Xlog.open({
  logDir: `${RNFS.DocumentDirectoryPath}/xlog`,
  namePrefix: 'marsrs',
  level: LogLevel.info,
  mode: AppenderMode.async,
});
xlog.consoleLogEnabled = __DEV__;

xlog.i('startup', 'hello from mars');

xlog.flush(true);   // 读文件、上传之前
xlog.close();
```

`logDir` 是唯一没有默认值的选项，其余都在[配置项](/zh/configuration)页上，用的
是这个移植的 Kotlin 给的那套名字。appender 不收这个目录时 `Xlog.open` 抛错 ——
`logDir` 或 `namePrefix` 是空的，或者那个目录写不了。

## 没有调用返回 `Promise`

这个模块的 method queue 是 `RCTJSThread`，所以它的方法在 JS 线程上被调用、也从
那里返回：返回值的方法在下一行之前就把值给回来了，没有哪个方法返回 `Promise`。
这就是这个 `Xlog` 能和 `platforms/apple/MarsRSXlog`、
`platforms/android/marsrs` 里的那个一个成员对一个成员对上的原因 ——
`Xlog.open(config)` 给出 appender，`xlog.i(tag, message)` 在它返回的时候已经落盘了。

五个设置项是属性而不是 `setLevel` / `getLevel` 成对出现，因为 JS 的属性正是
Swift 和 Kotlin 用的那个写法，而 TurboModule 的 setter 在写下的地方就被调用了：

```ts
xlog.level = LogLevel.warning;      // appender 自己的级别，读回来也是读它的
xlog.maxFileSizeBytes = 8 * 1024 * 1024;
```

`level` 是五个里唯一 C ABI 给了 getter 的那个，所以 `xlog.level` 读的是
appender 自己的；`mode`、`consoleLogEnabled`、`maxFileSizeBytes` 和
`maxAliveTimeSeconds` 回答的是这个实例最后写进去的那个值 —— C ABI 能回答的就这些，
这个移植的 Swift 和 Kotlin 答的也是这些。

## 写

写是 `android.util.Log` 的形状 —— `v`/`d`/`i`/`w`/`e`/`f`，各是一个 tag 加一条
消息；级别要到调用时才知道就用 `log(level, tag, message)`。

```ts
xlog.v('net', '…');
xlog.d('net', '…');
xlog.i('startup', '…');
xlog.w('net', '…');
xlog.e('login', '…');
xlog.f('login', '…');

xlog.log(LogLevel.debug, 'net', '…');
```

低于 appender 打开时那个级别的记录在格式化之前就被丢掉了。构造起来很贵的消息值
得先问一句 —— 记录横竖都会被丢，`isLoggable` 省下的是那个字符串：

```ts
if (xlog.isLoggable(LogLevel.debug)) {
  xlog.d('net', expensiveDescription());
}
```

## 开着的时候

| 作用 | 怎么写 |
|---|---|
| 改级别 | `xlog.level = LogLevel.warning` |
| 把级别读回来 | `xlog.level` |
| 切异步 / 同步 | `xlog.mode = AppenderMode.sync` |
| 同时打到控制台 | `xlog.consoleLogEnabled = true` |
| 到多大就换一个文件 | `xlog.maxFileSizeBytes = 8 * 1024 * 1024` |
| 超过这个秒数就删文件 | `xlog.maxAliveTimeSeconds = 10 * 24 * 3600` |
| 还开着吗 | `xlog.isOpen` |
| 把缓存排出去 | `xlog.flush(true)` |

`close()` 排掉剩下的、关掉这个 appender。同一个 `namePrefix` 的两个 `Xlog` 是同
一个 appender —— native 那边是一个模块、每个 prefix 存一个 appender，每个调用都
带上它说的是哪个 prefix —— 而且一个已经开着的 prefix 再 `Xlog.open` 一次，得到的
是已经开着的那个 appender，不是压在它上面的第二个：两个名字拿的是同一个 `Xlog`，
任意一个 `close` 就是两个都关。日志要单独读的那部分 App，给它一个自己的 prefix。

## 里面没有什么

记录里的文件、函数、行号是空的：JS 没有可以填进去的调用栈帧，C++ 自己写的也是空
的。当前文件在哪也不回答 —— 没有 `mars_xlog_current_log_path`、也没有
`Xlog.currentLogPath` —— 因为路径是 App 拿着自己给的那个目录问出来的，[日志文件
](/zh/log-files)页才是写它的地方。

[任务链路](/zh/stn)和[网络诊断](/zh/sdt)都不在里面：没有 `StnLogic`、没有 `SdtLogic`、
没有任务、也没有检查。缺的是 TypeScript —— 这个模块盖在上面的那个 C ABI 带着这两半，见
[C ABI](/zh/platforms/c-abi#任务链路)。但它解析到的原生侧，在两个平台上带的不是同一个东西：

| | 模块解析到什么 | 所以今天的任务 |
|---|---|---|
| Android | `io.github.orangeboychen.marsrs:marsrs` —— 整个移植的 AAR，STN 和 SDT 都在里面 | App 自己的 Android 代码能起一个，再用自己的 TurboModule 把它带到 JS |
| iOS | `marsrs-xlog.xcframework` 和 `mars_xlog.h` —— 只有日志，别的都没有 | 不行：`MarsRSNet` 没有被 vendored，所以没有 `MarsStn`、也没有 `MarsSdt` 可以链 |

所以“从平台侧起它”今天是个只对一半的答案：Android 上算，iOS 上要等 net 那个 framework 和
两个 net 头文件被打包到 xlog 那些旁边才算。
