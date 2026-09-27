# HarmonyOS

还没有 HarmonyOS 的包，也没有 ArkTS 封装：HarmonyOS 应用要自己为 OpenHarmony target
编译 C ABI，再通过 NAPI 调它。

```bash
scripts/build_harmony.sh dist/harmony   # <abi>/libmars_ffi.so
```

这会构建 `aarch64-unknown-linux-ohos`、`armv7-unknown-linux-ohos`、
`x86_64-unknown-linux-ohos` —— SDK 用公开的 OpenHarmony 那份，没设 `OHOS_SDK_HOME`
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
