# HarmonyOS

There is no HarmonyOS package yet, and no ArkTS wrapper: a HarmonyOS app builds
the C ABI for the OpenHarmony targets and reaches it through NAPI itself.

```bash
scripts/build_harmony.sh dist/harmony   # <abi>/libmars_ffi.so
```

That builds `aarch64-unknown-linux-ohos`, `armv7-unknown-linux-ohos` and
`x86_64-unknown-linux-ohos` — the SDK is the public OpenHarmony one, which the
script downloads when `OHOS_SDK_HOME` is not set.

## From an app

1. Put `libmars_ffi.so` under the module's `libs/<abi>/` — `arm64-v8a` for a
   real device.
2. Open the appender once, from C, when the app starts:

   ```c
   #include <mars_xlog.h>

   MarsXLogConfig config = {
       .mode = MarsAppenderAsync,
       .log_dir = log_dir,      /* the app's files directory */
       .name_prefix = "Ham",
       .compress_mode = MarsCompressZlib,
   };
   mars_xlog_open(&config);
   mars_xlog_write(MarsLevelInfo, "startup", __FILE__, __func__, __LINE__, "hello");
   ```

3. Wrap those two in the NAPI module of your own — one `napi_value` for
   `open(dir)` and one for `write(level, tag, message)` is all a logger needs —
   and call them from ArkTS.

Everything on [the C ABI page](/platforms/c-abi) applies: the config, the levels,
the instances and the error codes are the same symbols.
