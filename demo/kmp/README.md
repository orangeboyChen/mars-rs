# The Kotlin Multiplatform demo

A Kotlin/Native executable over `xlog-kmp`: opens an appender in `log/`, writes
six records, flushes, closes.

```bash
./gradlew runDebugExecutableMacosArm64     # on Apple silicon
./gradlew runReleaseExecutableLinuxX64     # on Linux x86-64
```

It needs nothing but a JDK: Gradle 9.8.0 comes down with the wrapper, and the
Kotlin/Native compiler with the build.

## Getting the package

`xlog-kmp` is on GitHub Packages, which answers nothing — not even a 404 —
without a token. So `settings.gradle.kts` reads one out of the environment and
not out of a file:

```bash
GITHUB_ACTOR=<you> GITHUB_TOKEN=<a token with read:packages> ./gradlew runDebugExecutableMacosArm64
```

The way in that needs no token at all is the release's `marsrs-kmp-maven.zip`:
unzip it somewhere and put `maven { url = uri("<dir>") }` where the GitHub
Packages block is.

## What is in the build

Gradle 9.8.0, and the Kotlin DSL throughout. Kotlin 2.4.20 — the newest stable
— and no Android Gradle Plugin: this demo runs as a native executable and builds
no Android artifact, and the plugin that decides which targets exist is Kotlin's
own.

## What to look at

- `src/commonMain` — the part every platform compiles, and the part an app's
  shared module keeps as it is. The `Xlog` it writes through is the same class
  on Android over JNI and everywhere else over the C ABI, and neither is
  visible from here, which is the whole point.
- `src/nativeMain` — `main`, and the only source in the build that is not
  shared: an app replaces it with the platform's own entry point, an `Activity`
  on Android or a `@main` on iOS, and keeps `commonMain` untouched.
- `binaries { executable { entryPoint = ... } }` — what turns a compiled module
  into a program Gradle can run. The same targets are library targets in an
  app.

An app that ships on a phone adds the targets it ships on, one line each —
`androidTarget()`, `iosArm64()`, `iosSimulatorArm64()`, `watchosArm64()`,
`mingwX64()` — and every one of them compiles the same `commonMain`.
