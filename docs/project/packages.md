# What a release ships

Every release ships a package per platform; `.github/workflows/release.yml`
builds them when it is run by hand (Actions → Release → Run workflow) with a
version, or with a bump and a channel — `v1.2.3-alpha.1`, `v1.2.3-beta.2`,
`v1.2.3`. Anything with a suffix is published as a GitHub pre-release.

| platform | what the release publishes | page |
|---|---|---|
| Rust | nine crates on crates.io, in dependency order, at the version of the tag | [Rust](/platforms/rust) |
| Apple | `marsrs-xlog.xcframework.zip`, `marsrs-net.xcframework.zip`, and the tag + checksums of both written into `Package.swift` | [SwiftPM](/platforms/swift) |
| Android | two AARs on JitPack over one `libmarsxlog.so` | [Android](/platforms/android) |
| Kotlin Multiplatform | `mars-rs-kmp` / `mars-rs-xlog-kmp` on GitHub Packages, plus `mars-kmp-native.zip` and `mars-kmp-maven.zip` | [Kotlin Multiplatform](/platforms/kotlin-multiplatform) |
| everything with a C FFI | `mars-rs-<version>-<host>.tar.gz` / `.zip` | [The C ABI](/platforms/c-abi) |

## Building the packages

```bash
scripts/build_xcframework.sh 0.1.0 dist   # marsrs-xlog.xcframework.zip, marsrs-net.xcframework.zip
scripts/build_android.sh dist/native      # <abi>/libmarsxlog.so
# the .so of dist/native has to be under android/<module>/libs first
(cd android && ./gradlew :mars-core:assembleRelease :mars-xlog:assembleRelease)
# <kotlin-target>/libmars_ffi.a into kmp/native, plus dist/native's .so into
# kmp/native/android — on macOS for the Apple ones, on Linux for the rest
scripts/build_kmp_native.sh kmp/native
(cd kmp && ./gradlew :mars-core:publishToMavenLocal :mars-xlog:publishToMavenLocal)
```

`kmp` is a Gradle build of its own and not two modules of `android`, because the
two need different Android Gradle Plugins: 8.13 here, where the Kotlin plugin
still owns the `kotlin` extension an Android target needs, and 9 in `../android`.

The scripts that only a workflow runs — resolving the version, installing the
NDK on a runner, waiting for JitPack, rewriting `Package.swift`, naming the
files a commit touched — are in `.github/scripts/`.
