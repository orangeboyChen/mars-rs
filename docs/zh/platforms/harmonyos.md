# HarmonyOS

两条路，区别是谁写 NAPI 模块：`marsrs-harmonyos-xlog` 是包 —— 一个 App 装上、用
ArkTS 写的 HAR；`marsrs-harmony-<version>.tar.gz` 是三个 `libmars_ffi.so` 加头文
件，给想自己写 NAPI 的 App。两条里挑一条，别都要：两边带着同一个 Rust 核心，一个
进程里两份 appender 就是两个 appender 往同一个文件写。

## 安装

```bash
ohpm install marsrs-harmonyos-xlog
```

ohpm 还没开始发布，所以在那之前包是从 release 里拿的：取下
`marsrs-harmonyos-xlog-<version>.har`，装那个文件。

```bash
ohpm install ./marsrs-harmonyos-xlog-<version>.har
```

```text
marsrs-harmonyos-xlog-<version>.har
    Index.ets                       `import { Xlog } from …` 解析到的入口
    oh-package.json5
    src/main/ets/xlog/Xlog.ets      ArkTS
    src/main/cpp/napi_init.cpp      它调的 NAPI 模块
    libs/arm64-v8a/libmarsrs_xlog.so
    libs/armeabi-v7a/libmarsrs_xlog.so
    libs/x86_64/libmarsrs_xlog.so
```

`libmarsrs_xlog.so` 是 `libmars_ffi`（C ABI 的静态库）外面套了 `napi_init.cpp`，
所以拿到 HAR 的 App 不需要再解析别的。那一层是 C 不是 C++，这对 App 是有影响的：
一个为了 `std::mutex` 而用 C++ 写的 NAPI 模块，等于让加载它的进程去加载一个本来
不需要的 C++ 运行时。发出去的是二进制，不是 App 构建时要跑一遍的 native 工程。

NAPI 模块叫 `marsrs_xlog`，也就是 `import xlogNapi from 'libmarsrs_xlog.so'` 解析
的那个名字；它导出什么写在 `src/main/cpp/types/libmarsrs_xlog/index.d.ts` 里 ——
hvigor 自己会在那儿找，所以 `oh-package.json5` 里不需要 `types` 字段。

## 打开、写、flush

```typescript
import { AppenderMode, CompressMode, LogLevel, Xlog } from 'marsrs-harmonyos-xlog';

const xlog: Xlog = Xlog.open({
  logDir: `${getContext().filesDir}/xlog/log`,
  cacheDir: `${getContext().filesDir}/xlog/cache`,
  namePrefix: 'marsrs',
  level: LogLevel.Info,
  mode: AppenderMode.Async,
  compressMode: CompressMode.Zlib,
});
xlog.consoleLogEnabled = true;

xlog.i('startup', 'hello from mars');

xlog.flush(true);   // 读文件或者上传之前
xlog.close();
```

`logDir` 是唯一没有默认值的配置项 —— 其余的在[配置项](/zh/configuration)那页，用的
是 port 的 Kotlin 给的那批名字。`Xlog.open` 在 appender 不收这个目录时抛错：空的 `logDir`，或者进程写不进去的目录
—— React Native 模块和 Kotlin 那边也是这样。

`logDir` 放在 `getContext().filesDir` 下是 App 该放的地方 —— App 自己的目录，
appender 能建，[日志文件](/zh/log-files)那页也是从它算出文件名。

## 没有哪个方法返回 `Promise`

NAPI 模块的每个方法都是同步的：调用从发起它的那个线程返回，不返回 promise —— 下一行
执行前记录已经在 C ABI 里了。这才让这个 `Xlog` 和 `platforms/android/marsrs`、
`platforms/react-native/marsrs-xlog` 的 `Xlog` 一个成员对一个成员，而不是 Flutter 那
个 —— Flutter 的调用要过 channel，回答的是 `Future`。

五个设置是属性，不是 `setLevel` / `getLevel` 一对对的方法，因为属性是 Swift 和
Kotlin 用的写法：

```typescript
xlog.level = LogLevel.Warning;      // appender 自己的值，从它读回来
xlog.maxFileSizeBytes = 8 * 1024 * 1024;
```

`level` 是五个里唯一 C ABI 有 getter 的，所以 `xlog.level` 读的是 appender 自己
的；`mode`、`consoleLogEnabled`、`maxFileSizeBytes`、`maxAliveTimeSeconds` 回答的是
这个实例最后写进去的值 —— C ABI 能回答的就这些，Swift 和 Kotlin 那边也是一样。

`LogLevel` 是 `Verbose` / `Debug` / `Info` / `Warning` / `Error` / `Fatal` /
`None`，用 PascalCase 而不是 Kotlin 的 `INFO`：ArkTS 的枚举写在满屏鸿蒙枚举的 App
里，这是它们用的写法。跨 ABI 走的都是 C ABI 自己那个整数。

## 写日志

写是 `android.util.Log` 的形状 —— `v`/`d`/`i`/`w`/`e`/`f`，各是一个 tag 加一条消息；
级别到调用时才知道就用 `log(level, tag, message)`。

```typescript
xlog.v('net', '…');
xlog.d('net', '…');
xlog.i('startup', '…');
xlog.w('net', '…');
xlog.e('login', '…');
xlog.f('login', '…');

xlog.log(LogLevel.Debug, 'net', '…');
```

低于 appender 打开时那个级别的记录在格式化之前就丢掉了。构造起来很贵的消息值得先问一
句 —— 记录反正都要丢，`isLoggable` 省下的是那个字符串：

```typescript
if (xlog.isLoggable(LogLevel.Debug)) {
  xlog.d('net', expensiveDescription());
}
```

## 开着的时候

| 要什么 | 怎么写 |
|---|---|
| 改级别 | `xlog.level = LogLevel.Warning` |
| 把级别读回来 | `xlog.level` |
| 切异步 / 同步 | `xlog.mode = AppenderMode.Sync` |
| 同时打到控制台 | `xlog.consoleLogEnabled = true` |
| 到某个大小换文件 | `xlog.maxFileSizeBytes = 8 * 1024 * 1024` |
| 到某个天数丢文件 | `xlog.maxAliveTimeSeconds = 10 * 24 * 3600` |
| 还开着吗 | `xlog.isOpen` |
| 把缓存倒进文件 | `xlog.flush(true)` |

`close()` 把剩下的倒完再关 appender。同一个 `namePrefix` 的两个 `Xlog` 是一个
appender —— native 那边每个 prefix 只握一个 handle，每次调用都带上它说的是哪个
prefix —— 已经开着的 prefix 再 `Xlog.open` 一次，回答的是之前那个 appender 而不是在
它上面再开一个，所以两个名字握着同一个 `Xlog`，任一个 `close` 两边都关了。App 里日志
要单独读的那部分该有自己的 prefix。

## 另一条路：三个 `.so`

自己写 NAPI 的 App 拿 `marsrs-harmony-<version>.tar.gz`，得到的是没包任何东西的
C ABI：

```text
marsrs-harmony-<version>/
    arm64-v8a/libmars_ffi.so       aarch64-unknown-linux-ohos
    armeabi-v7a/libmars_ffi.so     armv7-unknown-linux-ohos
    x86_64/libmars_ffi.so          x86_64-unknown-linux-ohos
    include/mars_xlog.h
```

把 `libmars_ffi.so` 放到模块的 `libs/<abi>/` 下 —— 真机是 `arm64-v8a` —— 然后在 C
里、App 启动时打开一次 appender：

```c
#include <mars_xlog.h>

MarsXLogConfig config = {
    .mode = MarsAppenderAsync,
    .log_dir = log_dir,      /* 应用的 files 目录 */
    .name_prefix = "marsrs",
    .compress_mode = MarsCompressZlib,
};
mars_xlog_open(&config);
mars_xlog_write(MarsLevelInfo, "startup", __FILE__, __func__, __LINE__, "hello");
```

[C ABI](/zh/platforms/c-abi)那页都适用：配置、级别、实例和错误码是
同一批符号。

## mars 的另一半

`libmars_ffi.so` 是整个移植，不只是日志那半：它是在默认的 `xlog` 之上带着 `sdt`
和 `stn` 两个 feature 构建的，库旁边放着的头文件是 `mars_xlog.h`、`mars_sdt.h`、
`mars_stn.h` 三个都有。所以 HarmonyOS 应用可以通过它为
日志写的那个 NAPI 封装，跑一个任务、或者做一次诊断：

```c
#include <mars_stn.h>

mars_stn_set_app(NULL, ask);          /* 那十八个问题，一个回调回答 */
mars_stn_start_task(&task);

/* 回答是这一趟还能等多少毫秒 */
long long due = mars_stn_due_time();
while (due >= 0) {                    /* App 自己排空队列 */
    usleep(due * 1000);
    mars_stn_run_pending();
    due = mars_stn_due_time();
}
```

这个包的 ArkTS 不碰这两半 —— HAR 是日志那半 —— 各自要 App 做什么在那两页上：[任务
链路](/zh/stn)和[网络诊断](/zh/sdt)。那里没有一样东西是 HarmonyOS 自己的 —— 是[C ABI
那页](/zh/platforms/c-abi)为 Linux、macOS、Windows 发布的同一个 C ABI，也是
`MarsRSNet`、Android 那两个 AAR 和 `marsrs-kmp` 写在它上面的那个。

## 里面没有的

记录里的文件名、函数名和行号是空的：没有值得写进记录的 ArkTS 帧，C++ 写的也是空
的。当前文件在哪不回答 —— 没有 `mars_xlog_current_log_path`，也没有
`Xlog.currentLogPath` —— 因为路径是 App 拿它给的目录自己问出来的，[日志文
件](/zh/log-files)那页是说出这个名字的地方。
