# HarmonyOS

还没有 HarmonyOS 的包，也没有 ArkTS 封装：HarmonyOS 应用带上 release 里那三个
`libmars_ffi.so`，再通过自己的 NAPI 调 C ABI。

```text
marsrs-harmony-<version>.tar.gz
marsrs-harmony-<version>/
    arm64-v8a/libmars_ffi.so       aarch64-unknown-linux-ohos
    armeabi-v7a/libmars_ffi.so     armv7-unknown-linux-ohos
    x86_64/libmars_ffi.so          x86_64-unknown-linux-ohos
    include/mars_xlog.h
```

`scripts/build_harmony.sh <dir>` 从源码构建的也是这三个 —— release 就是跑它，
没有 release 的 commit 也用它。SDK 用公开的 OpenHarmony 那份，没设 `OHOS_SDK_HOME`
时脚本自己下载。

## 应用里怎么用

1. 把 `libmars_ffi.so` 放到模块的 `libs/<abi>/` 下 —— 真机是 `arm64-v8a`。
2. 在 C 里、App 启动时打开一次 appender：

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

3. 在自己的 NAPI 模块里包一下 —— 一个 `open(dir)` 和一个
   `write(level, tag, message)`，日志要的就这么点 —— 再从 ArkTS 调它们。

[C ABI](/zh/platforms/c-abi)那页都适用：配置、级别、实例和错误码是同一批符号。
