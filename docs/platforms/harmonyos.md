# HarmonyOS

There is no HarmonyOS package, and no ArkTS wrapper: a HarmonyOS app carries the
three `libmars_ffi.so` of a release and reaches the C ABI through NAPI of its own.

```text
marsrs-harmony-<version>.tar.gz
marsrs-harmony-<version>/
    arm64-v8a/libmars_ffi.so       aarch64-unknown-linux-ohos
    armeabi-v7a/libmars_ffi.so     armv7-unknown-linux-ohos
    x86_64/libmars_ffi.so          x86_64-unknown-linux-ohos
    include/mars_xlog.h
```

The same three are what `scripts/build_harmony.sh <dir>` builds from source —
which is what a release runs, and the way to get them out of a commit that has
none. The SDK is the public OpenHarmony one, and the script downloads it when
`OHOS_SDK_HOME` is not set.

## From an app

1. Put `libmars_ffi.so` under the module's `libs/<abi>/` — `arm64-v8a` for a
   real device.
2. Open the appender once, from C, when the app starts:

   ```c
   #include <mars_xlog.h>

   MarsXLogConfig config = {
       .mode = MarsAppenderAsync,
       .log_dir = log_dir,      /* the app's files directory */
       .name_prefix = "marsrs",
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
