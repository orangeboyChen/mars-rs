# 日志文件

## 在哪，叫什么

一个 appender 一天写一个文件，写在配置给的日志目录里：

```text
<logDir>/<namePrefix>_YYYYMMDD.xlog
```

`XlogConfig(logDir = "/data/…/xlog/log", namePrefix = "marsrs")` 写出的就是
`marsrs_20260927.xlog`。目录不存在就创建；前缀就是这个 appender 的身份 —— 两个
appender 用了同一个前缀就写同一个文件，关掉其中一个，另一个也就写不了了。
哪一块日志要单独读，就给它一个自己的前缀。

当前正在写的那个文件在哪：

::: code-group

```rust [Rust]
xlog.current_log_path()                  // Option<PathBuf>
appender_get_current_log_path()          // 进程级那个 appender 的
```

```swift [Swift]
log.currentLogPath
```

```c [C]
char path[512];
mars_xlog_current_log_path(path, sizeof path);   // MARS_XLOG_OK，或负的错误码
```

:::

能给出这个路径的就是这三个包 —— 而且 Swift 那个是这个 `Xlog` 自己的，跟 Rust 和
Kotlin Multiplatform 模块里一样。其余平台要 App 自己拼出文件名，拼的时候用的还是交给
配置的那两样：目录和前缀，中间夹着当天日期。

一整**天**的文件是另一件事，要上传昨天日志的 App 问的就是它。这里有两个调用：一个是
确实存在的那些文件，一个是这一天会写进哪几个名字 —— 不管文件在不在：

::: code-group

```rust [Rust]
appender_getfilepath_from_timespan(1, "marsrs", Path::new(log_dir))  // 昨天的、确实存在的
appender_make_logfile_name(1, "marsrs", Path::new(log_dir))          // 名字，不管在不在
```

```swift [Swift]
log.logFiles(daysAgo: 1)
log.logFileNames(daysAgo: 1)
```

```c [C]
char path[512];
// 下标 0、1、2 ……；负的错误码 —— MARS_XLOG_ERR_NO_PATH —— 表示后面没有了
mars_xlog_getfilepath_from_timespan(1, "marsrs", log_dir, 0, path, sizeof path);
mars_xlog_make_logfile_name(1, "marsrs", log_dir, 0, path, sizeof path);
```

:::

`0` 是今天，`1` 是昨天。Rust 和 Swift 一次给出这一天的全部；C ABI 一次只给一个下标，
Swift 那两个调用做的就是一趟走完它的事。配了缓存目录、而且文件确实存在时，“名字”
会比“文件”多出一个：日志目录里那个，和它在缓存目录里的孪生文件。

## 异步：记录可能还在缓存里

默认模式下记录先落进一个 mmap 缓存文件，再由写线程搬进日志文件，所以 `write` 返回的时候
字节还没进日志文件。缓存放在 `cacheDir`，没给就放在日志文件旁边。

**读之前、上传之前先 flush** —— 本进程自己要读，或者另一个进程在本进程还写着的时候要读：

::: code-group

```rust [Rust]
xlog.flush_now()                         // 等文件写完
```

```swift [Swift]
log.flushNow()
```

```kotlin [Android]
xlog.flushNow()
```

```kotlin [Kotlin Multiplatform]
xlog.flushNow()
```

```typescript [HarmonyOS]
xlog.flushNow()
```

```dart [Flutter]
await xlog.flush()
```

```ts [React Native]
xlog.flushNow()
```

```c [C]
mars_xlog_flush_now_instance(0);
```

:::

排空是三个调用，不是一个带开关的调用：

| 调用 | 做什么 |
|---|---|
| `requestFlush()` —— `xlog.request_flush()`、`mars_xlog_request_flush_instance(0)` | 提出一次排空，然后立刻返回：它返回了，不代表记录已经在文件里，它也不说排空什么时候结束 |
| `flushNow()` —— `xlog.flush_now()`、`mars_xlog_flush_now_instance(0)` | 占着调用方线程排空：它返回时记录已经在磁盘上 |
| `await flush()` —— `xlog.flush().await`、`flush(handle)` | 同一次排空，交给别的线程：await 到它完成时，记录已经在磁盘上 |

三个调用的分别只在**谁等**，以及谁拿得到“排完了”这句话。`requestFlush()` 谁也不等
—— 它是定时器该调的那个 —— 提出一次排空就走，剩下的是写线程的事；没排完也不丢东西：
还在缓存里的记录在一个内核手里的文件里。读文件或上传之前要调的是 `flushNow()`，它
占着调用方线程等到记录落盘；`await flush()` 是同一次排空，交给别的线程，给不想占着
一个线程的调用方。Dart 没有 `flushNow()`
—— method channel 阻塞不了 Dart 这一侧；HarmonyOS 没有 `await flush()` —— 它的
NAPI 每个方法都是同步的。Rust 三个都有，它的 `await flush()` 是一个用
`std::thread::spawn` 写出来的 `Future`：不需要 runtime，任何执行器都能等它。

## App 退出的时候

**什么都不用调用。** 进程写到一半被杀掉，也不会丢东西：缓存里的记录在内核手里、不在进程
手里，所以比进程活得久；下一个同 `namePrefix` 的 appender 打开时会把它们排进日志文件，
就夹在一次普通启动的 `~~~~~ begin of mmap ~~~~~` 和 `~~~~~ end of mmap ~~~~~` 两行之间。
一次都没 flush 的 App 只丢一样东西：时间 —— 本次会话最后那几条，要等下次启动才进文件。

有两个包连这点时间都不丢，因为 App 一离开屏幕它们就 flush 了 —— 那是 Android 和 iOS
在杀进程之前给的最后一次通知：

| 平台 | 谁在 flush |
|---|---|
| Android | `Xlog(config, context)` —— App 的任意一个 `Context` 都会注册一个 `ComponentCallbacks2`，从 `TRIM_MEMORY_UI_HIDDEN` 往上就 flush |
| SwiftPM | 每个 `Xlog`，建好就开始：它看着 `didEnterBackground` 和 `willTerminate`，在 watchOS 上还看着 `WKExtension` 的 |
| 其余每个平台 | 下次启动，如上 |

`close()` 也会排空，所以退出时顺手关掉 appender 的 App 同样没事。上面的 flush 是为
另一种情况准备的：**App 还活着**的时候要读文件或上传。

同步模式是唯一的例外，也是唯一一个答案不是“什么都不用”的地方：它后面没有缓存文件兜底，
所以进程还攒着的那截 —— 不到 4 KiB —— 会跟着进程一起消失。用同步模式又想要这几条的 App
得自己 `close()` 或 `flushNow()`：上面那两个 hook 做了这件事，顺手关掉 appender 的
App 也做了。

## 轮转与保留

| 旋钮 | 作用 | 默认值 |
|---|---|---|
| `maxFileSizeBytes` | 文件到这个字节数就关闭、开新文件 | `0` —— 永不切分 |
| `maxAliveTimeSeconds` | 文件超过这个秒数就删掉 | `0` —— 留着（C++ 自己保留十天） |
| `cacheDays` | 异步缓存文件超过这个天数就删掉 | `0` —— 都留着 |

## 把文件读回来

这里写出来的 `.xlog` 就是 C++ 实现写出来的那个 `.xlog` —— 同样的帧结构、同样的压缩、
同样的加密，所以原本读 mars 日志的工具就能读它。

::: code-group

```bash [命令行]
cargo install marsrs-xlog          # `xlog` 装到 $PATH 上；每个 release 里
                                   # 也有同一个命令的压缩包
xlog decode --privkey=<hex> marsrs_20260927.xlog --out=marsrs.plain
```

```rust [Rust]
use marsrs::xlog::{get_period_logs, LogBuffer};

// 某一天两个小时之间的那些记录的字节范围
let (begin, end) = get_period_logs(std::path::Path::new("marsrs_20260927.xlog"), 0, 24)?;
```

```python [上游的工具]
python3 decode_mars_log_file.py marsrs_20260927.xlog      # Tencent/mars
```

:::

加密过的文件需要配置里那个公钥对应的私钥 —— `xlog decode` 用 `--privkey`
接它；没有私钥，谁也读不出那些记录。`xlog encode` 是另一半：输入里一行就是
一条记录，它把这些记录写成一个 `.xlog`，给了 `--pubkey`（那对密钥里的公钥）
就顺带加密。

不完整的文件也读得出来。读不出的那条记录 —— 进程在两次写入之间被杀掉留下的半块，
或者拷到别处时坏掉的某个字节 —— 会跳过去，它占的那段字节在输出里标出来，就标在原本
该是这条记录正文的地方：损坏后面那些记录还在命令行写出的文件里，不会跟着一起丢。

[命令行](/zh/xlog/cli)那一页是整条命令行 —— 怎么装、怎么造这对密钥、怎么读回一个文件、
怎么写一个。终端里 `xlog help` 打出来的是同样的东西；公钥到底对一条记录做了什么，
在[配置项](/zh/xlog/configuration#压缩与加密)。
