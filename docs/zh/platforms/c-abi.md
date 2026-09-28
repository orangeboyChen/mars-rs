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
    .name_prefix = "marsrs",
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

## 任务链路

`mars_stn.h` 是这个移植里跟服务器说话的那半，也是同一个库：不用多链接什么，也不用多
打开什么。

```c
#include <mars_stn.h>

mars_stn_set_app(NULL, ask);          /* 一个回调回答那十八个问题 */

MarsStnTask task;
memset(&task, 0, sizeof task);
task.taskid = mars_stn_gen_task_id();
task.channel_select = 0x3;            /* 两条连接 */
task.cgi = "/cgi-bin/hello";
mars_stn_start_task(&task);

/* 这个移植没有线程：队列由谁调用谁来排空 —— 而回答是这一趟还能等多少毫秒，
   MARS_STN_ERR_NO_DUE 是"没有可等的东西" */
long long due = mars_stn_due_time();
while (due >= 0) {
    usleep(due * 1000);
    mars_stn_run_pending();
    due = mars_stn_due_time();
}
```

[任务链路](/zh/stn)是它的全部 —— 两条连接、一个任务的各个字段、任务怎么结束、长连接要
App 做什么。

## 网络诊断

```c
#include <mars_sdt.h>

MarsSdtIpPort port = { "1.2.3.4", 80 };
MarsSdtHosts longlink[] = { { "default", &port, 1 } };
mars_sdt_set_http_netcheck_cgi("http://example.com/netcheck");
mars_sdt_start_active_check(longlink, 1, NULL, 0, 1 | 2, 10000);
/* 1 | 2 是 NET_CHECK_BASIC | NET_CHECK_LONG：ping 和 dns，然后 tcp。0 是一项都不查。 */
mars_sdt_run_checks(NULL, probe, 1);   /* 一次一个探针，在调用的线程上 */

char buffer[4096];
if (mars_sdt_take_report(buffer, sizeof buffer) >= 0) { send(buffer); }
```

四个探针 —— ping、dns、tcp、http —— 是调用方的：这个移植不持有任何 socket，所以
`mars_sdt_run_checks` 是向交给它的那个 `MarsSdtProbe` 问。

[网络诊断](/zh/sdt)是它的全部：那个模式、那份计划、以及报告的那份 JSON。

## 两部分的错误码

两个头文件里每个返回 `int` 的调用，要么返回 `MARS_STN_OK` / `MARS_SDT_OK`（0），要么返回
负的 `MARS_STN_ERR_*` / `MARS_SDT_ERR_*` —— 一个 `NULL` 参数、缓冲区太小、队列没收下这个
任务、报告装不下 —— 而 `MARS_STN_ERR_PANIC` / `MARS_SDT_ERR_PANIC` 是在边界上捕获到的
Rust panic。C ABI 里不会有任何东西 unwind 进 C；一次报告没装进缓冲区里的诊断会留着它的
结果，所以换一个更大的再来问一次的调用方，拿到的是那份诊断而不是一个空的。
