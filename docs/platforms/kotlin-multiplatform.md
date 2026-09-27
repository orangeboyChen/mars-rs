# Kotlin Multiplatform

One dependency in `commonMain`, and each platform compiles its own half of it:
the `androidMain` of the module talks to the JNI bridge of `marsrs-jni`, the
`nativeMain` of it talks to the C ABI of `marsrs-ffi` through cinterop.

```kotlin
// settings.gradle.kts
maven {
    url = uri("https://maven.pkg.github.com/orangeboyChen/mars-rs")
    credentials { username = "<user>"; password = "<token with read:packages>" }
}

// build.gradle.kts of the shared module
implementation("io.github.orangeboychen:mars-rs-kmp:0.1.0")       // the whole port
implementation("io.github.orangeboychen:mars-rs-xlog-kmp:0.1.0")  // xlog alone
```

| artifact | what is in it |
|---|---|
| `mars-rs-kmp` | `mars-rs-xlog-kmp`, re-exported — the pair `mars-rs`/`mars-rs-xlog` are on Android and `marsrs`/`marsrs-xlog` are in Swift. Its own declarations are six `typealias`es under `io.github.orangeboychen.marsrs`, one per name below: `marsrs-ffi` is an xlog C ABI today, so the whole port and its logging half are the same module. Taking this coordinate is what lets STN and SDT arrive without a rename. |
| `mars-rs-xlog-kmp` | `Xlog`, `XlogConfig`, `LogLevel` and the `Log` facade, over the JNI bridge on Android and over the C ABI of `marsrs-ffi` everywhere else |

Fourteen targets: Android — the AAR carries `libmarsxlog.so` for `arm64-v8a`,
`armeabi-v7a` and `x86_64` — plus `iosArm64`, `iosX64`, `iosSimulatorArm64`,
`macosX64`, `macosArm64`, `watchosArm64`, `watchosDeviceArm64`,
`watchosSimulatorArm64`, `tvosArm64`, `tvosSimulatorArm64`, `linuxX64`,
`linuxArm64` and `mingwX64`. The two Kotlin targets missing from the list are
the x86_64 simulators, `watchosX64` and `tvosX64`, and both are missing because
Rust has no triple for them: `x86_64-apple-watchos-sim` and `x86_64-apple-tvos`
are not targets `rustup` knows.

Nothing is compiled from Rust when the Kotlin module is built. Neither a
consumer nor a CI host has the toolchain for thirteen triples, and an Apple
archive cannot be cross-compiled from Linux at all — so the release builds them
(`scripts/build_kmp_native.sh`) and publishes `mars-kmp-native.zip`, one
`libmars_ffi.a` per Kotlin target plus the `.so` files of the JNI bridge, and
`mars-kmp-maven.zip`, which is the repository itself. An app that would rather
not authenticate to GitHub Packages takes the second: unzip it and add
`maven { url = uri("<dir>") }`.

The AARs on JitPack stay the packaging for an app that is Android only; this one
is for a `commonMain` that compiles for more than one platform.
