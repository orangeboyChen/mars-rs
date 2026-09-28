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

## The mars half

`libmars_ffi.so` is the whole port and not the logger alone: `build_harmony.sh`
builds `marsrs-ffi` with the `sdt,stn` features on top of the default `xlog`, and
what the script writes next to the library is all three of `mars_xlog.h`,
`mars_sdt.h` and `mars_stn.h`. So a HarmonyOS app can run a task, or a
diagnosis, through the same NAPI shim it writes for the logger:

```c
#include <mars_stn.h>

mars_stn_set_app(NULL, ask);          /* the eighteen questions, as one callback */
mars_stn_start_task(&task);

long long due = mars_stn_due_time();
while (due >= 0) {                    /* no threads in the port: the app drains the queues */
    mars_stn_run_pending();
    due = mars_stn_due_time();
}
```

There is no ArkTS wrapper for either half, and the two pages say what each one
asks of an app: [the task pipeline](/stn) and [the network diagnosis](/sdt).
Nothing there is HarmonyOS's own — it is the same C ABI
[its page](/platforms/c-abi) publishes for Linux, macOS and Windows, and the one
`MarsRSNet`, the Android AARs and `marsrs-kmp` are written over.
