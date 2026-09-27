# C ABI

`marsrs-<version>-<host>.tar.gz`（Linux、macOS）和 `.zip`（Windows）里是
`marsrs-ffi` 的头文件和库：

```text
include/mars_xlog.h     日志
include/mars_sdt.h      网络诊断
include/mars_stn.h      任务链路
libmars_ffi.a / libmars_ffi.so（.dylib、.dll）
```

为 `x86_64-unknown-linux-gnu`、`aarch64-apple-darwin`、`x86_64-pc-windows-msvc` 构建，
任务链路和网络诊断都包含在内。

## 打开、写、flush、关闭

```c
#include <mars_xlog.h>

MarsXLogConfig config = {
    .mode = MarsAppenderAsync,
    .log_dir = "/tmp/mars-log",
    .name_prefix = "Ham",
    .compress_mode = MarsCompressZlib,
    /* .pub_key、.compress_level、.cache_dir、.cache_days —— 见头文件 */
};

int rc = mars_xlog_open(&config);
if (rc != MARS_XLOG_OK) { /* MARS_XLOG_ERR_* —— 见头文件 */ }

mars_xlog_set_level(MarsLevelInfo);

mars_xlog_write(MarsLevelInfo, "startup", __FILE__, __func__, __LINE__, "hello from mars");

mars_xlog_flush_sync();   /* 返回时记录已经在磁盘上了 */
mars_xlog_close();
```

`MarsXLogConfig` 里每个指针都必须是 NUL 结尾的 UTF-8 或 `NULL`；`NULL` 等于"空"，
只有 `log_dir` 是必填。一次写入的 `tag`、`filename`、`func_name`、`message` 可以是
`NULL`。

`mars_xlog_open` 装一个进程级别的 appender，`mars_xlog_write` 往它写。
`mars_xlog_flush()` 只是通知写线程；`mars_xlog_flush_sync()` 会等。

## 不止一个 appender

```c
long long id = mars_xlog_new_instance(&config, MarsLevelInfo);
if (id != 0) {
    mars_xlog_write_instance(id, MarsLevelDebug, "net", __FILE__, __func__, __LINE__, "…");
    mars_xlog_flush_instance(id, 1 /* sync */);
    mars_xlog_get_level(id);
    mars_xlog_set_level_instance(id, MarsLevelWarning);
    mars_xlog_set_mode_instance(id, MarsAppenderSync);
    mars_xlog_release_instance(config.name_prefix);
}
```

`mars_xlog_new_instance` 拒绝这份配置时返回 `0`。一个实例由它的 `name_prefix` 标识，
`mars_xlog_get_instance(name_prefix)` 返回已经开着的那个。

## 链接

```bash
cc -I include -o app app.c libmars_ffi.a -lpthread -ldl     # 静态
cc -I include -o app app.c -L. -lmars_ffi                  # 动态
```

头文件就签在 crate 旁边，`crates/marsrs-ffi/include`，所以构建可以直接指向仓库，
不必拷一份。

## 当前文件与错误码

```c
char path[1024];
if (mars_xlog_current_log_path(path, sizeof path) == MARS_XLOG_OK) {
    /* path 就是 appender 正在写的地方 */
}
```

所有返回 `int` 的调用，要么返回 `MARS_XLOG_OK`（0），要么返回负的 `MARS_XLOG_ERR_*`：
`config` 是 `NULL`、mode 或压缩器不是合法值、`log_dir` 是空的、appender 拒绝了配置、
输出缓冲区太小、还没有打开的文件。`MARS_XLOG_ERR_PANIC` 是在边界上捕获到的 Rust
panic —— C ABI 里不会有任何东西 unwind 进 C。
