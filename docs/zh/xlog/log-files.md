# 日志文件

## 在哪，叫什么

一个 appender 每天往配置里的日志目录写一个文件：

```text
<logDir>/<namePrefix>_YYYYMMDD.xlog
```

`XlogConfig(logDir = "/data/…/xlog/log", namePrefix = "marsrs")` 写出的就是
`marsrs_20260927.xlog`。目录不存在时会创建；前缀就是这个 appender 的身份 —— 两个
appender 用了同一个前缀就写同一个文件，关掉其中一个，另一个也就写不了了。
哪一块日志要单独读，就给它一个自己的前缀。

当前正在写的那个文件在哪：

::: code-group

```rust [Rust]
appender_get_current_log_path()          // Option<PathBuf>
appender_get_current_log_path_instance(id)
```

```swift [Swift]
Xlog.currentLogPath
```

```c [C]
char path[512];
mars_xlog_current_log_path(path, sizeof path);   // MARS_XLOG_OK，或负的错误码
```

:::

## 异步：记录可能还在缓存里

默认模式把记录交给写线程、先进 mmap 缓存文件，所以 write 返回时字节还没进日志文件。
缓存放在 `cacheDir`，没给就放在日志文件旁边。

**读文件前、上传前要 flush** —— 本进程要读，或者另一个进程在本进程还在写的时候要读：

::: code-group

```rust [Rust]
appender_flush_sync()                    // 等文件写完
appender_flush_instance(id, true)
```

```swift [Swift]
log.flush(sync: true)
```

```kotlin [Android]
xlog.flush(sync = true)
```

```kotlin [Kotlin Multiplatform]
Xlog.flush(sync = true)
```

```c [C]
mars_xlog_flush_sync();
```

:::

`flush(sync = false)` —— `appender_flush()`、`mars_xlog_flush()` —— 只是通知写线程就返回，
适合定时调用，但上传前不能用它。

## App 退出的时候

**什么都不用调用。** 进程写到一半被杀掉，也不会丢东西：缓存里的记录在内核手里、不在进程
手里，所以比进程活得久；下一个同 `namePrefix` 的 appender 打开时会把它们排进日志文件，
就夹在一次普通启动的 `~~~~~ begin of mmap ~~~~~` 和 `~~~~~ end of mmap ~~~~~` 两行之间。
一次都没 flush 的 App 只丢一样东西：时间 —— 本次会话最后那几条，要等下次启动才进文件。

有两个包连这点时间都不丢，因为 App 一离开屏幕它们就 flush 了 —— 那是 Android 和 iOS
在杀进程之前给的最后一次通知：

| 平台 | 谁在 flush |
|---|---|
| Android | `Xlog(config, context)` —— App 任意一个 `Context` 都会注册一个 `ComponentCallbacks2`，从 `TRIM_MEMORY_UI_HIDDEN` 往上就 flush |
| SwiftPM | 每个 `Xlog`，建好就开始：它看着 `didEnterBackground` 和 `willTerminate`，在 watchOS 上还看着 `WKExtension` 的 |
| 其余每个平台 | 下次启动，如上 |

`close()` 也会排空，所以退出时顺手关掉 appender 的 App 同样没事。上面那两个 `flush` 是为
另一种情况准备的：**App 还活着**的时候要读文件或上传。

同步模式是唯一的例外，也是唯一一个答案不是“什么都不用”的地方：它后面没有缓存文件兜底，
所以进程还攒着的那截 —— 不到 4 KiB —— 会跟着进程一起消失。用同步模式又想要这几条的 App
得自己 `close()` 或 `flush(sync: true)`：上面那两个 hook 做了这件事，顺手关掉 appender 的
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

不完整的文件也读得出来。读不出的那条记录 —— 进程在两次写入之间被杀掉留下的
半块，或者拷到别处时坏掉的某个字节 —— 会被跳过，而它占的那段字节会在输出里、
本来该是这条记录正文的地方标出来：损坏后面那些记录还在命令行写出的文件
里，不会跟着一起丢。

[命令行](/zh/xlog/cli)那一页是整条命令行 —— 怎么装、怎么造这对密钥、怎么读回一个文件、
怎么写一个。终端里 `xlog help` 打出来的是同样的东西；公钥到底对一条记录做了什么，
在[配置项](/zh/xlog/configuration#压缩与加密)。
