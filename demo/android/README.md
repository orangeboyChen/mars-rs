# The Android demo

An app over the AAR: one `Activity`, one button, six records into a `.xlog`
file in the app's own directory.

```bash
./gradlew installDebug      # or :app:assembleDebug, for the APK alone
```

It needs a JDK 17 and an Android SDK, and it finds the SDK the way Gradle does —
`ANDROID_HOME`, or a `local.properties` with `sdk.dir` in it. There is
deliberately no `sdk.dir` in `gradle.properties`: it is the one line of a Gradle
build that is different on every machine.

## What is in the build

- Gradle 9.8.0, and the Kotlin DSL throughout: `.gradle.kts` for the scripts
  and `gradle/libs.versions.toml` for the coordinates.
- The Android Gradle Plugin 9.4.1, which compiles Kotlin itself — there is no
  `org.jetbrains.kotlin.android` plugin here, and applying one is what a build
  on AGP 9 gets told not to do.
- `minSdk 21`, which is the floor the AAR's own is at.

The dependency is JitPack's, and the coordinate is JitPack's spelling of the
repository rather than the Maven group the documentation quotes:
`com.github.orangeboyChen.mars-rs:xlog`. A JitPack group of one's own has to be
claimed before it resolves, and `io.github.orangeboychen.marsrs` is not — that
one answers 401, which looks exactly like a version that does not exist. The
release the demo is written against is `0.1.0-alpha.3`, which is what the
documentation quotes; `0.1.0` is not out yet. Both of those are one line of
`gradle/libs.versions.toml`.

## What to look at

- `Xlog.open(config, applicationContext)` — the same call it is in Kotlin
  Multiplatform, in Swift and in Dart, and the one place Android asks for a
  `Context`: it is what gives the app its directories.
- `xlog.v`, `d`, `i`, `w`, `e`, `f` — one method per level, and
  `xlog.log(level, tag, message)` for the call whose level is not known until it
  runs.
- `xlog.flushNow()` after the writes and `xlog.close()` in `onDestroy` — the
  two calls an app makes so that the last records are not lost when the process
  goes. `xlog.requestFlush()` is the third drain: it asks for the same one and
  returns at once, without ever saying when it is over.
- `BuildConfig.DEBUG` for `consoleLogEnabled` — off in a build that ships.

The APK carries `libmarsrsxlog.so` for `arm64-v8a`, `armeabi-v7a` and `x86_64`,
which is the whole of what the AAR brings: no NDK in this build, and no Rust
toolchain either.
