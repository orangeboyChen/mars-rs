# 日志文件

## 在哪，叫什么

一个 appender 每天往配置里的日志目录写一个文件：

```text
<logDir>/<namePrefix>_YYYYMMDD.xlog
```

`XlogConfig(logDir = "/data/…/xlog/log", namePrefix = "Ham")` 写出的就是
`Ham_20260927.xlog`。目录不存在时会创建；前缀是这个 appender 的身份 —— 两个
appender 用了同一个前缀就写同一个文件，关掉其中一个，另一个也就写不了了。
哪一块日志要单独读取，就给它一个自己的前缀。

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

**读文件前、上传前、进程退出前都要 flush** —— 被杀掉的进程最后那几条记录，要有人排空才到得了磁盘：

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
xlog-compat decode --privkey=<hex> --in=Ham_20260927.xlog --out=Ham.plain
```

```rust [Rust]
use marsrs::xlog::{get_period_logs, LogBuffer};

// 某一天两个小时之间的那些记录的字节范围
let (begin, end) = get_period_logs(std::path::Path::new("Ham_20260927.xlog"), 0, 24)?;
```

```python [上游的工具]
python3 decode_mars_log_file.py Ham_20260927.xlog      # Tencent/mars
```

:::

加密过的文件需要配置里那个公钥对应的私钥 —— `xlog-compat decode` 用
`--privkey` 接它；没有私钥，谁也读不出那些记录。
