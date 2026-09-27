# HarmonyOS

HarmonyOS is reached through the OpenHarmony targets —
`aarch64-unknown-linux-ohos`, `armv7-unknown-linux-ohos` and
`x86_64-unknown-linux-ohos` — and what builds for them today is the C ABI:

```bash
scripts/build_harmony.sh dist/harmony   # <abi>/libmars_ffi.so
```

There is no HarmonyOS package yet, and no ArkTS: a HarmonyOS app would load
`libmars_ffi.so` and call `mars_xlog_*` through NAPI itself. The three targets
are tier 2 with host tools, so a std ships for them and the build is a stable
one; the SDK it needs is the public OpenHarmony one, which the script downloads
when `OHOS_SDK_HOME` is not set.
